import { TooltipAriaDirective } from '../../shared/tooltip-aria.directive';
import { Component, OnInit, computed, inject, signal } from '@angular/core';
import { FormsModule } from '@angular/forms';
import { NgTemplateOutlet, SlicePipe } from '@angular/common';
import { MAT_DIALOG_DATA, MatDialogModule, MatDialogRef } from '@angular/material/dialog';
import { MatButtonModule } from '@angular/material/button';
import { MatCheckboxModule } from '@angular/material/checkbox';
import { MatFormFieldModule } from '@angular/material/form-field';
import { MatIconModule } from '@angular/material/icon';
import { MatInputModule } from '@angular/material/input';
import { MatProgressBarModule } from '@angular/material/progress-bar';
import { MatSelectModule } from '@angular/material/select';
import { MatSlideToggleModule } from '@angular/material/slide-toggle';
import { MatTooltipModule } from '@angular/material/tooltip';
import { Subject, debounceTime, distinctUntilChanged, switchMap, of, catchError } from 'rxjs';
import { ApiService, ApiFailure } from '../../api/api.service';
import { WorkspaceService } from '../../core/workspace.service';
import type { ConfigChange, DiskInfo, NixFileView, OptionDoc, OptionsStatus, PackageInfo, PackageSet, PartitionInfo, PopularPackage, Recipe, RecipeField, SecretsStatus } from '../../api/generated';

type Category = 'programs' | 'feature' | 'option' | 'custom';
type Step = 'pick' | 'details' | 'preview';

export interface WizardData {
  path: string;
  file: string;
  version: string;
}

