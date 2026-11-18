import { TooltipAriaDirective } from '../../shared/tooltip-aria.directive';
import { Component, OnInit, computed, inject, signal } from '@angular/core';
import { ActivatedRoute, Router, RouterLink, RouterLinkActive, RouterOutlet } from '@angular/router';
import { MatButtonModule } from '@angular/material/button';
import { MatDialog, MatDialogModule } from '@angular/material/dialog';
import { MatIconModule } from '@angular/material/icon';
import { MatListModule } from '@angular/material/list';
import { MatMenuModule } from '@angular/material/menu';
import { MatProgressBarModule } from '@angular/material/progress-bar';
import { MatSidenavModule } from '@angular/material/sidenav';
import { MatToolbarModule } from '@angular/material/toolbar';
import { MatTooltipModule } from '@angular/material/tooltip';
import { MatBadgeModule } from '@angular/material/badge';
import { WorkspaceService } from '../../core/workspace.service';
import { JobsService } from '../../api/jobs.service';
import { ApiService, ApiFailure } from '../../api/api.service';
import { BuildDialogComponent } from '../build/build-dialog.component';
import { NewFileDialogComponent } from '../new-file/new-file-dialog.component';
import { FileTreeComponent } from './file-tree.component';
import { MatButtonToggleModule } from '@angular/material/button-toggle';
import type { FileSummary, SearchResult } from '../../api/generated';

/** Manual light/dark override; "system" follows prefers-color-scheme. */
export function applyTheme(t: 'system' | 'light' | 'dark') {
  const root = document.documentElement;
  if (t === 'system') root.removeAttribute('data-theme'); else root.setAttribute('data-theme', t);
}
try { applyTheme((localStorage.getItem('nixos-manager.theme') as 'system' | 'light' | 'dark') || 'system'); } catch { /* ignore */ }

