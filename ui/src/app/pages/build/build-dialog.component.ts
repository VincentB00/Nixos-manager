import { Component, OnInit, inject, signal } from '@angular/core';
import { FormsModule } from '@angular/forms';
import { Router } from '@angular/router';
import { MAT_DIALOG_DATA, MatDialogModule, MatDialogRef } from '@angular/material/dialog';
import { MatButtonModule } from '@angular/material/button';
import { MatCheckboxModule } from '@angular/material/checkbox';
import { MatFormFieldModule } from '@angular/material/form-field';
import { MatIconModule } from '@angular/material/icon';
import { MatInputModule } from '@angular/material/input';
import { MatProgressBarModule } from '@angular/material/progress-bar';
import { MatRadioModule } from '@angular/material/radio';
import { MatSelectModule } from '@angular/material/select';
import { MatExpansionModule } from '@angular/material/expansion';
import { ApiService, ApiFailure } from '../../api/api.service';
import type { AfterBuild, DeployPreflight, DeployTarget, HardwareChoice, KeepConfiguration, RebuildAction, RemoteCheck } from '../../api/generated';

@Component({
  selector: 'app-build-dialog',
  imports: [FormsModule, MatDialogModule, MatButtonModule, MatCheckboxModule, MatFormFieldModule, MatIconModule, MatInputModule, MatProgressBarModule, MatRadioModule, MatSelectModule, MatExpansionModule],
  styles: [`
    .row { display: flex; gap: 12px; flex-wrap: wrap; }
    .row mat-form-field { flex: 1 1 200px; }
    .steps { margin: 8px 0 16px; padding-left: 18px; font-size: .9em; line-height: 1.6; }
    .warn { display: flex; gap: 8px; align-items: flex-start; padding: 8px 12px; border-radius: 8px; background: var(--mat-sys-error-container); color: var(--mat-sys-on-error-container); margin-bottom: 12px; }
    .note { display: flex; gap: 8px; align-items: flex-start; padding: 8px 12px; border-radius: 8px; background: var(--mat-sys-surface-container-high); margin-bottom: 12px; font-size: .9em; }
    pre.diff { max-height: 240px; overflow: auto; font-size: .78em; background: var(--mat-sys-surface-container); padding: 8px; border-radius: 6px; }
    .diff .add { color: #2e7d32; } .diff .del { color: #c62828; }
  `],
  template: `
    <h2 mat-dialog-title>Build &amp; apply</h2>
    <mat-dialog-content>
      @if (loading()) { <mat-progress-bar mode="indeterminate" /> }
      @if (error()) { <div class="warn"><mat-icon>error</mat-icon><span>{{ error() }}</span></div> }

      @if (pre(); as p) {
        @if (p.source_is_etc_nixos) {
          <div class="warn"><mat-icon>block</mat-icon>
            <span>The opened folder <b>is</b> /etc/nixos. Syncing onto itself is refused (same guard as sync.sh). Open a working copy elsewhere, or tick "skip sync" to only rebuild.</span></div>
        }
        <mat-form-field appearance="outline" style="width:100%">
          <mat-label>Deploy to</mat-label>
          <mat-select [ngModel]="targetName" (ngModelChange)="selectTarget($event)">
            <mat-option value="">This machine (sync to /etc/nixos, then rebuild)</mat-option>
            @for (t of p.targets; track t.name) { <mat-option [value]="t.name">{{ t.name }} <span class="mono muted">{{ t.ssh_target }}</span></mat-option> }
          </mat-select>
          <mat-hint>Remote targets are managed on the Remote page.</mat-hint>
        </mat-form-field>        @if (!target() && p.targets.length) {
          <mat-form-field appearance="outline" style="width:100%">
            <mat-label>Build on</mat-label>
            <mat-select [ngModel]="buildHostName" (ngModelChange)="selectBuildHost($event)">
              <mat-option value="">This machine</mat-option>
              @for (t of p.targets; track t.name) { <mat-option [value]="t.name">{{ t.name }} <span class="mono muted">{{ t.ssh_target }}</span> — compile there, activate here</mat-option> }
            </mat-select>
          </mat-form-field>
          @if (buildHost(); as b) {
            <div class="note"><mat-icon>memory</mat-icon>
              <div>Build on <b>{{ b.name }}</b> (<span class="mono">{{ b.ssh_target }}</span>), activate on this machine: the configuration is evaluated here, the sources are sent to the builder, it compiles, and the result is copied back and applied here as usual. /etc/nixos on the builder is not touched.
                @if (bchecking()) { <div style="margin-top:6px"><mat-progress-bar mode="indeterminate" /></div> }
                @else if (bcheck(); as c) {
                  @if (!c.reachable) {
                    <div class="warning-box" style="margin-top:6px"><mat-icon>warning</mat-icon><span>Could not reach the builder over SSH{{ c.message ? ': ' + c.message : '' }}. Use "Set up key access" or "Test connection" on the Remote page.</span></div>
                  } @else {
                    <div style="margin-top:6px">Builder: <b>{{ c.hostname ?? b.ssh_target }}</b>{{ c.nixos_version ? ' · NixOS ' + c.nixos_version : '' }} · trusted by nix: <b>{{ c.is_root ? 'yes (root)' : c.trusted === true ? 'yes' : c.trusted === false ? 'no' : 'unknown' }}</b></div>
                    @if (builderNeedsPassword()) {
                      <div class="warning-box" style="margin-top:6px"><mat-icon>key</mat-icon><span>Your user is not trusted by nix on the builder, so trust is granted there temporarily for this build; enter the builder's sudo password for <span class="mono">{{ c.user }}</span> below. It is used for this job only and never stored.</span></div>
                    }
                  }
                }
              </div>
            </div>
          }
        }


        @if (target(); as t) {
          <div class="note"><mat-icon>lan</mat-icon>
            <div>Build here, activate on <b>{{ t.name }}</b> (<span class="mono">{{ t.ssh_target }}</span>):
              <ol class="steps">
                @if (p.kind === 'flake') { <li>{{ upgrade ? 'Resolve the flake inputs fresh (upgrade; no flake.lock is used)' : 'Pin the inputs with the flake.lock from /etc/nixos on the target (or from this machine when the target has none)' }}</li> }
                <li>{{ useWorkspaceHardware ? 'Build with the working copy\'s ' + p.hardware.rel_path + ' (the target\'s own file is not used)' : 'Fetch and use the target\'s own /etc/nixos/hardware-configuration.nix' }}</li>
                <li>Evaluate and build the system from the working copy on this machine{{ t.build_host ? ', compiling on ' + t.build_host : '' }}</li>
                @if (!check()?.is_root) { <li>Check that your user is trusted by nix on the target; if not, grant it temporarily via sudo and remove it again at the end</li> }
                <li>Copy the result to the target over SSH</li>
                <li>Run <code>nixos-rebuild {{ action }}</code> there{{ t.use_remote_sudo ? ' with sudo' : '' }}</li>
              </ol>
              /etc/nixos on this machine and on the target are not touched; no local password is needed.
              @if (checking()) { <div style="margin-top:6px"><mat-progress-bar mode="indeterminate" /></div> }
              @else if (check(); as c) {
                @if (!c.reachable) {
                  <div class="warning-box" style="margin-top:6px"><mat-icon>warning</mat-icon><span>Could not reach the target over SSH{{ c.message ? ': ' + c.message : '' }}. Use "Set up key access" or "Test connection" on the Remote page.</span></div>
                } @else {
                  <div style="margin-top:6px">Target: <b>{{ c.hostname ?? t.ssh_target }}</b>{{ c.nixos_version ? ' · NixOS ' + c.nixos_version : '' }} · trusted by nix: <b>{{ c.is_root ? 'yes (root)' : c.trusted === true ? 'yes' : c.trusted === false ? 'no' : 'unknown' }}</b> · passwordless sudo: <b>{{ c.is_root ? 'n/a' : c.passwordless_sudo === true ? 'yes' : c.passwordless_sudo === false ? 'no' : 'unknown' }}</b></div>
                  @if (c.hardware_config === 'none' && !useWorkspaceHardware) {
                    <div class="error-box" style="margin-top:6px"><mat-icon>error</mat-icon><span>The target has no <span class="mono">/etc/nixos/hardware-configuration.nix</span> and no <span class="mono">nixos-generate-config</span>, so its hardware cannot be described. Run <span class="mono">sudo nixos-generate-config</span> on the target first.</span></div>
                  } @else if (c.hardware_config === 'generate' && !useWorkspaceHardware) {
                    <div class="warning-box" style="margin-top:6px"><mat-icon>warning</mat-icon><span>The target has no <span class="mono">/etc/nixos/hardware-configuration.nix</span>; one will be generated on the target with <span class="mono">nixos-generate-config --show-hardware-config</span> for this deployment.</span></div>
                  }
                  @if (c.needs_sudo_password) {
                    <div class="warning-box" style="margin-top:6px"><mat-icon>key</mat-icon><span>{{ c.trusted === false ? 'Your user is not trusted by nix on the target, so trust is granted temporarily for this deployment' : 'Activation needs sudo on the target' }}; enter the target's sudo password for <span class="mono">{{ c.user }}</span> below. It is used for this job only and never stored.</span></div>
                  }
                }
              }
              @if (keyManaged(t)) {
                <div style="margin-top:4px">Logs in with the key set up by this app (no passphrase needed).</div>
              } @else if (!p.ssh_agent) {
                <div class="warning-box" style="margin-top:6px"><mat-icon>warning</mat-icon><span>This target was not set up with "Set up key access", and no SSH agent is visible to the app, so a key with a passphrase cannot be used. Either set up key access on the Remote page or use a key without passphrase.</span></div>
              }
              <mat-checkbox [(ngModel)]="useWorkspaceHardware" style="margin-top:6px">Use the working copy's <span class="mono">{{ p.hardware.rel_path }}</span> instead of the target's /etc/nixos/hardware-configuration.nix</mat-checkbox>
              @if (!useWorkspaceHardware) {
                <div class="note" style="margin-top:4px"><mat-icon>info</mat-icon><span>The target's own <span class="mono">/etc/nixos/hardware-configuration.nix</span> is used; the working copy's <span class="mono">{{ p.hardware.rel_path }}</span> is left out of this build.</span></div>
              } @else if (!p.hardware.local_exists) {
                <div class="error-box" style="margin-top:4px"><mat-icon>error</mat-icon><span>The working copy has no <span class="mono">{{ p.hardware.rel_path }}</span>. Untick this, or copy the target's file into the working copy with the download button on the Remote page.</span></div>
              } @else if (p.hardware.pulled_from; as pf) {
                <div class="note" style="margin-top:4px"><mat-icon>verified</mat-icon><span>The working copy's file is the one copied from target <b>{{ pf.target }}</b> on {{ pf.at.slice(0, 10) }}{{ pf.target === t.name ? '' : ', which is a different target' }}.</span></div>
              } @else {
                <div class="warning-box" style="margin-top:4px"><mat-icon>warning</mat-icon><span>The working copy's file will be applied to the target. Make sure it describes the <b>target's</b> disks: wrong UUIDs can leave it unable to boot. The download button on the Remote page copies the target's own file into the working copy.</span></div>
              }
              @if (p.secrets.present) { <div style="margin-top:4px">Managed secrets are not installed on remote targets.</div> }
            </div>
          </div>
        } @else {
        <div class="note"><mat-icon>info</mat-icon>
          <div>This runs the same steps as <code>sync.sh</code> followed by <code>nixos-rebuild</code>, as root:
            <ol class="steps">
              <li>{{ upgrade ? 'Do not preserve' : 'Preserve' }} <code>/etc/nixos/flake.lock</code>{{ p.etc_has_flake_lock ? '' : ' (none there now)' }}</li>
              <li>Copy <code>/etc/nixos/hardware-configuration.nix</code> into the working copy{{ p.etc_has_hardware_config ? '' : ' (none there now)' }}</li>
              <li>Resolve a <code>configuration.nix</code> conflict if any</li>
              <li>Clear <code>/etc/nixos</code> and copy the working copy there</li>
              <li>Run <code>nixos-rebuild {{ action }}</code>{{ p.kind === 'flake' ? ' --flake /etc/nixos#' + (host || '') : '' }}{{ buildHost() ? ' --build-host ' + buildHost()!.ssh_target + ' (compiles on the builder)' : '' }}</li>
            </ol>
          </div>
        </div>
        }

        <div class="row">
          @if (p.kind === 'flake') {
            <mat-form-field appearance="outline">
              <mat-label>Host (flake output)</mat-label>
              @if (p.hosts.length) {
                <mat-select [(ngModel)]="host">
                  @for (h of p.hosts; track h) { <mat-option [value]="h">{{ h }}{{ h === p.current_hostname ? ' (this machine)' : '' }}</mat-option> }
                </mat-select>
              } @else {
                <input matInput [(ngModel)]="host" placeholder="leave empty to use the hostname" />
              }
            </mat-form-field>
          }
          <mat-form-field appearance="outline">
            <mat-label>Action</mat-label>
            <mat-select [(ngModel)]="action">
              <mat-option value="switch">switch — build, activate now, and make default at boot</mat-option>
              <mat-option value="boot">boot — build and make default at next boot</mat-option>
              <mat-option value="test">test — build and activate now, but not at boot</mat-option>
              <mat-option value="build">build — only build, change nothing</mat-option>
              <mat-option value="dry-build">dry-build — show what would be built</mat-option>
            </mat-select>
          </mat-form-field>
        </div>
        @if (!target() && buildHost() && builderNeedsPassword()) {
          <mat-form-field appearance="outline" style="width:100%">
            <mat-label>sudo password on the builder ({{ bcheck()!.user }})</mat-label>
            <input matInput type="password" [(ngModel)]="targetSudoPassword" autocomplete="off" />
          </mat-form-field>
        }
        @if (target() && check()?.needs_sudo_password) {
          <mat-form-field appearance="outline" style="width:100%">
            <mat-label>sudo password on the target ({{ check()!.user }})</mat-label>
            <input matInput type="password" [(ngModel)]="targetSudoPassword" autocomplete="off" (keyup.enter)="start()" />
          </mat-form-field>
        }
        <mat-form-field appearance="outline" style="width:100%">
          <mat-label>After a successful build</mat-label>
          <mat-select [(ngModel)]="after">
            <mat-option value="none">Nothing: stay on</mat-option>
            <mat-option value="reboot">{{ target() ? 'Reboot the target' : 'Reboot this machine' }}</mat-option>
            <mat-option value="poweroff">{{ target() ? 'Power off the target' : 'Power off this machine' }}</mat-option>
          </mat-select>
          <mat-hint>For builds you want to start and walk away from. The log is also saved on disk (Jobs page → earlier logs), so you can read it afterwards.</mat-hint>
        </mat-form-field>
        @if (after !== 'none') {
          <mat-checkbox [(ngModel)]="afterAlways" style="margin-top:4px">Also {{ after === 'reboot' ? 'reboot' : 'power off' }} when the build fails</mat-checkbox>
          <div class="warning-box" style="margin-top:6px"><mat-icon>power_settings_new</mat-icon><span>{{ target() ? 'The target' : 'This machine' }} will {{ after === 'reboot' ? 'reboot' : 'power off' }} about 5 seconds after the build {{ afterAlways ? 'ends, whether it succeeds or fails' : 'succeeds' }}. {{ afterAlways ? 'A manual Stop does not trigger it.' : 'Nothing happens if the build fails or is stopped.' }}{{ !target() && action !== 'switch' && action !== 'boot' ? ' Note: the action "' + action + '" does not activate anything, so a reboot only makes sense with switch or boot.' : '' }}</span></div>
        }
        <div class="row">
          <mat-checkbox [(ngModel)]="upgrade">{{ target() ? 'Upgrade: update flake inputs first' : 'Upgrade: drop flake.lock so all inputs update (sync.sh --upgrade)' }}</mat-checkbox>
          @if (!target()) { <mat-checkbox [(ngModel)]="skipSync">Skip sync: rebuild /etc/nixos as it is now</mat-checkbox> }
        </div>

        @if (p.hardware.differs && !skipSync && !target()) {
          <div class="warning-box"><mat-icon>warning</mat-icon>
            <div><b><span class="mono">{{ p.hardware.rel_path }}</span> in the working copy differs from <span class="mono">/etc/nixos/hardware-configuration.nix</span>.</b>
              This file describes this machine's disks and hardware and rarely needs changing; a wrong disk UUID can leave the machine unable to boot. The system copy is the source of truth.
              @if (p.hardware.pulled_from; as pf) { <div style="margin-top:4px"><b>Note:</b> the working copy's file was copied from the remote target <b>{{ pf.target }}</b> on {{ pf.at.slice(0, 10) }}; it describes that machine, not this one.</div> }
              <mat-radio-group [(ngModel)]="hardwareChoice" style="display:flex; flex-direction:column; gap:4px; margin-top:6px">
                <mat-radio-button value="system">Use the system copy (recommended): the working copy's file is replaced by it</mat-radio-button>
                <mat-radio-button value="workspace">I changed it on purpose: apply the working copy's version to this machine</mat-radio-button>
              </mat-radio-group>
            </div>
          </div>
          <mat-expansion-panel style="margin-bottom:8px">
            <mat-expansion-panel-header>Show hardware-configuration.nix differences</mat-expansion-panel-header>
            <pre class="diff">@for (l of hardwareDiffLines(); track $index) {<span [class.add]="l.startsWith('+')" [class.del]="l.startsWith('-')">{{ l }}
</span>}</pre>
          </mat-expansion-panel>
        }

        @if (p.configuration_conflict && !skipSync && !target()) {
          <div class="note"><mat-icon>difference</mat-icon>
            <span><code>configuration.nix</code> in /etc/nixos differs from the working copy. The working copy version (what you see in this app) will be used and copied to /etc/nixos.</span></div>
          <mat-expansion-panel style="margin-bottom:8px">
            <mat-expansion-panel-header>Show differences</mat-expansion-panel-header>
            <pre class="diff">@for (l of diffLines(); track $index) {<span [class.add]="l.startsWith('+')" [class.del]="l.startsWith('-')">{{ l }}
</span>}</pre>
          </mat-expansion-panel>
        }

        @if (p.secrets.present && !target()) {
          <div [class]="p.secrets.deployed_up_to_date ? 'note' : 'warning-box'"><mat-icon>key</mat-icon>
            <div>This configuration has managed secrets (<span class="mono">secrets.age</span>).
              @if (p.secrets.deployed_up_to_date) { They are already installed and unchanged; the password is optional. }
              @else { They are <b>new or changed</b>: enter the secrets password so they are installed before the rebuild, or leave it empty to keep the previously installed ones. }
            </div>
          </div>
          <mat-form-field appearance="outline" style="width:100%">
            <mat-label>Secrets password{{ p.secrets.deployed_up_to_date ? ' (optional)' : '' }}</mat-label>
            <input matInput type="password" [(ngModel)]="secretsPassword" autocomplete="off" />
          </mat-form-field>
        }
        @if (!target()) {
          <mat-form-field appearance="outline" style="width:100%; margin-top: 12px">
            <mat-label>Your sudo password</mat-label>
            <input matInput type="password" [(ngModel)]="password" autocomplete="current-password" (keyup.enter)="start()" />
            <mat-hint>Used once for this job, kept only in memory, never saved.</mat-hint>
          </mat-form-field>
        }
      }
    </mat-dialog-content>
    <mat-dialog-actions align="end">
      <button mat-button (click)="validate()" [disabled]="loading()" matTooltip="Evaluate and dry-build first, without sudo"><mat-icon>fact_check</mat-icon> Validate only</button>
      <span class="spacer"></span>
      <button mat-button mat-dialog-close>Cancel</button>
      <button mat-flat-button (click)="start()" [disabled]="!canStart()"><mat-icon>play_arrow</mat-icon> Start</button>
    </mat-dialog-actions>
  `,
})
export class BuildDialogComponent implements OnInit {
  private api = inject(ApiService);
  private ref = inject(MatDialogRef<BuildDialogComponent>);
  private router = inject(Router);
  data = inject<{ path: string; host: string | null }>(MAT_DIALOG_DATA);

