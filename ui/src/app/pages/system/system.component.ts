import { Component, OnInit, inject, signal } from '@angular/core';
import { Router } from '@angular/router';
import { MatButtonModule } from '@angular/material/button';
import { MatCardModule } from '@angular/material/card';
import { MatDialog } from '@angular/material/dialog';
import { MatIconModule } from '@angular/material/icon';
import { MatProgressBarModule } from '@angular/material/progress-bar';
import { MatTooltipModule } from '@angular/material/tooltip';
import { TooltipAriaDirective } from '../../shared/tooltip-aria.directive';
import { ApiService, ApiFailure } from '../../api/api.service';
import { WorkspaceService } from '../../core/workspace.service';
import { PromptDialogComponent, PromptData, PromptResult } from '../../shared/prompt-dialog.component';
import type { DeployTarget, Generation, OtherSystem, RemoteCheck, StoreUsage } from '../../api/generated';
import { FormsModule } from '@angular/forms';
import { MatFormFieldModule } from '@angular/material/form-field';
import { MatSelectModule } from '@angular/material/select';

/** The running system: generations (the undo story), rollback, store usage and garbage collection. */
@Component({
  selector: 'app-system',
  imports: [FormsModule, MatFormFieldModule, MatSelectModule, MatButtonModule, MatCardModule, MatIconModule, MatProgressBarModule, MatTooltipModule, TooltipAriaDirective],
  styles: [`
    .head { display: flex; align-items: center; gap: 12px; margin-bottom: 12px; }
    h2 { margin: 0; font-weight: 400; }
    .cards { display: grid; grid-template-columns: minmax(360px, 2fr) minmax(300px, 1fr); gap: 16px; align-items: start; }
    @media (max-width: 1000px) { .cards { grid-template-columns: 1fr; } }
    .gen { display: flex; align-items: center; gap: 12px; padding: 8px 0; border-bottom: 1px solid var(--mat-sys-surface-container); }
    .gen:last-child { border-bottom: 0; }
    .gen .n { font-family: monospace; min-width: 3em; text-align: right; }
    .gen .d { flex: 1; min-width: 0; }
    .gen .meta { font-size: .85em; color: var(--mat-sys-on-surface-variant); }
    .gen.current { background: var(--mat-sys-surface-container); border-radius: 8px; padding: 8px; }
    .bar { height: 10px; border-radius: 5px; background: var(--mat-sys-surface-container-high); overflow: hidden; margin: 8px 0; }
    .bar div { height: 100%; background: var(--mat-sys-primary); }
    .errors { padding: 8px 12px; border-radius: 8px; background: var(--mat-sys-error-container); color: var(--mat-sys-on-error-container); margin-bottom: 12px; font-size: .9em; }
    .meta { font-size: .85em; color: var(--mat-sys-on-surface-variant); line-height: 1.6; }
  `],
  template: `
    <div class="head">
      <h2>System</h2>
      @if (targets().length) {
        <mat-form-field appearance="outline" subscriptSizing="dynamic" style="width:260px; margin:0 0 0 8px">
          <mat-label>Machine</mat-label>
          <mat-select [ngModel]="machine()" (ngModelChange)="selectMachine($event)">
            <mat-option value="">This machine</mat-option>
            @for (t of targets(); track t.name) { <mat-option [value]="t.name">{{ t.name }} <span class="mono muted">{{ t.ssh_target }}</span></mat-option> }
          </mat-select>
        </mat-form-field>
      }
      <span class="meta">Every build creates a generation; older ones stay available to boot or roll back to until they are deleted.</span>
      <span class="spacer"></span>
      <button mat-icon-button (click)="load()" matTooltip="Reload"><mat-icon>refresh</mat-icon></button>
    </div>
    @if (target(); as t) {
      <div class="note" style="display:flex; gap:8px; align-items:flex-start; padding:8px 12px; border-radius:8px; background:var(--mat-sys-surface-container-high); margin-bottom:12px; font-size:.9em"><mat-icon>lan</mat-icon>
        <span>Showing <b>{{ remoteHost() ?? t.name }}</b> (<span class="mono">{{ t.ssh_target }}</span>) over SSH. Rolling back and deleting generations run there as root:
          {{ check()?.is_root ? 'the SSH user is root, no password needed.' : check()?.passwordless_sudo ? 'the SSH user has passwordless sudo, no password needed.' : 'you will be asked for the sudo password of the SSH user on that machine.' }}</span>
      </div>
    }
    @if (busy()) { <mat-progress-bar mode="indeterminate" style="margin-bottom:8px" /> }
    @if (error()) { <div class="errors">{{ error() }}</div> }
    <div class="cards">
      <mat-card appearance="outlined">
        <mat-card-header><mat-card-title>Generations</mat-card-title><mat-card-subtitle>{{ gens().length }} on {{ target() ? (remoteHost() ?? target()!.name) : 'this machine' }} · the current one is highlighted</mat-card-subtitle></mat-card-header>
        <mat-card-content>
          @for (g of gens(); track g.number) {
            <div class="gen" [class.current]="g.current">
              <span class="n">#{{ g.number }}</span>
              <div class="d">
                <div><b>{{ g.date }}</b>@if (g.current) { <span class="tag" style="margin-left:8px">current</span> }</div>
                <div class="meta">NixOS {{ g.nixos_version ?? '?' }}@if (g.kernel) { · kernel {{ g.kernel }} }</div>
              </div>
              @if (!g.current) {
                <button mat-stroked-button (click)="rollback(g)" matTooltip="Make this generation the active one now and the default at boot {{ ws.isRoot() ? '' : ' (needs your sudo password)' }}"><mat-icon>history</mat-icon> Roll back to this</button>
              }
            </div>
          } @empty {
            <div class="meta">No generations found under /nix/var/nix/profiles.</div>
          }
          <div class="meta" style="margin-top:12px">Rolling back activates an earlier generation without rebuilding anything. The boot menu lists the same generations, so a machine that does not boot can be recovered from there too.</div>
        </mat-card-content>
      </mat-card>

      <mat-card appearance="outlined">
        <mat-card-header><mat-card-title>Disk space</mat-card-title><mat-card-subtitle>the filesystem that holds /nix/store</mat-card-subtitle></mat-card-header>
        <mat-card-content>
          @if (store(); as s) {
            <div class="bar"><div [style.width.%]="s.size ? (100 * s.used / s.size) : 0"></div></div>
            <div class="meta"><b>{{ fmt(s.used) }}</b> used of {{ fmt(s.size) }} · {{ fmt(s.avail) }} free</div>
          }
          <div class="meta" style="margin:12px 0 8px">Old generations keep their packages in the store. Deleting them frees space; the current generation is never touched.</div>
          <div style="display:flex; gap:8px; flex-wrap:wrap">
            <button mat-flat-button (click)="gc(-1)" matTooltip="nix-collect-garbage without deleting any generation: frees packages that nothing references any more, such as leftover builds (needs the sudo password)"><mat-icon>cleaning_services</mat-icon> Collect garbage</button>
            <button mat-stroked-button (click)="gc(30)" matTooltip="nix-collect-garbage --delete-older-than 30d (needs your sudo password)"><mat-icon>delete_sweep</mat-icon> Delete older than 30 days</button>
            <button mat-stroked-button (click)="gc(0)" matTooltip="nix-collect-garbage -d: deletes every generation but the current one (needs your sudo password)"><mat-icon>delete_forever</mat-icon> Delete all old generations</button>
          </div>
        </mat-card-content>
      </mat-card>
        <mat-card appearance="outlined" style="grid-column: 1 / -1">
          <mat-card-header><mat-card-title>Builds for other machines</mat-card-title>
            <mat-card-subtitle>system closures in {{ target() ? 'the store of ' + (remoteHost() ?? target()!.name) : 'this store' }} that none of its generations uses — left behind by "build here, deploy there" or by builds for other hosts</mat-card-subtitle></mat-card-header>
          <mat-card-content>
            @if (othersLoading()) { <mat-progress-bar mode="indeterminate" /> }
            @else if (!others().length) { <div class="meta">None found. Everything in the store belongs to a generation of {{ target() ? 'that machine' : 'this machine' }}.</div> }
            @for (o of others(); track o.path) {
              <div class="gen">
                <mat-icon class="muted">dns</mat-icon>
                <div class="d">
                  <div><b>{{ o.host }}</b> <span class="meta">NixOS {{ o.nixos_version }}</span></div>
                  <div class="meta"><b>{{ fmt(o.exclusive_size) }}</b> used only by this build · {{ fmt(o.closure_size) }} whole closure · <span class="mono" style="font-size:.9em">{{ o.path.slice(11, 19) }}…</span></div>
                  @if (o.roots.length) { <div class="meta">kept alive by {{ o.roots.join(', ') }} — remove that link first</div> }
                </div>
                <button mat-stroked-button (click)="deleteOther([o])" [disabled]="o.roots.length > 0" [matTooltip]="'nix-store --delete, then a garbage collection' + (target() ? ' on the target' : '') + ' (needs the sudo password)'"><mat-icon>delete</mat-icon> Delete</button>
              </div>
            }
            @if (deletable().length > 1) {
              <div style="margin-top:10px"><button mat-stroked-button (click)="deleteOther(deletable())"><mat-icon>delete_sweep</mat-icon> Delete all {{ deletable().length }}</button></div>
            }
            <div class="meta" style="margin-top:12px">"Used only by this build" is measured against this machine's generations; builds of the same host share most of their content with each other, so deleting several frees less than the sum. Deleting removes the system closure; the garbage collection that follows frees the packages only it used. Packages shared with this machine's generations stay. The next deployment to that machine simply rebuilds or re-downloads what it needs.</div>
          </mat-card-content>
        </mat-card>
    </div>
  `,
})
export class SystemComponent implements OnInit {
  private api = inject(ApiService);
  private dialog = inject(MatDialog);
  private router = inject(Router);
  ws = inject(WorkspaceService);
  gens = signal<Generation[]>([]);
  store = signal<StoreUsage | null>(null);
  busy = signal(false);
  error = signal<string | null>(null);

