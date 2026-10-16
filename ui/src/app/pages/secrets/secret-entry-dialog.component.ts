import { Component, inject } from '@angular/core';
import { FormsModule } from '@angular/forms';
import { MAT_DIALOG_DATA, MatDialogModule, MatDialogRef } from '@angular/material/dialog';
import { MatButtonModule } from '@angular/material/button';
import { MatFormFieldModule } from '@angular/material/form-field';
import { MatInputModule } from '@angular/material/input';
import { MatSelectModule } from '@angular/material/select';
import type { SecretEntry, SecretKind } from '../../api/generated';

@Component({
  selector: 'app-secret-entry-dialog',
  imports: [FormsModule, MatDialogModule, MatButtonModule, MatFormFieldModule, MatInputModule, MatSelectModule],
  template: `
    <h2 mat-dialog-title>{{ editing ? 'Edit secret' : 'Add secret' }}</h2>
    <mat-dialog-content>
      <mat-form-field appearance="outline" style="width:100%"><mat-label>Name</mat-label>
        <input matInput [(ngModel)]="e.name" [disabled]="editing" class="mono" placeholder="nas-synology" />
        <mat-hint>Letters, digits, - and _. Becomes the file name under the secrets folder.</mat-hint>
      </mat-form-field>
      <mat-form-field appearance="outline" style="width:100%"><mat-label>Kind</mat-label>
        <mat-select [(ngModel)]="e.kind"><mat-option value="smb">SMB / network share login</mat-option><mat-option value="text">Text file (anything else)</mat-option></mat-select>
      </mat-form-field>
      @if (e.kind === 'smb') {
        <mat-form-field appearance="outline" style="width:100%"><mat-label>Username</mat-label><input matInput [(ngModel)]="e.username" autocomplete="off" /></mat-form-field>
        <mat-form-field appearance="outline" style="width:100%"><mat-label>Password</mat-label><input matInput type="password" [(ngModel)]="e.password" autocomplete="new-password" /></mat-form-field>
        <mat-form-field appearance="outline" style="width:100%"><mat-label>Domain (optional)</mat-label><input matInput [(ngModel)]="e.domain" /></mat-form-field>
      } @else {
        <mat-form-field appearance="outline" style="width:100%"><mat-label>Content</mat-label><textarea matInput rows="6" [(ngModel)]="e.content" class="mono" spellcheck="false"></textarea></mat-form-field>
      }
    </mat-dialog-content>
    <mat-dialog-actions align="end">
      <button mat-button mat-dialog-close>Cancel</button>
      <button mat-flat-button (click)="ref.close(e)" [disabled]="!valid()">Save</button>
    </mat-dialog-actions>
  `,
})
export class SecretEntryDialogComponent {
  data = inject<{ entry: SecretEntry | null }>(MAT_DIALOG_DATA);
  ref = inject(MatDialogRef<SecretEntryDialogComponent, SecretEntry>);
  editing = !!this.data.entry;
  e: SecretEntry = this.data.entry ? { ...this.data.entry } : { name: '', kind: 'smb' as SecretKind, username: '', password: '', domain: '', content: '' };
  valid() {
    if (!/^[A-Za-z0-9_-]+$/.test(this.e.name)) return false;
    return this.e.kind === 'smb' ? !!this.e.username : !!this.e.content;
  }
}