  pre = signal<DeployPreflight | null>(null);
  loading = signal(true);
  starting = signal(false);
  error = signal<string | null>(null);

  host = '';
  action: RebuildAction = 'switch';
  upgrade = false;
  skipSync = false;
  keep: KeepConfiguration | null = 'local';
  password = '';
  secretsPassword = '';
  targetName = '';
  targetSudoPassword = '';
  hardwareChoice: HardwareChoice = 'system';
  after: AfterBuild = 'none';
  afterAlways = false;
  buildHostName = '';
  bcheck = signal<RemoteCheck | null>(null);
  bchecking = signal(false);
  buildHost(): DeployTarget | null { return this.target() ? null : (this.pre()?.targets.find(t => t.name === this.buildHostName) ?? null); }
  /** Builders only need sudo for the temporary nix trust; a trusted user needs nothing. */
  builderNeedsPassword(): boolean { const c = this.bcheck(); return !!c && c.reachable && !c.is_root && c.trusted !== true; }
  selectBuildHost(name: string) {
    this.buildHostName = name;
    this.bcheck.set(null);
    this.targetSudoPassword = '';
    const b = this.buildHost();
    if (!b) return;
    this.bchecking.set(true);
    this.api.remoteCheck(b.name).subscribe({
      next: c => { this.bcheck.set(c); this.bchecking.set(false); },
      error: (e: ApiFailure) => { this.bchecking.set(false); this.error.set(e.message); },
    });
  }
  useWorkspaceHardware = false;
  check = signal<RemoteCheck | null>(null);
  checking = signal(false);
  target(): DeployTarget | null { return this.pre()?.targets.find(t => t.name === this.targetName) ?? null; }
  selectTarget(name: string) {
    this.targetName = name;
    this.check.set(null);
    this.targetSudoPassword = '';
    const t = this.target();
    if (!t) return;
    this.checking.set(true);
    this.api.remoteCheck(t.name).subscribe({
      next: c => { this.check.set(c); this.checking.set(false); },
      error: (e: ApiFailure) => { this.checking.set(false); this.error.set(e.message); },
    });
  }
  keyManaged(t: DeployTarget) {
    const host = t.ssh_target.split('@').pop() ?? t.ssh_target;
    return !!this.pre()?.app_key_exists && (this.pre()?.app_key_hosts ?? []).includes(host);
  }