@Component({
  selector: 'app-workspace',
  imports: [RouterOutlet, RouterLink, RouterLinkActive, MatToolbarModule, MatSidenavModule, MatListModule, MatIconModule, MatButtonModule, MatMenuModule, MatProgressBarModule, MatTooltipModule, TooltipAriaDirective, MatDialogModule, MatBadgeModule, MatButtonToggleModule, FileTreeComponent],
  styles: [`
    .pending { color: var(--nm-warning); margin-right: 6px; white-space: nowrap; }
    .search { display: flex; align-items: center; gap: 6px; padding: 4px 8px 4px 16px; }
    .search input { flex: 1; min-width: 0; background: var(--mat-sys-surface-container); border: 1px solid var(--mat-sys-outline-variant); border-radius: 16px; padding: 6px 10px; color: inherit; font: inherit; font-size: .9em; outline: none; }
    .search input:focus { border-color: var(--mat-sys-primary); }
    .file-item.hit { height: 30px; font-size: .82em; opacity: .9; }
    .file-item.hit .ln { display: inline-block; min-width: 2.6em; color: var(--mat-sys-on-surface-variant); font-family: monospace; }
    .file-item.hit [matListItemTitle] { font-family: monospace; white-space: nowrap; overflow: hidden; text-overflow: ellipsis; padding-left: 12px; }
    :host { display: flex; flex-direction: column; height: 100vh; }
    /* The toolbar wraps onto a second row instead of pushing Build off-screen. */
    mat-toolbar.mat-toolbar-single-row { gap: 4px 8px; height: auto; min-height: 64px; flex-wrap: wrap; justify-content: flex-end; padding: 4px 8px; }
    mat-toolbar .brand { color: inherit; text-decoration: none; white-space: nowrap; }
    /* basis 0: the path shrinks before anything wraps, and grows to at most 40vw */
    .path { font-size: 0.85em; opacity: .85; flex: 1 1 0; min-width: 0; max-width: 40vw; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
    .lbl { margin-left: 4px; }
    @media (max-width: 1000px) { .path { display: none; } }
    .narrow-only { display: none; }
    @media (max-width: 760px) {
      .lbl { display: none; }
      .narrow-hide { display: none !important; }
      .narrow-only { display: inline-flex; }
      mat-toolbar.mat-toolbar-single-row { --mdc-icon-button-state-layer-size: 40px; gap: 0 2px; padding: 4px; }
      mat-toolbar .mat-mdc-icon-button { padding: 8px; }
    }
    @media (max-width: 560px) { .brand { display: none; } }
    @media (max-width: 760px) {
      mat-toolbar .mat-mdc-button, mat-toolbar .mat-mdc-unelevated-button, mat-toolbar .mat-mdc-outlined-button { min-width: 0; padding: 0 10px; }
      mat-toolbar .mat-mdc-button > .mat-icon, mat-toolbar .mat-mdc-unelevated-button > .mat-icon, mat-toolbar .mat-mdc-outlined-button > .mat-icon { margin-right: 0; }
    }
    mat-sidenav-container { flex: 1; min-height: 0; }
    mat-sidenav { width: min(300px, 85vw); }
    .content { padding: clamp(8px, 2vw, 24px); max-width: 1400px; min-width: 0; }
    .file-item { font-size: 0.9em; }
    .file-item.disabled { opacity: .55; }
    .group { font-size: .75em; text-transform: uppercase; letter-spacing: .05em; padding: 12px 16px 4px; color: var(--mat-sys-on-surface-variant); }
    .dot { width: 8px; height: 8px; border-radius: 50%; display: inline-block; margin-right: 6px; background: var(--mat-sys-outline); }
    .dot.err { background: var(--mat-sys-error); }
    .running { color: var(--mat-sys-primary); }
    .modes { padding: 4px 12px 0; display: flex; align-items: center; gap: 8px; }
    .modes mat-button-toggle-group { --mat-standard-button-toggle-height: 28px; }
    .modes mat-icon { font-size: 18px; width: 18px; height: 18px; }
  `],
  template: `
    <mat-toolbar>
      <button mat-icon-button (click)="sidenavOpen.set(!sidenavOpen())" [matTooltip]="sidenavOpen() ? 'Hide the file list' : 'Show the file list'"><mat-icon>{{ sidenavOpen() ? 'menu_open' : 'menu' }}</mat-icon></button>
      <a mat-icon-button routerLink="/" class="narrow-hide" matTooltip="Choose another folder"><mat-icon>home</mat-icon></a>
      <a routerLink="overview" queryParamsHandling="merge" class="brand">Nixos-manager</a>
      <span class="path mono muted">{{ ws.path() }}</span>
      <span class="spacer"></span>
      @if (hosts().length > 0) {
        @if (ws.gitInfo(); as g) { @if (g.is_repo) {
          <button mat-button routerLink="git" queryParamsHandling="merge" [matTooltip]="'git branch · ' + (g.files.length ? g.files.length + ' uncommitted change(s)' : 'clean')"><mat-icon>fork_right</mat-icon><span class="lbl">{{ g.detached ? 'detached' : (g.branch ?? '?') }}</span>@if (g.files.length) { <span class="tag warning" style="margin-left:6px">{{ g.files.length }}</span> }</button>
        } }
        <button mat-button [matMenuTriggerFor]="hostMenu" [matTooltip]="'Host: ' + (ws.selectedHost() ?? 'none')"><mat-icon>dns</mat-icon><span class="lbl">{{ ws.selectedHost() ?? 'host' }}</span></button>
        <mat-menu #hostMenu="matMenu">
          @for (h of hosts(); track h) { <button mat-menu-item (click)="ws.selectHost(h)">{{ h }}</button> }
        </mat-menu>
      }
      <button mat-icon-button class="narrow-hide" [matMenuTriggerFor]="themeMenu" matTooltip="Appearance"><mat-icon>{{ theme() === 'dark' ? 'dark_mode' : theme() === 'light' ? 'light_mode' : 'brightness_auto' }}</mat-icon></button>
      <mat-menu #themeMenu="matMenu">
        <button mat-menu-item (click)="setTheme('system')"><mat-icon>brightness_auto</mat-icon> Follow the system</button>
        <button mat-menu-item (click)="setTheme('light')"><mat-icon>light_mode</mat-icon> Light</button>
        <button mat-menu-item (click)="setTheme('dark')"><mat-icon>dark_mode</mat-icon> Dark</button>
      </mat-menu>
      <button mat-icon-button class="narrow-hide" (click)="ws.reload()" matTooltip="Rescan folder"><mat-icon>refresh</mat-icon></button>
      <!-- narrow screens: the secondary actions move into one menu -->
      <button mat-icon-button class="narrow-only" [matMenuTriggerFor]="moreMenu" matTooltip="More"><mat-icon>more_vert</mat-icon></button>
      <mat-menu #moreMenu="matMenu">
        <button mat-menu-item (click)="ws.reload()"><mat-icon>refresh</mat-icon> Rescan folder</button>
        <button mat-menu-item [matMenuTriggerFor]="themeMenu"><mat-icon>contrast</mat-icon> Appearance</button>
        <a mat-menu-item routerLink="/"><mat-icon>home</mat-icon> Choose another folder</a>
      </mat-menu>
      <a mat-icon-button routerLink="jobs" queryParamsHandling="merge" matTooltip="Jobs">
        <mat-icon [matBadge]="runningCount() || null" [class.running]="runningCount() > 0">assignment</mat-icon>
      </a>
      @if (ws.buildStatus()?.changed_files?.length; as n) {
        <button mat-button class="pending" (click)="build()" [matTooltip]="'Changed since the last successful build (' + (ws.buildStatus()!.last_build_at ?? '').slice(0, 16).replace('T', ' ') + '): ' + ws.buildStatus()!.changed_files.join(', ') + '. Press Build to apply.'"><mat-icon>pending_actions</mat-icon> {{ n }}<span class="lbl">{{ n === 1 ? 'change' : 'changes' }} to apply</span></button>
      }
      <button mat-stroked-button (click)="validate()" matTooltip="Check syntax and evaluate the whole configuration without changing anything (no password needed)"><mat-icon>fact_check</mat-icon><span class="lbl">Validate</span></button>
      <button mat-flat-button (click)="build()" matTooltip="Build and apply"><mat-icon>build</mat-icon><span class="lbl">Build</span></button>
    </mat-toolbar>
    @if (ws.loading()) { <mat-progress-bar mode="indeterminate" /> }

    <mat-sidenav-container>
      <mat-sidenav [mode]="narrow() ? 'over' : 'side'" [opened]="sidenavOpen()" (closedStart)="sidenavOpen.set(false)" disableClose>
        <mat-nav-list>
          <a mat-list-item routerLink="overview" queryParamsHandling="merge" routerLinkActive="active">
            <mat-icon matListItemIcon>map</mat-icon><span matListItemTitle>Overview & map</span>
          </a>
          <a mat-list-item routerLink="secrets" queryParamsHandling="merge" routerLinkActive="active">
            <mat-icon matListItemIcon>key</mat-icon><span matListItemTitle>Secrets</span><span matListItemLine class="muted">encrypted logins for shares</span>
          </a>
          <a mat-list-item routerLink="system" queryParamsHandling="merge" routerLinkActive="active">
            <mat-icon matListItemIcon>history</mat-icon><span matListItemTitle>System</span><span matListItemLine class="muted">generations · roll back · disk space</span>
          </a>
          <a mat-list-item routerLink="remote" queryParamsHandling="merge" routerLinkActive="active">
            <mat-icon matListItemIcon>lan</mat-icon><span matListItemTitle>Remote</span><span matListItemLine class="muted">deploy targets · remote builders</span>
          </a>
          @if (ws.gitInfo()?.is_repo) {
            <a mat-list-item routerLink="git" queryParamsHandling="merge" routerLinkActive="active">
              <mat-icon matListItemIcon>commit</mat-icon><span matListItemTitle>Git</span><span matListItemLine class="muted">{{ gitLine() }}</span>
            </a>
          }
          @if (ws.overview()?.location?.kind === 'flake') {
            <a mat-list-item routerLink="flake" queryParamsHandling="merge" routerLinkActive="active">
              <mat-icon matListItemIcon>ac_unit</mat-icon><span matListItemTitle>Flake</span><span matListItemLine class="muted">inputs, package sets, global vars</span>
            </a>
          } @else if (ws.overview()?.location?.kind === 'legacy') {
            <a mat-list-item routerLink="channels" queryParamsHandling="merge" routerLinkActive="active">
              <mat-icon matListItemIcon>rss_feed</mat-icon><span matListItemTitle>Channels</span><span matListItemLine class="muted">package sources · switch to flakes</span>
            </a>
          }
          <div class="search">
            <mat-icon class="muted">search</mat-icon>
            <input #searchBox type="text" placeholder="Find in files… (name or text)" aria-label="Find in files" [value]="searchQ()" (input)="search(searchBox.value)" (keydown.escape)="$event.stopPropagation(); search(''); searchBox.value = ''" />
            @if (searchQ()) { <button mat-icon-button (click)="search(''); searchBox.value = ''" matTooltip="Clear"><mat-icon>close</mat-icon></button> }
          </div>
          @if (searchQ()) {
            <div class="group">{{ searching() ? 'Searching…' : plural(searchRes()?.files?.length ?? 0, 'file') + ' match' + ((searchRes()?.files?.length ?? 0) === 1 ? 'es' : '') }}{{ searchRes()?.truncated ? ' (more hits not shown)' : '' }}</div>
            @for (f of searchRes()?.files ?? []; track f.rel_path) {
              <a mat-list-item class="file-item" [routerLink]="f.rel_path.endsWith('flake.nix') ? 'flake' : 'file'" [queryParams]="{ f: f.rel_path, line: null }" queryParamsHandling="merge" [matTooltip]="f.rel_path" matTooltipPosition="right">
                <span matListItemTitle>{{ f.rel_path }}@if (f.name_match) { <span class="tag" style="margin-left:6px">name</span> }</span>
              </a>
              @for (m of f.matches; track m.line) {
                <a mat-list-item class="file-item hit" routerLink="file" [queryParams]="{ f: f.rel_path, line: m.line }" queryParamsHandling="merge" [matTooltip]="m.text" matTooltipPosition="right">
                  <span matListItemTitle><span class="ln">{{ m.line }}</span>{{ m.text }}</span>
                </a>
              }
            }
            @if (!searching() && !(searchRes()?.files?.length)) { <div class="muted" style="padding:8px 16px; font-size:.85em">Nothing found.</div> }
          } @else {
          <div class="modes">
            <mat-button-toggle-group [value]="viewMode()" (change)="setViewMode($event.value)" hideSingleSelectionIndicator>
              <mat-button-toggle value="list" matTooltip="Nix files only, flat"><mat-icon>view_list</mat-icon></mat-button-toggle>
              <mat-button-toggle value="tree" matTooltip="Every file and folder"><mat-icon>account_tree</mat-icon></mat-button-toggle>
            </mat-button-toggle-group>
            <span class="spacer"></span>
            <button mat-icon-button matTooltip="New module file" (click)="newFile()"><mat-icon>add</mat-icon></button>
          </div>
          @if (viewMode() === 'tree') {
            <div class="group">All files</div>
            <app-file-tree [files]="ws.overview()?.files ?? []" [dirs]="ws.overview()?.directories ?? []" [unreachable]="unreachableSet()" />
          } @else {
            <div class="group">Nix files</div>
            @for (f of ws.nixFiles(); track f.rel_path) {
              <a mat-list-item class="file-item" [class.disabled]="isUnreachable(f)" [routerLink]="f.kind === 'flake' ? 'flake' : 'file'" [queryParams]="{ f: f.rel_path }" queryParamsHandling="merge"
                 routerLinkActive="active" [matTooltip]="tooltip(f)" matTooltipPosition="right">
                <span matListItemTitle><span class="dot" [class.err]="!f.parse_ok"></span>{{ f.rel_path }}</span>
              </a>
            }
            @if (ws.otherFiles().length > 0) {
              <div class="group">Other files</div>
              @for (f of ws.otherFiles(); track f.rel_path) {
                <a mat-list-item class="file-item" [routerLink]="f.rel_path === 'secrets.age' ? 'secrets' : 'file'" [queryParams]="{ f: f.rel_path }" queryParamsHandling="merge" routerLinkActive="active">
                  <mat-icon matListItemIcon>{{ icon(f) }}</mat-icon>
                  <span matListItemTitle>{{ f.rel_path }}</span>
                </a>
              }
            }
          }
                  }
        </mat-nav-list>
      </mat-sidenav>
      <mat-sidenav-content>
        <div class="content">
          @if (ws.error()) { <div class="error-text">{{ ws.error() }}</div> }
          <router-outlet />
        </div>
      </mat-sidenav-content>
    </mat-sidenav-container>
  `,
})
export class WorkspaceComponent implements OnInit {
  ws = inject(WorkspaceService);
  private jobs = inject(JobsService);
  private api = inject(ApiService);
  private route = inject(ActivatedRoute);
  private router = inject(Router);
  private dialog = inject(MatDialog);

