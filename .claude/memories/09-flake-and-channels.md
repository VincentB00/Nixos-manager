# Flake page, channels page, switch-to-flakes

Added 2026-10-14/15 at Vincent's request: configuration.nix, flake.nix and
hardware-configuration.nix are the three key files; flake.nix gets special
treatment, and classic setups get channel management plus a one-click
conversion to flakes.

## Backend
- `crates/core/src/flake.rs`
  - `template(hostname, username, system, nixpkgs_ref)`: flake.nix modelled on
    Vincent's own (`vars`, `system`, `channels` list, `mkPkgs`/`channelPkgs`
    generating `pkgs-<name>` and `pkgs-<name>-32` specialArgs,
    `nixosConfigurations.<host>` with `./configuration.nix`). nixpkgs defaults
    to **nixos-unstable** (Vincent's requirement). The flake output name
    defaults to **main** (decided 2026-10-15: Vincent keeps one branch per
    machine, so identical flakes matter more than hostname lookup); the
    dialog lets the user type any name, and the app always passes the chosen
    host explicitly, so any name works. Comments sit above the
    inputs block so appended entries land at the end.
  - `convert(dir, ...)`: writes flake.nix (refuses if present) and appends
    `"nix-command" "flakes"` to `nix.settings.experimental-features` in
    configuration.nix via the merge-in edit engine.
  - `info()` → `FlakeInfo`: inputs (both `inputs = { a.url=..; b = {url; inputs.x.follows} }`
    and top-level `inputs.a.url` forms), follows, `vars`, `channels`,
    `system`, hosts, `has_template_structure`, lock data (rev/date/source per
    input) read from `<dir>/flake.lock` or `/etc/nixos/flake.lock`.
  - `apply(FlakeOp)`: add_input / remove_input / set_input_url / add_channel
    (input nixpkgs-<name> + channels entry) / remove_channel / set_var /
    remove_var. Span-based, re-parse-validated. Verified on Vincent's real
    flake.nix (read-only) and on a converted copy of test-nix-manager.
- `crates/core/src/channels.rs`: system channels from
  `/nix/var/nix/profiles/per-user/root/channels/<name>/.version` (no root
  needed; URLs unknown without root), user channels via `nix-channel --list`,
  upstream list from the nix-channels S3 listing (fallback static list).
  Actions add/remove/update as jobs; system scope runs through
  `deploy::run_root_script` (extracted sudo runner) with the UI password.
- Routes: `/api/flake/{info,edit,convert}`, `/api/channels`,
  `/api/channels/action`.

## UI
- Sidebar: "Flake" entry for flake configs, "Channels" entry for classic
  ones. flake.nix in the file list/tree opens the Flake page (code view one
  click away).
- `pages/flake/flake.component.ts`: cards for main nixpkgs (branch change,
  locked rev/date), extra package sets ("Add a previous nixpkgs version"
  picks from upstream channel names, derives name 25_11), global vars
  (add/edit/remove), all inputs (add with follows, change URL, remove), hosts
  + lock path. Non-template flakes: inputs only, with a note.
- `pages/channels/channels.component.ts`: system/user channels, add, remove,
  update, "Change NixOS version" (re-adds `nixos` channel), password prompt
  via the shared PromptDialog (`password: true`), jobs page for output.
  Hero card "Switch to flakes" → `convert-flake-dialog.component.ts`
  (hostname prefilled from the machine, username, branch select).
- Overview card also offers Switch to flakes / Flake settings.

## Notes
- After conversion the deploy uses `--flake /etc/nixos#<host>` automatically
  because `location_info` now reports kind=flake.
- The template's `nix flake` evaluation was checked with
  `nix eval path:<copy>#nixosConfigurations.<host>.config.networking.hostName`.

## flake.lock lives in /etc/nixos only (Vincent, 2026-10-16)
"I don't want the workspace to have flake.lock at all." Rules now:
- Opening a folder removes a stray `<workdir>/flake.lock`
  (`flake::remove_workspace_lock`, called from the hardware-sync endpoint,
  reported as `HardwareSync.lock_removed` → snackbar). Never touches
  /etc/nixos itself.
- `flake::read_lock` reads /etc/nixos/flake.lock only (Flake page "locked"
  info, lock_matches).
- Staged copies (validate + remote deploy) skip `flake.lock` by name;
  validate borrows /etc/nixos's; remote deploy pins to the TARGET's
  /etc/nixos/flake.lock (fetched over ssh via `fetch_target_file`), falls
  back to this machine's, resolves fresh on upgrade or when neither exists.
  The lock is never written back to the working folder (the old copy-back
  in run_remote is gone — it is what had put a flake.lock into lg-nixos).
- Local build script: after `cp -r workdir/* /etc/nixos`, `rm -f
  /etc/nixos/flake.lock` before restoring the preserved one, so a stray
  workspace lock can never seed the system (test in `script_mirrors_sync_sh`).
Consequence: remote deploys stay pinned per target; a target's lock only
changes when a deployment with "upgrade" is done there… but note the
target's /etc/nixos is NOT written by remote deploys, so its lock never
advances by itself — an "upgrade" deploy resolves fresh each time until
someone updates /etc/nixos/flake.lock on the target (e.g. a local build
there). Acceptable for now; revisit if Vincent wants the resolved lock
pushed to the target's /etc/nixos.
