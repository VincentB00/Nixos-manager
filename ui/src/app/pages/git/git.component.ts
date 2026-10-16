import { Component, OnInit, computed, inject, signal } from '@angular/core';
import { FormsModule } from '@angular/forms';
import { MatButtonModule } from '@angular/material/button';
import { MatCardModule } from '@angular/material/card';
import { MatCheckboxModule } from '@angular/material/checkbox';
import { MatDialog } from '@angular/material/dialog';
import { MatExpansionModule } from '@angular/material/expansion';
import { MatFormFieldModule } from '@angular/material/form-field';
import { MatIconModule } from '@angular/material/icon';
import { MatInputModule } from '@angular/material/input';
import { MatMenuModule } from '@angular/material/menu';
import { MatProgressBarModule } from '@angular/material/progress-bar';
import { MatSnackBar } from '@angular/material/snack-bar';
import { MatTooltipModule } from '@angular/material/tooltip';
import { ApiService, ApiFailure } from '../../api/api.service';
import { WorkspaceService } from '../../core/workspace.service';
import { PromptDialogComponent, PromptData, PromptResult } from '../../shared/prompt-dialog.component';
import type { GitBranch, GitFileStatus, GitInfo } from '../../api/generated';

/** Git for the working folder: branches, status with diffs, commit with a generated message, push / fetch / pull. */
@Component({
  selector: 'app-git',
  imports: [FormsModule, MatButtonModule, MatCardModule, MatCheckboxModule, MatExpansionModule, MatFormFieldModule, MatIconModule, MatInputModule, MatMenuModule, MatProgressBarModule, MatTooltipModule],
  styles: [`
    .head { display: flex; align-items: center; gap: 12px; margin-bottom: 12px; flex-wrap: wrap; }
    h2 { margin: 0; font-weight: 400; }
    .cards { display: grid; grid-template-columns: minmax(380px, 1.3fr) minmax(320px, 1fr); gap: 16px; align-items: start; }
    @media (max-width: 1000px) { .cards { grid-template-columns: 1fr; } }
    .row { display: flex; align-items: center; gap: 8px; padding: 4px 0; }
    .row .p { flex: 1; min-width: 0; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; font-family: monospace; font-size: .92em; }
    .state { font-size: .72em; padding: 1px 6px; border-radius: 4px; background: var(--mat-sys-surface-container-high); text-transform: uppercase; letter-spacing: .04em; flex: none; }
    .state.added, .state.untracked { color: var(--mat-sys-primary); }
    .state.deleted { color: var(--mat-sys-error); }
    .state.conflict { color: var(--mat-sys-error); font-weight: 600; }
    .diff { font-family: monospace; font-size: .8em; white-space: pre; overflow: auto; max-height: 320px; margin: 0; padding: 8px; background: var(--mat-sys-surface-container); border-radius: 6px; }
    .diff .add { color: #7bd88f; } .diff .del { color: #ff7b72; } .diff .hunk { color: var(--mat-sys-on-surface-variant); }
    .meta { font-size: .85em; color: var(--mat-sys-on-surface-variant); line-height: 1.6; }
    .errors { padding: 8px 12px; border-radius: 8px; background: var(--mat-sys-error-container); color: var(--mat-sys-on-error-container); margin-bottom: 12px; font-size: .9em; white-space: pre-wrap; }
    .out { font-family: monospace; font-size: .8em; white-space: pre-wrap; color: var(--mat-sys-on-surface-variant); margin-top: 8px; }
    .branch { display: inline-flex; align-items: center; gap: 6px; }
    textarea { font-family: monospace; }
  `],
  template: `
    <div class="head">
      <h2>Git</h2>
      @if (info(); as g) {
        @if (g.is_repo) {
          <button mat-stroked-button [matMenuTriggerFor]="branchMenu" (menuOpened)="loadBranches()" matTooltip="Switch branch"><mat-icon>fork_right</mat-icon> {{ g.detached ? 'detached HEAD' : (g.branch ?? '?') }}</button>
          <mat-menu #branchMenu="matMenu">
            @for (b of branches(); track b.name) {
              <button mat-menu-item (click)="switchTo(b)" [disabled]="b.current">
                <mat-icon>{{ b.current ? 'check' : b.remote_only ? 'cloud' : 'fork_right' }}</mat-icon>
                <span>{{ b.name }}</span>
                @if (b.remote_only) { <span class="muted" style="margin-left:6px">(remote)</span> }
              </button>
            }
            @if (!branches().length) { <button mat-menu-item disabled>loading…</button> }
          </mat-menu>
          <button mat-stroked-button (click)="newBranch()" matTooltip="Create a branch from the current commit and switch to it"><mat-icon>add</mat-icon> New branch</button>
          <span class="spacer"></span>
          <span class="meta">
            @if (g.upstream) { tracking <span class="mono">{{ g.upstream }}</span> · {{ g.ahead }} ahead · {{ g.behind }} behind }
            @else if (g.remotes.length) { not pushed yet (remote: {{ g.remotes[0].name }}) }
            @else { no remote configured }
          </span>
          <button mat-icon-button (click)="net('fetch')" matTooltip="Fetch: update the ahead/behind counts from the remote" [disabled]="busy() || !g.remotes.length"><mat-icon>sync</mat-icon></button>
          <button mat-icon-button (click)="net('pull')" matTooltip="Pull (fast-forward only)" [disabled]="busy() || !g.upstream || g.behind === 0"><mat-icon>download</mat-icon></button>
          <button mat-icon-button (click)="net('push')" matTooltip="Push the committed changes" [disabled]="busy() || !g.remotes.length || (g.upstream !== null && g.ahead === 0)"><mat-icon>upload</mat-icon></button>
        }
      }
    </div>
    @if (busy()) { <mat-progress-bar mode="indeterminate" style="margin-bottom:8px" /> }
    @if (error()) { <div class="errors">{{ error() }}</div> }

    @if (info(); as g) {
      @if (!g.is_repo) {
        <mat-card appearance="outlined"><mat-card-content>
          <p>This working folder is not a git repository. To keep a history of your configuration and sync it between machines, open a terminal in the folder and run:</p>
          <pre class="diff">git init
git add .
git commit -m "Initial configuration"</pre>
          <p class="meta">Then reopen this page. Adding a remote (GitHub, Gitea, …) lets you push from here.</p>
        </mat-card-content></mat-card>
      } @else {
        <div class="cards">
          <mat-card appearance="outlined">
            <mat-card-header><mat-card-title>Changes</mat-card-title>
              <mat-card-subtitle>{{ g.files.length ? g.files.length + ' file(s) changed since the last commit' : 'nothing to commit, working tree clean' }}</mat-card-subtitle></mat-card-header>
            <mat-card-content>
              @if (g.files.length) {
                <div class="row" style="border-bottom:1px solid var(--mat-sys-surface-container); margin-bottom:4px">
                  <mat-checkbox [checked]="allSelected()" [indeterminate]="someSelected() && !allSelected()" (change)="toggleAll($event.checked)">all</mat-checkbox>
                  <span class="meta">tick the files to include in the commit</span>
                </div>
                <mat-accordion displayMode="flat" multi>
                  @for (f of g.files; track f.path) {
                    <mat-expansion-panel (opened)="loadDiff(f)" class="mat-elevation-z0">
                      <mat-expansion-panel-header>
                        <mat-panel-title style="gap:8px; align-items:center">
                          <mat-checkbox [checked]="selected().has(f.path)" (click)="$event.stopPropagation()" (change)="toggle(f.path, $event.checked)" />
                          <span class="state" [class]="'state ' + f.state">{{ f.state }}</span>
                          <span class="p" [matTooltip]="f.path">{{ f.path }}@if (f.old_path) { <span class="muted"> ← {{ f.old_path }}</span> }</span>
                        </mat-panel-title>
                      </mat-expansion-panel-header>
                      @if (diffs().get(f.path); as d) {
                        <pre class="diff">@for (l of d.split('\\n'); track $index) {<span [class.add]="l.startsWith('+') && !l.startsWith('+++')" [class.del]="l.startsWith('-') && !l.startsWith('---')" [class.hunk]="l.startsWith('@@')">{{ l }}
</span>}</pre>
                      } @else { <div class="meta">loading diff…</div> }
                    </mat-expansion-panel>
                  }
                </mat-accordion>
              }
              @if (g.last_commit; as c) {
                <div class="meta" style="margin-top:10px">Last commit: <b>{{ c.subject }}</b> · {{ c.author }} · {{ c.date.slice(0, 16).replace('T', ' ') }} · <span class="mono">{{ c.hash.slice(0, 8) }}</span></div>
              }
            </mat-card-content>
          </mat-card>

          <mat-card appearance="outlined">
            <mat-card-header><mat-card-title>Commit</mat-card-title>
              <mat-card-subtitle>{{ g.user_name ? g.user_name + ' <' + g.user_email + '>' : 'name and e-mail not set yet' }}</mat-card-subtitle></mat-card-header>
            <mat-card-content>
              @if (!g.user_name || !g.user_email) {
                <div class="warning-box"><mat-icon>badge</mat-icon><span>Git needs a name and an e-mail to record who made each commit. <a href="javascript:void(0)" (click)="setIdentity()">Set them for this repository</a>.</span></div>
              }
              <mat-form-field appearance="outline" style="width:100%">
                <mat-label>Message</mat-label>
                <textarea matInput [ngModel]="message()" (ngModelChange)="message.set($event)" rows="7" [placeholder]="g.files.length ? 'What changed and why' : 'nothing to commit'"></textarea>
                <mat-hint>First line is the summary; the rest are details. Pre-filled from the changed settings; edit freely.</mat-hint>
              </mat-form-field>
              <div class="row" style="margin-top:10px; flex-wrap:wrap">
                <button mat-stroked-button (click)="suggest()" [disabled]="busy() || !g.files.length" matTooltip="Regenerate the message from the current changes"><mat-icon>auto_fix_high</mat-icon> Suggest message</button>
                <span class="spacer"></span>
                <button mat-stroked-button (click)="commit(false)" [disabled]="!canCommit()"><mat-icon>check</mat-icon> Commit</button>
                <button mat-flat-button (click)="commit(true)" [disabled]="!canCommit() || !g.remotes.length"><mat-icon>upload</mat-icon> Commit & push</button>
              </div>
              @if (g.remotes.length) {
                <div class="meta" style="margin-top:8px">Remote: @for (r of g.remotes; track r.name) { <span class="mono">{{ r.name }}</span> {{ r.url }} }</div>
              }
              @if (output()) { <div class="out">{{ output() }}</div> }
            </mat-card-content>
          </mat-card>
        </div>
      }
    }
  `,
})
export class GitComponent implements OnInit {
  private api = inject(ApiService);
  private ws = inject(WorkspaceService);
  private dialog = inject(MatDialog);
  private snack = inject(MatSnackBar);

