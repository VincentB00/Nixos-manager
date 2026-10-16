import { Injectable, inject, signal } from '@angular/core';
import { Subject } from 'rxjs';
import type { JobEvent, JobInfo, LogLine } from './generated';
import { ApiService } from './api.service';

export interface JobState {
  info: JobInfo;
  log: LogLine[];
}

/** Keeps a live mirror of daemon jobs through the /api/ws WebSocket. */
@Injectable({ providedIn: 'root' })
export class JobsService {
  private api = inject(ApiService);
  private socket?: WebSocket;
  private reconnectTimer?: ReturnType<typeof setTimeout>;

  readonly jobs = signal<Map<string, JobState>>(new Map());
  readonly connected = signal(false);
  /** Emits every event; components streaming a log subscribe to this. */
  readonly events$ = new Subject<JobEvent>();

  constructor() {
    this.connect();
    this.refresh();
  }

  refresh() {
    this.api.jobs().subscribe(list => {
      this.jobs.update(map => {
        const next = new Map(map);
        for (const info of list) {
          const existing = next.get(info.id);
          next.set(info.id, { info, log: existing?.log ?? [] });
        }
        return next;
      });
    });
  }

  /** Load the full log for one job (after a reload, the live map has none). */
  load(id: string) {
    this.api.job(id).subscribe(detail => {
      this.jobs.update(map => {
        const next = new Map(map);
        next.set(id, { info: detail.info, log: detail.log });
        return next;
      });
    });
  }

  private connect() {
    const proto = location.protocol === 'https:' ? 'wss' : 'ws';
    const url = `${proto}://${location.host}/api/ws`;
    try {
      this.socket = new WebSocket(url);
    } catch {
      this.scheduleReconnect();
      return;
    }
    this.socket.onopen = () => { this.connected.set(true); this.refresh(); };
    this.socket.onclose = () => { this.connected.set(false); this.scheduleReconnect(); };
    this.socket.onerror = () => { this.socket?.close(); };
    this.socket.onmessage = ev => {
      let event: JobEvent;
      try { event = JSON.parse(ev.data as string) as JobEvent; } catch { return; }
      this.apply(event);
      this.events$.next(event);
    };
  }

  private scheduleReconnect() {
    clearTimeout(this.reconnectTimer);
    this.reconnectTimer = setTimeout(() => this.connect(), 2000);
  }

  private apply(event: JobEvent) {
    this.jobs.update(map => {
      const next = new Map(map);
      switch (event.event) {
        case 'created':
          next.set(event.job.id, { info: event.job, log: [] });
          break;
        case 'log': {
          const j = next.get(event.job_id);
          if (j) next.set(event.job_id, { info: { ...j.info, log_lines: j.log.length + 1 }, log: [...j.log, event.line] });
          break;
        }
        case 'status': {
          const j = next.get(event.job_id);
          if (j) next.set(event.job_id, { ...j, info: { ...j.info, status: event.status, error: event.error } });
          break;
        }
      }
      return next;
    });
  }
}