  hosts = computed(() => this.ws.overview()?.location.hosts ?? []);
  unreachableSet = computed(() => new Set(this.ws.overview()?.unreachable ?? []));
  viewMode = signal<'list' | 'tree'>(this.loadViewMode());

  private loadViewMode(): 'list' | 'tree' {
    try { return localStorage.getItem('nixos-manager.fileView') === 'tree' ? 'tree' : 'list'; } catch { return 'list'; }
  }
  setViewMode(m: 'list' | 'tree') {
    this.viewMode.set(m);
    try { localStorage.setItem('nixos-manager.fileView', m); } catch { /* private mode */ }
  }
  runningCount = computed(() => [...this.jobs.jobs().values()].filter(j => j.info.status === 'running' || j.info.status === 'pending').length);

  ngOnInit() {
    this.mq?.addEventListener('change', this.mqHandler);
    this.route.queryParamMap.subscribe(q => {
      const path = q.get('path');
      if (!path) { this.router.navigate(['/']); return; }
      const view = q.get('view');
      if (view === 'tree' || view === 'list') this.setViewMode(view);
      if (this.ws.path() !== path) this.ws.open(path);
      if (q.get('build')) {
        // "press Build" shortcut from a snackbar
        this.router.navigate([], { relativeTo: this.route, queryParams: { build: null }, queryParamsHandling: 'merge', replaceUrl: true });
        setTimeout(() => this.build(), 300);
      }
    });
  }

