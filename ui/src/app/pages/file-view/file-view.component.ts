import { TooltipAriaDirective } from '../../shared/tooltip-aria.directive';
import { Component, DestroyRef, ElementRef, effect, inject, signal, viewChild } from '@angular/core';
import { ActivatedRoute, Router } from '@angular/router';
import { FormsModule } from '@angular/forms';
import { MatButtonModule } from '@angular/material/button';
import { MatIconModule } from '@angular/material/icon';
import { MatProgressBarModule } from '@angular/material/progress-bar';
import { MatSnackBar, MatSnackBarModule } from '@angular/material/snack-bar';
import { MatTooltipModule } from '@angular/material/tooltip';
import { MatButtonToggleModule } from '@angular/material/button-toggle';
import { ApiService, ApiFailure } from '../../api/api.service';
import { WorkspaceService } from '../../core/workspace.service';
import { NodeComponent } from './node.component';
import { MatDialog } from '@angular/material/dialog';
import { WizardDialogComponent, WizardData } from '../wizard/wizard-dialog.component';
import { OrganizeDialogComponent } from './organize-dialog.component';
import { MatMenuModule } from '@angular/material/menu';
import { CodeEditorComponent } from '../../shared/code-editor.component';
import { byteToIndex } from '../../shared/byte-offsets';
import { PromptDialogComponent, PromptData, PromptResult } from '../../shared/prompt-dialog.component';
import type { EditOp, NixFileView } from '../../api/generated';

/** rnix messages like "unexpected TOKEN_ASSIGN at 254..260, wanted any of [TOKEN_SEMICOLON]" → words. */
export function plainParseError(msg: string): string {
  const tok: Record<string, string> = { TOKEN_SEMICOLON: '";"', TOKEN_R_BRACE: '"}"', TOKEN_R_BRACK: '"]"', TOKEN_R_PAREN: '")"', TOKEN_ASSIGN: '"="', TOKEN_COLON: '":"', TOKEN_IDENT: 'a name', TOKEN_STRING_START: 'a string', TOKEN_L_BRACE: '"{"', TOKEN_L_BRACK: '"["', TOKEN_COMMA: '","', TOKEN_DOT: '"."' };
  const t = (n: string) => tok[n] ?? n.replace(/^TOKEN_/, '').toLowerCase().replace(/_/g, ' ');
  const m = msg.match(/unexpected (TOKEN_\w+)(?: at \d+\.\.\d+)?, wanted any of \[([^\]]*)\]/);
  if (m) {
    const wanted = m[2].split(',').map(x => t(x.trim())).filter(Boolean);
    return `expected ${wanted.join(' or ')} here (found ${t(m[1])}); often a missing ";" at the end of the previous line`;
  }
  if (/unexpected end of file/.test(msg)) return 'the file ends too early: a "}" or "]" or ";" is missing';
  return msg.replace(/ at \d+\.\.\d+/, '');
}

