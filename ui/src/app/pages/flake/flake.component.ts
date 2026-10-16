import { Component, OnInit, computed, inject, signal } from '@angular/core';
import { RouterLink } from '@angular/router';
import { SlicePipe } from '@angular/common';
import { MatButtonModule } from '@angular/material/button';
import { MatCardModule } from '@angular/material/card';
import { MatDialog } from '@angular/material/dialog';
import { MatIconModule } from '@angular/material/icon';
import { MatProgressBarModule } from '@angular/material/progress-bar';
import { MatSnackBar } from '@angular/material/snack-bar';
import { MatTooltipModule } from '@angular/material/tooltip';
import { MatMenuModule } from '@angular/material/menu';
import { ApiService, ApiFailure } from '../../api/api.service';
import { WorkspaceService } from '../../core/workspace.service';
import { PromptDialogComponent, PromptData, PromptResult } from '../../shared/prompt-dialog.component';
import type { FlakeInfo, FlakeInput, FlakeOp, FlakeVar } from '../../api/generated';

/** Dedicated view for flake.nix: main nixpkgs, extra package sets, inputs, global vars, hosts. */
@Component({
  selector: 'app-flake',
  imports: [RouterLink, SlicePipe, MatButtonModule, MatCardModule, MatIconModule, MatProgressBarModule, MatTooltipModule, MatMenuModule],
  styles: [`
    .head { display: flex; align-items: center; gap: 12px; margin-bottom: 12px; flex-wrap: wrap; }
    h2 { margin: 0; font-weight: 400; }
    .cards { display: grid; grid-template-columns: repeat(auto-fit, minmax(340px, 1fr)); gap: 16px; }
    mat-card { min-width: 0; }
    .row { display: flex; align-items: center; gap: 10px; padding: 6px 0; border-bottom: 1px solid var(--mat-sys-surface-container); min-width: 0; }
    .row:last-child { border-bottom: 0; }
    .row .t { flex: 1; min-width: 0; }
    .row .t b { font-weight: 500; }
    .row .d { font-size: .82em; color: var(--mat-sys-on-surface-variant); overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
    .tag { font-size: .7em; padding: 1px 6px; border-radius: 4px; background: var(--mat-sys-surface-container-high); text-transform: uppercase; letter-spacing: .04em; flex: none; }
    .lock { font-size: .78em; color: var(--mat-sys-on-surface-variant); }
    .errors { padding: 8px 12px; border-radius: 8px; background: var(--mat-sys-error-container); color: var(--mat-sys-on-error-container); margin-bottom: 12px; font-size: .9em; }
    .note { font-size: .85em; color: var(--mat-sys-on-surface-variant); margin-top: 8px; line-height: 1.45; }
    mat-card-actions { gap: 6px; flex-wrap: wrap; }
    code { font-size: .9em; }
  `],
  template: `
    @if (busy()) { <mat-progress-bar mode="indeterminate" /> }
    <div class="head">
      <h2 class="mono">flake.nix</h2>
      @if (info()?.description) { <span class="muted">{{ info()!.description }}</span> }
      <span class="spacer"></span>
      <a mat-stroked-button routerLink="../file" [queryParams]="{ f: 'flake.nix' }" queryParamsHandling="merge"><mat-icon>code</mat-icon> Edit as code</a>
      <button mat-icon-button (click)="load()" matTooltip="Reload"><mat-icon>refresh</mat-icon></button>
    </div>
    @if (error()) { <div class="errors">{{ error() }}</div> }

    @if (info(); as i) {
      @if (i.parse_errors.length) { <div class="errors"><b>flake.nix has syntax errors.</b> Fix them in code view; the wizards below are disabled until then.</div> }
      @if (!i.has_template_structure) {
        <div class="warning-box"><mat-icon>info</mat-icon><span>This flake does not follow the Nixos-manager template (no <code>vars</code> / <code>channels</code> let-block), so only the inputs can be managed here. Everything else is available in code view.</span></div>
      }

      <div class="cards">
        <!-- Main nixpkgs -->
        <mat-card appearance="outlined">
          <mat-card-header><mat-card-title>Main package source</mat-card-title><mat-card-subtitle>the <code>nixpkgs</code> input every module gets as <code>pkgs</code></mat-card-subtitle></mat-card-header>
          <mat-card-content>
            @if (nixpkgs(); as n) {
              <div class="row"><div class="t"><b class="mono">{{ n.url ?? '(no url)' }}</b>
                @if (n.locked) { <div class="lock">locked at {{ n.locked.rev | slice:0:12 }} · {{ n.locked.date | slice:0:10 }}</div> } @else { <div class="lock">not locked yet (the lock file is written on the first build)</div> }
              </div></div>
            } @else { <div class="muted">No <code>nixpkgs</code> input found.</div> }
            <div class="note">A branch like <b>nixos-unstable</b> gives the newest packages; a release branch like <b>nixos-26.05</b> changes less often. Updating the lock (Build → Upgrade) moves to the newest commit of the chosen branch.</div>
          </mat-card-content>
          <mat-card-actions>
            <button mat-stroked-button (click)="changeBranch()" [disabled]="locked()"><mat-icon>swap_horiz</mat-icon> Change branch</button>
          </mat-card-actions>
        </mat-card>

        <!-- Extra package sets -->
        <mat-card appearance="outlined">
          <mat-card-header><mat-card-title>Extra package sets</mat-card-title><mat-card-subtitle>older or newer nixpkgs available to modules as <code>pkgs-&lt;name&gt;</code></mat-card-subtitle></mat-card-header>
          <mat-card-content>
            @for (c of channelInputs(); track c.channel) {
              <div class="row">
                <span class="tag">pkgs-{{ c.channel }}</span>
                <div class="t"><b class="mono">{{ c.input.url ?? '(no url)' }}</b>
                  @if (c.input.locked) { <div class="lock">locked at {{ c.input.locked.rev | slice:0:12 }} · {{ c.input.locked.date | slice:0:10 }}</div> }
                  @if (!c.listed) { <div class="lock warning-text">input exists but "{{ c.channel }}" is not in the channels list</div> }
                </div>
                <button mat-icon-button matTooltip="Remove this package set" (click)="removeChannel(c.channel)" [disabled]="locked()"><mat-icon>delete</mat-icon></button>
              </div>
            } @empty { <div class="muted">None yet. Use this when a package you need only exists, or only works, in another NixOS version.</div> }
            <div class="note">Example: with <b>25_11</b> added, a module can write <code>pkgs-25_11.somePackage</code> to take that package from NixOS 25.11 while everything else stays on the main source.</div>
          </mat-card-content>
          <mat-card-actions>
            <button mat-flat-button (click)="addChannel()" [disabled]="locked() || !i.has_template_structure"><mat-icon>add</mat-icon> Add a previous nixpkgs version</button>
          </mat-card-actions>
        </mat-card>

        <!-- Global vars -->
        <mat-card appearance="outlined">
          <mat-card-header><mat-card-title>Global values</mat-card-title><mat-card-subtitle>passed to every module as <code>vars</code></mat-card-subtitle></mat-card-header>
          <mat-card-content>
            @for (v of i.vars ?? []; track v.name) {
              <div class="row">
                <div class="t"><b class="mono">{{ v.name }}</b><div class="d mono">{{ v.value }}</div></div>
                <button mat-icon-button matTooltip="Edit" (click)="editVar(v)" [disabled]="locked()"><mat-icon>edit</mat-icon></button>
                <button mat-icon-button matTooltip="Remove" (click)="removeVar(v)" [disabled]="locked()"><mat-icon>delete</mat-icon></button>
              </div>
            } @empty { <div class="muted">No global values yet.</div> }
            <div class="note">Use them in any module, e.g. <code>users.users.&#36;{{ '{' }}vars.mainUsername{{ '}' }}</code>, so a name or path is defined once.</div>
          </mat-card-content>
          <mat-card-actions>
            <button mat-stroked-button (click)="addVar()" [disabled]="locked() || !hasLet()"><mat-icon>add</mat-icon> Add value</button>
          </mat-card-actions>
        </mat-card>

        <!-- Inputs -->
        <mat-card appearance="outlined">
          <mat-card-header><mat-card-title>All inputs</mat-card-title><mat-card-subtitle>external sources this flake depends on</mat-card-subtitle></mat-card-header>
          <mat-card-content>
            @for (inp of i.inputs; track inp.name) {
              <div class="row">
                <div class="t"><b class="mono">{{ inp.name }}</b><div class="d mono">{{ inp.url ?? '(no url)' }}</div>
                  @if (inp.follows.length) { <div class="lock">@for (f of inp.follows; track f.input) { {{ f.input }} → {{ f.target }} } </div> }
                  @if (inp.locked) { <div class="lock">{{ inp.locked.rev | slice:0:12 }} · {{ inp.locked.date | slice:0:10 }}</div> }
                </div>
                <button mat-icon-button matTooltip="Change URL" (click)="editInputUrl(inp)" [disabled]="locked()"><mat-icon>edit</mat-icon></button>
                <button mat-icon-button matTooltip="Remove input" (click)="removeInput(inp)" [disabled]="locked() || inp.name === 'nixpkgs'"><mat-icon>delete</mat-icon></button>
              </div>
            }
          </mat-card-content>
          <mat-card-actions>
            <button mat-stroked-button (click)="addInput()" [disabled]="locked()"><mat-icon>add</mat-icon> Add input</button>
          </mat-card-actions>
        </mat-card>

        <!-- Hosts & lock -->
        <mat-card appearance="outlined">
          <mat-card-header><mat-card-title>Hosts</mat-card-title><mat-card-subtitle><code>nixosConfigurations</code> this flake defines</mat-card-subtitle></mat-card-header>
          <mat-card-content>
            @for (h of i.hosts; track h) { <div class="row"><mat-icon>dns</mat-icon><div class="t"><b class="mono">{{ h }}</b></div></div> }
            @empty { <div class="muted">No hosts found statically.</div> }
            <div class="note">
              @if (i.lock_path) { Lock file: <code>{{ i.lock_path }}</code>. } @else { No lock file yet; it is created by the first build. }
              System: <code>{{ i.system ?? '?' }}</code>.
            </div>
          </mat-card-content>
        </mat-card>
      </div>
    }
  `,
})
export class FlakeComponent implements OnInit {
  private api = inject(ApiService);
  private ws = inject(WorkspaceService);
  private dialog = inject(MatDialog);
  private snack = inject(MatSnackBar);

