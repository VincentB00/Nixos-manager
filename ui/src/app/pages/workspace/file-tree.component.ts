import { TooltipAriaDirective } from '../../shared/tooltip-aria.directive';
import { Component, ElementRef, computed, inject, input, signal, viewChild } from '@angular/core';
import { Router, RouterLink, RouterLinkActive } from '@angular/router';
import { NgTemplateOutlet } from '@angular/common';
import { MatIconModule } from '@angular/material/icon';
import { MatTooltipModule } from '@angular/material/tooltip';
import { MatMenuModule, MatMenuTrigger } from '@angular/material/menu';
import { MatButtonModule } from '@angular/material/button';
import { MatDialog } from '@angular/material/dialog';
import { MatSnackBar } from '@angular/material/snack-bar';
import { ApiService, ApiFailure } from '../../api/api.service';
import { WorkspaceService } from '../../core/workspace.service';
import { PromptDialogComponent, PromptData, PromptResult } from '../../shared/prompt-dialog.component';
import { NewFileDialogComponent } from '../new-file/new-file-dialog.component';
import type { FileSummary } from '../../api/generated';

export interface TreeNode {
  name: string;
  path: string;
  children: TreeNode[];
  file?: FileSummary;
  /** A folder that exists on disk but has no files (created in the UI). */
  isDir: boolean;
}

const ROOT: TreeNode = { name: '(root)', path: '', children: [], isDir: true };

/** Filesystem tree of every file in the configuration folder, with create /
 *  rename / duplicate / delete actions on a context menu. */
