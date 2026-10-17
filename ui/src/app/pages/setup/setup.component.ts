import { TooltipAriaDirective } from '../../shared/tooltip-aria.directive';
import { MatDialog } from '@angular/material/dialog';
import { PromptDialogComponent, PromptData, PromptResult } from '../../shared/prompt-dialog.component';
import { Component, OnInit, inject, signal } from '@angular/core';
import { Router } from '@angular/router';
import { FormsModule } from '@angular/forms';
import { SlicePipe } from '@angular/common';
import { MatButtonModule } from '@angular/material/button';
import { MatCardModule } from '@angular/material/card';
import { MatFormFieldModule } from '@angular/material/form-field';
import { MatIconModule } from '@angular/material/icon';
import { MatInputModule } from '@angular/material/input';
import { MatListModule } from '@angular/material/list';
import { MatProgressBarModule } from '@angular/material/progress-bar';
import { MatTooltipModule } from '@angular/material/tooltip';
import { ApiService, ApiFailure } from '../../api/api.service';
import { WorkspaceService } from '../../core/workspace.service';
import type { BrowseResult, LocationSuggestion, RecentLocation } from '../../api/generated';

@Component({
  selector: 'app-setup',
  imports: [FormsModule, SlicePipe, MatButtonModule, MatCardModule, MatFormFieldModule, MatIconModule, MatInputModule, MatListModule, MatProgressBarModule, MatTooltipModule, TooltipAriaDirective],
  styles: [`
    :host { display: block; min-height: 100%; padding: 32px 16px; box-sizing: border-box; }
    .wrap { max-width: 880px; margin: 0 auto; display: grid; gap: 16px; min-width: 0; }
    .wrap > * { min-width: 0; }
    .mono { overflow-wrap: anywhere; }
    h1 { font-weight: 400; margin: 0 0 4px; }
    .grid { display: grid; grid-template-columns: minmax(0, 1fr) minmax(0, 1fr); gap: 16px; }
    .grid > * { min-width: 0; }
    @media (max-width: 760px) { .grid { grid-template-columns: 1fr; } }
    .path-row { display: flex; gap: 8px; align-items: center; }
    .path-row mat-form-field { flex: 1; min-width: 0; }
    .browser { max-height: 320px; overflow: auto; border: 1px solid var(--mat-sys-outline-variant); border-radius: 8px; }
    .entry { cursor: pointer; }
    .entry:hover { background: var(--mat-sys-surface-container); }
    .crumbs { display: flex; gap: 2px; align-items: center; flex-wrap: wrap; font-size: 0.9em; }
    .crumbs button { min-width: 0; padding: 0 6px; }
    .kind { font-size: 0.75em; text-transform: uppercase; letter-spacing: .04em; }
  `],
  template: `
    <div class="wrap">
      <div>
        <h1>Nixos-manager</h1>
        <div class="muted">Pick the folder that holds your NixOS configuration (a flake.nix or configuration.nix).</div>
      </div>

      @if (error()) { <div class="error-text">{{ error() }}</div> }

      <mat-card appearance="outlined">
        <mat-card-content>
          <div class="path-row">
            <mat-form-field appearance="outline" subscriptSizing="dynamic">
              <mat-label>Configuration folder</mat-label>
              <input matInput [(ngModel)]="path" (keyup.enter)="open(path)" placeholder="/etc/nixos or ~/nixos" class="mono" />
            </mat-form-field>
            <button mat-stroked-button (click)="browse(path)"><mat-icon>folder_open</mat-icon> Browse</button>
            <button mat-flat-button (click)="open(path)" [disabled]="!path || opening()"><mat-icon>login</mat-icon> Open</button>
          </div>
          @if (opening()) { <mat-progress-bar mode="indeterminate" /> }
        </mat-card-content>
      </mat-card>

      <div class="grid">
        <mat-card appearance="outlined">
          <mat-card-header><mat-card-title>Recent</mat-card-title></mat-card-header>
          <mat-card-content>
            @if (recent().length === 0) { <div class="muted">Nothing opened yet.</div> }
            <mat-nav-list>
              @for (r of recent(); track r.path) {
                <a mat-list-item (click)="open(r.path)">
                  <mat-icon matListItemIcon>history</mat-icon>
                  <span matListItemTitle class="mono">{{ r.path }}</span>
                  <span matListItemLine class="muted">{{ r.host ? 'host ' + r.host + ' · ' : '' }}{{ r.last_opened | slice:0:16 }}</span>
                  <button mat-icon-button matListItemMeta matTooltip="Forget" (click)="forget(r, $event)"><mat-icon>close</mat-icon></button>
                </a>
              }
            </mat-nav-list>
          </mat-card-content>
        </mat-card>

        <mat-card appearance="outlined">
          <mat-card-header><mat-card-title>Found on this machine</mat-card-title></mat-card-header>
          <mat-card-content>
            @if (scanning()) { <mat-progress-bar mode="indeterminate" /> }
            @else if (suggestions().length === 0) { <div class="muted">No flake.nix or configuration.nix found in the usual places.</div> }
            <mat-nav-list>
              @for (s of suggestions(); track s.path) {
                <a mat-list-item (click)="open(s.path)">
                  <mat-icon matListItemIcon>{{ s.kind === 'flake' ? 'ac_unit' : 'description' }}</mat-icon>
                  <span matListItemTitle class="mono">{{ s.path }}</span>
                  <span matListItemLine class="muted"><span class="kind">{{ s.kind }}</span> · {{ s.reason }}</span>
                  @if (s.path === '/etc/nixos') { <button mat-stroked-button matListItemMeta (click)="startFromSystem($event)" matTooltip="Copy /etc/nixos into a folder in your home that you can edit; Build copies it back and rebuilds"><mat-icon>content_copy</mat-icon> Start from this</button> }
                </a>
              }
            </mat-nav-list>
          </mat-card-content>
        </mat-card>
      </div>

      @if (listing(); as l) {
        <mat-card appearance="outlined">
          <mat-card-header>
            <mat-card-title>Browse</mat-card-title>
            <mat-card-subtitle>
              <div class="crumbs">
                @for (c of crumbs(); track c.path) {
                  <button mat-button (click)="browse(c.path)">{{ c.name }}</button><span class="muted">/</span>
                }
              </div>
            </mat-card-subtitle>
          </mat-card-header>
          <mat-card-content>
            @if (l.looks_like_config) {
              <div class="flex" style="margin-bottom:8px">
                <mat-icon color="primary">check_circle</mat-icon>
                <span>This folder looks like a NixOS configuration.</span>
                <span class="spacer"></span>
                <button mat-flat-button (click)="open(l.path)">Open this folder</button>
              </div>
            }
            <div class="browser">
              <mat-list>
                @if (l.parent) {
                  <mat-list-item class="entry" (click)="browse(l.parent!)"><mat-icon matListItemIcon>arrow_upward</mat-icon><span matListItemTitle>..</span></mat-list-item>
                }
                @for (e of l.entries; track e.path) {
                  <mat-list-item class="entry" (click)="e.is_dir ? browse(e.path) : null">
                    <mat-icon matListItemIcon>{{ e.is_dir ? 'folder' : 'description' }}</mat-icon>
                    <span matListItemTitle>{{ e.name }}</span>
                  </mat-list-item>
                }
              </mat-list>
            </div>
          </mat-card-content>
        </mat-card>
      }
    </div>
  `,
})
export class SetupComponent implements OnInit {
  private dialog = inject(MatDialog);
  private api = inject(ApiService);
  private router = inject(Router);
  private ws = inject(WorkspaceService);

