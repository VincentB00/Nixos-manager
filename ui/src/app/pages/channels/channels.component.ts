import { TooltipAriaDirective } from '../../shared/tooltip-aria.directive';
import { RouterLink } from '@angular/router';
import { Component, OnInit, inject, signal } from '@angular/core';
import { Router } from '@angular/router';
import { MatButtonModule } from '@angular/material/button';
import { MatCardModule } from '@angular/material/card';
import { MatDialog } from '@angular/material/dialog';
import { MatIconModule } from '@angular/material/icon';
import { MatProgressBarModule } from '@angular/material/progress-bar';
import { MatTooltipModule } from '@angular/material/tooltip';
import { ApiService, ApiFailure } from '../../api/api.service';
import { WorkspaceService } from '../../core/workspace.service';
import { PromptDialogComponent, PromptData, PromptResult } from '../../shared/prompt-dialog.component';
import { ConvertFlakeDialogComponent } from '../flake/convert-flake-dialog.component';
import type { ChannelAction, ChannelEntry, ChannelScope, ChannelsInfo } from '../../api/generated';

/** Classic configs: view and manage nix channels, or switch to flakes. */
@Component({
  selector: 'app-channels',
  imports: [RouterLink, MatButtonModule, MatCardModule, MatIconModule, MatProgressBarModule, MatTooltipModule, TooltipAriaDirective],
  styles: [`
    .head { display: flex; align-items: center; gap: 12px; margin-bottom: 12px; }
    h2 { margin: 0; font-weight: 400; }
    .cards { display: grid; grid-template-columns: repeat(auto-fit, minmax(340px, 1fr)); gap: 16px; }
    .row { display: flex; align-items: center; gap: 10px; padding: 6px 0; border-bottom: 1px solid var(--mat-sys-surface-container); }
    .row:last-child { border-bottom: 0; }
    .row .t { flex: 1; min-width: 0; }
    .row .d { font-size: .82em; color: var(--mat-sys-on-surface-variant); overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
    .note { font-size: .85em; color: var(--mat-sys-on-surface-variant); margin-top: 8px; line-height: 1.45; }
    .errors { padding: 8px 12px; border-radius: 8px; background: var(--mat-sys-error-container); color: var(--mat-sys-on-error-container); margin-bottom: 12px; font-size: .9em; }
    mat-card-actions { gap: 6px; flex-wrap: wrap; }
    .hero { border-color: var(--mat-sys-primary); }
  `],
  template: `
    @if (busy()) { <mat-progress-bar mode="indeterminate" /> }
    <div class="head"><h2>Package sources (channels)</h2><span class="spacer"></span><button mat-icon-button (click)="load()" matTooltip="Reload"><mat-icon>refresh</mat-icon></button></div>
    @if (error()) { <div class="errors">{{ error() }}</div> }
    <p class="muted">Without a flake, NixOS takes its packages from <b>channels</b>: named snapshots of the package collection that you update by hand. The <code>nixos</code> system channel decides which NixOS version you run.</p>

    <div class="cards">
      @if (ws.overview()?.location?.kind !== 'flake') {
      <mat-card appearance="outlined" class="hero">
        <mat-card-header><mat-card-title>Switch to flakes (recommended)</mat-card-title></mat-card-header>
        <mat-card-content>
          <div class="note">Flakes replace channels with a <code>flake.nix</code> that pins exact versions, can mix several NixOS versions, and lets this app manage package sources, global values and extra package sets for you. The template starts on <b>nixos-unstable</b>.</div>
        </mat-card-content>
        <mat-card-actions><button mat-flat-button (click)="convert()"><mat-icon>ac_unit</mat-icon> Switch to flakes</button></mat-card-actions>
      </mat-card>
      } @else {
      <div class="note"><mat-icon>ac_unit</mat-icon><span>This configuration already uses flakes: package sources come from the inputs on the <a routerLink="../flake" queryParamsHandling="merge">Flake</a> page. The channel actions below only matter for tools that still read channels.</span></div>
      }

      <mat-card appearance="outlined">
        <mat-card-header><mat-card-title>System channels</mat-card-title><mat-card-subtitle>used by nixos-rebuild (root)</mat-card-subtitle></mat-card-header>
        <mat-card-content>
          @for (c of info()?.system ?? []; track c.name) {
            <div class="row">
              <mat-icon>{{ c.name === 'nixos' ? 'star' : 'rss_feed' }}</mat-icon>
              <div class="t"><b class="mono">{{ c.name }}</b><div class="d">{{ c.version ? 'version ' + c.version : '' }}</div></div>
              <button mat-icon-button matTooltip="Update this channel" (click)="act('update', 'system', c)"><mat-icon>sync</mat-icon></button>
              <button mat-icon-button matTooltip="Remove" (click)="act('remove', 'system', c)" [disabled]="c.name === 'nixos'"><mat-icon>delete</mat-icon></button>
            </div>
          } @empty { <div class="muted">No system channels found.</div> }
          <div class="note">Running NixOS {{ info()?.nixos_version ?? '?' }}. Changing the <code>nixos</code> channel to another version upgrades or downgrades the whole system on the next build. Actions here ask for your sudo password.</div>
        </mat-card-content>
        <mat-card-actions>
          <button mat-stroked-button (click)="add('system')"><mat-icon>add</mat-icon> Add channel</button>
          <button mat-stroked-button (click)="changeNixos()"><mat-icon>swap_horiz</mat-icon> Change NixOS version</button>
          <button mat-stroked-button (click)="act('update', 'system', null)"><mat-icon>sync</mat-icon> Update all</button>
        </mat-card-actions>
      </mat-card>

      <mat-card appearance="outlined">
        <mat-card-header><mat-card-title>Your user channels</mat-card-title><mat-card-subtitle>used by nix-env / nix-shell for your user only</mat-card-subtitle></mat-card-header>
        <mat-card-content>
          @for (c of info()?.user ?? []; track c.name) {
            <div class="row">
              <mat-icon>person</mat-icon>
              <div class="t"><b class="mono">{{ c.name }}</b><div class="d mono">{{ c.url }}</div></div>
              <button mat-icon-button matTooltip="Update" (click)="act('update', 'user', c)"><mat-icon>sync</mat-icon></button>
              <button mat-icon-button matTooltip="Remove" (click)="act('remove', 'user', c)"><mat-icon>delete</mat-icon></button>
            </div>
          } @empty { <div class="muted">None. That is normal when everything is installed through configuration.nix.</div> }
        </mat-card-content>
        <mat-card-actions><button mat-stroked-button (click)="add('user')"><mat-icon>add</mat-icon> Add channel</button></mat-card-actions>
      </mat-card>
    </div>
  `,
})
export class ChannelsComponent implements OnInit {
  private api = inject(ApiService);
  ws = inject(WorkspaceService);
  private dialog = inject(MatDialog);
  private router = inject(Router);

