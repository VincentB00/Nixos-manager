import { Component, Injectable, inject, signal } from '@angular/core';
import { FormsModule } from '@angular/forms';
import { MAT_DIALOG_DATA, MatDialog, MatDialogModule, MatDialogRef } from '@angular/material/dialog';
import { MatButtonModule } from '@angular/material/button';
import { MatCheckboxModule } from '@angular/material/checkbox';
import { MatIconModule } from '@angular/material/icon';
import { MatProgressBarModule } from '@angular/material/progress-bar';
import { Observable } from 'rxjs';
import { ApiService, ApiFailure } from '../api/api.service';
import type { HostKeyForgetResult } from '../api/generated';

export interface HostKeyData {
  ssh_target: string;
  port: number | null;
  /** The ssh error text, when known. */
  detail?: string | null;
}

/**
 * "ssh does not trust this machine's host key": explains the two possible
 * causes, makes the user confirm they are sure, then removes the stored key
 * and records the key the machine presents now. Resolves to the result, or
 * undefined when cancelled.
 */
@Component({
  selector: 'app-host-key-dialog',
  imports: [FormsModule, MatDialogModule, MatButtonModule, MatCheckboxModule, MatIconModule, MatProgressBarModule],
  styles: [`
    p { line-height: 1.5; margin: 0 0 10px; }
    .danger { display: flex; gap: 8px; align-items: flex-start; padding: 10px 12px; border-radius: 8px; background: color-mix(in srgb, var(--mat-sys-error) 12%, transparent); border-left: 4px solid var(--mat-sys-error); margin: 10px 0; }
    .danger mat-icon { color: var(--mat-sys-error); flex: none; }
    ol { margin: 6px 0 10px; padding-left: 20px; line-height: 1.5; }
    .detail { font-family: monospace; font-size: .8em; white-space: pre-wrap; overflow-wrap: anywhere; background: var(--mat-sys-surface-container); padding: 6px 8px; border-radius: 6px; max-height: 120px; overflow: auto; }
    .fp { font-family: monospace; font-size: .8em; overflow-wrap: anywhere; }
    code { font-family: monospace; }
  `],
  template: `
    <h2 mat-dialog-title>ssh does not trust {{ host }}</h2>
    <mat-dialog-content>
      <p>ssh remembers the public key of every machine it has talked to (in <code>~/.ssh/known_hosts</code>). The key <b>{{ host }}</b> presents now is not the one stored, or none is stored yet and the app connects without prompts. There are two possible reasons:</p>
      <ol>
        <li><b>Harmless:</b> the machine was reinstalled, its SSH keys were regenerated, or this is the first contact.</li>
        <li><b>Serious:</b> something between you and the machine is intercepting the connection and presenting its own key.</li>
      </ol>
      <div class="danger"><mat-icon>gpp_maybe</mat-icon><span>Continuing removes the stored key and trusts whatever key the machine presents <i>right now</i>. Only do this if you are <b>absolutely sure</b> you are talking to the right machine, for example because you reinstalled it yourself or you checked its fingerprint on its own console with <code>ssh-keygen -lf /etc/ssh/ssh_host_ed25519_key.pub</code>.</span></div>
      @if (data.detail) { <div class="detail">{{ data.detail }}</div> }
      <mat-checkbox [(ngModel)]="sure" style="margin-top:8px">I am sure this is the right machine and I understand that a changed key can mean an attack.</mat-checkbox>
      @if (busy()) { <mat-progress-bar mode="indeterminate" style="margin-top:8px" /> }
      @if (error()) { <div class="error-text" style="margin-top:8px">{{ error() }}</div> }
      @if (result(); as r) {
        <p style="margin-top:10px"><b>Done.</b> Removed {{ r.removed }} stored entr{{ r.removed === 1 ? 'y' : 'ies' }}, recorded {{ r.learned }} key{{ r.learned === 1 ? '' : 's' }} from {{ r.host }}.</p>
        @if (r.fingerprints.length) { <div class="muted" style="font-size:.85em">New fingerprints (compare with the machine's console):</div> @for (f of r.fingerprints; track f) { <div class="fp">{{ f }}</div> } }
        @if (!r.learned) { <div class="warning-text" style="font-size:.9em; margin-top:6px">No key could be read from the machine; the next connection will still be refused until it is reachable.</div> }
      }
    </mat-dialog-content>
    <mat-dialog-actions align="end">
      @if (result(); as r) {
        <button mat-flat-button (click)="ref.close(r)">{{ retryLabel }}</button>
      } @else {
        <button mat-button mat-dialog-close>Cancel</button>
        <button mat-flat-button color="warn" [disabled]="!sure || busy()" (click)="go()"><mat-icon>key_off</mat-icon> Remove the stored key and trust the current one</button>
      }
    </mat-dialog-actions>
  `,
})
export class HostKeyDialogComponent {
  data = inject<HostKeyData>(MAT_DIALOG_DATA);
  ref = inject(MatDialogRef<HostKeyDialogComponent, HostKeyForgetResult | undefined>);
  private api = inject(ApiService);
  host = this.data.ssh_target.includes('@') ? this.data.ssh_target.split('@').slice(1).join('@') : this.data.ssh_target;
  sure = false;
  busy = signal(false);
  error = signal<string | null>(null);
  result = signal<HostKeyForgetResult | null>(null);
  retryLabel = 'Retry now';

  go() {
    this.busy.set(true); this.error.set(null);
    this.api.forgetHostKey(this.data.ssh_target, this.data.port).subscribe({
      next: r => { this.busy.set(false); this.result.set(r); if (!r.learned) this.retryLabel = 'Close'; },
      error: (e: ApiFailure) => { this.busy.set(false); this.error.set(e.message); },
    });
  }
}

/** Opens the host-key dialog; emits the result when the key was reset, undefined when cancelled. */
@Injectable({ providedIn: 'root' })
export class HostKeyService {
  private dialog = inject(MatDialog);
  /** Does an error message look like an ssh host-key refusal? (mirrors core remote::host_key_problem) */
  static looksLikeHostKeyProblem(msg: string | null | undefined): boolean {
    if (!msg) return false;
    const t = msg.toLowerCase();
    return t.includes('host key verification failed') || t.includes('remote host identification has changed') || (t.includes('offending') && t.includes('key in')) || (t.includes('host key for') && t.includes('has changed')) || t.includes('refused the host key');
  }
  offer(ssh_target: string, port: number | null, detail?: string | null): Observable<HostKeyForgetResult | undefined> {
    return this.dialog.open<HostKeyDialogComponent, HostKeyData, HostKeyForgetResult | undefined>(HostKeyDialogComponent, { width: '620px', data: { ssh_target, port, detail: detail ?? null } }).afterClosed();
  }
}