  info = signal<GitInfo | null>(null);
  branches = signal<GitBranch[]>([]);
  selected = signal<Set<string>>(new Set());
  diffs = signal<Map<string, string>>(new Map());
  busy = signal(false);
  error = signal<string | null>(null);
  output = signal<string | null>(null);
  message = signal('');

  allSelected = computed(() => { const g = this.info(); return !!g && g.files.length > 0 && g.files.every(f => this.selected().has(f.path)); });
  someSelected = computed(() => this.selected().size > 0);

  private path(): string { return this.ws.path() ?? ''; }

  ngOnInit() { this.refresh(true); }

  refresh(resuggest = false) {
    this.busy.set(true);
    this.api.gitInfo(this.path()).subscribe({
      next: g => {
        this.info.set(g);
        this.ws.gitInfo.set(g);
        this.busy.set(false);
        // keep the selection in sync with what still exists; new files start selected
        const paths = new Set(g.files.map(f => f.path));
        const sel = new Set([...this.selected()].filter(p => paths.has(p)));
        for (const f of g.files) if (!this.diffs().has(f.path) && !this.selected().size) sel.add(f.path);
        if (!this.selected().size) for (const f of g.files) sel.add(f.path);
        this.selected.set(sel);
        this.diffs.set(new Map());
        if (resuggest && g.is_repo && g.files.length && !this.message().trim()) this.suggest();
      },
      error: (e: ApiFailure) => { this.busy.set(false); this.error.set(e.message); },
    });
  }