  /** Narrow screens get an overlay drawer with a toolbar toggle. */
  narrow = signal(typeof window !== 'undefined' && window.matchMedia('(max-width: 1000px)').matches);
  sidenavOpen = signal(!this.narrow());
  private mq = typeof window !== 'undefined' ? window.matchMedia('(max-width: 1000px)') : null;
  private mqHandler = (e: MediaQueryListEvent) => { this.narrow.set(e.matches); this.sidenavOpen.set(!e.matches); };
  searchQ = signal('');
  searchRes = signal<SearchResult | null>(null);
  searching = signal(false);
  private searchTimer: ReturnType<typeof setTimeout> | null = null;
  search(q: string) {
    this.searchQ.set(q);
    if (this.searchTimer) clearTimeout(this.searchTimer);
    if (!q.trim()) { this.searchRes.set(null); this.searching.set(false); return; }
    this.searching.set(true);
    this.searchTimer = setTimeout(() => {
      const path = this.ws.path();
      if (!path) return;
      this.api.search(path, q, 'both').subscribe({
        next: r => { if (r.query === q.trim()) { this.searchRes.set(r); this.searching.set(false); } },
        error: () => this.searching.set(false),
      });
    }, 250);
  }

  theme = signal<'system' | 'light' | 'dark'>((() => { try { return (localStorage.getItem('nixos-manager.theme') as 'system' | 'light' | 'dark') || 'system'; } catch { return 'system'; } })());
  setTheme(t: 'system' | 'light' | 'dark') {
    this.theme.set(t);
    try { localStorage.setItem('nixos-manager.theme', t); } catch { /* private mode */ }
    applyTheme(t);
  }
  plural(n: number, word: string): string { return `${n} ${word}${n === 1 ? '' : 's'}`; }
  gitLine(): string {
    const g = this.ws.gitInfo();
    if (!g?.is_repo) return '';
    const parts = [g.detached ? 'detached HEAD' : (g.branch ?? '?')];
    if (g.files.length) parts.push(`${g.files.length} uncommitted`);
    if (g.ahead) parts.push(`${g.ahead} to push`);
    if (g.behind) parts.push(`${g.behind} to pull`);
    return parts.join(' · ');
  }

