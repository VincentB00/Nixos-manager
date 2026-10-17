import { TooltipAriaDirective } from '../../shared/tooltip-aria.directive';
import { Component, OnInit, computed, inject, signal } from '@angular/core';
import { RouterLink } from '@angular/router';
import { SlicePipe } from '@angular/common';
import { MatListModule } from '@angular/material/list';
import { MatIconModule } from '@angular/material/icon';
import { MatExpansionModule } from '@angular/material/expansion';
import { MatButtonModule } from '@angular/material/button';
import { MatDialog } from '@angular/material/dialog';
import { MatSnackBar } from '@angular/material/snack-bar';
import { MatTooltipModule } from '@angular/material/tooltip';
import { PromptDialogComponent, PromptData, PromptResult } from '../../shared/prompt-dialog.component';
import { JobsService } from '../../api/jobs.service';
import { ApiService } from '../../api/api.service';
import type { JobLogFile } from '../../api/generated';

@Component({
  selector: 'app-jobs',
  imports: [RouterLink, SlicePipe, MatListModule, MatIconModule, MatExpansionModule, MatButtonModule, MatTooltipModule, TooltipAriaDirective],
  styles: [`
    mat-expansion-panel-header { height: auto !important; min-height: 48px; padding-top: 6px; padding-bottom: 6px; }
    mat-expansion-panel-header mat-panel-title { white-space: normal; word-break: break-word; min-width: 0; }
    mat-expansion-panel-header mat-panel-description { flex: 0 0 auto; margin-left: 12px; white-space: nowrap; }
  `],
  template: `
    <h2>Jobs</h2>
    @if (list().length === 0) { <div class="muted">No jobs yet. Validate or Build in the toolbar starts one.</div> }
    <mat-nav-list>
      @for (j of list(); track j.id) {
        <a mat-list-item [routerLink]="['./', j.id]" queryParamsHandling="merge">
          <mat-icon matListItemIcon [class.error-text]="j.status === 'failed'">{{ icon(j.status) }}</mat-icon>
          <span matListItemTitle>{{ j.title }}</span>
          <span matListItemLine class="muted">{{ j.status }} · started {{ j.started_at | slice:0:19 }}{{ j.error ? ' · ' + j.error : '' }}</span>
        </a>
      }
    </mat-nav-list>

    @if (history().length) {
      <div style="display:flex; align-items:center; gap:12px; margin-top:24px">
        <h3 style="font-weight:400; margin:0">Earlier logs on disk</h3>
        <span class="muted" style="font-size:.85em">{{ history().length }} file(s) · {{ totalSize() }}</span>
        <span class="spacer"></span>
        <button mat-stroked-button (click)="clearAll()"><mat-icon>delete_sweep</mat-icon> Clear all logs</button>
      </div>
      <div class="muted" style="font-size:.85em; margin:4px 0 8px">Every build log is also saved under ~/.local/state/nixos-manager/logs, so it survives a reboot or a restart of the app. Logs of running jobs are never deleted.</div>
      <mat-accordion displayMode="flat">
        @for (h of history(); track h.name) {
          <mat-expansion-panel class="mat-elevation-z0" (opened)="load(h)">
            <mat-expansion-panel-header>
              <mat-panel-title style="gap:8px"><mat-icon [class.error-text]="h.status?.startsWith('failed')">{{ icon(h.status?.split(' ')[0] ?? 'running') }}</mat-icon> {{ h.title }}</mat-panel-title>
              <mat-panel-description>{{ h.status ?? 'no end recorded (power cut or still running)' }} · {{ (h.started_at ?? '') | slice:0:19 }}</mat-panel-description>
            </mat-expansion-panel-header>
            <div style="display:flex; justify-content:flex-end; margin-bottom:4px"><button mat-button (click)="remove(h)" matTooltip="Delete this log file"><mat-icon>delete</mat-icon> Delete</button></div>
            <pre style="font-family:monospace; font-size:.8em; white-space:pre-wrap; max-height:420px; overflow:auto; margin:0">{{ texts().get(h.name) ?? 'loading…' }}</pre>
          </mat-expansion-panel>
        }
      </mat-accordion>
    }
  `,
})
export class JobsComponent implements OnInit {
  private jobs = inject(JobsService);
  private api = inject(ApiService);
  history = signal<JobLogFile[]>([]);
  texts = signal<Map<string, string>>(new Map());
  private dialog = inject(MatDialog);
  private snack = inject(MatSnackBar);
  ngOnInit() { this.loadHistory(); }
  loadHistory() { this.api.jobHistory().subscribe({ next: h => this.history.set(h), error: () => {} }); }
  totalSize(): string {
    const b = this.history().reduce((a, h) => a + Number(h.size), 0);
    return b < 1024 ? `${b} B` : b < 1048576 ? `${(b / 1024).toFixed(1)} KB` : `${(b / 1048576).toFixed(1)} MB`;
  }
  remove(h: JobLogFile) {
    this.dialog.open<PromptDialogComponent, PromptData, PromptResult>(PromptDialogComponent, { width: '420px', data: { title: 'Delete this log?', message: h.title, okLabel: 'Delete', danger: true } }).afterClosed().subscribe(r => { if (r) this.removeNow(h); });
  }
  private removeNow(h: JobLogFile) {
    this.api.jobHistoryDelete(h.name).subscribe({
      next: () => { this.snack.open('Log deleted', undefined, { duration: 2500 }); this.loadHistory(); },
      error: e => this.snack.open(e.message, 'OK', { duration: 5000 }),
    });
  }
  clearAll() {
    this.dialog.open<PromptDialogComponent, PromptData, PromptResult>(PromptDialogComponent, { width: '460px', data: { title: 'Delete all saved logs?', message: `${this.history().length} file(s), ${this.totalSize()}. Logs of running jobs are kept.`, okLabel: 'Delete all', danger: true } })
      .afterClosed().subscribe(r => {
        if (!r) return;
        this.api.jobHistoryClear().subscribe({
          next: res => { this.snack.open(`${res.deleted} log(s) deleted`, undefined, { duration: 3000 }); this.texts.set(new Map()); this.loadHistory(); },
          error: e => this.snack.open(e.message, 'OK', { duration: 5000 }),
        });
      });
  }
  load(h: JobLogFile) {
    if (this.texts().has(h.name)) return;
    this.api.jobHistoryRead(h.name).subscribe({ next: t => { const m = new Map(this.texts()); m.set(h.name, t); this.texts.set(m); }, error: e => { const m = new Map(this.texts()); m.set(h.name, 'cannot read: ' + e.message); this.texts.set(m); } });
  }
  list = computed(() => [...this.jobs.jobs().values()].map(j => j.info).sort((a, b) => b.started_at.localeCompare(a.started_at)));
  icon(status: string): string {
    const icons: Record<string, string> = { pending: 'schedule', running: 'autorenew', succeeded: 'check_circle', failed: 'error', cancelled: 'cancel' };
    return icons[status] ?? 'help';
  }
}