@Component({
  selector: 'app-file-tree',
  imports: [RouterLink, RouterLinkActive, NgTemplateOutlet, MatIconModule, MatTooltipModule, TooltipAriaDirective, MatMenuModule, MatButtonModule],
  styles: [`
    :host { display: block; font-size: .9em; padding: 0 8px 8px; }
    .row { display: flex; align-items: center; gap: 4px; padding: 2px 2px 2px 6px; border-radius: 6px; cursor: pointer; color: inherit; text-decoration: none; white-space: nowrap; }
    .row:hover { background: var(--mat-sys-surface-container); }
    .row.active { background: var(--mat-sys-secondary-container); color: var(--mat-sys-on-secondary-container); }
    .row.dim { opacity: .55; }
    .row mat-icon { font-size: 18px; width: 18px; height: 18px; flex: none; }
    .folder mat-icon.chev { color: var(--mat-sys-on-surface-variant); }
    .name { overflow: hidden; text-overflow: ellipsis; flex: 1; }
    .dot { width: 7px; height: 7px; border-radius: 50%; background: var(--mat-sys-error); flex: none; margin-left: 4px; }
    .kids { margin-left: 14px; border-left: 1px solid var(--mat-sys-surface-container-high); padding-left: 2px; }
    .more { opacity: 0; width: 24px; height: 24px; line-height: 24px; padding: 0; flex: none; }
    .more mat-icon { font-size: 18px; }
    .row:hover .more, .row:focus-within .more { opacity: 1; }
    .root-actions { display: flex; gap: 2px; padding: 0 0 4px 2px; }
    .anchor { position: fixed; width: 0; height: 0; }
    .mi-danger { color: var(--mat-sys-error); }
  `],
  template: `
    <div class="root-actions">
      <button mat-icon-button matTooltip="New folder at the top level" (click)="newFolder(root)"><mat-icon>create_new_folder</mat-icon></button>
      <button mat-icon-button matTooltip="New module file at the top level" (click)="newModule(root)"><mat-icon>note_add</mat-icon></button>
      <button mat-icon-button matTooltip="More actions for the top level" (click)="openMenu($event, root)"><mat-icon>more_horiz</mat-icon></button>
    </div>
    <ng-container *ngTemplateOutlet="level; context: { nodes: tree() }" />

    <ng-template #level let-nodes="nodes">
      @for (n of nodes; track n.path) {
        @if (n.isDir) {
          <div class="row folder" (click)="toggle(n.path)" (contextmenu)="openMenu($event, n)">
            <mat-icon class="chev">{{ isOpen(n.path) ? 'expand_more' : 'chevron_right' }}</mat-icon>
            <mat-icon>{{ isOpen(n.path) ? 'folder_open' : 'folder' }}</mat-icon>
            <span class="name">{{ n.name }}</span>
            <button mat-icon-button class="more" (click)="openMenu($event, n)" matTooltip="Folder actions"><mat-icon>more_vert</mat-icon></button>
          </div>
          @if (isOpen(n.path)) {
            <div class="kids">
              @if (n.children.length === 0) { <div class="row dim" style="cursor:default"><mat-icon style="visibility:hidden">x</mat-icon><span class="name muted">empty</span></div> }
              <ng-container *ngTemplateOutlet="level; context: { nodes: n.children }" />
            </div>
          }
        } @else {
          <a class="row" [class.dim]="unreachable().has(n.path)" [routerLink]="n.file?.kind === 'flake' ? 'flake' : (n.path === 'secrets.age' ? 'secrets' : 'file')" [queryParams]="{ f: n.path }" queryParamsHandling="merge"
             routerLinkActive="active" [matTooltip]="tip(n.file)" matTooltipPosition="right" (contextmenu)="openMenu($event, n)">
            <mat-icon style="visibility:hidden">chevron_right</mat-icon>
            <mat-icon>{{ icon(n.file) }}</mat-icon>
            <span class="name">{{ n.name }}</span>
            @if (n.file && !n.file.parse_ok) { <span class="dot" matTooltip="Syntax error"></span> }
            <button mat-icon-button class="more" (click)="openMenu($event, n)" matTooltip="File actions"><mat-icon>more_vert</mat-icon></button>
          </a>
        }
      }
    </ng-template>

    <div class="anchor" #anchor [style.left.px]="menuPos().x" [style.top.px]="menuPos().y" [matMenuTriggerFor]="menu"></div>
    <mat-menu #menu="matMenu">
      @if (target(); as t) {
        <div mat-menu-item disabled style="opacity:.8"><span class="mono">{{ t.path || '(top level)' }}</span></div>
        @if (t.isDir) {
          <button mat-menu-item (click)="newModule(t)"><mat-icon>note_add</mat-icon> New module file here</button>
          <button mat-menu-item (click)="newPlainFile(t)"><mat-icon>description</mat-icon> New empty file here</button>
          <button mat-menu-item (click)="newFolder(t)"><mat-icon>create_new_folder</mat-icon> New folder here</button>
          @if (t.path) {
            <button mat-menu-item (click)="rename(t)"><mat-icon>drive_file_rename_outline</mat-icon> Rename / move folder</button>
            <button mat-menu-item (click)="remove(t)"><mat-icon class="mi-danger">delete</mat-icon> Delete folder</button>
          }
        } @else {
          @if (t.path.endsWith('.nix') && !importedSomewhere(t.path)) {
            <button mat-menu-item (click)="importInto(t)"><mat-icon>add_link</mat-icon> Import from configuration.nix</button>
          }
          <button mat-menu-item (click)="rename(t)"><mat-icon>drive_file_rename_outline</mat-icon> Rename / move</button>
          <button mat-menu-item (click)="duplicate(t)"><mat-icon>content_copy</mat-icon> Duplicate</button>
          <button mat-menu-item (click)="remove(t)"><mat-icon class="mi-danger">delete</mat-icon> Delete</button>
        }
      }
    </mat-menu>
  `,
})
export class FileTreeComponent {
  files = input.required<FileSummary[]>();
  /** All directories, including empty ones (from discovery). */
  dirs = input<string[]>([]);
  unreachable = input<Set<string>>(new Set());

  private api = inject(ApiService);
  private ws = inject(WorkspaceService);
  private dialog = inject(MatDialog);
  private snack = inject(MatSnackBar);
  private router = inject(Router);
  private trigger = viewChild.required(MatMenuTrigger);
  private anchorEl = viewChild.required<ElementRef<HTMLElement>>('anchor');

  root = ROOT;
  private closed = signal<Set<string>>(new Set());
  /** Folders created in the UI that have no files yet (so they still show). */
  private extraDirs = signal<Set<string>>(new Set());
  target = signal<TreeNode | null>(null);
  menuPos = signal({ x: 0, y: 0 });

  tree = computed<TreeNode[]>(() => {
    const root: TreeNode = { name: '', path: '', children: [], isDir: true };
    const ensureDir = (parts: string[]) => {
      let cur = root;
      let acc = '';
      for (const part of parts) {
        acc = acc ? `${acc}/${part}` : part;
        let next = cur.children.find(c => c.name === part && c.isDir);
        if (!next) { next = { name: part, path: acc, children: [], isDir: true }; cur.children.push(next); }
        cur = next;
      }
      return cur;
    };
    for (const f of this.files()) {
      const parts = f.rel_path.split('/');
      const dir = ensureDir(parts.slice(0, -1));
      dir.children.push({ name: parts[parts.length - 1], path: f.rel_path, children: [], file: f, isDir: false });
    }
    for (const d of this.dirs()) ensureDir(d.split('/'));
    for (const d of this.extraDirs()) ensureDir(d.split('/'));
    const sort = (n: TreeNode) => {
      n.children.sort((a, b) => (b.isDir ? 1 : 0) - (a.isDir ? 1 : 0) || a.name.localeCompare(b.name));
      n.children.forEach(sort);
    };
    sort(root);
    return root.children;
  });

