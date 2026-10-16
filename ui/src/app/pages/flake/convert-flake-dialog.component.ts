import { Component, OnInit, inject, signal } from '@angular/core';
import { FormsModule } from '@angular/forms';
import { Router } from '@angular/router';
import { MAT_DIALOG_DATA, MatDialogModule, MatDialogRef } from '@angular/material/dialog';
import { MatButtonModule } from '@angular/material/button';
import { MatFormFieldModule } from '@angular/material/form-field';
import { MatIconModule } from '@angular/material/icon';
import { MatInputModule } from '@angular/material/input';
import { MatSelectModule } from '@angular/material/select';
import { MatProgressBarModule } from '@angular/material/progress-bar';
import { ApiService, ApiFailure } from '../../api/api.service';
import { WorkspaceService } from '../../core/workspace.service';

/** "Switch to flakes": writes flake.nix from the template and enables flakes in configuration.nix. */
@Component({
  selector: 'app-convert-flake-dialog',
  imports: [FormsModule, MatDialogModule, MatButtonModule, MatFormFieldModule, MatIconModule, MatInputModule, MatSelectModule, MatProgressBarModule],
  styles: [`
    ul { margin: 6px 0 12px; padding-left: 18px; line-height: 1.5; font-size: .92em; }
    .warn { color: var(--mat-sys-error); }
  `],
  template: `
    <h2 mat-dialog-title>Switch to flakes</h2>
    <mat-dialog-content>
      @if (busy()) { <mat-progress-bar mode="indeterminate" /> }
      <p class="muted">A flake pins exactly which version of NixOS packages you build from, and lets this app manage package sources, global values, and extra package sets. This will:</p>
      <ul>
        <li>create <code>flake.nix</code> next to your <code>configuration.nix</code>, with nixpkgs on <b>{{ nixpkgsRef }}</b></li>
        <li>add <code>nix-command</code> and <code>flakes</code> to the experimental features in configuration.nix</li>
        <li>make builds use <code>nixos-rebuild --flake /etc/nixos#{{ hostname }}</code> from now on</li>
      </ul>
      <mat-form-field appearance="outline" style="width:100%"><mat-label>Host name (flake output)</mat-label>
        <input matInput [(ngModel)]="hostname" class="mono" />
        <mat-hint>"main" keeps every machine's flake identical. Naming it after the hostname ({{ machineHost }}) lets plain nixos-rebuild find it without #name.</mat-hint>
      </mat-form-field>
      <mat-form-field appearance="outline" style="width:100%"><mat-label>Main user (for the global vars)</mat-label>
        <input matInput [(ngModel)]="username" class="mono" />
      </mat-form-field>
      <mat-form-field appearance="outline" style="width:100%"><mat-label>nixpkgs branch</mat-label>
        <mat-select [(ngModel)]="nixpkgsRef">@for (c of branches(); track c) { <mat-option [value]="c">{{ c }}{{ c === 'nixos-unstable' ? ' (newest packages, default)' : '' }}</mat-option> }</mat-select>
      </mat-form-field>
      @if (error()) { <div class="warn">{{ error() }}</div> }
    </mat-dialog-content>
    <mat-dialog-actions align="end">
      <button mat-button mat-dialog-close>Cancel</button>
      <button mat-flat-button (click)="convert()" [disabled]="busy() || !hostname.trim()"><mat-icon>ac_unit</mat-icon> Create flake.nix</button>
    </mat-dialog-actions>
  `,
})
export class ConvertFlakeDialogComponent implements OnInit {
  private api = inject(ApiService);
  private ws = inject(WorkspaceService);
  private router = inject(Router);
  private ref = inject(MatDialogRef<ConvertFlakeDialogComponent>);
  data = inject<{ path: string }>(MAT_DIALOG_DATA);

  hostname = 'main';
  machineHost = this.ws.overview()?.machine.hostname ?? 'nixos';
  username = '';
  nixpkgsRef = 'nixos-unstable';
  branches = signal<string[]>(['nixos-unstable']);
  busy = signal(false);
  error = signal<string | null>(null);

  ngOnInit() {
    this.api.channels().subscribe(c => this.branches.set(c.available.length ? c.available : ['nixos-unstable']));
  }

  convert() {
    this.busy.set(true);
    this.error.set(null);
    this.api.flakeConvert({ path: this.data.path, hostname: this.hostname.trim(), username: this.username.trim() || null, nixpkgs_ref: this.nixpkgsRef }).subscribe({
      next: () => {
        this.busy.set(false);
        this.ref.close(true);
        this.ws.reload();
        this.router.navigate(['/w', 'flake'], { queryParams: { path: this.data.path } });
      },
      error: (e: ApiFailure) => { this.busy.set(false); this.error.set(e.message); },
    });
  }
}