@Component({
  selector: 'app-file-view',
  imports: [FormsModule, MatButtonModule, MatIconModule, MatProgressBarModule, MatSnackBarModule, MatTooltipModule, TooltipAriaDirective, MatButtonToggleModule, MatMenuModule, NodeComponent, CodeEditorComponent],
  styles: [`
    .args-line { display: flex; align-items: center; gap: 8px; margin: -4px 0 8px; flex-wrap: wrap; }
    .head { display: flex; align-items: center; gap: 12px; flex-wrap: wrap; margin-bottom: 8px; }
    h2 { margin: 0; font-weight: 400; }
    .args { display: flex; gap: 4px; flex-wrap: wrap; }
    .args span { font-size: .75em; padding: 1px 8px; border-radius: 10px; background: var(--mat-sys-surface-container-high); }
    .errors { padding: 8px 12px; border-radius: 8px; background: var(--mat-sys-error-container); color: var(--mat-sys-on-error-container); margin-bottom: 12px; font-size: .9em; }
    textarea.whole { width: 100%; min-height: 70vh; font-family: ui-monospace, monospace; font-size: .85em; padding: 10px; border-radius: 8px; border: 1px solid var(--mat-sys-outline-variant); background: var(--mat-sys-surface); color: inherit; box-sizing: border-box; }
    .desc { margin: 0 0 12px; color: var(--mat-sys-on-surface-variant); white-space: pre-line; }
    .hint { font-size: .85em; }
    .empty { display: flex; gap: 14px; align-items: flex-start; padding: 16px; border: 1px dashed var(--mat-sys-outline-variant); border-radius: 12px; margin-bottom: 12px; }
    .empty mat-icon { color: var(--mat-sys-primary); font-size: 32px; width: 32px; height: 32px; }
  `],
  template: `
    @if (busy()) { <mat-progress-bar mode="indeterminate" /> }
    @if (view(); as v) {
      @if (isHardwareFile()) {
        <div class="warning-box"><mat-icon>memory</mat-icon><span><b>Machine-owned file.</b> hardware-configuration.nix is copied from this machine's /etc/nixos every time the folder is opened, so changes made here are overwritten. For this machine, regenerate it with <span class="mono">sudo nixos-generate-config</span>; for a remote target, copy the target's file in from the Remote page. Shown read-only.</span></div>
      }
      <div class="head">
        <h2 class="mono">{{ v.rel_path }}</h2>
        <span class="muted hint">{{ v.line_count }} lines</span>
        <span class="spacer"></span>
        @if (isNix() && mode() === 'form') {
          <button mat-flat-button (click)="openWizard()" [disabled]="v.parse_errors.length > 0" [matTooltip]="v.parse_errors.length ? 'Fix the syntax errors in this file first (switch to Code)' : 'Guided: install programs, turn on a service, set an option'"><mat-icon>add_circle</mat-icon> Add something</button>
        }
        @if (isNix() && mode() === 'form') {
          <button mat-icon-button (click)="expandAll(true)" matTooltip="Expand all groups"><mat-icon>unfold_more</mat-icon></button>
          <button mat-icon-button (click)="expandAll(false)" matTooltip="Collapse all groups"><mat-icon>unfold_less</mat-icon></button>
        }
        @if (isNix()) {
          <button mat-stroked-button (click)="organize()" [disabled]="v.parse_errors.length > 0 || busy()" matTooltip="Group settings that share a prefix into nested blocks, so the file reads top-down. Only the layout changes; the resulting system is identical."><mat-icon>auto_fix_high</mat-icon> Organize</button>
        }
        @if (isNix()) {
          <mat-button-toggle-group [value]="mode()" (change)="switchMode($event.value)" hideSingleSelectionIndicator>
            <mat-button-toggle value="form"><mat-icon>view_list</mat-icon> Form</mat-button-toggle>
            <mat-button-toggle value="code"><mat-icon>code</mat-icon> Code</mat-button-toggle>
          </mat-button-toggle-group>
        } @else { <span class="muted hint">Not a Nix file: shown as plain text</span> }
        @if (canUndo()) { <button mat-icon-button (click)="undo()" matTooltip="Undo the last change made in this app to this file"><mat-icon>undo</mat-icon></button> }
        <button mat-icon-button (click)="load()" matTooltip="Reload from disk"><mat-icon>refresh</mat-icon></button>
      </div>
      @if (v.args.length) {
        <div class="args-line"><span class="muted hint">receives</span> <span class="args" matTooltip="Values this module receives from the system">@for (a of v.args; track a) { <span class="mono">{{ a }}</span> }@if (v.has_ellipsis) { <span>…</span> }</span></div>
      }
      @if (description()) { <p class="desc">{{ description() }}</p> }
      @if (v.parse_errors.length && isNix()) {
        <div class="errors">
          <b>This file has syntax errors.</b> The form view may be incomplete; fix it in Code view.
          <ul>@for (e of v.parse_errors; track $index) { <li>{{ e.message }}@if (e.span) { (line {{ e.span.line }}) }</li> }</ul>
        </div>
      }
      @if (!writable()) {
        <div class="warning-box"><mat-icon>lock</mat-icon><span>{{ ws.overview()?.location?.is_etc_nixos ? 'This is the live system configuration; it is owned by root and cannot be edited here. Work on a copy in your home folder instead: edits happen there, and Build copies it back and rebuilds the system.' : 'This folder is not writable; changes will fail to save.' }}
          @if (ws.overview()?.location?.is_etc_nixos) { <button mat-stroked-button style="margin-left:10px" (click)="startFromSystem()"><mat-icon>content_copy</mat-icon> Start from this machine's configuration</button> }</span></div>
      }

      @if (mode() === 'form' && isNix()) {
        @if (isEmptyModule(v)) {
          <div class="empty">
            <mat-icon>auto_awesome</mat-icon>
            <div>
              <b>This module is empty.</b>
              <div class="muted">A module is a place to put settings. Use the wizard to install programs, turn on a service like SSH or Docker, add a user, or set any NixOS option. You can also switch to Code view and type Nix directly.</div>
              <button mat-flat-button style="margin-top:10px" (click)="openWizard()"><mat-icon>add_circle</mat-icon> Add something</button>
            </div>
          </div>
        }
        <app-node [node]="v.root" [source]="v.source" [expandState]="expandState()" (edit)="apply($event)" />
      } @else {
        @if (ws.flakeVars().length) {
          <div class="flex" style="margin-bottom:6px">
            <button mat-stroked-button [matMenuTriggerFor]="wholeVarsMenu"><mat-icon>data_object</mat-icon> Insert global value at cursor</button>
            <mat-menu #wholeVarsMenu="matMenu">
              @for (gv of ws.flakeVars(); track gv.name) {
                <button mat-menu-item (click)="insertIntoCode('vars.' + gv.name)"><span class="mono">vars.{{ gv.name }}</span><span class="muted mono" style="margin-left:8px;font-size:.8em">{{ gv.value }}</span></button>
              }
              <div mat-menu-item disabled style="opacity:.8">Inside a string</div>
              @for (gv of ws.flakeVars(); track gv.name) {
                <button mat-menu-item (click)="insertIntoCode(interp(gv.name))"><span class="mono">&#36;{{ '{' }}vars.{{ gv.name }}{{ '}' }}</span></button>
              }
            </mat-menu>
            @if (!v.args.includes('vars') && v.args.length) { <span class="muted hint">This module does not receive <span class="mono">vars</span> yet; it is added when you save a file that uses it.</span> }
          </div>
        }
        <app-code-editor #wholeArea [value]="codeText" (valueChange)="codeText = $event" (save)="codeText !== v.source && saveWhole()" (ready)="editorReady()" [language]="isNix() ? 'nix' : 'plaintext'" [readOnly]="isHardwareFile()" [markers]="markers()" height="calc(100vh - 290px)" />
        <div class="flex" style="margin-top:8px">
          <button mat-flat-button (click)="saveWhole()" [disabled]="codeText === v.source"><mat-icon>save</mat-icon> Save file</button>
          <button mat-button (click)="codeText = v.source" [disabled]="codeText === v.source">Revert</button>
          <span class="muted hint">{{ isNix() ? 'The file is checked for syntax errors before it is written.' : 'Saved as-is.' }} Ctrl+S saves too.</span>
        </div>
      }
    } @else if (error()) {
      <div class="errors">{{ error() }}</div>
    }
  `,
})
export class FileViewComponent {
  private router = inject(Router);
  private api = inject(ApiService);
  private route = inject(ActivatedRoute);
  private ws = inject(WorkspaceService);
  private snack = inject(MatSnackBar);
  private dialog = inject(MatDialog);
  private destroyRef = inject(DestroyRef);

