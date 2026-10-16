import { Component, computed, inject } from '@angular/core';
import { RouterLink } from '@angular/router';
import { SlicePipe } from '@angular/common';
import { MatListModule } from '@angular/material/list';
import { MatIconModule } from '@angular/material/icon';
import { JobsService } from '../../api/jobs.service';

@Component({
  selector: 'app-jobs',
  imports: [RouterLink, SlicePipe, MatListModule, MatIconModule],
  template: `
    <h2>Jobs</h2>
    @if (list().length === 0) { <div class="muted">No builds yet. Use the Build button in the toolbar.</div> }
    <mat-nav-list>
      @for (j of list(); track j.id) {
        <a mat-list-item [routerLink]="['./', j.id]" queryParamsHandling="merge">
          <mat-icon matListItemIcon [class.error-text]="j.status === 'failed'">{{ icon(j.status) }}</mat-icon>
          <span matListItemTitle>{{ j.title }}</span>
          <span matListItemLine class="muted">{{ j.status }} · started {{ j.started_at | slice:0:19 }}{{ j.error ? ' · ' + j.error : '' }}</span>
        </a>
      }
    </mat-nav-list>
  `,
})
export class JobsComponent {
  private jobs = inject(JobsService);
  list = computed(() => [...this.jobs.jobs().values()].map(j => j.info).sort((a, b) => b.started_at.localeCompare(a.started_at)));
  icon(status: string): string {
    const icons: Record<string, string> = { pending: 'schedule', running: 'autorenew', succeeded: 'check_circle', failed: 'error', cancelled: 'cancel' };
    return icons[status] ?? 'help';
  }
}