  isOpen(path: string) { return !this.closed().has(path); }
  toggle(path: string) {
    this.closed.update(s => { const n = new Set(s); n.has(path) ? n.delete(path) : n.add(path); return n; });
  }
  importedSomewhere(path: string) { return (this.ws.fileMap().get(path)?.imported_by.length ?? 0) > 0; }

  openMenu(ev: MouseEvent, node: TreeNode) {
    ev.preventDefault();
    ev.stopPropagation();
    this.target.set(node);
    this.menuPos.set({ x: ev.clientX, y: ev.clientY });
    this.trigger().closeMenu();
    setTimeout(() => this.trigger().openMenu());
  }

  // ── actions ──────────────────────────────────────────────────────────────

  private prompt(data: PromptData) {
    return this.dialog.open<PromptDialogComponent, PromptData, PromptResult>(PromptDialogComponent, { width: '480px', data }).afterClosed();
  }
  private fail(e: ApiFailure) { this.snack.open(e.message, 'Dismiss', { duration: 8000 }); }
  private done(msg: string) { this.snack.open(msg, undefined, { duration: 2500 }); this.ws.reload(); }
  private join(dir: string, name: string) { return dir ? `${dir}/${name}` : name; }
  private validName = (v: string) => (/[\\/]/.test(v) ? 'Use a single name without slashes' : v.startsWith('.') ? 'Hidden names are not shown in the tree' : null);

  newFolder(dir: TreeNode) {
    this.prompt({ title: 'New folder', label: 'Folder name', placeholder: 'modules', hint: `Inside ${dir.path || 'the top level'}`, validate: this.validName, okLabel: 'Create' })
      .subscribe(r => {
        if (!r) return;
        const rel = this.join(dir.path, r.value);
        this.api.mkdir(this.ws.path()!, rel).subscribe({
          next: () => { this.extraDirs.update(s => new Set(s).add(rel)); this.done(`Created folder ${rel}`); },
          error: e => this.fail(e),
        });
      });
  }

  newModule(dir: TreeNode) {
    this.dialog.open(NewFileDialogComponent, { width: '560px', data: { path: this.ws.path(), dir: dir.path } }).afterClosed().subscribe(created => {
      if (created) { this.ws.reload(); this.router.navigate(['/w', 'file'], { queryParams: { path: this.ws.path(), f: created }, queryParamsHandling: 'merge' }); }
    });
  }

  newPlainFile(dir: TreeNode) {
    this.prompt({ title: 'New empty file', label: 'File name', placeholder: 'notes.txt', hint: `Inside ${dir.path || 'the top level'}`, validate: this.validName, okLabel: 'Create' })
      .subscribe(r => {
        if (!r) return;
        const rel = this.join(dir.path, r.value);
        this.api.newFile({ path: this.ws.path()!, file: rel, source: '', import_into: null }).subscribe({
          next: () => { this.done(`Created ${rel}`); this.router.navigate(['/w', 'file'], { queryParams: { f: rel }, queryParamsHandling: 'merge' }); },
          error: e => this.fail(e),
        });
      });
  }

  rename(node: TreeNode) {
    const isNix = !node.isDir && node.path.endsWith('.nix');
    this.prompt({
      title: node.isDir ? 'Rename or move folder' : 'Rename or move file',
      label: 'New path (relative to the configuration folder)',
      value: node.path,
      hint: 'Use slashes to move it into another folder',
      checkbox: (isNix || node.isDir) ? { label: 'Update imports and references in other files', checked: true } : undefined,
      okLabel: 'Rename',
      validate: v => (v.startsWith('/') || v.split('/').includes('..') ? 'Must stay inside the configuration folder' : null),
    }).subscribe(r => {
      if (!r || r.value === node.path) return;
      this.api.rename(this.ws.path()!, node.path, r.value, r.checked).subscribe({
        next: res => {
          this.extraDirs.update(s => { const n = new Set(s); n.delete(node.path); return n; });
          const extra = res.updated_files.length ? ` · updated ${res.updated_files.length} file(s)` : '';
          this.done(`Renamed to ${res.to}${extra}`);
          if (!node.isDir) this.router.navigate(['/w', 'file'], { queryParams: { f: res.to }, queryParamsHandling: 'merge' });
        },
        error: e => this.fail(e),
      });
    });
  }

