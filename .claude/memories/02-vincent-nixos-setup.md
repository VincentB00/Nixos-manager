# Vincent's current NixOS setup

Last updated: 2026-10-01. This is the first real-world config the tool must
handle, and the layout it should be able to migrate from.

## Where the config lives

- Working repo: `/home/vincent/Desktop/code/private/nixos`, remote
  `github.com/VincentB00/nixos`. This is the source of truth, not `/etc/nixos`.
- `/etc/nixos` is a bind mount from a second NVMe partition, owned
  `nobody:nogroup`, with no `.git`. It is populated by `sync.sh`.
- Desktop machine: hostname `Desktop1-nixos`, NixOS 26.05 (flake pins
  nixos-26.05 plus nixos-unstable and nixos-25.11), KDE Plasma 6 on Wayland,
  AMD GPU, Nix 2.34.

## Layout conventions

- **Flat directory, one `.nix` file per concern**: `game-tool.nix`,
  `docker.nix`, `amd-gpu.nix`, `mount-smb.nix`, `homeManager.nix`, about 40
  files. Each is a plain NixOS module with no options of its own.
- **`configuration.nix` is the composition point.** Its `imports` list is the
  feature toggle: a module is enabled or disabled by commenting its line.
- **One git branch per machine**: `main`/`desktop1`, `desktop2` (the current
  desktop), `thinkpad-T590`, `cyber-server-1`, `cyber-server-2`, `VM2-25.05`.
  Each branch has its own `flake.nix`, imports list, hostname in
  `networking.nix`, and host-specific modules (`intel.nix`, `T590.nix`,
  `kubernetes.nix`, `unbound.nix`). Every branch exposes a single output
  `nixosConfigurations.main`. Branch diffs show accidental drift between hosts.
- **Multi-channel packages via specialArgs.** `flake.nix` has a `channels`
  list and generates `pkgs-<name>` and `pkgs-<name>-32` (i686) for each, e.g.
  `pkgs-unstable`, `pkgs-25_11`. Modules choose per package:
  `pkgs-unstable.steam` next to `pkgs.lutris`.
- **Shared `vars`** in specialArgs: `mainUsername`, `mainUserGroup`,
  `mainUserEmail`, `mainUserHome`, `mainNixosVersion`.
- **Home Manager as a NixOS module** (`homeManager.nix`), plus plasma-manager
  passed through specialArgs. Contains real hand-written Nix logic (helper
  functions generating Thunderbird accounts), which is why the tool must not
  try to round-trip user modules.
- `hardware-configuration.nix` is gitignored; copied from `/etc/nixos`.
- `script/` holds ad-hoc bash tools (generation switcher, GPU switch, channel
  manager using legacy `nix-channel`, disk wipe). Differs per branch.

## Deploy workflow today

`sync.sh` (run as root from the working copy): backs up `/etc/nixos/flake.lock`,
`rm -rf /etc/nixos/*`, copies the working tree in, restores the lock (or
deletes it with `--upgrade`), then `nixos-rebuild`. `flake.lock` is therefore
never committed. With flakes none of this copying is necessary; the tool
should build directly from the checkout.

## Known issues observed (for the owner, not for the tool to fix)

- Git remote URL embeds a GitHub personal access token in plain text.
  Recommended rotating it and switching to SSH or a credential helper.
- SMB credentials are plain files referenced by absolute `/etc/nixos/...` paths.
- Commit messages are all "update".
- None of nh, nvd, nom, nix-tree are installed.