  info = signal<FlakeInfo | null>(null);
  busy = signal(false);
  error = signal<string | null>(null);
  available = signal<string[]>([]);

  nixpkgs = computed(() => this.info()?.inputs.find(i => i.name === 'nixpkgs') ?? null);
  channelInputs = computed(() => {
    const i = this.info();
    if (!i) return [];
    const listed = new Set(i.channels ?? []);
    return i.inputs.filter(x => x.is_nixpkgs_channel).map(input => {
      const channel = input.name.replace(/^nixpkgs-/, '');
      return { input, channel, listed: listed.has(channel) };
    });
  });
  locked = computed(() => this.busy() || (this.info()?.parse_errors.length ?? 0) > 0);
  hasLet = computed(() => !!this.info()?.vars || !!this.info()?.has_template_structure);

  ngOnInit() {
    this.load();
    this.api.channels().subscribe(c => this.available.set(c.available));
  }

  load() {
    const path = this.ws.path();
    if (!path) return;
    this.busy.set(true);
    this.api.flakeInfo(path).subscribe({
      next: i => { this.info.set(i); this.busy.set(false); this.error.set(null); },
      error: (e: ApiFailure) => { this.error.set(e.message); this.busy.set(false); },
    });
  }

  private prompt(data: PromptData) {
    return this.dialog.open<PromptDialogComponent, PromptData, PromptResult>(PromptDialogComponent, { width: '520px', data }).afterClosed();
  }