  file = signal<string | null>(null);
  view = signal<NixFileView | null>(null);
  busy = signal(false);
  error = signal<string | null>(null);
  mode = signal<'form' | 'code'>('form');
  expandState = signal<{ open: boolean; seq: number } | null>(null);
  expandAll(open: boolean) { this.expandState.set({ open, seq: (this.expandState()?.seq ?? 0) + 1 }); }
  codeText = '';

  constructor() {
    const sub = this.route.queryParamMap.subscribe(q => {
      const f = q.get('f');
      const line = Number(q.get('line') ?? '');
      if (f && f !== this.file()) {
        if (this.dirty() && !window.confirm('You have unsaved changes in the code editor. Discard them and open ' + f + '?')) {
          this.router.navigate([], { relativeTo: this.route, queryParams: { f: this.file() }, queryParamsHandling: 'merge' });
          return;
        }
        this.file.set(f);
        if (!f.endsWith('.nix')) this.mode.set('code');
        this.load();
      }
      if (f && line > 0) {
        // from the search results: show the code at that line
        this.pendingLine = line;
        this.mode.set('code');
        this.wholeArea()?.revealLine(line);
      }
    });
    this.destroyRef.onDestroy(() => sub.unsubscribe());
    effect(() => {
      const v = this.view();
      if (v) this.codeText = v.source;
    });
  }