  duplicate(node: TreeNode) {
    const m = node.path.match(/^(.*?)(\.[^./]+)?$/);
    const suggested = `${m?.[1] ?? node.path}-copy${m?.[2] ?? ''}`;
    this.prompt({ title: 'Duplicate file', label: 'Path of the copy', value: suggested, okLabel: 'Duplicate' }).subscribe(r => {
      if (!r) return;
      this.api.duplicate(this.ws.path()!, node.path, r.value).subscribe({
        next: () => { this.done(`Copied to ${r.value}`); this.router.navigate(['/w', 'file'], { queryParams: { f: r.value }, queryParamsHandling: 'merge' }); },
        error: e => this.fail(e),
      });
    });
  }

  remove(node: TreeNode) {
    const importers = node.file?.imported_by ?? [];
    const warn = importers.length ? ` It is imported by ${importers.join(', ')}.` : '';
    this.prompt({
      title: node.isDir ? 'Delete folder?' : 'Delete file?',
      message: `${node.path} will be deleted from disk. There is no undo.${warn}`,
      checkbox: node.isDir ? { label: 'Also delete everything inside it' + (importers.length ? ' and remove the imports pointing into it' : ''), checked: false } : importers.length ? { label: `Also remove the import(s) in ${importers.join(', ')} so the configuration keeps working`, checked: true } : undefined,
      okLabel: 'Delete', danger: true,
    }).subscribe(r => {
      if (!r) return;
      this.api.delete(this.ws.path()!, node.path, node.isDir && r.checked, importers.length > 0 && r.checked).subscribe({
        next: () => {
          this.extraDirs.update(s => { const n = new Set(s); n.delete(node.path); return n; });
          this.done(`Deleted ${node.path}`);
          if (this.router.url.includes('/w/file') && decodeURIComponent(this.router.url).includes('f=' + node.path)) this.router.navigate(['/w', 'overview'], { queryParamsHandling: 'merge', queryParams: { f: null, line: null } });
        },
        error: e => this.fail(e),
      });
    });
  }

  importInto(node: TreeNode) {
    const candidates = this.ws.nixFiles().filter(f => f.kind === 'configuration' || f.imports.some(i => i.via === 'imports')).map(f => f.rel_path);
    const into = candidates.includes('configuration.nix') ? 'configuration.nix' : candidates[0];
    if (!into) { this.snack.open('No file with an imports list found', 'Dismiss', { duration: 5000 }); return; }
    this.prompt({ title: 'Add to imports', label: 'Import it from this file', value: into, hint: 'A .nix file whose imports list will get the new entry', okLabel: 'Add import' }).subscribe(r => {
      if (!r) return;

      this.api.file(this.ws.path()!, r.value).subscribe({
        next: v => {
          const imports = v.root.children.find(c => c.label === 'imports');
          if (!imports) { this.snack.open(`${r.value} has no imports list`, 'Dismiss', { duration: 5000 }); return; }
          const from = r.value.split('/').slice(0, -1);
          const to = node.path.split('/');
          let common = 0;
          while (common < from.length && common < to.length && from[common] === to[common]) common++;
          const literal = (from.length - common ? '../'.repeat(from.length - common) : './') + to.slice(common).join('/');
          this.api.edit(this.ws.path()!, r.value, v.version, { op: 'list_append', node_id: imports.id, source: literal }).subscribe({
            next: () => this.done(`Added ${literal} to ${r.value}`),
            error: e => this.fail(e),
          });
        },
        error: e => this.fail(e),
      });
    });
  }

  icon(f?: FileSummary) {
    switch (f?.kind) {
      case 'flake': return 'ac_unit';
      case 'flake-lock': return 'lock';
      case 'configuration': return 'flag';
      case 'hardware-configuration': return 'memory';
      case 'module': return 'extension';
      case 'secret': return 'key';
      case 'script': return 'terminal';
      case 'data': return 'data_object';
      default: return 'insert_drive_file';
    }
  }

  tip(f?: FileSummary) {
    if (!f) return '';
    const parts: string[] = [f.kind];
    if (f.description) parts.push(f.description);
    if (!f.parse_ok) parts.push('Has syntax errors');
    if (this.unreachable().has(f.rel_path)) parts.push('Not imported by anything reachable from the entry file');
    if (f.namespaces.length) parts.push(f.namespaces.slice(0, 6).join(', '));
    return parts.join('\n');
  }
}
