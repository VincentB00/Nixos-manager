# Project goal

Last updated: 2026-10-01

Nixos-manager is a GUI for managing NixOS systems. Vincent (repo owner,
VincentB00 on GitHub) stated three requirements:

1. **As flexible as possible.** It must not force a particular config layout.
   It should open any flake and offer at least read-only insight and the
   standard operations (build, switch, rollback, update inputs, gc).
2. **Built mainly on the NixOS flake feature.** Flakes are the primary model:
   `nixosConfigurations.<host>`, flake inputs and lock, `nix eval`, `nix build`.
3. **Runs on any NixOS machine**, not only Vincent's desktop. That includes
   headless servers and VMs with no display, other desktops (GNOME, Hyprland,
   X11 or Wayland), and potentially aarch64.

Requirement 3 is the one that shaped the architecture most: a toolkit window
cannot run on a headless server, so the UI is a web page served by a local
daemon. See 04-architecture.md.

## Design principles derived from the requirements

- **Layered flexibility.** Introspection and operations need no conventions in
  the target flake. Editing features only activate when the repo adopts an
  optional convention (a GUI-owned file per host). Every feature degrades
  gracefully to read-only.
- **Never rewrite hand-written Nix.** Edits go to files the tool owns. The
  NixOS module system merges them with the user's own modules.
- **Drive the system's nix CLI**, do not bundle one. Parse JSON output
  defensively because hosts may run different NixOS releases.
- **Only the activation step needs root.** Builds and evals go through the Nix
  daemon as an unprivileged user.

## Prior art to look at (do not depend on)

- nixos-conf-editor and nix-software-center (snowfallorg, Rust + GTK, largely
  unmaintained): proof that editing from options.json works; they use nix-editor.
- nh, nvd, nix-output-monitor: Rust tools we may call or borrow ideas from.
- Cockpit: same daemon-plus-web shape; Vincent already runs it. A Cockpit
  plugin was considered as a shortcut and set aside in favor of a standalone
  service (decision 2026-10-01, see 03-stack-decisions.md).