/** Guided "Add something to this module" flow. Returns the new NixFileView when something was applied. */
@Component({
  selector: 'app-wizard-dialog',
  imports: [FormsModule, NgTemplateOutlet, SlicePipe, MatDialogModule, MatButtonModule, MatCheckboxModule, MatFormFieldModule, MatIconModule, MatInputModule, MatProgressBarModule, MatSelectModule, MatSlideToggleModule, MatTooltipModule, TooltipAriaDirective],
  styles: [`
    mat-dialog-content { min-height: 420px; }
    .cards { display: grid; grid-template-columns: repeat(auto-fit, minmax(220px, 1fr)); gap: 12px; margin-top: 8px; }
    .card { border: 1px solid var(--mat-sys-outline-variant); border-radius: 12px; padding: 14px; cursor: pointer; display: flex; gap: 12px; align-items: flex-start; }
    .card:hover { background: var(--mat-sys-surface-container); border-color: var(--mat-sys-primary); }
    .card mat-icon { color: var(--mat-sys-primary); flex: none; }
    .card b { display: block; margin-bottom: 4px; }
    .card .muted { font-size: .9em; }
    .crumb { display: flex; align-items: center; gap: 6px; font-size: .9em; color: var(--mat-sys-on-surface-variant); margin-bottom: 8px; }
    .list { max-height: 360px; overflow: auto; border: 1px solid var(--mat-sys-outline-variant); border-radius: 8px; }
    .item { display: flex; align-items: center; gap: 10px; padding: 8px 12px; cursor: pointer; border-bottom: 1px solid var(--mat-sys-surface-container); }
    .item:hover { background: var(--mat-sys-surface-container); }
    .item.sel { background: var(--mat-sys-secondary-container); }
    .item .t { flex: 1; min-width: 0; }
    .item .t b { font-weight: 500; }
    .item .d { font-size: .85em; color: var(--mat-sys-on-surface-variant); white-space: nowrap; overflow: hidden; text-overflow: ellipsis; }
    .tag { font-size: .7em; padding: 1px 6px; border-radius: 4px; background: var(--mat-sys-surface-container-high); text-transform: uppercase; letter-spacing: .04em; flex: none; }
    .fields { display: grid; gap: 2px; margin-top: 4px; }
    .field-row { display: flex; align-items: center; gap: 12px; }
    .help { font-size: .82em; color: var(--mat-sys-on-surface-variant); margin: -10px 0 10px 2px; }
    pre.preview { background: var(--mat-sys-surface-container); padding: 10px 12px; border-radius: 8px; font-size: .85em; white-space: pre-wrap; }
    .chips { display: flex; flex-wrap: wrap; gap: 6px; margin: 8px 0; }
    .chip { display: inline-flex; align-items: center; gap: 4px; padding: 2px 4px 2px 10px; border-radius: 14px; background: var(--mat-sys-secondary-container); color: var(--mat-sys-on-secondary-container); font-size: .85em; }
    .chip button { width: 22px; height: 22px; line-height: 22px; padding: 0; }
    .chip button mat-icon { font-size: 16px; width: 16px; height: 16px; }
    .doc { background: var(--mat-sys-surface-container); border-radius: 8px; padding: 10px 12px; font-size: .9em; margin: 8px 0; }
    .doc .desc { white-space: pre-wrap; }
    .doc dl { display: grid; grid-template-columns: auto 1fr; gap: 2px 10px; margin: 8px 0 0; font-size: .85em; }
    .doc dt { color: var(--mat-sys-on-surface-variant); } .doc dd { margin: 0; }
    .note { display: flex; gap: 8px; align-items: flex-start; padding: 8px 12px; border-radius: 8px; background: var(--mat-sys-surface-container-high); font-size: .9em; margin-top: 8px; }
    .warn { color: var(--mat-sys-error); font-size: .9em; }
    .sec { margin: 14px 0 6px; font-size: .78em; text-transform: uppercase; letter-spacing: .06em; color: var(--mat-sys-on-surface-variant); }
    .sec-head { margin: 16px 0 8px; padding-bottom: 4px; border-bottom: 1px solid var(--mat-sys-outline-variant); font-weight: 500; }
    .sugg { display: flex; flex-wrap: wrap; gap: 6px; margin: -6px 0 10px; }
    .sugg label { display: inline-flex; align-items: center; gap: 4px; padding: 2px 10px 2px 6px; border-radius: 14px; border: 1px solid var(--mat-sys-outline-variant); font-size: .85em; cursor: pointer; }
    .sugg label.on { background: var(--mat-sys-secondary-container); color: var(--mat-sys-on-secondary-container); border-color: transparent; }
    .disk { border: 1px solid var(--mat-sys-outline-variant); border-radius: 8px; margin-bottom: 8px; overflow: hidden; }
    .disk-head { display: flex; align-items: center; gap: 8px; padding: 6px 10px; background: var(--mat-sys-surface-container); font-size: .9em; }
    .part { display: flex; align-items: center; gap: 10px; padding: 6px 10px; cursor: pointer; border-top: 1px solid var(--mat-sys-surface-container); }
    .part:hover { background: var(--mat-sys-surface-container); }
    .part.sel { background: var(--mat-sys-secondary-container); color: var(--mat-sys-on-secondary-container); }
    .part.off { opacity: .5; cursor: not-allowed; }
    .part .t { flex: 1; min-width: 0; }
    .part .d { font-size: .82em; color: var(--mat-sys-on-surface-variant); }
    textarea.lines { width: 100%; min-height: 80px; font-family: ui-monospace, monospace; font-size: .85em; padding: 8px; border-radius: 6px; border: 1px solid var(--mat-sys-outline-variant); background: var(--mat-sys-surface); color: inherit; box-sizing: border-box; }
  `],
  template: `
    <h2 mat-dialog-title>Add to {{ data.file }}</h2>
    <mat-dialog-content>
      @if (busy()) { <mat-progress-bar mode="indeterminate" /> }
      @if (error()) { <div class="warn">{{ error() }}</div> }

      <!-- ── Step 1: what ─────────────────────────────────────────── -->
      @if (step() === 'pick') {
        <div class="muted">What would you like to add?</div>
        <div class="cards">
          <div class="card" (click)="choose('programs')"><mat-icon>apps</mat-icon><div><b>Install programs</b><span class="muted">Search the NixOS package collection and pick what to install.</span></div></div>
          <div class="card" (click)="choose('feature')"><mat-icon>toggle_on</mat-icon><div><b>Turn on a feature or service</b><span class="muted">SSH, Docker, printing, Bluetooth, a user account, time zone, desktop… with a short form.</span></div></div>
          <div class="card" (click)="choose('option')"><mat-icon>tune</mat-icon><div><b>Any NixOS option</b><span class="muted">Search all {{ optStatus()?.count || '' }} options with their official documentation. For advanced users.</span></div></div>
          <div class="card" (click)="choose('custom')"><mat-icon>code</mat-icon><div><b>Custom setting</b><span class="muted">Type an option path and a Nix value yourself.</span></div></div>
        </div>
      }

      <!-- ── Step 2: details ──────────────────────────────────────── -->
      @if (step() === 'details') {
        <div class="crumb"><button mat-button (click)="back()"><mat-icon>arrow_back</mat-icon> Back</button><span>{{ categoryTitle() }}</span></div>

        @switch (category()) {
          @case ('programs') {
            <ng-container *ngTemplateOutlet="setPicker" />
            <mat-form-field appearance="outline" style="width:100%" subscriptSizing="dynamic">
              <mat-label>Search programs</mat-label>
              <input matInput [ngModel]="pkgQuery()" (ngModelChange)="pkgQuery.set($event); pkgSearch$.next($event)" placeholder="e.g. firefox, image editor, python" cdkFocusInitial />
              <mat-icon matSuffix>search</mat-icon>
              <mat-hint>{{ pkgHint() }}</mat-hint>
            </mat-form-field>
            @if (selectedPkgs().length) {
              <div class="chips">@for (p of selectedPkgs(); track p.attr + p.set) { <span class="chip">{{ p.set !== 'pkgs' ? p.set + '.' : '' }}{{ p.attr }} <button mat-icon-button (click)="togglePkg(p)"><mat-icon>close</mat-icon></button></span> }</div>
            }
            @if (pkgQuery().length < 2 && popular().length) {
              <div class="muted" style="margin:4px 0 6px; font-size:.85em">Popular programs — click to add:</div>
              <div class="chips" style="margin-bottom:10px">@for (p of popular(); track p.attr) { <button class="chip" type="button" (click)="pickPopular(p)" [matTooltip]="p.description + ' · ' + p.attr">{{ p.title }}</button> }</div>
            }
            <div class="list">
              @for (p of pkgResults(); track p.attr) {
                <div class="item" [class.sel]="isSelected(p)" (click)="togglePkg(p)">
                  <mat-checkbox [checked]="isSelected(p)" (click)="$event.preventDefault()" />
                  <div class="t"><b class="mono">{{ p.attr }}</b><span class="muted" style="margin-left:8px">{{ p.version }}</span><div class="d">{{ p.description }}</div></div>
                </div>
              } @empty {
                <div class="item muted" style="cursor:default">{{ pkgQuery().length < 2 ? 'Type at least two letters.' : (busy() ? 'Searching…' : 'No packages found.') }}</div>
              }
            </div>
          }

          @case ('feature') {
            @if (!recipe()) {
              <mat-form-field appearance="outline" style="width:100%" subscriptSizing="dynamic">
                <mat-label>Filter</mat-label>
                <input matInput [ngModel]="recipeFilter()" (ngModelChange)="recipeFilter.set($event)" placeholder="e.g. ssh, user, printer, mount" cdkFocusInitial />
                <mat-icon matSuffix>search</mat-icon>
              </mat-form-field>
              <div class="list" style="margin-top:8px">
                @for (group of recipeGroups(); track group.name) {
                  <div class="sec" style="padding: 0 12px">{{ group.name }}</div>
                  @for (r of group.items; track r.id) {
                    <div class="item" (click)="pickRecipe(r)">
                      <div class="t"><b>{{ r.title }}</b><div class="d">{{ r.description }}</div></div>
                      <mat-icon>chevron_right</mat-icon>
                    </div>
                  }
                }
              </div>
            } @else {
              <h3 style="margin:4px 0">{{ recipe()!.title }}</h3>
              <div class="muted" style="margin-bottom:8px">{{ recipe()!.description }}</div>
              <ng-container *ngTemplateOutlet="fieldsTpl" />
              <ng-container *ngTemplateOutlet="setPicker" />
            }
          }

          @case ('option') {
            @if (optStatus() && !optStatus()!.ready) {
              <div class="note"><mat-icon>info</mat-icon>
                <div>The option index is not loaded yet.
                  @if (optStatus()!.building) { Building it now; this can take a few minutes the first time. }
                  @else { <button mat-stroked-button (click)="buildOptions()">Build the option index</button> }
                  @if (optStatus()!.error) { <div class="warn">{{ optStatus()!.error }}</div> }
                </div>
              </div>
            }
            @if (!option()) {
              <mat-form-field appearance="outline" style="width:100%" subscriptSizing="dynamic">
                <mat-label>Search options</mat-label>
                <input matInput [ngModel]="optQuery()" (ngModelChange)="optQuery.set($event); optSearch$.next($event)" placeholder="e.g. openssh, firewall ports, kernel" cdkFocusInitial />
                <mat-icon matSuffix>search</mat-icon>
              </mat-form-field>
              <div class="list">
                @for (o of optResults(); track o.name) {
                  <div class="item" (click)="pickOption(o)">
                    <div class="t"><b class="mono">{{ o.name }}</b> <span class="tag">{{ o.type | slice:0:28 }}</span><div class="d">{{ o.description }}</div></div>
                    <mat-icon>chevron_right</mat-icon>
                  </div>
                } @empty { <div class="item muted" style="cursor:default">{{ optQuery().length < 2 ? 'Type to search.' : 'No options found.' }}</div> }
              </div>
            } @else {
              <ng-container *ngTemplateOutlet="optionTpl" />
            }
          }

          @case ('custom') {
            <ng-container *ngTemplateOutlet="fieldsTpl" />
          }
        }
      }

      <!-- ── Step 3: preview ──────────────────────────────────────── -->
      @if (step() === 'preview') {
        <div class="crumb"><button mat-button (click)="step.set('details')"><mat-icon>arrow_back</mat-icon> Back</button><span>Review</span></div>
        <div class="muted">This will be added to <span class="mono">{{ data.file }}</span>. Existing settings with the same name are updated; lists are extended.</div>
        <pre class="preview mono">{{ preview() }}</pre>
        @for (w of warnings(); track w) { <div class="warning-box"><mat-icon>warning</mat-icon><span>{{ w }}</span></div> }
        @if (notes()) { <div class="note"><mat-icon>lightbulb</mat-icon><span>{{ notes() }}</span></div> }
      }
    </mat-dialog-content>

    <mat-dialog-actions align="end">
      <button mat-button mat-dialog-close>Cancel</button>
      @if (step() === 'details') {
        <button mat-flat-button (click)="toPreview()" [disabled]="!canContinue() || busy()">Continue <mat-icon>arrow_forward</mat-icon></button>
      }
      @if (step() === 'preview') {
        <button mat-flat-button (click)="apply()" [disabled]="busy()"><mat-icon>add</mat-icon> Add to module</button>
      }
    </mat-dialog-actions>

    <!-- Package set picker (only when the flake offers more than one) -->
    <ng-template #setPicker>
      @if (sets().length > 1) {
        <mat-form-field appearance="outline" style="width:100%" subscriptSizing="dynamic">
          <mat-label>Take packages from</mat-label>
          <mat-select [value]="pkgSet()" (valueChange)="changeSet($event)">
            @for (s of sets(); track s.name) { <mat-option [value]="s.name"><span class="mono">{{ s.name }}</span> · {{ s.label }}</mat-option> }
          </mat-select>
          <mat-hint>Packages are written as <span class="mono">{{ pkgSet() === 'pkgs' ? 'name' : pkgSet() + '.name' }}</span>; extra sets come from the flake's inputs.</mat-hint>
        </mat-form-field>
      }
    </ng-template>

    <!-- Recipe / custom fields -->
    <ng-template #fieldsTpl>
      <div class="fields">
        @for (group of fieldGroups(); track group.name) {
          @if (group.name) { <div class="sec-head">{{ group.name }}</div> }
          @for (f of group.fields; track f.name) {
            @switch (f.kind) {
              @case ('bool') {
                <div class="field-row"><mat-slide-toggle [checked]="values()[f.name] === 'true'" (change)="setValue(f.name, $event.checked ? 'true' : 'false')">{{ f.label }}</mat-slide-toggle></div>
                @if (f.help) { <div class="help" style="margin-top:2px">{{ f.help }}</div> }
              }
              @case ('select') {
                <mat-form-field appearance="outline"><mat-label>{{ f.label }}</mat-label>
                  <mat-select [value]="values()[f.name]" (valueChange)="setValue(f.name, $event)">@for (o of f.options; track o) { <mat-option [value]="o">{{ o }}</mat-option> }</mat-select>
                </mat-form-field>
                @if (f.help) { <div class="help">{{ f.help }}</div> }
              }
              @case ('password') {
                <mat-form-field appearance="outline"><mat-label>{{ f.label }}</mat-label>
                  <input matInput type="password" autocomplete="new-password" [value]="values()[f.name] ?? ''" (input)="setValue(f.name, $any($event.target).value)" />
                  <mat-icon matSuffix matTooltip="Only a SHA-512 hash is written to the file">lock</mat-icon>
                </mat-form-field>
                @if (f.help) { <div class="help">{{ f.help }}</div> }
              }
              @case ('partition') {
                <div class="muted" style="font-size:.9em; margin-bottom:6px">{{ f.label }}{{ f.required ? ' *' : '' }}
                  <button mat-icon-button (click)="loadDisks(true)" matTooltip="Rescan drives" style="width:28px;height:28px;line-height:28px;padding:0"><mat-icon style="font-size:18px;width:18px;height:18px">refresh</mat-icon></button>
                </div>
                @if (disksLoading()) { <mat-progress-bar mode="indeterminate" /> }
                @for (d of disks(); track d.path) {
                  <div class="disk">
                    <div class="disk-head"><mat-icon>{{ d.removable ? 'usb' : 'storage' }}</mat-icon><b>{{ d.model || d.path }}</b><span class="muted">{{ d.path }} · {{ size(d.size) }}{{ d.transport ? ' · ' + d.transport : '' }}</span></div>
                    @for (p of d.partitions; track p.path) {
                      <div class="part" [class.sel]="values()[f.name] === p.uuid" [class.off]="!p.uuid || p.system" (click)="pickPartition(f, p)"
                           [matTooltip]="p.system ? 'This partition is part of the running system' : (!p.uuid ? 'No file system UUID: format the partition first' : '')">
                        <mat-icon>{{ values()[f.name] === p.uuid ? 'radio_button_checked' : 'radio_button_unchecked' }}</mat-icon>
                        <div class="t"><b class="mono">{{ p.path }}</b><span class="muted" style="margin-left:8px">{{ size(p.size) }}</span>{{ p.label ? ' · ' + p.label : '' }}{{ p.partlabel && p.partlabel !== p.label ? ' · ' + p.partlabel : '' }}
                          <div class="d">{{ p.fstype ?? 'no file system' }}{{ p.mountpoint ? ' · mounted at ' + p.mountpoint : ' · not mounted' }}{{ p.uuid ? ' · ' + p.uuid : '' }}</div>
                        </div>
                      </div>
                    } @empty { <div class="part off">No partitions</div> }
                  </div>
                } @empty { @if (!disksLoading()) { <div class="muted" style="margin-bottom:8px">No drives found.</div> } }
                <mat-form-field appearance="outline" subscriptSizing="dynamic" style="width:100%"><mat-label>or type the UUID</mat-label>
                  <input matInput [value]="values()[f.name] ?? ''" (input)="setValue(f.name, $any($event.target).value)" class="mono" [placeholder]="f.placeholder ?? ''" />
                </mat-form-field>
                @if (f.help) { <div class="help" style="margin-top:6px">{{ f.help }}</div> }
              }
              @case ('secret') {
                @if (secrets(); as sec) {
                  @if (!sec.exists) {
                    <div class="note"><mat-icon>key</mat-icon><div>No managed secrets yet. Create them on the <b>Secrets</b> page, then come back. You can also choose another login method above.</div></div>
                  } @else if (!sec.unlocked) {
                    <div class="note"><mat-icon>lock</mat-icon><div>Unlock the secrets to pick one.
                      <div class="field-row" style="margin-top:6px"><input class="mono" type="password" [(ngModel)]="secretsPw" placeholder="secrets password" style="font:inherit;padding:6px 8px;border-radius:6px;border:1px solid var(--mat-sys-outline-variant);background:var(--mat-sys-surface);color:inherit" (keyup.enter)="unlockSecrets()" />
                      <button mat-stroked-button (click)="unlockSecrets()" [disabled]="!secretsPw">Unlock</button></div></div></div>
                  } @else {
                    <mat-form-field appearance="outline"><mat-label>{{ f.label }}</mat-label>
                      <mat-select [value]="values()[f.name]" (valueChange)="setValue(f.name, $event)">
                        @for (e of sec.entries; track e.name) { <mat-option [value]="e.name"><span class="mono">{{ e.name }}</span> · {{ e.kind === 'smb' ? 'user ' + e.username : 'text' }}</mat-option> }
                      </mat-select>
                    </mat-form-field>
                    @if (!sec.entries.length) { <div class="help">No entries yet; add one on the Secrets page.</div> }
                  }
                }
                @if (f.help) { <div class="help">{{ f.help }}</div> }
              }
              @case ('lines') {
                <div class="muted" style="font-size:.9em; margin-bottom:4px">{{ f.label }}</div>
                <textarea class="lines" [value]="values()[f.name] ?? ''" (input)="setValue(f.name, $any($event.target).value)" [placeholder]="f.placeholder ?? ''" spellcheck="false"></textarea>
                @if (f.help) { <div class="help" style="margin-top:4px">{{ f.help }}</div> }
              }
              @case ('list') {
                @if (f.options.length) {
                  <div class="muted" style="font-size:.9em; margin-bottom:6px">{{ f.label }}</div>
                  <div class="sugg">
                    @for (o of f.options; track o) {
                      <label [class.on]="listHas(f.name, o)"><mat-checkbox [checked]="listHas(f.name, o)" (change)="listToggle(f.name, o, $event.checked)" style="display:none" />{{ listHas(f.name, o) ? '✓ ' : '' }}{{ o }}</label>
                    }
                  </div>
                }
                <mat-form-field appearance="outline"><mat-label>{{ f.options.length ? 'Other ' + f.label.toLowerCase() : f.label }}{{ f.required ? ' *' : '' }}</mat-label>
                  <input matInput [value]="values()[f.name] ?? ''" (input)="setValue(f.name, $any($event.target).value)" [placeholder]="f.placeholder ?? 'a, b, c'" class="mono" />
                </mat-form-field>
                @if (f.help) { <div class="help">{{ f.help }}</div> }
              }
              @default {
                <mat-form-field appearance="outline"><mat-label>{{ f.label }}{{ f.required ? ' *' : '' }}</mat-label>
                  <input matInput [value]="values()[f.name] ?? ''" (input)="setValue(f.name, $any($event.target).value)" [placeholder]="f.placeholder ?? ''" [type]="f.kind === 'number' ? 'number' : 'text'" [class.mono]="f.kind !== 'text'" />
                </mat-form-field>
                @if (f.help) { <div class="help">{{ f.help }}</div> }
              }
            }
          }
        }
        @if (activeFields().length === 0) { <div class="muted">Nothing to configure. Continue to review.</div> }
      </div>
    </ng-template>

    <!-- Single option editor -->
    <ng-template #optionTpl>
      <div class="crumb"><button mat-button (click)="option.set(null)"><mat-icon>arrow_back</mat-icon> Other option</button></div>
      <h3 class="mono" style="margin:4px 0">{{ option()!.name }}</h3>
      <div class="doc">
        <div class="desc">{{ option()!.description }}</div>
        <dl>
          <dt>Type</dt><dd>{{ option()!.type }}</dd>
          @if (option()!.default) { <dt>Default</dt><dd class="mono">{{ option()!.default }}</dd> }
          @if (option()!.example) { <dt>Example</dt><dd class="mono" style="white-space:pre-wrap">{{ option()!.example }}</dd> }
        </dl>
      </div>
      @if (optionIsBool()) {
        <div class="field-row"><mat-slide-toggle [checked]="optValue() === 'true'" (change)="optValue.set($event.checked ? 'true' : 'false')">{{ optValue() === 'true' ? 'Enabled' : 'Disabled' }}</mat-slide-toggle></div>
      } @else {
        <mat-form-field appearance="outline" style="width:100%"><mat-label>Value (as Nix)</mat-label>
          <input matInput class="mono" [ngModel]="optValue()" (ngModelChange)="optValue.set($event)" [placeholder]="optionPlaceholder()" />
          <mat-hint>{{ optionHint() }}</mat-hint>
        </mat-form-field>
        @if (optionIsList()) { <mat-checkbox [ngModel]="optAppend()" (ngModelChange)="optAppend.set($event)">Add to the existing list instead of replacing it</mat-checkbox> }
      }
    </ng-template>
  `,
})
export class WizardDialogComponent implements OnInit {
  private api = inject(ApiService);
  private ref = inject(MatDialogRef<WizardDialogComponent, NixFileView | undefined>);
  data = inject<WizardData>(MAT_DIALOG_DATA);