  plain(m: string) { return plainParseError(m); }
  isNix() { return (this.file() ?? '').endsWith('.nix'); }
  /** Parse errors as editor markers with a plain-language message. */
  markers() {
    const v = this.view();
    if (!v) return [];
    return v.parse_errors.map(e => {
      const idx = e.span ? byteToIndex(v.source, e.span.start) : v.source.length;
      const before = v.source.slice(0, idx);
      const line = (before.match(/\n/g)?.length ?? 0) + 1;
      const column = idx - before.lastIndexOf('\n');
      return { line, column, message: plainParseError(e.message) };
    });
  }
  startFromSystem() {
    this.dialog.open<PromptDialogComponent, PromptData, PromptResult>(PromptDialogComponent, { width: '560px', data: {
      title: 'Copy this configuration to a folder you can edit',
      message: 'The live configuration in /etc/nixos is copied into the folder below (flake.lock stays in /etc/nixos). You edit the copy here; Build copies it back and rebuilds the system.',
      label: 'Folder', value: '~/nixos', okLabel: 'Copy and open', checkbox: { label: 'Make it a git repository (recommended: history and undo)', checked: true },
    } }).afterClosed().subscribe(r => {
      if (!r?.value.trim()) return;
      this.busy.set(true);
      this.api.startFromSystem(r.value.trim(), r.checked).subscribe({
        next: info => { this.busy.set(false); this.router.navigate(['/w', 'overview'], { queryParams: { path: info.path } }); },
        error: (e: ApiFailure) => { this.busy.set(false); this.snack.open(e.message, 'Dismiss', { duration: 10000 }); },
      });
    });
  }

  isHardwareFile() { return (this.file() ?? '').split('/').pop() === 'hardware-configuration.nix'; }
  /** Unsaved Code-view edits: ask before they would be lost (mode switch, other file, other page). */
  dirty(): boolean { const v = this.view(); return !!v && this.mode() === 'code' && this.codeText !== v.source; }
  canLeave(): boolean { return !this.dirty() || window.confirm('You have unsaved changes in the code editor. Leave and discard them?'); }
  description() { return this.ws.fileMap().get(this.file() ?? '')?.description ?? null; }
  writable() { return this.ws.overview()?.location.writable ?? true; }

  load() {
    const path = this.ws.path();
    const f = this.file();
    if (!path || !f) return;
    this.busy.set(true);
    this.error.set(null);
    this.api.file(path, f).subscribe({
      next: v => { this.view.set(v); this.busy.set(false); },
      error: (e: ApiFailure) => { this.error.set(e.message); this.view.set(null); this.busy.set(false); },
    });
  }

  switchMode(m: 'form' | 'code') {
    if (m === 'form' && !this.canLeave()) { this.mode.set('code'); return; }
    if (m === 'code') this.codeText = this.view()?.source ?? '';
    this.mode.set(m);
  }

  /** Per-session undo: the file content before each change made from this app. */
  private history: { file: string; source: string }[] = [];
  historyLen = signal(0);
  canUndo() { const f = this.file(); return this.historyLen() > 0 && this.history.some(h => h.file === f); }
  undo() {
    const f = this.file();
    const idx = this.history.map(h => h.file).lastIndexOf(f ?? '');
    if (idx < 0) return;
    const [h] = this.history.splice(idx, 1);
    this.historyLen.set(this.history.length);
    this.apply({ op: 'replace_file', source: h.source }, false);
    this.snack.open('Undone', undefined, { duration: 1500 });
  }
  private remember(v: NixFileView) {
    this.history.push({ file: v.rel_path, source: v.source });
    if (this.history.length > 30) this.history.shift();
    this.historyLen.set(this.history.length);
  }

