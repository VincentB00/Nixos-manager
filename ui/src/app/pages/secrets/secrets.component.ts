import { Component, OnInit, inject, signal } from '@angular/core';
import { FormsModule } from '@angular/forms';
import { MatButtonModule } from '@angular/material/button';
import { MatCardModule } from '@angular/material/card';
import { MatDialog } from '@angular/material/dialog';
import { MatFormFieldModule } from '@angular/material/form-field';
import { MatIconModule } from '@angular/material/icon';
import { MatInputModule } from '@angular/material/input';
import { MatProgressBarModule } from '@angular/material/progress-bar';
import { MatSnackBar } from '@angular/material/snack-bar';
import { MatTooltipModule } from '@angular/material/tooltip';
import { MatMenuModule } from '@angular/material/menu';
import { ApiService, ApiFailure } from '../../api/api.service';
import { WorkspaceService } from '../../core/workspace.service';
import { PromptDialogComponent, PromptData, PromptResult } from '../../shared/prompt-dialog.component';
import { SecretEntryDialogComponent } from './secret-entry-dialog.component';
import type { SecretEntry, SecretsStatus } from '../../api/generated';

/** Managed secrets: one passphrase-encrypted secrets.age in the config folder. */
@Component({
  selector: 'app-secrets',
  imports: [FormsModule, MatButtonModule, MatCardModule, MatFormFieldModule, MatIconModule, MatInputModule, MatProgressBarModule, MatTooltipModule, MatMenuModule],
  styles: [`
    .head { display: flex; align-items: center; gap: 12px; margin-bottom: 12px; flex-wrap: wrap; }
    h2 { margin: 0; font-weight: 400; }
    .cards { display: grid; grid-template-columns: repeat(auto-fit, minmax(360px, 1fr)); gap: 16px; }
    .row { display: flex; align-items: center; gap: 10px; padding: 6px 0; border-bottom: 1px solid var(--mat-sys-surface-container); }
    .row:last-child { border-bottom: 0; }
    .row .t { flex: 1; min-width: 0; }
    .row .d { font-size: .82em; color: var(--mat-sys-on-surface-variant); overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
    .note { font-size: .85em; color: var(--mat-sys-on-surface-variant); margin-top: 8px; line-height: 1.5; }
    .errors { padding: 8px 12px; border-radius: 8px; background: var(--mat-sys-error-container); color: var(--mat-sys-on-error-container); margin-bottom: 12px; font-size: .9em; }
    .ok { color: #2e7d32; } .warn { color: var(--nm-warning); }
    ul { margin: 6px 0; padding-left: 18px; line-height: 1.5; }
    mat-card-actions { gap: 6px; flex-wrap: wrap; }
    .unlock { display: flex; gap: 8px; align-items: center; margin-top: 8px; }
    .unlock mat-form-field { flex: 1; }
  `],
  template: `
    @if (busy()) { <mat-progress-bar mode="indeterminate" /> }
    <div class="head"><h2>Secrets</h2><span class="spacer"></span><button mat-icon-button (click)="load()" matTooltip="Reload"><mat-icon>refresh</mat-icon></button></div>
    @if (error()) { <div class="errors">{{ error() }}</div> }

    @if (st(); as s) {
      @if (!s.exists) {
        <mat-card appearance="outlined">
          <mat-card-header><mat-card-title>Keep passwords out of your configuration</mat-card-title></mat-card-header>
          <mat-card-content>
            <p class="note">Anything written directly in a .nix file ends up in the world-readable Nix store, and files next to your configuration usually end up in git. Managed secrets fix that:</p>
            <ul class="note">
              <li>Logins are stored in <span class="mono">secrets.age</span>, encrypted with a password you choose. The file is safe to commit and to copy to /etc/nixos.</li>
              <li>When you build, you enter the password once and each secret is installed as a root-only file under <span class="mono">{{ s.deploy.deploy_dir }}</span>. Modules reference it by path, never by value.</li>
              <li>It is a standard age file: <span class="mono">age -d secrets.age</span> recovers it without this app.</li>
            </ul>
            <div class="unlock">
              <mat-form-field appearance="outline" subscriptSizing="dynamic"><mat-label>Choose a password</mat-label><input matInput type="password" [(ngModel)]="pw1" autocomplete="new-password" /></mat-form-field>
              <mat-form-field appearance="outline" subscriptSizing="dynamic"><mat-label>Repeat it</mat-label><input matInput type="password" [(ngModel)]="pw2" autocomplete="new-password" (keyup.enter)="create()" /></mat-form-field>
              <button mat-flat-button (click)="create()" [disabled]="pw1.length < 4 || pw1 !== pw2 || busy()"><mat-icon>lock</mat-icon> Create secrets file</button>
            </div>
            <div class="note">Forgetting this password means re-entering the secrets; there is no recovery.</div>
          </mat-card-content>
        </mat-card>
      } @else if (!s.unlocked) {
        <mat-card appearance="outlined">
          <mat-card-header><mat-card-title>Unlock secrets</mat-card-title><mat-card-subtitle>secrets.age is encrypted; enter its password to view or change entries</mat-card-subtitle></mat-card-header>
          <mat-card-content>
            <div class="unlock">
              <mat-form-field appearance="outline" subscriptSizing="dynamic"><mat-label>Secrets password</mat-label><input matInput type="password" [(ngModel)]="pw1" autocomplete="current-password" (keyup.enter)="unlock()" cdkFocusInitial /></mat-form-field>
              <button mat-flat-button (click)="unlock()" [disabled]="!pw1 || busy()"><mat-icon>lock_open</mat-icon> Unlock</button>
            </div>
            <div class="note">Deployed secrets: <span [class]="s.deploy.deployed_up_to_date ? 'ok' : 'warn'">{{ s.deploy.deployed_up_to_date ? 'up to date' : 'not installed or out of date; the next build will ask for this password' }}</span></div>
          </mat-card-content>
        </mat-card>
      } @else {
        <div class="cards">
          <mat-card appearance="outlined">
            <mat-card-header><mat-card-title>Entries</mat-card-title><mat-card-subtitle>installed at build time under <span class="mono">{{ s.deploy.deploy_dir }}/&lt;name&gt;</span></mat-card-subtitle></mat-card-header>
            <mat-card-content>
              @for (e of s.entries; track e.name) {
                <div class="row">
                  <mat-icon>{{ e.kind === 'smb' ? 'folder_shared' : 'description' }}</mat-icon>
                  <div class="t"><b class="mono">{{ e.name }}</b><div class="d">{{ e.kind === 'smb' ? 'SMB login · user ' + e.username + (e.domain ? ' · domain ' + e.domain : '') : 'text · ' + e.content }}</div>
                    <div class="d mono">{{ s.deploy.deploy_dir }}/{{ e.name }}</div></div>
                  <button mat-icon-button [matMenuTriggerFor]="copyMenu" matTooltip="Copy the installed path"><mat-icon>content_copy</mat-icon></button>
                  <mat-menu #copyMenu="matMenu">
                    <button mat-menu-item (click)="copy(s.deploy.deploy_dir + '/' + e.name)"><mat-icon>link</mat-icon><span class="mono">{{ s.deploy.deploy_dir }}/{{ e.name }}</span></button>
                    @if (e.kind === 'smb') {
                      <button mat-menu-item (click)="copy('&quot;credentials=' + s.deploy.deploy_dir + '/' + e.name + '&quot;')"><mat-icon>folder_shared</mat-icon><span class="mono">"credentials={{ s.deploy.deploy_dir }}/{{ e.name }}"</span><span class="muted" style="margin-left:8px">mount option</span></button>
                    }
                  </mat-menu>
                  <button mat-icon-button matTooltip="Reveal" (click)="reveal(e)"><mat-icon>visibility</mat-icon></button>
                  <button mat-icon-button matTooltip="Edit" (click)="edit(e)"><mat-icon>edit</mat-icon></button>
                  <button mat-icon-button matTooltip="Remove" (click)="remove(e)"><mat-icon>delete</mat-icon></button>
                </div>
              } @empty { <div class="muted">No secrets yet. Add an SMB login, then pick it in the "Mount a network share" wizard.</div> }
              <div class="note">Deployed: <span [class]="s.deploy.deployed_up_to_date ? 'ok' : 'warn'">{{ s.deploy.deployed_up_to_date ? 'up to date' : 'changed since the last build; the next build will ask for the secrets password' }}</span></div>
            </mat-card-content>
            <mat-card-actions>
              <button mat-flat-button (click)="add()"><mat-icon>add</mat-icon> Add secret</button>
              <button mat-stroked-button (click)="changePassword()"><mat-icon>key</mat-icon> Change password</button>
              <button mat-stroked-button (click)="lock()"><mat-icon>lock</mat-icon> Lock</button>
            </mat-card-actions>
          </mat-card>
          <mat-card appearance="outlined">
            <mat-card-header><mat-card-title>How to use a secret</mat-card-title></mat-card-header>
            <mat-card-content class="note">
              In any module, reference the installed file by path. The "Mount a network share" wizard does this for you with the <b>managed</b> login option:
              <pre class="mono" style="white-space:pre-wrap">fileSystems."/mnt/nas".options = [ "credentials={{ s.deploy.deploy_dir }}/my-nas" ];</pre>
              The value never appears in your .nix files or in the Nix store. Rebuilds from a terminal keep using the last installed files; only new or changed secrets need a build from this app.
            </mat-card-content>
          </mat-card>
        </div>
      }
    }
  `,
})
export class SecretsComponent implements OnInit {
  private api = inject(ApiService);
  private ws = inject(WorkspaceService);
  private dialog = inject(MatDialog);
  private snack = inject(MatSnackBar);