  step = signal<Step>('pick');
  category = signal<Category | null>(null);
  busy = signal(false);
  error = signal<string | null>(null);

  // package sets
  private ws = inject(WorkspaceService);
  sets = computed<PackageSet[]>(() => this.ws.packageSets().filter(s => !s.bits32));
  pkgSet = signal('pkgs');

  // programs
  pkgQuery = signal('');
  pkgResults = signal<PackageInfo[]>([]);
  popular = signal<PopularPackage[]>([]);
  pickPopular(p: PopularPackage) {
    this.togglePkg({ attr: p.attr, pname: p.attr, version: '', description: p.description } as PackageInfo);
  }
  selectedPkgs = signal<(PackageInfo & { set: string })[]>([]);
  pkgSlowHint = signal(false);
  pkgSearch$ = new Subject<string>();

  // features
  recipes = signal<Recipe[]>([]);
  recipeFilter = signal('');
  recipe = signal<Recipe | null>(null);
  values = signal<Record<string, string>>({});

  // options
  optStatus = signal<OptionsStatus | null>(null);
  optQuery = signal('');
  optResults = signal<OptionDoc[]>([]);
  option = signal<OptionDoc | null>(null);
  optValue = signal('');
  optAppend = signal(false);
  optSearch$ = new Subject<string>();

