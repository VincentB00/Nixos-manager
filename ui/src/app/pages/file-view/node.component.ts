import { TooltipAriaDirective } from '../../shared/tooltip-aria.directive';
import { Component, ElementRef, computed, effect, input, output, signal, viewChild } from '@angular/core';
import { sliceBytes } from '../../shared/byte-offsets';
import { FormsModule } from '@angular/forms';
import { MatDialog } from '@angular/material/dialog';
import { OptionDocDialogComponent } from '../../shared/option-doc-dialog.component';
import { MatSnackBar } from '@angular/material/snack-bar';
import { NgTemplateOutlet } from '@angular/common';
import { MatButtonModule } from '@angular/material/button';
import { MatIconModule } from '@angular/material/icon';
import { MatSlideToggleModule } from '@angular/material/slide-toggle';
import { MatTooltipModule } from '@angular/material/tooltip';
import { MatMenuModule } from '@angular/material/menu';
import type { EditOp, Node, NodeKind, ScalarValue } from '../../api/generated';
import { inject } from '@angular/core';
import { WorkspaceService } from '../../core/workspace.service';
import { CodeEditorComponent } from '../../shared/code-editor.component';

@Component({
  selector: 'app-node',
  imports: [FormsModule, NgTemplateOutlet, MatButtonModule, MatIconModule, MatSlideToggleModule, MatTooltipModule, TooltipAriaDirective, MatMenuModule, CodeEditorComponent],
  styles: [`
    @media (max-width: 700px) { .head { flex-wrap: wrap; } .head input, .head .inline { min-width: 120px !important; max-width: 100%; } .add-row { flex-wrap: wrap; } }
    :host { display: block; }
    .node { border-left: 2px solid transparent; padding: 2px 0 2px 8px; margin: 2px 0; }
    .node:hover { border-left-color: var(--mat-sys-outline-variant); }
    .node.disabled { opacity: .55; }
    .head { display: flex; align-items: center; gap: 8px; min-height: 36px; flex-wrap: nowrap; min-width: 0; }
    .label { font-weight: 500; }
    .kind { font-size: .7em; text-transform: uppercase; letter-spacing: .05em; padding: 1px 6px; border-radius: 4px; background: var(--mat-sys-surface-container-high); color: var(--mat-sys-on-surface-variant); }
    .with { font-size: .75em; padding: 1px 6px; border-radius: 10px; border: 1px dashed var(--mat-sys-outline-variant); }
    .comment { font-size: .85em; color: var(--mat-sys-on-surface-variant); margin-left: 8px; min-width: 0; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
    .actions { display: flex; gap: 0; opacity: 0; transition: opacity .15s; flex: none; margin-left: auto; }
    .node:hover > .head > .actions, .node:focus-within > .head > .actions { opacity: 1; }
    .children { margin-left: 14px; padding-left: 8px; border-left: 1px solid var(--mat-sys-surface-container-high); }
    input.inline { font: inherit; padding: 4px 8px; border-radius: 6px; border: 1px solid var(--mat-sys-outline-variant); background: var(--mat-sys-surface); color: inherit; min-width: 220px; }
    input.inline.changed { border-color: var(--mat-sys-primary); }
    pre.code { margin: 4px 0; padding: 8px 10px; border-radius: 6px; background: var(--mat-sys-surface-container); font-size: .85em; overflow: auto; max-height: 320px; }
    textarea.code { width: 100%; min-height: 120px; font-family: ui-monospace, monospace; font-size: .85em; padding: 8px; border-radius: 6px; border: 1px solid var(--mat-sys-primary); background: var(--mat-sys-surface); color: inherit; box-sizing: border-box; }
    .add-row { display: flex; gap: 6px; align-items: center; margin: 4px 0 4px 22px; flex-wrap: wrap; }
    .add-row input.inline { min-width: 160px; }
    .count { font-size: .8em; color: var(--mat-sys-on-surface-variant); }
    .vars-btn { width: 30px; height: 30px; line-height: 30px; padding: 0; }
    .vars-btn mat-icon { font-size: 18px; width: 18px; height: 18px; }
    .mi-val { font-size: .8em; color: var(--mat-sys-on-surface-variant); margin-left: 8px; }
    select.set { font: inherit; font-size: .8em; padding: 3px 6px; border-radius: 6px; border: 1px solid var(--mat-sys-outline-variant); background: var(--mat-sys-surface-container); color: inherit; max-width: 180px; }
  `],
  template: `
    <div class="node" [class.disabled]="node().disabled">
      <div class="head">
        @if (collapsible()) {
          <button mat-icon-button (click)="open.set(!open())"><mat-icon>{{ open() ? 'expand_more' : 'chevron_right' }}</mat-icon></button>
        }
        @if (node().label) { <span class="label mono">{{ node().label }}</span> }
        @if (node().with_namespace) { <span class="with mono" matTooltip="Items are looked up inside this set">with {{ node().with_namespace }}</span> }
        <span class="kind">{{ kindLabel() }}</span>
        @if (collapsible()) { <span class="count">{{ node().children.length }} {{ node().kind === 'list' ? (node().children.length === 1 ? 'item' : 'items') : (node().children.length === 1 ? 'setting' : 'settings') }}</span> }

        @if (node().disabled) {
          @if ((node().source ?? '').includes('\n')) {
            <span class="muted" style="font-size:.85em">commented out ({{ (node().source ?? '').split('\n').length }} lines)</span>
          } @else {
            <span class="mono muted">{{ node().value }}</span>
          }
          <button mat-stroked-button (click)="emit({ op: 'toggle_disabled', node_id: node().id })"><mat-icon>visibility</mat-icon> Enable</button>
        } @else if (!codeMode()) {
          @switch (node().kind) {
            @case ('bool') {
              <mat-slide-toggle #tgl [checked]="node().value === 'true'" (change)="setScalar({ type: 'bool', value: $event.checked }); tgl.checked = node().value === 'true'" />
            }
            @case ('null') { <span class="mono muted">null</span> }
            @case ('string') { @if (node().interpolated) { <code>{{ node().value }}</code> } @else { <ng-container *ngTemplateOutlet="inlineInput" /> } }
            @case ('int') { <ng-container *ngTemplateOutlet="inlineInput" /> }
            @case ('float') { <ng-container *ngTemplateOutlet="inlineInput" /> }
            @case ('path') { <ng-container *ngTemplateOutlet="inlineInput" /> }
            @case ('ref') {
              <ng-container *ngTemplateOutlet="inlineInput" />
              @if (pkgRef(); as r) {
                <select class="set mono" (change)="changePkgSet(r, $any($event.target).value)" matTooltip="Which package set this comes from">
                  @for (s of sets(); track s.name) { <option [value]="s.name" [selected]="s.name === r.set">{{ s.name }}</option> }
                  @if (!knownSet(r.set)) { <option [value]="r.set" selected>{{ r.set }} (not in this flake)</option> }
                </select>
              }
            }
            @case ('inherit') { <code>{{ node().value }}</code> }
          }
        }
        @if (node().comment) { <span class="comment" [matTooltip]="node().comment!">{{ node().comment }}</span> }
        <span class="actions">
          @if (node().label && node().kind !== 'inherit' && parentKind() === 'attrset') {
            <button mat-icon-button [matTooltip]="'What is ' + fullPath() + '?'" (click)="showDoc()"><mat-icon>info_outline</mat-icon></button>
          }
          @if (!node().disabled && node().kind !== 'inherit') {
            <button mat-icon-button matTooltip="Edit as Nix code" (click)="startCode()"><mat-icon>code</mat-icon></button>
          }
          @if ((isListItem() || isBinding()) && !node().disabled) {
            <button mat-icon-button [matTooltip]="node().kind === 'attrset' ? 'Disable this whole group (kept in the file as a comment; Enable brings it back)' : 'Disable (kept in the file as a comment; Enable brings it back)'" (click)="emit({ op: 'toggle_disabled', node_id: node().id })"><mat-icon>visibility_off</mat-icon></button>
          }
          @if (removable()) {
            <button mat-icon-button matTooltip="Remove" (click)="remove()"><mat-icon>delete</mat-icon></button>
          }
        </span>
      </div>

      @if (node().disabled && (node().source ?? '').includes('\n')) {
        <pre class="code muted">{{ node().source }}</pre>
      }
      @if (codeMode()) {
        @if (vars().length) {
          <div class="flex" style="margin: 4px 0">
            <button mat-stroked-button [matMenuTriggerFor]="codeVarsMenu"><mat-icon>data_object</mat-icon> Insert global value</button>
            <mat-menu #codeVarsMenu="matMenu">
              @for (v of vars(); track v.name) {
                <button mat-menu-item (click)="insertIntoCode('vars.' + v.name)"><span class="mono">vars.{{ v.name }}</span><span class="mi-val mono">{{ v.value }}</span></button>
              }
              <div mat-menu-item disabled style="opacity:.8">Inside a string</div>
              @for (v of vars(); track v.name) {
                <button mat-menu-item (click)="insertIntoCode(interp(v.name))"><span class="mono">&#36;{{ '{' }}vars.{{ v.name }}{{ '}' }}</span></button>
              }
            </mat-menu>
          </div>
        }
        <app-code-editor #codeArea [value]="codeText" (valueChange)="codeText = $event" (save)="saveCode()" height="180px" />
        <div class="flex">
          <button mat-flat-button (click)="saveCode()">Save</button>
          <button mat-button (click)="codeMode.set(false)">Cancel</button>
          <span class="muted" style="font-size:.85em">Replaces just this value. Must be a valid Nix expression.</span>
        </div>
      } @else if (!node().disabled) {
        @switch (node().kind) {
          @case ('code') { <pre class="code">{{ node().source }}</pre> }
          @case ('function') { <pre class="code">{{ node().source }}</pre> }
          @case ('attrset') {
            @if (open()) {
              <div class="children">
                @for (c of node().children; track c.id) { <app-node [node]="c" [source]="source()" parentKind="attrset" [prefix]="fullPath()" [expandState]="expandState()" (edit)="edit.emit($event)" /> }
              </div>
              <div class="add-row">
                <input class="inline mono" placeholder="option, e.g. services.openssh.enable" [(ngModel)]="newKey" (keyup.enter)="addBinding()" />
                <span class="muted">=</span>
                <input class="inline mono" placeholder="value, e.g. true or &quot;text&quot;" [(ngModel)]="newValue" (keyup.enter)="addBinding()" />
                <button mat-stroked-button (click)="addBinding()" [disabled]="!newKey || !newValue"><mat-icon>add</mat-icon> Add setting</button>
              </div>
            }
          }
          @case ('list') {
            @if (open()) {
              <div class="children">
                @for (c of node().children; track c.id) { <app-node [node]="c" [source]="source()" parentKind="list" [prefix]="fullPath()" [withNamespace]="node().with_namespace" [expandState]="expandState()" (edit)="edit.emit($event)" /> }
              </div>
              <div class="add-row">
                <input class="inline mono" [placeholder]="node().with_namespace ? 'item, e.g. htop' : 'item, e.g. pkgs.htop or ./file.nix'" [(ngModel)]="newItem" (keyup.enter)="addItem()" />
                <button mat-stroked-button (click)="addItem()" [disabled]="!newItem"><mat-icon>add</mat-icon> Add item</button>
              </div>
            }
          }
          @default {
            @if (collapsible() && open()) {
              <div class="children">@for (c of node().children; track c.id) { <app-node [node]="c" [source]="source()" [parentKind]="node().kind" [prefix]="fullPath()" [expandState]="expandState()" (edit)="edit.emit($event)" /> }</div>
            }
          }
        }
      }
    </div>

    <ng-template #inlineInput>
      <input class="inline mono" [class.changed]="draft() !== null" [value]="draft() ?? node().value" (input)="draft.set($any($event.target).value)"
             (keyup.enter)="commitDraft()" (keyup.escape)="draft.set(null)" (blur)="commitDraft()" [placeholder]="node().kind" />
      @if (draft() !== null) { <button mat-icon-button matTooltip="Save (Enter)" (click)="commitDraft()"><mat-icon>check</mat-icon></button> }
      @if (vars().length) {
        <button mat-icon-button class="vars-btn" [matMenuTriggerFor]="varsMenu" matTooltip="Use a global value from flake.nix"><mat-icon>data_object</mat-icon></button>
        <mat-menu #varsMenu="matMenu">
          <div mat-menu-item disabled style="opacity:.8">Use as the value</div>
          @for (v of vars(); track v.name) {
            <button mat-menu-item (click)="useVar(v.name)"><span class="mono">vars.{{ v.name }}</span><span class="mi-val mono">{{ v.value }}</span></button>
          }
          @if (node().kind === 'string') {
            <div mat-menu-item disabled style="opacity:.8">Insert into the text</div>
            @for (v of vars(); track v.name) {
              <button mat-menu-item (click)="insertVar(v.name)"><span class="mono">&#36;{{ '{' }}vars.{{ v.name }}{{ '}' }}</span></button>
            }
          }
        </mat-menu>
      }
    </ng-template>
  `,
})
export class NodeComponent {
  private dialog = inject(MatDialog);
  private snack = inject(MatSnackBar);
  node = input.required<Node>();
  source = input.required<string>();
  parentKind = input<NodeKind | 'root'>('root');
  /** Attribute path of the parent group, so a row knows its full option name. */
  prefix = input<string>('');
  fullPath = computed(() => { const l = this.node().label; const p = this.prefix(); return l ? (p ? `${p}.${l}` : l) : p; });
  showDoc() {
    const name = this.fullPath();
    if (!name) return;
    this.dialog.open(OptionDocDialogComponent, { width: '640px', maxWidth: '95vw', data: { name } });
  }
  edit = output<EditOp>();

