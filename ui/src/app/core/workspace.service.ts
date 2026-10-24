import { Injectable, computed, inject, signal } from '@angular/core';
import { MatSnackBar } from '@angular/material/snack-bar';
import { ApiService, ApiFailure } from '../api/api.service';
import type { BuildStatus, ConfigOverview, FileSummary, FlakeVar, GitInfo, PackageSet } from '../api/generated';

/** The currently opened configuration folder and its discovery result. */
@Injectable({ providedIn: 'root' })
export class WorkspaceService {
  private api = inject(ApiService);
  private snack = inject(MatSnackBar);

  readonly path = signal<string | null>(null);
  readonly overview = signal<ConfigOverview | null>(null);
  readonly loading = signal(false);
  readonly error = signal<string | null>(null);
  readonly selectedHost = signal<string | null>(null);
  /** Package sets modules can draw from (pkgs, pkgs-<channel>, ...). */
  /** Global values from flake.nix (`vars = { ... }`), empty for classic configs. */
  readonly flakeVars = signal<FlakeVar[]>([]);
  /** Git state of the folder (null until loaded; is_repo=false when not a repository). */
  readonly gitInfo = signal<GitInfo | null>(null);
  /** Files changed since the last successful build of this folder (toolbar chip). */
  readonly buildStatus = signal<BuildStatus | null>(null);
  /** The daemon runs as root (run.sh started with sudo): local root actions ask for no sudo password. */
  readonly isRoot = computed(() => this.overview()?.machine.running_as_root ?? false);
  private storedHost: string | null = null;

  /** Pick the host to build and remember it for this folder. */
  selectHost(h: string | null) {
    this.selectedHost.set(h);
    const path = this.path();
    if (path) this.api.setHost(path, h).subscribe({ error: () => {} });
  }

  refreshBuildStatus() {
    const path = this.path();
    if (!path) return;
    this.api.buildStatus(path).subscribe({ next: b => this.buildStatus.set(b), error: () => this.buildStatus.set(null) });
  }
  readonly packageSets = signal<PackageSet[]>([{ name: 'pkgs', label: 'main', input: null, flake_ref: null, is_default: true, bits32: false }]);

  readonly nixFiles = computed(() => (this.overview()?.files ?? []).filter(f => f.rel_path.endsWith('.nix')));
  readonly otherFiles = computed(() => (this.overview()?.files ?? []).filter(f => !f.rel_path.endsWith('.nix')));
  readonly fileMap = computed(() => {
    const m = new Map<string, FileSummary>();
    for (const f of this.overview()?.files ?? []) m.set(f.rel_path, f);
    return m;
  });

  open(path: string) {
    const isNew = this.path() !== path;
    if (isNew) {
      this.path.set(path);
      this.overview.set(null);
    }
    // hardware-configuration.nix: this machine's /etc/nixos copy is the source
    // of truth and is copied into the folder on open (users rarely edit it).
    this.api.hardwareSync(path).subscribe({
      next: r => {
        if (r.lock_removed) {
          this.snack.open('flake.lock removed from the working folder: it lives in /etc/nixos only', 'OK', { duration: 8000 });
          if (r.status !== 'copied' && r.status !== 'refreshed') this.reload();
        }
        if (r.status === 'copied' || r.status === 'refreshed') {
          this.snack.open(r.message, 'OK', { duration: 8000 });
          this.reload();
        } else if (r.status === 'not-writable') {
          this.snack.open(r.message, 'OK', { duration: 8000 });
        }
      },
      error: () => {},
    });
    this.reload();
  }

  reload() {
    const path = this.path();
    if (!path) return;
    this.loading.set(true);
    this.error.set(null);
    this.api.packageSets(path).subscribe({ next: sets => this.packageSets.set(sets), error: () => {} });
    this.api.gitInfo(path).subscribe({ next: g => this.gitInfo.set(g), error: () => this.gitInfo.set(null) });
    this.refreshBuildStatus();
    this.api.state().subscribe({
      next: st => {
        const canon = path.replace(/\/+$/, '');
        this.storedHost = st.recent_locations.find(r => r.path.replace(/\/+$/, '') === canon)?.host ?? null;
        const hosts = this.overview()?.location.hosts ?? [];
        if (this.storedHost && hosts.includes(this.storedHost)) this.selectedHost.set(this.storedHost);
      },
      error: () => {},
    });
    this.api.flakeInfo(path).subscribe({ next: f => this.flakeVars.set(f.vars ?? []), error: () => this.flakeVars.set([]) });
    this.api.overview(path).subscribe({
      next: ov => {
        this.overview.set(ov);
        this.loading.set(false);
        const hosts = ov.location.hosts;
        const current = this.selectedHost();
        if (!current || !hosts.includes(current)) {
          const machineHost = ov.machine.hostname;
          const stored = this.storedHost && hosts.includes(this.storedHost) ? this.storedHost : null;
          this.selectedHost.set(stored ?? hosts.find(h => h === machineHost) ?? hosts[0] ?? null);
        }
      },
      error: (e: ApiFailure) => {
        this.error.set(e.message);
        this.loading.set(false);
      },
    });
  }

  close() {
    this.path.set(null);
    this.overview.set(null);
    this.selectedHost.set(null);
  }
}