  loadBranches() {
    this.api.gitBranches(this.path()).subscribe({ next: b => this.branches.set(b), error: (e: ApiFailure) => this.error.set(e.message) });
  }

  loadDiff(f: GitFileStatus) {
    if (this.diffs().has(f.path)) return;
    this.api.gitDiff(this.path(), f.path).subscribe({
      next: d => { const m = new Map(this.diffs()); m.set(f.path, d || '(no textual difference)'); this.diffs.set(m); },
      error: (e: ApiFailure) => { const m = new Map(this.diffs()); m.set(f.path, 'cannot show diff: ' + e.message); this.diffs.set(m); },
    });
  }

  toggle(path: string, on: boolean) { const s = new Set(this.selected()); if (on) s.add(path); else s.delete(path); this.selected.set(s); }
  toggleAll(on: boolean) { this.selected.set(on ? new Set((this.info()?.files ?? []).map(f => f.path)) : new Set()); }

  canCommit(): boolean {
    const g = this.info();
    return !!g && g.is_repo && !this.busy() && this.selected().size > 0 && this.message().trim().length > 0;
  }

  suggest() {
    this.api.gitSuggest(this.path()).subscribe({
      next: s => { this.message.set(s.body ? `${s.subject}\n\n${s.body}` : s.subject); },
      error: (e: ApiFailure) => this.error.set(e.message),
    });
  }

  switchTo(b: GitBranch) {
    this.error.set(null);
    this.busy.set(true);
    this.api.gitSwitch(this.path(), b.name).subscribe({
      next: g => { this.busy.set(false); this.snack.open(`Switched to ${g.branch}`, undefined, { duration: 3000 }); this.message.set(''); this.afterChange(g); },
      error: (e: ApiFailure) => { this.busy.set(false); this.error.set(e.message); },
    });
  }