  path = '';
  recent = signal<RecentLocation[]>([]);
  suggestions = signal<LocationSuggestion[]>([]);
  listing = signal<BrowseResult | null>(null);
  scanning = signal(true);
  opening = signal(false);
  error = signal<string | null>(null);
  crumbs = signal<{ name: string; path: string }[]>([]);

  ngOnInit() {
    this.ws.close();
    this.api.state().subscribe(s => {
      this.recent.set(s.recent_locations);
      if (s.last_location) this.path = s.last_location;
    });
    this.api.suggestions().subscribe({
      next: s => { this.suggestions.set(s); this.scanning.set(false); },
      error: () => this.scanning.set(false),
    });
  }

  browse(path: string) {
    this.error.set(null);
    this.api.browse(path || undefined).subscribe({
      next: l => {
        this.listing.set(l);
        this.path = l.path;
        const parts = l.path.split('/').filter(Boolean);
        const crumbs = [{ name: '/', path: '/' }];
        let acc = '';
        for (const p of parts) { acc += '/' + p; crumbs.push({ name: p, path: acc }); }
        this.crumbs.set(crumbs);
      },
      error: (e: ApiFailure) => this.error.set(e.message),
    });
  }

  open(path: string) {
    if (!path) return;
    this.opening.set(true);
    this.error.set(null);
    this.api.openLocation(path).subscribe({
      next: info => {
        this.opening.set(false);
        this.router.navigate(['/w', 'overview'], { queryParams: { path: info.path } });
      },
      error: (e: ApiFailure) => { this.opening.set(false); this.error.set(e.message); },
    });
  }

  startFromSystem(ev: Event) {
    ev.stopPropagation();
    this.dialog.open<PromptDialogComponent, PromptData, PromptResult>(PromptDialogComponent, { width: '560px', data: {
      title: 'Start from this machine\'s configuration',
      message: 'The live configuration in /etc/nixos is owned by root, so it cannot be edited directly. It is copied into the folder below (flake.lock stays in /etc/nixos). You edit the copy; Build copies it back and rebuilds the system.',
      label: 'Folder for the copy', value: '~/nixos', okLabel: 'Copy and open', checkbox: { label: 'Make it a git repository (recommended: history and undo)', checked: true },
    } }).afterClosed().subscribe(r => {
      if (!r?.value.trim()) return;
      this.opening.set(true);
      this.api.startFromSystem(r.value.trim(), r.checked).subscribe({
        next: info => { this.opening.set(false); this.router.navigate(['/w', 'overview'], { queryParams: { path: info.path } }); },
        error: (e: ApiFailure) => { this.opening.set(false); this.error.set(e.message); },
      });
    });
  }

  forget(r: RecentLocation, ev: Event) {
    ev.stopPropagation();
    this.api.forgetLocation(r.path).subscribe(s => this.recent.set(s.recent_locations));
  }
}
