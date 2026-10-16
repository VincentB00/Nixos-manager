import { AfterViewInit, Component, ElementRef, OnDestroy, computed, effect, inject, input, signal, viewChild } from '@angular/core';
import { MatIconModule } from '@angular/material/icon';
import { MatDialog } from '@angular/material/dialog';
import { ApiService, ApiFailure } from '../../api/api.service';
import { PromptDialogComponent, PromptData, PromptResult } from '../../shared/prompt-dialog.component';
import { MatProgressBarModule } from '@angular/material/progress-bar';
import { MatButtonModule } from '@angular/material/button';
import { RouterLink } from '@angular/router';
import { Terminal } from '@xterm/xterm';
import { FitAddon } from '@xterm/addon-fit';
import { JobsService } from '../../api/jobs.service';
import type { LogLine } from '../../api/generated';

@Component({
  selector: 'app-job-log',
  imports: [MatIconModule, MatProgressBarModule, MatButtonModule, RouterLink],
  styles: [`
    .head { display: flex; align-items: center; gap: 12px; margin-bottom: 8px; }
    .term { height: calc(100vh - 220px); min-height: 300px; background: #1e1e1e; border-radius: 8px; padding: 8px; }
    .status { padding: 2px 10px; border-radius: 12px; font-size: .85em; background: var(--mat-sys-surface-container-high); }
    .status.failed { background: var(--mat-sys-error-container); color: var(--mat-sys-on-error-container); }
    .status.succeeded { background: var(--mat-sys-primary-container); color: var(--mat-sys-on-primary-container); }
  `],
  template: `
    <div class="head">
      <a mat-icon-button routerLink="../" queryParamsHandling="merge"><mat-icon>arrow_back</mat-icon></a>
      <h2 style="margin:0">{{ job()?.info.title ?? 'Job' }}</h2>
      <span [class]="'status ' + (job()?.info.status ?? '')">{{ job()?.info.status }}</span>
      @if (job()?.info.error) { <span class="error-text">{{ job()?.info.error }}</span> }
      @if (job()?.info.log_file; as f) { <span class="muted" style="font-size:.8em" [title]="f">log saved on disk</span> }
      <span class="spacer"></span>
      @if (job()?.info.status === 'running' || job()?.info.status === 'pending') {
        <button mat-stroked-button color="warn" (click)="stop()" [disabled]="stopping()"><mat-icon>stop_circle</mat-icon> {{ stopping() ? 'Stopping…' : 'Stop' }}</button>
      }
    </div>
    @if (job()?.info.status === 'running' || job()?.info.status === 'pending') { <mat-progress-bar mode="indeterminate" /> }
    <div #term class="term"></div>
  `,
})
export class JobLogComponent implements AfterViewInit, OnDestroy {
  id = input.required<string>();
  private jobs = inject(JobsService);
  private api = inject(ApiService);
  private dialog = inject(MatDialog);
  stopping = signal(false);

  stop() {
    const kind = this.job()?.info.kind ?? '';
    const msg = kind === 'deploy'
      ? 'Stopping while the system is being evaluated or built is safe: nothing has been applied yet. Stopping during activation (the "switching" lines near the end) can leave the machine half-switched until the next build. Temporary trust granted on a target is still removed.'
      : 'The running command and everything it started are terminated.';
    this.dialog.open<PromptDialogComponent, PromptData, PromptResult>(PromptDialogComponent, { width: '520px', data: { title: 'Stop this job?', message: msg, okLabel: 'Stop', danger: true } })
      .afterClosed().subscribe(r => {
        if (!r) return;
        this.stopping.set(true);
        this.api.stopJob(this.id()).subscribe({ next: () => {}, error: (e: ApiFailure) => { this.stopping.set(false); this.terminal?.writeln(`\x1b[31m${e.message}\x1b[0m`); } });
      });
  }
  private termEl = viewChild.required<ElementRef<HTMLDivElement>>('term');
  private terminal?: Terminal;
  private fit?: FitAddon;
  private written = 0;
  private resizeObs?: ResizeObserver;

  job = computed(() => this.jobs.jobs().get(this.id()));

  constructor() {
    effect(() => {
      const j = this.job();
      if (!j || !this.terminal) return;
      if (j.log.length < this.written) { this.terminal.clear(); this.written = 0; }
      for (let i = this.written; i < j.log.length; i++) this.write(j.log[i]);
      this.written = j.log.length;
    });
    effect(() => { this.jobs.load(this.id()); this.written = 0; this.terminal?.clear(); });
  }

  ngAfterViewInit() {
    this.terminal = new Terminal({ convertEol: true, fontSize: 13, scrollback: 20000, theme: { background: '#1e1e1e' }, disableStdin: true });
    this.fit = new FitAddon();
    this.terminal.loadAddon(this.fit);
    this.terminal.open(this.termEl().nativeElement);
    this.fit.fit();
    this.resizeObs = new ResizeObserver(() => this.fit?.fit());
    this.resizeObs.observe(this.termEl().nativeElement);
    const j = this.job();
    if (j) { for (const l of j.log) this.write(l); this.written = j.log.length; }
  }

  private write(l: LogLine) {
    if (!this.terminal) return;
    if (l.stream === 'system') this.terminal.writeln(`\x1b[36m${l.text}\x1b[0m`);
    else if (l.stream === 'stderr') this.terminal.writeln(`\x1b[33m${l.text}\x1b[0m`);
    else this.terminal.writeln(l.text);
  }

  ngOnDestroy() {
    this.resizeObs?.disconnect();
    this.terminal?.dispose();
  }
}