  open = signal(true);
  codeMode = signal(false);
  codeText = '';
  draft = signal<string | null>(null);
  newKey = '';
  newValue = '';
  newItem = '';

  private ws = inject(WorkspaceService);
  sets = computed(() => this.ws.packageSets());
  /** If this ref is a package reference, which set it uses and the attr within it. */
  pkgRef = computed<{ set: string; attr: string; bare: boolean } | null>(() => {
    const n = this.node();
    if (n.kind !== 'ref' || n.disabled) return null;
    const v = n.value ?? '';
    const m = v.match(/^(pkgs(?:-[A-Za-z0-9_]+)?)\.(.+)$/);
    if (m) return { set: m[1], attr: m[2], bare: false };
    // Bare name inside a `with pkgs-x;` list → belongs to that set.
    const ns = this.withNamespace();
    if (ns && /^pkgs(-[A-Za-z0-9_]+)?$/.test(ns) && /^[A-Za-z_][A-Za-z0-9_'.-]*$/.test(v)) return { set: ns, attr: v, bare: true };
    return null;
  });
  withNamespace = input<string | null>(null);
  /** Broadcast from the file view: expand or collapse every group. `seq` makes repeats re-trigger. */
  expandState = input<{ open: boolean; seq: number } | null>(null);
  private expandEffect = effect(() => {
    const s = this.expandState();
    if (!s) return;
    // The root group always stays open; "collapse all" folds its children.
    this.open.set(s.open || this.parentKind() === 'root');
  });
  knownSet(name: string) { return this.sets().some(x => x.name === name); }
  vars = computed(() => this.ws.flakeVars());

  private codeArea = viewChild<CodeEditorComponent>('codeArea');
  interp(name: string) { return '$' + '{vars.' + name + '}'; }
  insertIntoCode(text: string) {
    this.codeArea()?.insertAtCursor(text);
  }


  /** Replace the whole value with a reference to a flake global. */
  useVar(name: string) {
    this.draft.set(null);
    this.setScalar({ type: 'ref', value: `vars.${name}` });
  }

  /** Append `${vars.name}` to the current text; saved as an interpolated string. */
  insertVar(name: string) {
    const current = this.draft() ?? this.node().value ?? '';
    this.draft.set(current + '${vars.' + name + '}');
  }

  collapsible = computed(() => ['attrset', 'list', 'letin', 'concat', 'merge'].includes(this.node().kind) && !this.node().disabled);
  isListItem = computed(() => this.parentKind() === 'list');
  isBinding = computed(() => this.parentKind() === 'attrset' && !!this.node().label && this.node().kind !== 'inherit');
  removable = computed(() => (this.parentKind() === 'list' || this.parentKind() === 'attrset') && !this.node().disabled);

  kindLabel() {
    const names: Record<string, string> = { attrset: 'group', list: 'list', bool: 'on/off', int: 'number', float: 'number', string: 'text', path: 'file', ref: 'reference', null: 'null', inherit: 'inherit', letin: 'let', concat: 'joined lists', merge: 'merged sets', function: 'function', code: 'code' };
    return names[this.node().kind] ?? this.node().kind;
  }

  emit(op: EditOp) { this.edit.emit(op); }

  setScalar(value: ScalarValue) {
    this.emit({ op: 'set_scalar', node_id: this.node().id, value });
  }

  changePkgSet(r: { set: string; attr: string; bare: boolean }, newSet: string) {
    if (newSet === r.set) return;
    const ns = this.withNamespace();
    const text = ns === newSet ? r.attr : `${newSet}.${r.attr}`;
    this.setScalar({ type: 'ref', value: text });
  }

  commitDraft() {
    const d = this.draft();
    if (d === null) return;
    this.draft.set(null);
    if (d === this.node().value) return;
    const n = this.node();
    let value: ScalarValue;
    switch (n.kind) {
      case 'int': {
        const v = Number(d);
        if (!Number.isInteger(v)) {
          if (/^[a-zA-Z_][\w.-]*$/.test(d.trim()) && d.includes('.')) { value = { type: 'ref', value: d.trim() }; break; } // e.g. vars.port
          this.snack.open(`"${d}" is not a whole number; the value was not changed`, 'OK', { duration: 5000 });
          return;
        }
        value = { type: 'int', value: v };
        break;
      }
      case 'float': {
        const v = Number(d);
        if (!Number.isFinite(v)) {
          if (/^[a-zA-Z_][\w.-]*$/.test(d.trim()) && d.includes('.')) { value = { type: 'ref', value: d.trim() }; break; }
          this.snack.open(`"${d}" is not a number; the value was not changed`, 'OK', { duration: 5000 });
          return;
        }
        value = { type: 'float', value: v };
        break;
      }
      case 'string':
        if (d.includes('${')) {
          // Interpolated: quote by hand (the scalar path would escape the ${).
          const quoted = '"' + d.replace(/\\/g, '\\\\').replace(/"/g, '\\"') + '"';
          this.emit({ op: 'set_raw', node_id: n.id, source: quoted });
          return;
        }
        value = { type: 'string', value: d }; break;
      case 'path': value = { type: 'path', value: d }; break;
      default: value = d === 'true' || d === 'false' ? { type: 'bool', value: d === 'true' } : { type: 'ref', value: d };
    }
    this.setScalar(value);
  }

  startCode() {
    const n = this.node();
    this.codeText = sliceBytes(this.source(), n.value_span.start, n.value_span.end);
    this.codeMode.set(true);
  }

  saveCode() {
    this.codeMode.set(false);
    this.emit({ op: 'set_raw', node_id: this.node().id, source: this.codeText });
  }

  remove() {
    if (this.parentKind() === 'list') this.emit({ op: 'list_remove', node_id: this.node().id });
    else this.emit({ op: 'remove_binding', node_id: this.node().id });
  }

  addBinding() {
    if (!this.newKey || !this.newValue) return;
    this.emit({ op: 'add_binding', node_id: this.node().id, key: this.newKey.trim(), source: this.newValue.trim() });
    this.newKey = '';
    this.newValue = '';
  }

  addItem() {
    {
      const raw = (this.newItem ?? '').trim();
      const existing = this.node().children.map(c => (c.value ?? c.source ?? '').trim());
      if (raw && existing.includes(raw)) { this.snack.open(`"${raw}" is already in this list`, 'OK', { duration: 4000 }); return; }
    }
    if (!this.newItem) return;
    this.emit({ op: 'list_append', node_id: this.node().id, source: this.newItem.trim() });
    this.newItem = '';
  }
}