  targets = signal<DeployTarget[]>([]);
  machine = signal('');
  check = signal<RemoteCheck | null>(null);
  remoteHost = signal<string | null>(null);
  target(): DeployTarget | null { return this.targets().find(t => t.name === this.machine()) ?? null; }

  others = signal<OtherSystem[]>([]);
  othersLoading = signal(false);
  deletable() { return this.others().filter(o => !o.roots.length); }

  ngOnInit() {
    this.api.targets().subscribe({ next: t => this.targets.set(t), error: () => {} });
    this.load();
  }

  loadOthers() {
    this.othersLoading.set(true);
    this.others.set([]);
    const t = this.target();
    const call = t ? this.api.systemRemoteOtherSystems(t.name) : this.api.systemOtherSystems();
    call.subscribe({ next: o => { this.others.set(o); this.othersLoading.set(false); }, error: (e: ApiFailure) => { this.othersLoading.set(false); this.error.set(e.message); } });
  }

  deleteOther(list: OtherSystem[]) {
    const total = list.reduce((a, o) => a + Number(o.exclusive_size), 0);
    const t = this.target();
    const where = t ? (this.remoteHost() ?? t.name) : 'this machine';
    const needPw = t ? this.needsRemotePassword() : !this.ws.isRoot();
    this.dialog.open<PromptDialogComponent, PromptData, PromptResult>(PromptDialogComponent, { width: '560px', data: {
      title: (list.length === 1 ? `Delete the build for ${list[0].host}` : `Delete ${list.length} builds for other machines`) + ` on ${where}?`,
      message: `${list.map(o => o.host + ' (' + o.nixos_version + ')').join(', ')} — frees about ${this.fmt(total)} once garbage is collected. Nothing on those machines changes; only the store on ${where} loses the copy.`,
      label: needPw ? (t ? `sudo password on ${where}` : 'Your sudo password') : undefined, password: true, okLabel: 'Delete', danger: true,
      checkbox: { label: 'Collect garbage afterwards (frees the dependencies too; can take a while)', checked: true },
    } }).afterClosed().subscribe(r => {
      if (!r || (needPw && !r.value)) return;
      const call = t ? this.api.systemRemoteDeletePaths(t.name, list.map(o => o.path), r.value || null, r.checked) : this.api.systemDeletePaths(list.map(o => o.path), r.value, r.checked);
      call.subscribe({
        next: job => this.router.navigate(['/w', 'jobs', job.id], { queryParamsHandling: 'merge' }),
        error: (e: ApiFailure) => this.error.set(e.message),
      });
    });
  }