  newBranch() {
    this.dialog.open<PromptDialogComponent, PromptData, PromptResult>(PromptDialogComponent, { width: '480px', data: {
      title: 'New branch', message: `Created from the current commit on ${this.info()?.branch ?? 'HEAD'}; uncommitted changes come along.`,
      label: 'Branch name', placeholder: 'laptop', okLabel: 'Create',
    } }).afterClosed().subscribe(r => {
      if (!r?.value.trim()) return;
      this.busy.set(true);
      this.api.gitCreateBranch(this.path(), r.value.trim()).subscribe({
        next: g => { this.busy.set(false); this.snack.open(`On new branch ${g.branch}`, undefined, { duration: 3000 }); this.afterChange(g); },
        error: (e: ApiFailure) => { this.busy.set(false); this.error.set(e.message); },
      });
    });
  }

  setIdentity() {
    this.dialog.open<PromptDialogComponent, PromptData, PromptResult>(PromptDialogComponent, { width: '480px', data: { title: 'Your name for commits', label: 'Name', value: this.info()?.user_name ?? '', okLabel: 'Next' } })
      .afterClosed().subscribe(n => {
        if (!n?.value.trim()) return;
        this.dialog.open<PromptDialogComponent, PromptData, PromptResult>(PromptDialogComponent, { width: '480px', data: { title: 'Your e-mail for commits', label: 'E-mail', value: this.info()?.user_email ?? '', okLabel: 'Save' } })
          .afterClosed().subscribe(m => {
            if (!m?.value.trim()) return;
            this.api.gitIdentity(this.path(), n.value.trim(), m.value.trim()).subscribe({
              next: g => { this.info.set(g); this.snack.open('Identity saved for this repository', undefined, { duration: 3000 }); },
              error: (e: ApiFailure) => this.error.set(e.message),
            });
          });
      });
  }

  commit(push: boolean, password: string | null = null) {
    if (!this.canCommit()) return;
    const g = this.info()!;
    if (!g.user_name || !g.user_email) { this.setIdentity(); return; }
    this.error.set(null);
    this.output.set(null);
    this.busy.set(true);
    const all = this.allSelected();
    this.api.gitCommit(this.path(), this.message().trim(), all ? null : [...this.selected()], push, password).subscribe({
      next: r => {
        this.busy.set(false);
        this.message.set('');
        this.selected.set(new Set());
        if (r.output) this.output.set(r.output);
        this.snack.open(push ? 'Committed and pushed' : 'Committed', undefined, { duration: 3000 });
        this.afterChange(r.info);
      },
      error: (e: ApiFailure) => {
        this.busy.set(false);
        if (e.status === 403 && push) {
          // the commit is done; only the push needs credentials
          this.message.set('');
          this.selected.set(new Set());
          this.refresh();
          this.askPassword(e.message, pw => this.net('push', pw));
        } else {
          this.error.set(e.message);
        }
      },
    });
  }

  net(op: 'push' | 'fetch' | 'pull', password: string | null = null) {
    this.error.set(null);
    this.output.set(null);
    this.busy.set(true);
    const call = op === 'push' ? this.api.gitPush(this.path(), password) : op === 'fetch' ? this.api.gitFetch(this.path(), password) : this.api.gitPull(this.path(), password);
    call.subscribe({
      next: r => {
        this.busy.set(false);
        this.output.set(r.output || `${op}: done`);
        this.snack.open(op === 'push' ? 'Pushed' : op === 'pull' ? 'Pulled' : 'Fetched', undefined, { duration: 2500 });
        this.afterChange(r.info);
      },
      error: (e: ApiFailure) => {
        this.busy.set(false);
        if (e.status === 403) this.askPassword(e.message, pw => this.net(op, pw));
        else this.error.set(e.message);
      },
    });
  }

  private askPassword(why: string, then: (pw: string) => void) {
    this.dialog.open<PromptDialogComponent, PromptData, PromptResult>(PromptDialogComponent, { width: '560px', data: {
      title: 'The remote asked for a password',
      message: `${why}\n\nEnter the password, token or key passphrase for the remote. It is used for this one operation and never stored.`,
      label: 'Password / token / passphrase', password: true, okLabel: 'Retry',
    } }).afterClosed().subscribe(r => { if (r?.value) then(r.value); });
  }

  /** Branch switches and pulls change files on disk: reload the workspace too. */
  private afterChange(g: GitInfo) {
    this.info.set(g);
    this.ws.gitInfo.set(g);
    this.selected.set(new Set(g.files.map(f => f.path)));
    this.diffs.set(new Map());
    this.ws.reload();
  }
}
