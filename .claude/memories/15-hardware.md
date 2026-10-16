# hardware-configuration.nix handling (2026-10-16, Vincent's design)

Trigger: the lg-laptop deploy activated a config whose hardware-configuration
.nix (last touched 2025-08-15 in the lg-nixos checkout) named disk UUIDs that
no longer exist on the laptop → boot.mount / swap.target / local-fs.target
failed → systemd emergency mode → laptop off the network, job hung. Vincent:
"hardware-configuration.nix is a config the user should rarely touch; the
system copy is the source of truth."

Rules (crates/core/src/hardware.rs):
1. **Open a working folder** → `POST /api/location/hardware-sync {path}` is
   called by `WorkspaceService.open()` (so also on URL reload). It copies
   /etc/nixos/hardware-configuration.nix into the folder: `copied` when
   missing, `refreshed` when different (overwrites; snackbar tells the user),
   `unchanged`, `no-system-file`, `not-writable`, `skipped` (folder IS
   /etc/nixos). File location = `rel_path_in()`: root if present, else the
   single hardware-configuration.nix found in a subfolder (hosts/x/…), else
   root. Two candidates → root.
2. **Build on this machine** (incl. remote builders / build_host):
   `DeployPreflight.hardware: HardwareStatus {system_exists, local_exists,
   rel_path, differs, unified_diff}`. When `differs`, the build dialog shows
   an amber box with a radio: "Use the system copy (recommended)" (default)
   vs "I changed it on purpose: apply the working copy's version", plus a
   diff expander. `DeployRequest.hardware: Option<HardwareChoice>`
   (`system` | `workspace`). The root build script copies
   /etc/nixos/hardware-configuration.nix over `$WORKDIR/$HW_REL` unless the
   choice is `workspace` (then it logs "as confirmed" and copies nothing).
3. **Build here, deploy to target**: the working copy's file is IGNORED. After
   staging, `remote::fetch_target_hardware` runs `target_hardware_script()`
   over ssh: `/etc/nixos/hardware-configuration.nix` if present (`NM_HW=etc`),
   else `nixos-generate-config --show-hardware-config` (`NM_HW=generate`,
   must mention fileSystems/boot.), else `NM_HW=none` → job fails before
   building with "run sudo nixos-generate-config on the target". The fetched
   file is written into the staged copy at `rel_path_in(workdir)`.
   `RemoteCheck.hardware_config` ("etc"|"generate"|"none") drives the build
   dialog: red error box + Start disabled for none, amber note for generate.

Tests: hardware.rs unit tests (copy/refresh/unchanged/no system file,
subfolder detection, ambiguity), deploy.rs `script_mirrors_sync_sh` covers
both choices, remote.rs `target_hardware_script_parses`. E2E on the 7799 test
daemon with the fake target: a workspace file with a marker never reaches the
staged build (fake nixos-rebuild copies `<flake>/hardware-configuration.nix`
to state/ for inspection).

Not done / ideas: UUID existence check against the target (moot now that the
target's own file is used); a "restore from system" button on the file view;
remote `generate` path runs nixos-generate-config as the SSH user (works
without root, may print warnings which are discarded).

## Pull from target + "use the working copy's file" (2026-10-16, Vincent)
- Remote page: per-target download button → `POST /api/remote/pull-hardware
  {target, path}` → `remote::fetch_target_hardware` (etc or generate) →
  `hardware::install_pulled` writes it at `rel_path_in(workdir)` and records
  `AppState.hardware_pulls[path] = HardwarePull {target, sha256, at}`.
- The sync-on-open and the preflight take that record: while the file's
  sha256 still matches, sync returns `from-target` (file kept, no snackbar)
  and `HardwareStatus.pulled_from` is set (dialogs mention "copied from
  target X on <date>"). Once edited, it is an ordinary drifted file again and
  this machine's copy wins on the next open (test
  `pulled_copy_survives_sync_until_edited`).
- Build dialog, remote target: checkbox "Use the working copy's file instead
  of the target's /etc/nixos/hardware-configuration.nix" (off by default).
  Off → plain info note (Vincent: a warning there worries users); warnings only when the working copy file is used.
  On → green-ish note when it is a pulled copy of that target, amber warning
  otherwise, red + Start disabled when the working copy has none.
  Sent as `DeployRequest.hardware = 'workspace' | 'system'`; `run_remote`
  skips the fetch for `workspace` and logs that the target's file is NOT used.