  selectMachine(name: string) {
    this.machine.set(name);
    this.check.set(null);
    this.remoteHost.set(null);
    this.load();
    const t = this.target();
    if (t) this.api.remoteCheck(t.name).subscribe({ next: c => this.check.set(c), error: () => {} });
  }

  load() {
    this.busy.set(true);
    this.error.set(null);
    const t = this.target();
    if (t) {
      this.gens.set([]);
      this.store.set(null);
      this.api.systemRemote(t.name).subscribe({
        next: r => { this.gens.set(r.generations); this.store.set(r.store); this.remoteHost.set(r.hostname); this.busy.set(false); },
        error: (e: ApiFailure) => { this.error.set(e.message); this.busy.set(false); },
      });
      this.loadOthers();
      return;
    }
    this.api.generations().subscribe({ next: g => { this.gens.set(g); this.busy.set(false); }, error: (e: ApiFailure) => { this.error.set(e.message); this.busy.set(false); } });
    this.api.systemStore().subscribe({ next: s => this.store.set(s), error: () => {} });
    this.loadOthers();
  }

  /** Root or passwordless sudo on the target: no password field in the confirmation. */
  private needsRemotePassword(): boolean { const c = this.check(); return !c || (!c.is_root && c.passwordless_sudo !== true); }

