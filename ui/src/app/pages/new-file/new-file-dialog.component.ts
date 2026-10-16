import { Component, inject, signal } from '@angular/core';
import { FormsModule } from '@angular/forms';
import { MAT_DIALOG_DATA, MatDialogModule, MatDialogRef } from '@angular/material/dialog';
import { MatButtonModule } from '@angular/material/button';
import { MatFormFieldModule } from '@angular/material/form-field';
import { MatInputModule } from '@angular/material/input';
import { MatSelectModule } from '@angular/material/select';
import { ApiService, ApiFailure } from '../../api/api.service';
import { WorkspaceService } from '../../core/workspace.service';

@Component({
  selector: 'app-new-file-dialog',
  imports: [FormsModule, MatDialogModule, MatButtonModule, MatFormFieldModule, MatInputModule, MatSelectModule],
  template: `
    <h2 mat-dialog-title>New module file</h2>
    <mat-dialog-content>
      <p class="muted">A module is a .nix file that sets some options. It only takes effect once a file that is already loaded imports it.</p>
      <mat-form-field appearance="outline" style="width:100%">
        <mat-label>File name</mat-label>
        <input matInput [(ngModel)]="name" placeholder="my-module.nix" class="mono" />
      </mat-form-field>
      <mat-form-field appearance="outline" style="width:100%">
        <mat-label>Import it from</mat-label>
        <mat-select [(ngModel)]="importInto">
          <mat-option [value]="null">nowhere yet (I will add the import myself)</mat-option>
          @for (f of candidates(); track f) { <mat-option [value]="f">{{ f }}</mat-option> }
        </mat-select>
      </mat-form-field>
      <mat-form-field appearance="outline" style="width:100%">
        <mat-label>Initial contents</mat-label>
        <textarea matInput [(ngModel)]="source" rows="8" class="mono"></textarea>
      </mat-form-field>
      @if (error()) { <div class="error-text">{{ error() }}</div> }
    </mat-dialog-content>
    <mat-dialog-actions align="end">
      <button mat-button mat-dialog-close>Cancel</button>
      <button mat-flat-button (click)="create()" [disabled]="!name.endsWith('.nix') || busy()">Create</button>
    </mat-dialog-actions>
  `,
})
export class NewFileDialogComponent {
  private api = inject(ApiService);
  private ws = inject(WorkspaceService);
  private ref = inject(MatDialogRef<NewFileDialogComponent>);
  data = inject<{ path: string; dir?: string }>(MAT_DIALOG_DATA);

  name = this.data.dir ? this.data.dir.replace(/\/?$/, '/') : '';
  importInto: string | null = this.ws.overview()?.location.kind === 'legacy' ? 'configuration.nix' : (this.ws.fileMap().has('configuration.nix') ? 'configuration.nix' : null);
  source = `{ config, pkgs, ... }:\n\n{\n  # Put options here, for example:\n  # services.openssh.enable = true;\n}\n`;
  error = signal<string | null>(null);
  busy = signal(false);

  candidates() {
    return this.ws.nixFiles().filter(f => f.kind !== 'flake-lock' && f.kind !== 'hardware-configuration').map(f => f.rel_path);
  }

  create() {
    this.busy.set(true);
    this.error.set(null);
    this.api.newFile({ path: this.data.path, file: this.name.trim(), source: this.source, import_into: this.importInto }).subscribe({
      next: () => this.ref.close(this.name.trim()),
      error: (e: ApiFailure) => { this.error.set(e.message); this.busy.set(false); },
    });
  }
}