  st = signal<SecretsStatus | null>(null);
  busy = signal(false);
  error = signal<string | null>(null);
  pw1 = '';
  pw2 = '';

  ngOnInit() { this.load(); }

  private run(obs: ReturnType<ApiService['secretsStatus']>, done?: string) {
    this.busy.set(true);
    this.error.set(null);
    obs.subscribe({
      next: s => { this.st.set(s); this.busy.set(false); this.pw1 = ''; this.pw2 = ''; if (done) this.snack.open(done, undefined, { duration: 2500 }); },
      error: (e: ApiFailure) => { this.busy.set(false); this.error.set(e.message); },
    });
  }
  load() { this.run(this.api.secretsStatus(this.ws.path()!)); }
  create() { this.run(this.api.secretsCreate(this.ws.path()!, this.pw1), 'secrets.age created'); }
  unlock() { this.run(this.api.secretsUnlock(this.ws.path()!, this.pw1)); }
  lock() { this.run(this.api.secretsLock(this.ws.path()!), 'Locked'); }

  copy(text: string) {
    const done = () => this.snack.open(`Copied ${text}`, undefined, { duration: 2500 });
    if (navigator.clipboard?.writeText) {
      navigator.clipboard.writeText(text).then(done, () => this.fallbackCopy(text, done));
    } else {
      this.fallbackCopy(text, done);
    }
  }
  /** Clipboard API needs a secure context; plain http://127.0.0.1 counts, but fall back to execCommand elsewhere. */
  private fallbackCopy(text: string, done: () => void) {
    const ta = document.createElement('textarea');
    ta.value = text;
    ta.style.position = 'fixed';
    ta.style.opacity = '0';
    document.body.appendChild(ta);
    ta.select();
    try { document.execCommand('copy'); done(); } catch { this.snack.open('Copy failed; select the path and copy it by hand', 'Dismiss', { duration: 5000 }); }
    document.body.removeChild(ta);
  }

