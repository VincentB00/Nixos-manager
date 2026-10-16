import { Component, inject } from '@angular/core';
import { FormsModule } from '@angular/forms';
import { MAT_DIALOG_DATA, MatDialogModule, MatDialogRef } from '@angular/material/dialog';
import { MatButtonModule } from '@angular/material/button';
import { MatCheckboxModule } from '@angular/material/checkbox';
import { MatFormFieldModule } from '@angular/material/form-field';
import { MatInputModule } from '@angular/material/input';
import type { DeployTarget } from '../../api/generated';

@Component({
  selector: 'app-target-dialog',
  imports: [FormsModule, MatDialogModule, MatButtonModule, MatCheckboxModule, MatFormFieldModule, MatInputModule],
  template: `
    <h2 mat-dialog-title>{{ data.target ? 'Edit target' : 'Add deploy target' }}</h2>
    <mat-dialog-content>
      <mat-form-field appearance="outline" style="width:100%"><mat-label>Name</mat-label><input matInput [(ngModel)]="t.name" placeholder="cyber-server-1" /></mat-form-field>
      <mat-form-field appearance="outline" style="width:100%"><mat-label>SSH address</mat-label>
        <input matInput [(ngModel)]="t.ssh_target" class="mono" placeholder="root@10.0.0.5 or vincent@server" />
        <mat-hint>Key-based login must already work from a terminal as your user.</mat-hint>
      </mat-form-field>
      <mat-form-field appearance="outline" style="width:100%"><mat-label>Flake host (nixosConfigurations.&lt;name&gt;)</mat-label>
        <input matInput [ngModel]="t.flake_host ?? ''" (ngModelChange)="t.flake_host = $event || null" class="mono" placeholder="main" />
        <mat-hint>Leave empty to use the host selected in the toolbar.</mat-hint>
      </mat-form-field>
      <mat-form-field appearance="outline" style="width:100%"><mat-label>Build host (optional)</mat-label>
        <input matInput [ngModel]="t.build_host ?? ''" (ngModelChange)="t.build_host = $event || null" class="mono" placeholder="leave empty to build here" />
        <mat-hint>Another machine that does the compiling (nixos-rebuild --build-host).</mat-hint>
      </mat-form-field>
      <mat-checkbox [(ngModel)]="t.use_remote_sudo">Use sudo on the target for activation (needed unless the SSH user is root)</mat-checkbox>
      <div class="muted" style="font-size:.85em; margin: 2px 0 0 30px">The build dialog checks the target first and asks for the target's sudo password only when it is needed (user not trusted by nix there, or sudo requires a password).</div>
    </mat-dialog-content>
    <mat-dialog-actions align="end">
      <button mat-button mat-dialog-close>Cancel</button>
      <button mat-flat-button (click)="ref.close(t)" [disabled]="!t.name.trim() || !t.ssh_target.trim()">Save</button>
    </mat-dialog-actions>
  `,
})
export class TargetDialogComponent {
  data = inject<{ target: DeployTarget | null }>(MAT_DIALOG_DATA);
  ref = inject(MatDialogRef<TargetDialogComponent, DeployTarget>);
  t: DeployTarget = this.data.target ? { ...this.data.target } : { name: '', ssh_target: '', flake_host: null, build_host: null, use_remote_sudo: true };
}