  // preview
  changes = signal<ConfigChange[]>([]);
  preview = signal('');
  warnings = signal<string[]>([]);
  notes = signal<string | null>(null);

  private customRecipe: Recipe = {
    id: 'custom', title: 'Custom setting', description: '', category: 'Advanced', keywords: [], notes: null,
    fields: [
      { name: 'key', label: 'Option', kind: 'text', default: null, help: 'Attribute path, e.g. services.openssh.enable', placeholder: 'services.foo.enable', options: [], required: true, section: null },
      { name: 'value', label: 'Value (Nix)', kind: 'text', default: null, help: 'Written as Nix: true, 42, "text", [ 1 2 ], pkgs.foo', placeholder: 'true', options: [], required: true, section: null },
    ],
  };

  ngOnInit() {
    this.api.packagesPopular().subscribe({ next: p => this.popular.set(p), error: () => {} });
    this.api.recipes().subscribe(r => this.recipes.set(r.filter(x => x.id !== 'custom')));
    this.api.optionsStatus().subscribe(s => this.optStatus.set(s));
    this.pkgSearch$.pipe(debounceTime(350), distinctUntilChanged(), switchMap(q => {
      if (q.trim().length < 2) { return of(null); }
      this.busy.set(true);
      return this.api.packagesSearch(q, this.pkgSet(), this.data.path).pipe(catchError((e: ApiFailure) => { this.error.set(e.message); return of(null); }));
    })).subscribe(res => {
      this.busy.set(false);
      if (res) { this.pkgResults.set(res.results); this.pkgSlowHint.set(false); }
      else this.pkgResults.set([]);
    });
    this.optSearch$.pipe(debounceTime(250), distinctUntilChanged(), switchMap(q => q.trim().length < 2 ? of([]) : this.api.optionsSearch(q).pipe(catchError(() => of([])))))
      .subscribe(r => this.optResults.set(r));
  }