  private run(op: FlakeOp, done: string) {
    const path = this.ws.path();
    const i = this.info();
    if (!path || !i) return;
    this.busy.set(true);
    this.api.flakeEdit(path, i.version, op).subscribe({
      next: ni => { this.info.set(ni); this.busy.set(false); this.snack.open(done, undefined, { duration: 2500 }); this.ws.reload(); },
      error: (e: ApiFailure) => {
        this.busy.set(false);
        this.snack.open(e.message, 'Dismiss', { duration: 8000 });
        if (e.code === 'conflict') this.load();
      },
    });
  }

  private refChoices() {
    const a = this.available();
    return a.length ? a : ['nixos-unstable'];
  }

  changeBranch() {
    const current = (this.nixpkgs()?.url ?? '').match(/ref=([^&]+)/)?.[1] ?? 'nixos-unstable';
    this.prompt({ title: 'Main nixpkgs branch', label: 'Branch', value: this.refChoices().includes(current) ? current : this.refChoices()[0], choices: this.refChoices(), hint: 'nixos-unstable: newest packages · nixos-XX.YY: stable release', okLabel: 'Change' })
      .subscribe(r => { if (r) this.run({ op: 'set_input_url', name: 'nixpkgs', url: `github:nixos/nixpkgs?ref=${r.value}` }, `nixpkgs now follows ${r.value}`); });
  }