  info = signal<ChannelsInfo | null>(null);
  busy = signal(false);
  error = signal<string | null>(null);

  ngOnInit() { this.load(); }

  load() {
    this.busy.set(true);
    this.api.channels().subscribe({
      next: i => { this.info.set(i); this.busy.set(false); this.error.set(null); },
      error: (e: ApiFailure) => { this.error.set(e.message); this.busy.set(false); },
    });
  }

  private prompt(data: PromptData) {
    return this.dialog.open<PromptDialogComponent, PromptData, PromptResult>(PromptDialogComponent, { width: '520px', data }).afterClosed();
  }

  convert() { this.dialog.open(ConvertFlakeDialogComponent, { width: '560px', data: { path: this.ws.path() } }); }

  private start(action: ChannelAction, scope: ChannelScope, name: string | null, url: string | null, password: string | null) {
    this.api.channelAction({ action, scope, name, url, password }).subscribe({
      next: job => this.router.navigate(['/w', 'jobs', job.id], { queryParamsHandling: 'merge' }),
      error: (e: ApiFailure) => this.error.set(e.message),
    });
  }

  private withPassword(scope: ChannelScope, title: string, then: (pw: string | null) => void) {
    if (scope === 'user') { then(null); return; }
    this.prompt({ title, message: 'System channels belong to root. Enter your sudo password to continue; it is used once and never stored.', label: 'Sudo password', password: true, okLabel: 'Run' })
      .subscribe(r => { if (r) then(r.value); });
  }

  act(action: ChannelAction, scope: ChannelScope, c: ChannelEntry | null) {
    const name = c?.name ?? null;
    const title = action === 'update' ? `Update ${name ?? 'all channels'}` : `Remove channel ${name}`;
    if (action === 'remove') {
      this.prompt({ title, message: `${name} will no longer be available to ${scope === 'system' ? 'the system' : 'your user'}.`, okLabel: 'Remove', danger: true })
        .subscribe(r => { if (r) this.withPassword(scope, title, pw => this.start('remove', scope, name, null, pw)); });
      return;
    }
    this.withPassword(scope, title, pw => this.start(action, scope, name, null, pw));
  }

  add(scope: ChannelScope) {
    const choices = this.info()?.available ?? [];
    this.prompt({ title: 'Add channel', label: 'Channel', choices, value: choices[0], hint: 'Pick a NixOS channel; it is registered under its own name', okLabel: 'Next' }).subscribe(r => {
      if (!r) return;
      this.prompt({ title: 'Register it as', label: 'Name', value: r.value, hint: 'The name modules will see, e.g. nixos-unstable', okLabel: 'Add' }).subscribe(r2 => {
        if (!r2) return;
        this.withPassword(scope, `Add channel ${r2.value}`, pw => this.start('add', scope, r2.value, `https://nixos.org/channels/${r.value}`, pw));
      });
    });
  }

  changeNixos() {
    const choices = this.info()?.available ?? [];
    this.prompt({ title: 'Change NixOS version', message: 'This re-points the "nixos" system channel. The system changes version on the next build (switch).', label: 'Version', choices, value: choices[0], okLabel: 'Change' })
      .subscribe(r => { if (r) this.withPassword('system', 'Change NixOS version', pw => this.start('add', 'system', 'nixos', `https://nixos.org/channels/${r.value}`, pw)); });
  }
}