  choose(c: Category) {
    this.category.set(c);
    this.error.set(null);
    this.step.set('details');
    if (c === 'custom') { this.recipe.set(this.customRecipe); this.values.set({}); }
    if (c === 'programs' && !this.pkgResults().length) this.pkgSlowHint.set(true);
    if (c === 'option') this.api.optionsStatus().subscribe(s => { this.optStatus.set(s); if (!s.ready && !s.building) this.buildOptions(); });
  }

  back() {
    if (this.category() === 'feature' && this.recipe()) { this.recipe.set(null); return; }
    if (this.category() === 'option' && this.option()) { this.option.set(null); return; }
    this.step.set('pick');
    this.category.set(null);
    this.recipe.set(null);
    this.option.set(null);
  }

  categoryTitle() {
    return { programs: 'Install programs', feature: 'Turn on a feature or service', option: 'Any NixOS option', custom: 'Custom setting' }[this.category() ?? 'custom'];
  }

  // ── programs ──
  pkgHint() {
    if (this.pkgSlowHint()) return 'The first search builds a cache and can take 20 seconds; later searches are instant.';
    return `${this.selectedPkgs().length} selected`;
  }
  isSelected(p: PackageInfo) { return this.selectedPkgs().some(s => s.attr === p.attr && s.set === this.pkgSet()); }
  togglePkg(p: PackageInfo & { set?: string }) {
    const set = p.set ?? this.pkgSet();
    this.selectedPkgs.update(list => list.some(s => s.attr === p.attr && s.set === set) ? list.filter(s => !(s.attr === p.attr && s.set === set)) : [...list, { ...p, set }]);
  }
  changeSet(name: string) {
    this.pkgSet.set(name);
    const q = this.pkgQuery();
    if (q.trim().length >= 2) { this.pkgResults.set([]); this.pkgSearch$.next(q + ' '); this.pkgSearch$.next(q); }
  }