  ngOnInit() {
    this.api.deployPreflight(this.data.path).subscribe({
      next: p => {
        this.pre.set(p);
        this.loading.set(false);
        this.host = this.data.host ?? p.hosts.find(h => h === p.current_hostname) ?? p.hosts[0] ?? '';
      },
      error: (e: ApiFailure) => { this.error.set(e.message); this.loading.set(false); },
    });
  }

  diffLines() {
    return (this.pre()?.configuration_conflict?.unified_diff ?? '').split('\n');
  }
  hardwareDiffLines(): string[] {
    return (this.pre()?.hardware.unified_diff ?? '').split('\n');
  }

  validate() {
    this.api.validate(this.data.path, this.host || null).subscribe({
      next: job => { this.ref.close(); this.router.navigate(['/w', 'jobs', job.id], { queryParams: { path: this.data.path } }); },
      error: (e: ApiFailure) => this.error.set(e.message),
    });
  }

  canStart() {
    const p = this.pre();
    if (!p || this.starting()) return false;
    if (this.target()) {
      if (this.checking()) return false;
      const c = this.check();
      if (c && !c.reachable) return false;
      if (this.useWorkspaceHardware) { if (!p.hardware.local_exists) return false; }
      else if (c?.hardware_config === 'none') return false;
      if (c?.needs_sudo_password && !this.targetSudoPassword) return false;
      return true;
    }
    if (!this.password) return false;
    if (p.source_is_etc_nixos && !this.skipSync) return false;
    if (this.buildHost()) {
      if (this.bchecking()) return false;
      const c = this.bcheck();
      if (c && !c.reachable) return false;
      if (this.builderNeedsPassword() && !this.targetSudoPassword) return false;
    }
    return true;
  }

  start() {
    if (!this.canStart()) return;
    const p = this.pre()!;
    this.starting.set(true);
    this.error.set(null);
    this.api.deploy({
      path: this.data.path,
      host: this.host || null,
      action: this.action,
      upgrade: this.upgrade,
      keep_configuration: p.configuration_conflict ? 'local' : null,
      hardware: this.target() ? (this.useWorkspaceHardware ? 'workspace' : 'system') : (p.hardware.differs ? this.hardwareChoice : 'system'),
      skip_sync: this.skipSync,
      password: this.password,
      secrets_password: this.target() ? null : (this.secretsPassword || null),
      target: this.target(),
      target_sudo_password: (this.target() || this.buildHost()) ? (this.targetSudoPassword || null) : null,
      build_host: this.buildHost(),
      after: this.after,
      after_even_on_failure: this.after !== 'none' && this.afterAlways,
    }).subscribe({
      next: job => {
        this.password = '';
        this.secretsPassword = '';
        this.targetSudoPassword = '';
        this.ref.close(job);
        this.router.navigate(['/w', 'jobs', job.id], { queryParams: { path: this.data.path } });
      },
      error: (e: ApiFailure) => { this.error.set(e.message); this.starting.set(false); },
    });
  }
}
