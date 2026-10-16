import { Component, inject } from '@angular/core';
import { FormsModule } from '@angular/forms';
import { MAT_DIALOG_DATA, MatDialogModule, MatDialogRef } from '@angular/material/dialog';
import { MatButtonModule } from '@angular/material/button';
import { MatFormFieldModule } from '@angular/material/form-field';
import { MatInputModule } from '@angular/material/input';
import { MatCheckboxModule } from '@angular/material/checkbox';
import { MatSelectModule } from '@angular/material/select';

export interface PromptData {
  title: string;
  message?: string;
  /** When set, an input is shown with this label. */
  label?: string;
  value?: string;
  placeholder?: string;
  hint?: string;
  /** Optional checkbox under the input. */
  checkbox?: { label: string; checked: boolean };
  okLabel?: string;
  danger?: boolean;
  /** Render the input as a password field. */
  password?: boolean;
  /** When set, a dropdown with these choices is shown instead of a free text input. */
  choices?: string[];
  /** Returns an error message for an invalid value, or null. */
  validate?: (v: string) => string | null;
}

export interface PromptResult {
  value: string;
  checked: boolean;
}

/** Small reusable prompt/confirm dialog. Resolves to PromptResult or undefined when cancelled. */
@Component({
  selector: 'app-prompt-dialog',
  imports: [FormsModule, MatDialogModule, MatButtonModule, MatFormFieldModule, MatInputModule, MatCheckboxModule, MatSelectModule],
  template: `
    <h2 mat-dialog-title>{{ data.title }}</h2>
    <mat-dialog-content>
      @if (data.message) { <p>{{ data.message }}</p> }
      @if (data.label) {
        <mat-form-field appearance="outline" style="width:100%">
          <mat-label>{{ data.label }}</mat-label>
          @if (data.choices?.length) {
            <mat-select [(ngModel)]="value">@for (c of data.choices!; track c) { <mat-option [value]="c">{{ c }}</mat-option> }</mat-select>
          } @else {
            <input matInput [(ngModel)]="value" [placeholder]="data.placeholder ?? ''" (keyup.enter)="ok()" [class.mono]="!data.password" [type]="data.password ? 'password' : 'text'" [attr.autocomplete]="data.password ? 'current-password' : null" cdkFocusInitial />
          }
          @if (data.hint) { <mat-hint>{{ data.hint }}</mat-hint> }
          @if (error()) { <mat-error>{{ error() }}</mat-error> }
        </mat-form-field>
        @if (error()) { <div class="error-text" style="font-size:.85em">{{ error() }}</div> }
      }
      @if (data.checkbox) { <mat-checkbox [(ngModel)]="checked">{{ data.checkbox.label }}</mat-checkbox> }
    </mat-dialog-content>
    <mat-dialog-actions align="end">
      <button mat-button mat-dialog-close>Cancel</button>
      <button mat-flat-button [color]="data.danger ? 'warn' : undefined" (click)="ok()" [disabled]="!!data.label && !value.trim()">{{ data.okLabel ?? 'OK' }}</button>
    </mat-dialog-actions>
  `,
})
export class PromptDialogComponent {
  data = inject<PromptData>(MAT_DIALOG_DATA);
  private ref = inject(MatDialogRef<PromptDialogComponent, PromptResult>);
  value = this.data.value ?? '';
  checked = this.data.checkbox?.checked ?? false;

  error() { return this.data.validate?.(this.value) ?? null; }

  ok() {
    if (this.data.label && (!this.value.trim() || this.error())) return;
    this.ref.close({ value: this.value.trim(), checked: this.checked });
  }
}
