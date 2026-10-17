import { Component, inject, signal } from '@angular/core';
import { MAT_DIALOG_DATA, MatDialogModule } from '@angular/material/dialog';
import { MatButtonModule } from '@angular/material/button';
import { MatIconModule } from '@angular/material/icon';
import { MatProgressBarModule } from '@angular/material/progress-bar';
import { ApiService } from '../api/api.service';
import type { OptionDoc } from '../api/generated';

/** "What is this option?" — the NixOS option documentation for an attribute path. */
@Component({
  selector: 'app-option-doc-dialog',
  imports: [MatDialogModule, MatButtonModule, MatIconModule, MatProgressBarModule],
  styles: [`
    .desc { white-space: pre-wrap; line-height: 1.5; margin: 8px 0 12px; }
    dl { display: grid; grid-template-columns: max-content 1fr; gap: 4px 14px; font-size: .9em; }
    dt { color: var(--mat-sys-on-surface-variant); }
    code, .mono { font-family: monospace; }
  `],
  template: `
    <h2 mat-dialog-title class="mono">{{ data.name }}</h2>
    <mat-dialog-content>
      @if (loading()) { <mat-progress-bar mode="indeterminate" /> }
      @else if (doc(); as d) {
        <div class="desc">{{ d.description || 'No description.' }}</div>
        <dl>
          <dt>Type</dt><dd class="mono">{{ d.type }}</dd>
          @if (d.default !== null) { <dt>Default</dt><dd class="mono">{{ d.default }}</dd> }
          @if (d.example !== null) { <dt>Example</dt><dd class="mono">{{ d.example }}</dd> }
          @if (d.declarations.length) { <dt>Declared in</dt><dd class="mono">{{ d.declarations[0] }}</dd> }
        </dl>
      } @else {
        <p>No NixOS option is documented under this exact name. It may be a nested value of an option documented under a shorter name (for example <span class="mono">{{ parent() }}</span>), a package attribute, or a value private to this configuration.</p>
      }
    </mat-dialog-content>
    <mat-dialog-actions align="end">
      <a mat-button [href]="'https://search.nixos.org/options?query=' + encode(data.name)" target="_blank" rel="noopener"><mat-icon>open_in_new</mat-icon> search.nixos.org</a>
      <button mat-flat-button mat-dialog-close>Close</button>
    </mat-dialog-actions>
  `,
})
export class OptionDocDialogComponent {
  data = inject<{ name: string }>(MAT_DIALOG_DATA);
  private api = inject(ApiService);
  loading = signal(true);
  doc = signal<OptionDoc | null>(null);

  constructor() {
    const tryNames = [this.data.name, ...this.candidates(this.data.name)];
    const next = (i: number) => {
      if (i >= tryNames.length) { this.loading.set(false); return; }
      this.api.optionGet(tryNames[i]).subscribe({ next: d => { this.doc.set(d); this.loading.set(false); }, error: () => next(i + 1) });
    };
    next(0);
  }

  /** users.users.vincent.shell → users.users.<name>.shell, then shorter parents. */
  private candidates(name: string): string[] {
    const parts = name.split('.');
    const out: string[] = [];
    for (let i = 1; i < parts.length - 1; i++) {
      const alt = [...parts]; alt[i] = '<name>';
      out.push(alt.join('.'));
    }
    for (let i = parts.length - 1; i > 1; i--) out.push(parts.slice(0, i).join('.'));
    return out;
  }
  parent() { const p = this.data.name.split('.'); return p.slice(0, Math.max(1, p.length - 1)).join('.'); }
  encode(s: string) { return encodeURIComponent(s); }
}
