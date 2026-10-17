import { Component, inject } from '@angular/core';
import { FormsModule } from '@angular/forms';
import { MAT_DIALOG_DATA, MatDialogModule, MatDialogRef } from '@angular/material/dialog';
import { MatButtonModule } from '@angular/material/button';
import { MatCheckboxModule } from '@angular/material/checkbox';
import { MAT_FORM_FIELD_DEFAULT_OPTIONS, MatFormFieldModule } from '@angular/material/form-field';
import { MatInputModule } from '@angular/material/input';
import type { DeployTarget } from '../../api/generated';

@Component({
  providers: [{ provide: MAT_FORM_FIELD_DEFAULT_OPTIONS, useValue: { appearance: 'outline', subscriptSizing: 'dynamic' } }],
  styles: [`mat-dialog-content { padding-top: 12px !important; } mat-form-field { margin-bottom: 12px; }`],
  selector: 'app-target-dialog',
  imports: [FormsModule, MatDialogModule, MatButtonModule, MatCheckboxModule, MatFormFieldModule, MatInputModule],
  template: `
    <h2 mat-dialog-title>{{ data.target ? 'Edit target' : 'Add deploy target' }}</h2>
    <mat-dialog-content>
      <mat-form-field appearance="outline" style="width:100%"><mat-label>Name</mat-label><input matInput [(ngModel)]="t.name" placeholder="cyber-server-1" /></mat-form-field>
      <div style="display:flex; gap:10px; flex-wrap:wrap">
        <mat-form-field appearance="outline" style="flex:2 1 220px"><mat-label>Host</mat-label>
          <input matInput [(ngModel)]="host" class="mono" placeholder="10.0.0.5 or server.lan" />
          <mat-hint>Name or IP address of the machine.</mat-hint>
        </mat-form-field>
        <mat-form-field appearance="outline" style="flex:1 1 140px"><mat-label>User</mat-label>
          <input matInput [(ngModel)]="user" class="mono" placeholder="vincent" />
          <mat-hint>Login name there; root needs no sudo.</mat-hint>
        </mat-form-field>
        <mat-form-field appearance="outline" style="flex:0 1 110px"><mat-label>Port</mat-label>
          <input matInput type="number" [(ngModel)]="port" min="1" max="65535" placeholder="22" />
          <mat-hint>22 unless changed.</mat-hint>
        </mat-form-field>
      </div>
      <div class="muted" style="font-size:.85em; margin:0 0 12px">Key-based login must work from a terminal as your user, or use "Set up key access" on the Remote page after saving.</div>
      <mat-form-field appearance="outline" style="width:100%"><mat-label>Flake host (nixosConfigurations.&lt;name&gt;)</mat-label>
        <input matInput [ngModel]="t.flake_host ?? ''" (ngModelChange)="t.flake_host = $event || null" class="mono" placeholder="main" />
        <mat-hint>Leave empty to use the host selected in the toolbar.</mat-hint>
      </mat-form-field>
      
      <mat-checkbox [(ngModel)]="t.use_remote_sudo">Use sudo on the target for activation (needed unless the SSH user is root)</mat-checkbox>
      <div class="muted" style="font-size:.85em; margin: 2px 0 0 30px">The build dialog checks the target first and asks for the target's sudo password only when it is needed (user not trusted by nix there, or sudo requires a password).</div>
    </mat-dialog-content>
    <mat-dialog-actions align="end">
      <button mat-button mat-dialog-close>Cancel</button>
      <button mat-flat-button (click)="save()" [disabled]="!t.name.trim() || !host.trim()">Save</button>
    </mat-dialog-actions>
  `,
})
export class TargetDialogComponent {
  data = inject<{ target: DeployTarget | null }>(MAT_DIALOG_DATA);
  ref = inject(MatDialogRef<TargetDialogComponent, DeployTarget>);
  t: DeployTarget = this.data.target ? { ...this.data.target } : { name: '', ssh_target: '', flake_host: null, build_host: null, use_remote_sudo: true, port: null };
  // the address is edited as three plain fields and stored as user@host (+ port)
  user = this.t.ssh_target.includes('@') ? this.t.ssh_target.split('@')[0] : '';
  host = this.t.ssh_target.includes('@') ? this.t.ssh_target.split('@').slice(1).join('@') : this.t.ssh_target;
  port: number | null = this.t.port ?? null;
  save() {
    const h = this.host.trim();
    const u = this.user.trim();
    this.t.ssh_target = u ? `${u}@${h}` : h;
    const p = Number(this.port);
    this.t.port = p && p !== 22 && p > 0 && p < 65536 ? p : null;
    if (u === 'root') this.t.use_remote_sudo = false;
    this.ref.close(this.t);
  }
}