  apply(op: EditOp, record = true) {
    const path = this.ws.path();
    const v = this.view();
    if (!path || !v) return;
    if (record) this.remember(v);
    this.busy.set(true);
    this.api.edit(path, v.rel_path, v.version, op).subscribe({
      next: nv => {
        this.view.set(nv);
        this.busy.set(false);
        this.ws.refreshBuildStatus();
        this.snack.open('Saved', undefined, { duration: 1500 });
        if (op.op === 'replace_file') this.ensureVarsArg(nv);
        if (op.op === 'toggle_disabled' || op.op === 'list_append' || op.op === 'list_remove' || op.op === 'replace_file') this.ws.reload();
      },
      error: (e: ApiFailure) => {
        this.busy.set(false);
        this.snack.open(e.message, 'Dismiss', { duration: 8000 });
        if (e.code === 'conflict') this.load();
      },
    });
  }

  isEmptyModule(v: NixFileView) {
    return v.root.kind === 'attrset' && v.root.children.length === 0;
  }

  openWizard() {
    const v = this.view();
    const path = this.ws.path();
    if (!v || !path) return;
    const data: WizardData = { path, file: v.rel_path, version: v.version };
    this.dialog.open(WizardDialogComponent, { width: '760px', maxWidth: '95vw', data }).afterClosed().subscribe((nv: NixFileView | undefined) => {
      if (nv) {
        this.remember(v);
        this.view.set(nv);
        this.ws.reload();
        this.snack.open(`Added to ${nv.rel_path}. The system does not have it yet: press Build to apply it.`, 'Build', { duration: 10000 }).onAction().subscribe(() => this.router.navigate(['/w', 'overview'], { queryParamsHandling: 'merge', queryParams: { build: 1 } }));
      }
    });
  }

  organize() {
    const v = this.view();
    const path = this.ws.path();
    if (!v || !path) return;
    const current = this.mode() === 'code' ? this.codeText : v.source;
    const proceed = () => {
      this.busy.set(true);
      this.api.organize(path, v.rel_path, v.version, true).subscribe({
        next: res => {
          this.busy.set(false);
          this.dialog.open(OrganizeDialogComponent, { width: '820px', maxWidth: '95vw', data: { file: v.rel_path, result: res } }).afterClosed().subscribe(ok => {
            if (!ok) return;
            this.busy.set(true);
            this.api.organize(path, v.rel_path, v.version, false).subscribe({
              next: r2 => {
                this.busy.set(false);
                if (r2.view) { this.view.set(r2.view); this.codeText = r2.view.source; }
                this.snack.open(`Organized into ${r2.groups_created} group${r2.groups_created === 1 ? '' : 's'}`, undefined, { duration: 2500 });
              },
              error: (e: ApiFailure) => { this.busy.set(false); this.snack.open(e.message, 'Dismiss', { duration: 8000 }); if (e.code === 'conflict') this.load(); },
            });
          });
        },
        error: (e: ApiFailure) => { this.busy.set(false); this.snack.open(e.message, 'Dismiss', { duration: 8000 }); },
      });
    };
    if (current !== v.source) {
      this.snack.open('Save or revert your code changes first', 'Dismiss', { duration: 5000 });
      return;
    }
    proceed();
  }

  private wholeArea = viewChild<CodeEditorComponent>('wholeArea');
  interp(name: string) { return '$' + '{vars.' + name + '}'; }
  insertIntoCode(text: string) {
    this.wholeArea()?.insertAtCursor(text);
  }
  private pendingLine = 0;
  editorReady() {
    if (this.pendingLine > 0) {
      const l = this.pendingLine;
      this.pendingLine = 0;
      setTimeout(() => this.wholeArea()?.revealLine(l), 50);
    }
  }


  saveWhole() {
    this.apply({ op: 'replace_file', source: this.codeText });
  }

  /** After a raw save: if the module uses `vars` but does not receive it, add the argument. */
  private ensureVarsArg(v: NixFileView) {
    const path = this.ws.path();
    if (!path || !/(^|[^A-Za-z0-9_.-])vars\./.test(v.source) || v.args.includes('vars') || v.args.length === 0) return;
    this.api.edit(path, v.rel_path, v.version, { op: 'ensure_arg', name: 'vars' }).subscribe({
      next: nv => { this.view.set(nv); this.snack.open('Added `vars` to the module arguments', undefined, { duration: 3000 }); },
      error: () => {},
    });
  }
}