  // ── features ──
  recipeGroups = computed(() => {
    const q = this.recipeFilter().trim().toLowerCase();
    const match = (r: Recipe) => !q || r.title.toLowerCase().includes(q) || r.description.toLowerCase().includes(q) || r.keywords.some(k => k.includes(q));
    const groups = new Map<string, Recipe[]>();
    for (const r of this.recipes().filter(match)) groups.set(r.category, [...(groups.get(r.category) ?? []), r]);
    return [...groups.entries()].map(([name, items]) => ({ name, items }));
  });
  pickRecipe(r: Recipe) {
    this.recipe.set(r);
    if (r.fields.some(f => f.kind === 'partition')) this.loadDisks();
    if (r.fields.some(f => f.kind === 'secret')) this.loadSecrets();
    const v: Record<string, string> = {};
    for (const f of r.fields) if (f.default != null) v[f.name] = f.default;
    this.values.set(v);
  }
  activeFields = computed(() => this.recipe()?.fields ?? []);
  fieldGroups = computed(() => {
    const groups: { name: string; fields: RecipeField[] }[] = [];
    for (const f of this.activeFields()) {
      const name = f.section ?? '';
      let g = groups.find(x => x.name === name);
      if (!g) { g = { name, fields: [] }; groups.push(g); }
      g.fields.push(f);
    }
    return groups;
  });
  setValue(name: string, value: string) { this.values.update(v => ({ ...v, [name]: value })); }

