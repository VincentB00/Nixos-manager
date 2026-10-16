import { Component, computed, inject } from '@angular/core';
import { RouterLink } from '@angular/router';
import { MatButtonModule } from '@angular/material/button';
import { MatCardModule } from '@angular/material/card';
import { MatIconModule } from '@angular/material/icon';
import { MatTooltipModule } from '@angular/material/tooltip';
import { WorkspaceService } from '../../core/workspace.service';
import { MatDialog } from '@angular/material/dialog';
import { ConvertFlakeDialogComponent } from '../flake/convert-flake-dialog.component';
import type { FileSummary, ImportRef } from '../../api/generated';

interface TreeRow { file: string; depth: number; enabled: boolean; via: ImportRef['via']; exists: boolean; cyclic: boolean; summary?: FileSummary }

@Component({
  selector: 'app-overview',
  imports: [RouterLink, MatButtonModule, MatCardModule, MatIconModule, MatTooltipModule],
  styles: [`
    .cards { display: grid; grid-template-columns: repeat(auto-fit, minmax(260px, 1fr)); gap: 16px; margin-bottom: 16px; }
    .kv { display: grid; grid-template-columns: auto 1fr; gap: 4px 12px; font-size: .9em; margin: 0; }
    .kv dt { color: var(--mat-sys-on-surface-variant); }
    .kv dd { margin: 0; }
    .tree { font-size: .92em; }
    .row { display: flex; align-items: center; gap: 8px; padding: 3px 0; border-bottom: 1px solid var(--mat-sys-surface-container); }
    .row a { text-decoration: none; color: inherit; }
    .row a:hover { text-decoration: underline; }
    .row.off { opacity: .5; }
    .row .ns { display: flex; gap: 4px; flex-wrap: wrap; }
    .ns span { font-size: .72em; padding: 1px 6px; border-radius: 10px; background: var(--mat-sys-surface-container-high); }
    .tag { font-size: .72em; padding: 1px 6px; border-radius: 4px; border: 1px solid var(--mat-sys-outline-variant); }
    .tag.missing { color: var(--mat-sys-error); border-color: var(--mat-sys-error); }
    .desc { font-size: .8em; max-width: 30%; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
    .guide { line-height: 1.5; }
  `],
  template: `
    @if (ws.overview(); as ov) {
      <div class="cards">
        <mat-card appearance="outlined">
          <mat-card-header><mat-card-title>This configuration</mat-card-title></mat-card-header>
          <mat-card-content>
            <dl class="kv">
              <dt>Type</dt><dd>{{ ov.location.kind === 'flake' ? 'Flake (flake.nix)' : ov.location.kind === 'legacy' ? 'Classic (configuration.nix)' : 'Unknown' }}</dd>
              <dt>Entry file</dt><dd class="mono">{{ ov.location.entry_file ?? '—' }}</dd>
              <dt>Hosts</dt><dd>{{ ov.location.hosts.length ? ov.location.hosts.join(', ') : (ov.location.kind === 'legacy' ? 'this machine' : 'none found statically') }}</dd>
              <dt>Nix files</dt><dd>{{ ws.nixFiles().length }}</dd>
              <dt>Hardware config</dt><dd>{{ ov.location.has_hardware_config ? 'present' : 'missing here (copied from /etc/nixos on sync)' }}</dd>
              <dt>flake.lock</dt><dd>{{ ov.location.has_flake_lock ? 'present (stray: it belongs in /etc/nixos only and is removed when the folder is opened)' : 'kept in /etc/nixos only' }}</dd>
              <dt>Writable</dt><dd>{{ ov.location.writable ? 'yes' : 'no — edits will fail' }}</dd>
            </dl>
            @if (ov.location.kind === 'legacy') {
              <div style="margin-top:10px" class="flex">
                <a mat-stroked-button routerLink="../channels" queryParamsHandling="merge"><mat-icon>rss_feed</mat-icon> Channels</a>
                <button mat-flat-button (click)="convert()"><mat-icon>ac_unit</mat-icon> Switch to flakes</button>
              </div>
            } @else if (ov.location.kind === 'flake') {
              <div style="margin-top:10px"><a mat-stroked-button routerLink="../flake" queryParamsHandling="merge"><mat-icon>ac_unit</mat-icon> Flake settings</a></div>
            }
          </mat-card-content>
        </mat-card>
        <mat-card appearance="outlined">
          <mat-card-header><mat-card-title>This machine</mat-card-title></mat-card-header>
          <mat-card-content>
            <dl class="kv">
              <dt>Hostname</dt><dd>{{ ov.machine.hostname ?? '—' }}</dd>
              <dt>NixOS</dt><dd>{{ ov.machine.nixos_version ?? '—' }}</dd>
              <dt>Nix</dt><dd>{{ ov.machine.nix_version ?? '—' }}</dd>
            </dl>
          </mat-card-content>
        </mat-card>
        <mat-card appearance="outlined">
          <mat-card-header><mat-card-title>How to read the map</mat-card-title></mat-card-header>
          <mat-card-content class="guide muted">
            The entry file imports modules; each module sets options in a few areas (shown as chips). Greyed rows are commented out.
            Click a file to view and edit it.
          </mat-card-content>
        </mat-card>
      </div>

      <h3>Import map</h3>
      <div class="tree">
        @for (r of rows(); track $index) {
          <div class="row" [class.off]="!r.enabled" [style.padding-left.px]="r.depth * 22">
            <mat-icon [matTooltip]="r.via === 'imports' ? 'imported as a module' : 'referenced as a file'">{{ r.depth === 0 ? 'flag' : r.via === 'imports' ? 'subdirectory_arrow_right' : 'link' }}</mat-icon>
            @if (r.exists && r.file.endsWith('.nix')) {
              <a routerLink="../file" [queryParams]="{ f: r.file }" queryParamsHandling="merge" class="mono">{{ r.file }}</a>
            } @else { <span class="mono">{{ r.file }}</span> }
            @if (!r.enabled) { <span class="tag">disabled</span> }
            @if (!r.exists) { <span class="tag warning">missing</span> }
            @if (r.cyclic) { <span class="tag">already shown</span> }
            @if (r.summary && !r.summary.parse_ok) { <span class="tag missing">syntax error</span> }
            <span class="spacer"></span>
            @if (r.summary?.description) { <span class="muted desc">{{ r.summary!.description }}</span> }
            <span class="ns">@for (n of (r.summary?.namespaces ?? []).slice(0, 5); track n) { <span>{{ n }}</span> }</span>
          </div>
        }
      </div>

      @if (ov.unreachable.length) {
        <h3>Not imported anywhere</h3>
        <div class="muted" style="margin-bottom:8px">These .nix files exist but nothing reachable from {{ ov.location.entry_file }} imports them. Either they are optional modules or leftovers.</div>
        <div class="chip-row">
          @for (u of ov.unreachable; track u) { <a mat-stroked-button routerLink="../file" [queryParams]="{ f: u }" queryParamsHandling="merge" class="mono">{{ u }}</a> }
        </div>
      }
    }
  `,
})
export class OverviewComponent {
  ws = inject(WorkspaceService);
  private dialog = inject(MatDialog);

  convert() { this.dialog.open(ConvertFlakeDialogComponent, { width: '560px', data: { path: this.ws.path() } }); }

  rows = computed<TreeRow[]>(() => {
    const ov = this.ws.overview();
    if (!ov) return [];
    const map = this.ws.fileMap();
    const out: TreeRow[] = [];
    const seen = new Set<string>();
    const walk = (file: string, depth: number, enabled: boolean, via: ImportRef['via'], exists: boolean) => {
      const cyclic = seen.has(file);
      out.push({ file, depth, enabled, via, exists, cyclic, summary: map.get(file) });
      if (cyclic || !enabled || !exists) return;
      seen.add(file);
      const s = map.get(file);
      for (const imp of s?.imports ?? []) walk(imp.target, depth + 1, imp.enabled, imp.via, imp.exists);
    };
    for (const root of ov.roots) walk(root, 0, true, 'imports', true);
    return out;
  });
}
