import { Component, OnInit, inject, signal } from '@angular/core';
import { Router } from '@angular/router';
import { MatButtonModule } from '@angular/material/button';
import { MatCardModule } from '@angular/material/card';
import { MatDialog } from '@angular/material/dialog';
import { MatIconModule } from '@angular/material/icon';
import { MatProgressBarModule } from '@angular/material/progress-bar';
import { MatSnackBar } from '@angular/material/snack-bar';
import { MatTooltipModule } from '@angular/material/tooltip';
import { ApiService, ApiFailure } from '../../api/api.service';
import { JobsService } from '../../api/jobs.service';
import { WorkspaceService } from '../../core/workspace.service';
import { PromptDialogComponent, PromptData, PromptResult } from '../../shared/prompt-dialog.component';
import { TargetDialogComponent } from './target-dialog.component';
import type { DeployTarget } from '../../api/generated';

/** Remote machines: deploy targets (build here, activate there) and the root SSH key for remote builders. */
@Component({
  selector: 'app-remote',
  imports: [MatButtonModule, MatCardModule, MatIconModule, MatProgressBarModule, MatTooltipModule],
  styles: [`
    .head { display: flex; align-items: center; gap: 12px; margin-bottom: 12px; }
    h2 { margin: 0; font-weight: 400; }
    .cards { display: grid; grid-template-columns: repeat(auto-fit, minmax(380px, 1fr)); gap: 16px; }
    .row { display: flex; align-items: center; gap: 10px; padding: 6px 0; border-bottom: 1px solid var(--mat-sys-surface-container); }
    .row:last-child { border-bottom: 0; }
    .row .t { flex: 1; min-width: 0; }
    .row .d { font-size: .82em; color: var(--mat-sys-on-surface-variant); overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
    .note { font-size: .85em; color: var(--mat-sys-on-surface-variant); margin-top: 8px; line-height: 1.5; }
    .errors { padding: 8px 12px; border-radius: 8px; background: var(--mat-sys-error-container); color: var(--mat-sys-on-error-container); margin-bottom: 12px; font-size: .9em; }
    ol { margin: 6px 0; padding-left: 18px; line-height: 1.5; }
    pre.key { white-space: pre-wrap; word-break: break-all; background: var(--mat-sys-surface-container); padding: 8px 10px; border-radius: 6px; font-size: .8em; }
    mat-card-actions { gap: 6px; flex-wrap: wrap; }
  `],
  template: `
    @if (busy()) { <mat-progress-bar mode="indeterminate" /> }
    <div class="head"><h2>Remote machines</h2></div>
    @if (error()) { <div class="errors">{{ error() }}</div> }

    <div class="cards">
      <mat-card appearance="outlined">
        <mat-card-header><mat-card-title>Deploy targets</mat-card-title><mat-card-subtitle>build here, activate there (nixos-rebuild --target-host)</mat-card-subtitle></mat-card-header>
        <mat-card-content>
          @for (t of targets(); track t.name) {
            <div class="row">
              <mat-icon>dns</mat-icon>
              <div class="t"><b>{{ t.name }}</b><div class="d mono">{{ t.ssh_target }}{{ t.flake_host ? ' · #' + t.flake_host : '' }}{{ t.build_host ? ' · build on ' + t.build_host : '' }}{{ t.use_remote_sudo ? ' · remote sudo' : '' }}</div></div>
              <button mat-icon-button matTooltip="Set up key access (install this app's SSH key on the target using its password once)" (click)="setupKey(t)"><mat-icon>vpn_key</mat-icon></button>
              <button mat-icon-button matTooltip="Test the SSH connection" (click)="test(t)"><mat-icon>network_check</mat-icon></button>
              <button mat-icon-button matTooltip="Copy this target's /etc/nixos/hardware-configuration.nix into the working copy (replaces the file there)" (click)="pullHardware(t)"><mat-icon>download</mat-icon></button>
              <button mat-icon-button matTooltip="Edit" (click)="edit(t)"><mat-icon>edit</mat-icon></button>
              <button mat-icon-button matTooltip="Remove" (click)="remove(t)"><mat-icon>delete</mat-icon></button>
            </div>
          } @empty { <div class="muted">No targets yet. Add one, then pick it under "Deploy to" in the Build dialog.</div> }
          <div class="note">The configuration is evaluated and built on this machine from the working copy, then only the result is copied to the target over SSH and activated there. Deployments log in with an SSH key: use the <mat-icon style="font-size:16px;width:16px;height:16px;vertical-align:middle">vpn_key</mat-icon> button to set that up with the target's password once. When your user is not yet trusted by nix on the target, or sudo there needs a password, the build dialog asks for the target's sudo password and grants trust temporarily for that deployment. By default the target's own <span class="mono">/etc/nixos/hardware-configuration.nix</span> is used for the build and the working copy's file is ignored; the build dialog has a checkbox to use the working copy's file instead. The <mat-icon style="font-size:16px;width:16px;height:16px;vertical-align:middle">download</mat-icon> button copies the target's file into the working copy (kept as is when the folder is opened again, until you edit it).</div>
        </mat-card-content>
        <mat-card-actions><button mat-flat-button (click)="add()"><mat-icon>add</mat-icon> Add target</button></mat-card-actions>
      </mat-card>

      <mat-card appearance="outlined">
        <mat-card-header><mat-card-title>Remote builders</mat-card-title><mat-card-subtitle>let a faster machine do the compiling for this one</mat-card-subtitle></mat-card-header>
        <mat-card-content>
          <ol class="note">
            <li>Here: generate root's SSH key (the nix daemon connects to builders as root). Optionally give the builder's address so its host key is trusted right away.</li>
            <li>On the builder: run the wizard recipe <b>Let this machine build for others</b> and paste the public key shown below.</li>
            <li>Here: run the recipe <b>Use another machine as a builder</b> with the builder's address.</li>
          </ol>
          @if (rootKey()) { <div class="note">Root's public key on this machine:</div><pre class="key mono">{{ rootKey() }}</pre> }
        </mat-card-content>
        <mat-card-actions>
          <button mat-flat-button (click)="generateKey()"><mat-icon>vpn_key</mat-icon> {{ rootKey() ? 'Show / refresh root key' : 'Generate root SSH key' }}</button>
          @if (rootKey()) { <button mat-stroked-button (click)="copyKey()"><mat-icon>content_copy</mat-icon> Copy key</button> }
        </mat-card-actions>
      </mat-card>
    </div>
  `,
})
export class RemoteComponent implements OnInit {
  private api = inject(ApiService);
  private jobs = inject(JobsService);
  private dialog = inject(MatDialog);
  private snack = inject(MatSnackBar);
  private router = inject(Router);
  private ws = inject(WorkspaceService);

