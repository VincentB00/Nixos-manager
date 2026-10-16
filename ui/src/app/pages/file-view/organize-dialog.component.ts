import { Component, inject } from '@angular/core';
import { MAT_DIALOG_DATA, MatDialogModule, MatDialogRef } from '@angular/material/dialog';
import { MatButtonModule } from '@angular/material/button';
import { MatIconModule } from '@angular/material/icon';
import type { OrganizeResult } from '../../api/generated';

/** Shows what "Organize" would change and asks for confirmation. */
@Component({
  selector: 'app-organize-dialog',
  imports: [MatDialogModule, MatButtonModule, MatIconModule],
  styles: [`
    pre.diff { max-height: 60vh; overflow: auto; font-size: .8em; background: var(--mat-sys-surface-container); padding: 10px 12px; border-radius: 8px; margin: 8px 0 0; }
    .add { color: #2e7d32; } .del { color: #c62828; } .hunk { color: var(--mat-sys-on-surface-variant); }
    @media (prefers-color-scheme: dark) { .add { color: #81c784; } .del { color: #ef9a9a; } }
  `],
  template: `
    <h2 mat-dialog-title>Organize {{ data.file }}</h2>
    <mat-dialog-content>
      @if (!data.result.changed) {
        <p>Nothing to organize: settings are already grouped.</p>
      } @else {
        <p class="muted">Settings that share a prefix are grouped into nested blocks ({{ data.result.groups_created }} group{{ data.result.groups_created === 1 ? '' : 's' }}). Values and comments stay as they are; only the layout changes.</p>
        <pre class="diff mono">@for (l of lines(); track $index) {<span [class.add]="l.startsWith('+') && !l.startsWith('+++')" [class.del]="l.startsWith('-') && !l.startsWith('---')" [class.hunk]="l.startsWith('@@')">{{ l }}
</span>}</pre>
      }
    </mat-dialog-content>
    <mat-dialog-actions align="end">
      <button mat-button mat-dialog-close>{{ data.result.changed ? 'Cancel' : 'Close' }}</button>
      @if (data.result.changed) { <button mat-flat-button (click)="ref.close(true)"><mat-icon>auto_fix_high</mat-icon> Apply</button> }
    </mat-dialog-actions>
  `,
})
export class OrganizeDialogComponent {
  data = inject<{ file: string; result: OrganizeResult }>(MAT_DIALOG_DATA);
  ref = inject(MatDialogRef<OrganizeDialogComponent, boolean>);
  lines() { return this.data.result.unified_diff.split('\n'); }
}