  private prompt(data: PromptData) {
    return this.dialog.open<PromptDialogComponent, PromptData, PromptResult>(PromptDialogComponent, { width: '480px', data }).afterClosed();
  }

  add() {
    this.dialog.open<SecretEntryDialogComponent, { entry: SecretEntry | null }, SecretEntry>(SecretEntryDialogComponent, { width: '520px', data: { entry: null } })
      .afterClosed().subscribe(e => { if (e) this.run(this.api.secretsSet(this.ws.path()!, e), `Saved ${e.name}`); });
  }
  edit(e: SecretEntry) {
    this.api.secretsGet(this.ws.path()!, e.name).subscribe({
      next: full => this.dialog.open<SecretEntryDialogComponent, { entry: SecretEntry }, SecretEntry>(SecretEntryDialogComponent, { width: '520px', data: { entry: full } })
        .afterClosed().subscribe(ne => { if (ne) this.run(this.api.secretsSet(this.ws.path()!, ne), `Saved ${ne.name}`); }),
      error: (err: ApiFailure) => this.error.set(err.message),
    });
  }
  reveal(e: SecretEntry) {
    this.api.secretsGet(this.ws.path()!, e.name).subscribe({
      next: full => this.prompt({ title: full.name, message: full.kind === 'smb' ? `username: ${full.username}\npassword: ${full.password}${full.domain ? '\ndomain: ' + full.domain : ''}` : full.content, okLabel: 'Close' }).subscribe(),
      error: (err: ApiFailure) => this.error.set(err.message),
    });
  }
  remove(e: SecretEntry) {
    this.prompt({ title: `Remove ${e.name}?`, message: 'Modules that reference it will fail to mount until you change them. The installed file is removed on the next build.', okLabel: 'Remove', danger: true })
      .subscribe(r => { if (r) this.run(this.api.secretsRemove(this.ws.path()!, e.name), `Removed ${e.name}`); });
  }
  changePassword() {
    this.prompt({ title: 'New secrets password', label: 'Password', password: true, hint: 'At least 4 characters. You will need it on the next build.', okLabel: 'Change' })
      .subscribe(r => { if (r) this.run(this.api.secretsChangePassword(this.ws.path()!, r.value), 'Password changed'); });
  }
}