  // ── secrets picker ──
  secrets = signal<SecretsStatus | null>(null);
  secretsPw = '';
  loadSecrets() { this.api.secretsStatus(this.data.path).subscribe({ next: s => this.secrets.set(s), error: () => {} }); }
  unlockSecrets() {
    this.api.secretsUnlock(this.data.path, this.secretsPw).subscribe({ next: s => { this.secrets.set(s); this.secretsPw = ''; }, error: (e: ApiFailure) => this.error.set(e.message) });
  }

  // ── disks (partition picker) ──
  disks = signal<DiskInfo[]>([]);
  disksLoading = signal(false);
  loadDisks(force = false) {
    if (this.disks().length && !force) return;
    this.disksLoading.set(true);
    this.api.disks().subscribe({ next: d => { this.disks.set(d); this.disksLoading.set(false); }, error: (e: ApiFailure) => { this.disksLoading.set(false); this.error.set(e.message); } });
  }
  pickPartition(f: RecipeField, p: PartitionInfo) {
    if (!p.uuid || p.system) return;
    this.setValue(f.name, p.uuid);
    // Fill the file system field when the recipe has one and the type is a known choice.
    const fs = this.activeFields().find(x => x.name === 'fstype');
    if (fs && p.fstype && fs.options.includes(p.fstype)) this.setValue('fstype', p.fstype);
  }
  size(bytes: number) {
    const u = ['B', 'KB', 'MB', 'GB', 'TB'];
    let i = 0; let n = bytes;
    while (n >= 1000 && i < u.length - 1) { n /= 1000; i++; }
    return `${n < 10 ? n.toFixed(1) : Math.round(n)} ${u[i]}`;
  }
  private listItems(name: string) { return (this.values()[name] ?? '').split(',').map(x => x.trim()).filter(Boolean); }
  listHas(name: string, item: string) { return this.listItems(name).includes(item); }
  listToggle(name: string, item: string, on: boolean) {
    const items = this.listItems(name).filter(x => x !== item);
    if (on) items.push(item);
    this.setValue(name, items.join(', '));
  }