  addChannel() {
    const current = this.nixpkgs()?.url ?? '';
    const choices = this.refChoices().filter(c => !current.includes(`ref=${c}`));
    this.prompt({
      title: 'Add a previous nixpkgs version',
      message: 'Modules will receive it as pkgs-<name>, e.g. pkgs-25_11. The input is added to flake.nix and the name to the channels list.',
      label: 'NixOS version', value: choices[0], choices, okLabel: 'Add',
    }).subscribe(r => {
      if (!r) return;
      const name = r.value.replace(/^nixos-/, '').replace(/[.-]/g, '_');
      this.run({ op: 'add_channel', name, url: `github:nixos/nixpkgs?ref=${r.value}` }, `Added pkgs-${name} (from ${r.value})`);
    });
  }

  removeChannel(name: string) {
    this.prompt({ title: `Remove pkgs-${name}?`, message: `Modules that use pkgs-${name} will fail to build until you change them.`, okLabel: 'Remove', danger: true })
      .subscribe(r => { if (r) this.run({ op: 'remove_channel', name }, `Removed pkgs-${name}`); });
  }

  addVar() {
    this.prompt({ title: 'Add a global value', label: 'Name', placeholder: 'mainUserEmail', hint: 'Letters, digits, _ and -', okLabel: 'Next' }).subscribe(r => {
      if (!r) return;
      this.prompt({ title: `Value for vars.${r.value}`, label: 'Value (Nix)', placeholder: '"text"', hint: 'Strings need quotes: "like this". Numbers and true/false are bare.', okLabel: 'Add' })
        .subscribe(r2 => { if (r2) this.run({ op: 'set_var', name: r.value, value: r2.value }, `Added vars.${r.value}`); });
    });
  }

  editVar(v: FlakeVar) {
    this.prompt({ title: `vars.${v.name}`, label: 'Value (Nix)', value: v.value, okLabel: 'Save' })
      .subscribe(r => { if (r && r.value !== v.value) this.run({ op: 'set_var', name: v.name, value: r.value }, `Updated vars.${v.name}`); });
  }

  removeVar(v: FlakeVar) {
    this.prompt({ title: `Remove vars.${v.name}?`, message: 'Modules that use it will fail to evaluate.', okLabel: 'Remove', danger: true })
      .subscribe(r => { if (r) this.run({ op: 'remove_var', name: v.name }, `Removed vars.${v.name}`); });
  }

  addInput() {
    this.prompt({ title: 'Add input', label: 'Name', placeholder: 'home-manager', okLabel: 'Next' }).subscribe(r => {
      if (!r) return;
      this.prompt({ title: `URL for ${r.value}`, label: 'Flake URL', placeholder: 'github:nix-community/home-manager', checkbox: { label: 'Make it use this flake\'s nixpkgs (inputs.nixpkgs.follows)', checked: true }, okLabel: 'Add' })
        .subscribe(r2 => { if (r2) this.run({ op: 'add_input', name: r.value, url: r2.value, follows_nixpkgs: r2.checked }, `Added input ${r.value}`); });
    });
  }

  editInputUrl(inp: FlakeInput) {
    this.prompt({ title: `URL for ${inp.name}`, label: 'Flake URL', value: inp.url ?? '', okLabel: 'Save' })
      .subscribe(r => { if (r && r.value !== inp.url) this.run({ op: 'set_input_url', name: inp.name, url: r.value }, `Updated ${inp.name}`); });
  }

  removeInput(inp: FlakeInput) {
    this.prompt({ title: `Remove input ${inp.name}?`, message: 'Anything in the flake that references it will break.', okLabel: 'Remove', danger: true })
      .subscribe(r => { if (r) this.run({ op: 'remove_input', name: inp.name }, `Removed ${inp.name}`); });
  }
}