  isUnreachable(f: FileSummary) {
    return this.ws.overview()?.unreachable.includes(f.rel_path) ?? false;
  }

  tooltip(f: FileSummary) {
    const parts: string[] = [];
    if (f.description) parts.push(f.description);
    if (!f.parse_ok) parts.push('Has syntax errors');
    if (this.isUnreachable(f)) parts.push('Not imported by anything reachable from the entry file');
    if (f.namespaces.length) parts.push(f.namespaces.slice(0, 6).join(', '));
    return parts.join('\n');
  }

  icon(f: FileSummary) {
    switch (f.kind) {
      case 'secret': return 'key';
      case 'script': return 'terminal';
      case 'flake-lock': return 'lock';
      case 'data': return 'data_object';
      default: return 'insert_drive_file';
    }
  }

  validate() {
    const path = this.ws.path();
    if (!path) return;
    this.api.validate(path, this.ws.selectedHost()).subscribe({
      next: job => this.router.navigate(['/w', 'jobs', job.id], { queryParams: { path } }),
      error: (e: ApiFailure) => this.ws.error.set(e.message),
    });
  }

  build() {
    this.dialog.open(BuildDialogComponent, { width: '720px', maxWidth: '95vw', data: { path: this.ws.path(), host: this.ws.selectedHost() } });
  }

  newFile() {
    this.dialog.open(NewFileDialogComponent, { width: '560px', data: { path: this.ws.path() } }).afterClosed().subscribe(created => {
      if (created) {
        this.ws.reload();
        this.router.navigate(['/w', 'file'], { queryParams: { path: this.ws.path(), f: created } });
      }
    });
  }
}