  // ── options ──
  buildOptions() {
    this.api.optionsBuild().subscribe(s => {
      this.optStatus.set(s);
      if (s.building) {
        const timer = setInterval(() => this.api.optionsStatus().subscribe(st => { this.optStatus.set(st); if (!st.building) clearInterval(timer); }), 3000);
      }
    });
  }
  pickOption(o: OptionDoc) {
    this.option.set(o);
    this.optAppend.set(false);
    this.optValue.set(o.type === 'boolean' ? (o.default === 'true' ? 'false' : 'true') : '');
  }
  optionIsBool() { return this.option()?.type === 'boolean'; }
  optionIsList() { return (this.option()?.type ?? '').startsWith('list of'); }
  optionPlaceholder() {
    const t = this.option()?.type ?? '';
    if (t.startsWith('string')) return '"text"';
    if (t.includes('integer')) return '42';
    if (t.startsWith('list of')) return this.optAppend() ? 'one item, e.g. 8080 or "name"' : '[ 1 2 ]';
    if (t.startsWith('package')) return 'pkgs.something';
    return this.option()?.example ?? '';
  }
  optionHint() {
    const t = this.option()?.type ?? '';
    if (t.startsWith('string')) return 'Strings need quotes: "like this"';
    if (t.startsWith('list of')) return 'A Nix list like [ "a" "b" ], or one item if adding to the list';
    return 'Any Nix expression';
  }

  // ── flow ──
  canContinue() {
    switch (this.category()) {
      case 'programs': return this.selectedPkgs().length > 0;
      case 'feature': return !!this.recipe() && this.activeFields().every(f => !f.required || !!(this.values()[f.name] ?? '').trim());
      case 'option': return !!this.option() && !!this.optValue().trim();
      case 'custom': return !!(this.values()['key'] ?? '').trim() && !!(this.values()['value'] ?? '').trim();
      default: return false;
    }
  }

  toPreview() {
    this.error.set(null);
    this.warnings.set([]);
    this.notes.set(null);
    const cat = this.category();
    if (cat === 'programs') {
      const changes: ConfigChange[] = this.selectedPkgs().map(p => ({ kind: 'add_package', attr: p.attr, set: p.set === 'pkgs' ? null : p.set }));
      this.changes.set(changes);
      this.preview.set(`environment.systemPackages = with pkgs; [ ${this.selectedPkgs().map(p => (p.set === 'pkgs' ? '' : p.set + '.') + p.attr).join(' ')} ];  # added to the list`);
      this.step.set('preview');
      return;
    }
    if (cat === 'option') {
      const o = this.option()!;
      const v = this.optValue().trim();
      const changes: ConfigChange[] = this.optAppend() && this.optionIsList()
        ? [{ kind: 'append_list', key: o.name, item: v }]
        : [{ kind: 'set', key: o.name, value: v }];
      this.changes.set(changes);
      this.preview.set(this.optAppend() && this.optionIsList() ? `${o.name} = [ ${v} ];  # added to the list` : `${o.name} = ${v};`);
      this.step.set('preview');
      return;
    }
    const id = cat === 'custom' ? 'custom' : this.recipe()!.id;
    this.busy.set(true);
    this.api.renderRecipe(id, this.values(), this.pkgSet() === 'pkgs' ? null : this.pkgSet()).subscribe({
      next: r => {
        this.busy.set(false);
        this.changes.set(r.changes);
        this.preview.set(r.preview);
        this.warnings.set(r.warnings);
        this.notes.set(r.notes);
        this.step.set('preview');
      },
      error: (e: ApiFailure) => { this.busy.set(false); this.error.set(e.message); },
    });
  }

  apply() {
    this.busy.set(true);
    this.error.set(null);
    this.api.applyChanges(this.data.path, this.data.file, this.data.version, this.changes()).subscribe({
      next: view => { this.busy.set(false); this.ref.close(view); },
      error: (e: ApiFailure) => { this.busy.set(false); this.error.set(e.message); },
    });
  }
}