  fmt(b: number): string {
    const u = ['B', 'KB', 'MB', 'GB', 'TB'];
    let i = 0; let v = b;
    while (v >= 1024 && i < u.length - 1) { v /= 1024; i++; }
    return `${v.toFixed(i >= 3 ? 1 : 0)} ${u[i]}`;
  }

  rollback(g: Generation) {
    const t = this.target();
    const where = t ? (this.remoteHost() ?? t.name) : 'this machine';
    const needPw = t ? this.needsRemotePassword() : !this.ws.isRoot();
    this.dialog.open<PromptDialogComponent, PromptData, PromptResult>(PromptDialogComponent, { width: '520px', data: {
      title: `Roll back ${where} to generation #${g.number}?`,
      message: `${g.date} · NixOS ${g.nixos_version ?? '?'}. ${where} switches to that generation now (services restart as needed) and it becomes the default at boot. Your working folder is not changed, so the next Build goes back to your current configuration.`,
      label: needPw ? (t ? `sudo password on ${where}` : 'Your sudo password') : undefined, password: true, okLabel: 'Roll back', danger: true,
    } }).afterClosed().subscribe(r => {
      if (!r || (needPw && !r.value)) return;
      const call = t ? this.api.systemRemoteRollback(t.name, g.number, r.value || null) : this.api.systemRollback(g.number, r.value);
      call.subscribe({
        next: job => this.router.navigate(['/w', 'jobs', job.id], { queryParamsHandling: 'merge' }),
        error: (e: ApiFailure) => this.error.set(e.message),
      });
    });
  }

  /** days: -1 = keep every generation (plain collection), 0 = all old generations, n = older than n days. */
  gc(days: number) {
    const t = this.target();
    const where = t ? (this.remoteHost() ?? t.name) : 'this machine';
    const needPw = t ? this.needsRemotePassword() : !this.ws.isRoot();
    const keep = days < 0;
    this.dialog.open<PromptDialogComponent, PromptData, PromptResult>(PromptDialogComponent, { width: '520px', data: {
      title: keep ? `Collect garbage on ${where}?` : (days === 0 ? 'Delete all old generations' : `Delete generations older than ${days} days`) + ` on ${where}?`,
      message: keep ? 'Every generation stays, so rollback keeps working. Only store paths that nothing references any more are deleted: leftover builds, old package versions no generation uses. Safe; can take a few minutes on a large store.' : days === 0 ? 'Only the current generation stays; you will not be able to roll back afterwards. Unreferenced packages are removed from the store.' : `Generations older than ${days} days are removed, then unreferenced packages are deleted from the store. Newer generations stay available for rollback.`,
      label: needPw ? (t ? `sudo password on ${where}` : 'Your sudo password') : undefined, password: true, okLabel: keep ? 'Collect' : 'Delete', danger: !keep,
    } }).afterClosed().subscribe(r => {
      if (!r || (needPw && !r.value)) return;
      const call = t ? this.api.systemRemoteGc(t.name, Math.max(days, 0), r.value || null, keep) : this.api.systemGc(Math.max(days, 0), r.value, keep);
      call.subscribe({
        next: job => this.router.navigate(['/w', 'jobs', job.id], { queryParamsHandling: 'merge' }),
        error: (e: ApiFailure) => this.error.set(e.message),
      });
    });
  }
}