  targets = signal<DeployTarget[]>([]);
  busy = signal(false);
  error = signal<string | null>(null);
  rootKey = signal<string | null>(null);

  ngOnInit() { this.api.targets().subscribe(t => this.targets.set(t)); }

  private save(next: DeployTarget[], done: string) {
    this.busy.set(true);
    this.api.saveTargets(next).subscribe({
      next: t => { this.targets.set(t); this.busy.set(false); this.snack.open(done, undefined, { duration: 2500 }); },
      error: (e: ApiFailure) => { this.busy.set(false); this.error.set(e.message); },
    });
  }
  add() {
    this.dialog.open<TargetDialogComponent, { target: DeployTarget | null }, DeployTarget>(TargetDialogComponent, { width: '560px', data: { target: null } })
      .afterClosed().subscribe(t => { if (t) this.save([...this.targets().filter(x => x.name !== t.name), t], `Saved ${t.name}`); });
  }
  edit(t: DeployTarget) {
    this.dialog.open<TargetDialogComponent, { target: DeployTarget }, DeployTarget>(TargetDialogComponent, { width: '560px', data: { target: t } })
      .afterClosed().subscribe(nt => { if (nt) this.save(this.targets().map(x => (x.name === t.name ? nt : x)), `Saved ${nt.name}`); });
  }
  remove(t: DeployTarget) { this.save(this.targets().filter(x => x.name !== t.name), `Removed ${t.name}`); }
  setupKey(t: DeployTarget) {
    this.dialog.open<PromptDialogComponent, PromptData, PromptResult>(PromptDialogComponent, { width: '560px', data: {
      title: `Set up key access to ${t.name}`,
      message: `Logs in to ${t.ssh_target} once with a password and installs this app's SSH key there, so later deployments need no password. The password is used for this one connection and never stored. If a key already works, nothing is changed.`,
      label: `SSH password for ${t.ssh_target}`, password: true, okLabel: 'Set up',
    } }).afterClosed().subscribe(r => {
      if (!r) return;
      this.api.setupKey(t.name, r.value).subscribe({
        next: job => this.router.navigate(['/w', 'jobs', job.id], { queryParamsHandling: 'merge' }),
        error: (e: ApiFailure) => this.error.set(e.message),
      });
    });
  }

  pullHardware(t: DeployTarget) {
    const path = this.ws.path();
    if (!path) { this.snack.open('Open a working folder first', 'OK', { duration: 4000 }); return; }
    this.dialog.open<PromptDialogComponent, PromptData, PromptResult>(PromptDialogComponent, { width: '560px', data: {
      title: `Copy hardware-configuration.nix from ${t.name}`,
      message: `Reads /etc/nixos/hardware-configuration.nix on ${t.ssh_target} (or generates one there with nixos-generate-config if missing) and replaces the working copy's hardware-configuration.nix with it. The copy is remembered, so opening this folder again keeps it instead of this machine's file.`,
      okLabel: 'Copy from target',
    } }).afterClosed().subscribe(r => {
      if (!r) return;
      this.busy.set(true);
      this.api.pullHardware(t.name, path).subscribe({
        next: res => {
          this.busy.set(false);
          this.snack.open(`${res.rel_path} replaced with the file from ${res.target}${res.source === 'generate' ? ' (generated there with nixos-generate-config)' : ''}`, 'OK', { duration: 8000 });
          this.ws.reload();
        },
        error: (e: ApiFailure) => { this.busy.set(false); this.error.set(e.message); },
      });
    });
  }

  test(t: DeployTarget) {
    this.api.remoteTest(t.ssh_target).subscribe({
      next: job => this.router.navigate(['/w', 'jobs', job.id], { queryParamsHandling: 'merge' }),
      error: (e: ApiFailure) => this.error.set(e.message),
    });
  }

  generateKey() {
    this.dialog.open<PromptDialogComponent, PromptData, PromptResult>(PromptDialogComponent, { width: '520px', data: {
      title: 'Root SSH key for builders',
      message: 'Creates /root/.ssh/id_ed25519 if missing and shows its public key. Needs your sudo password.',
      label: 'Sudo password', password: true, okLabel: 'Generate',
      checkbox: { label: 'Also record a builder\'s host key (asks for its address next)', checked: false },
    } }).afterClosed().subscribe(r => {
      if (!r) return;
      const run = (host: string | null) => {
        this.busy.set(true);
        this.api.rootKey(r.value, host).subscribe({
          next: job => this.waitForKey(job.id),
          error: (e: ApiFailure) => { this.busy.set(false); this.error.set(e.message); },
        });
      };
      if (r.checked) {
        this.dialog.open<PromptDialogComponent, PromptData, PromptResult>(PromptDialogComponent, { width: '480px', data: { title: 'Builder address', label: 'Hostname or IP', placeholder: 'desktop1-nixos', okLabel: 'Continue' } })
          .afterClosed().subscribe(h => run(h?.value ?? null));
      } else run(null);
    });
  }
  private waitForKey(jobId: string) {
    const tick = () => {
      this.api.job(jobId).subscribe(d => {
        if (d.info.status === 'running' || d.info.status === 'pending') { setTimeout(tick, 1000); return; }
        this.busy.set(false);
        const line = d.log.find(l => l.text.startsWith('PUBKEY '));
        if (d.info.status === 'succeeded' && line) { this.rootKey.set(line.text.slice(7).trim()); }
        else { this.error.set(d.info.error ?? 'could not read the key'); this.router.navigate(['/w', 'jobs', jobId], { queryParamsHandling: 'merge' }); }
      });
    };
    tick();
  }
  copyKey() {
    const k = this.rootKey();
    if (!k) return;
    navigator.clipboard?.writeText(k).then(() => this.snack.open('Copied', undefined, { duration: 2000 }), () => this.snack.open('Copy failed; select the key and copy it by hand', 'Dismiss', { duration: 5000 }));
  }
}
