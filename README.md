# Nixos-manager

A web GUI for NixOS configurations, built on flakes (classic `configuration.nix`
setups work too). The goal is to make a NixOS machine manageable by someone who
does not know NixOS: open the folder that holds your configuration, see every
file and what it does, change settings through forms, and rebuild from the
browser.

**Status:** early. The first big feature (discover, map, edit, build) works;
expect rough edges.

## What it does today

- **Pick a configuration folder** (remembered for next time), with suggestions
  found on the machine and a server-side folder browser.
- **Discover and map**: every file classified (flake, modules, hardware config,
  scripts, secrets), the import graph from the entry file, commented-out
  modules shown as disabled, files nothing imports flagged, each module tagged
  with the option areas it touches.
- **Form view of any .nix file**: settings become toggles, text fields and
  lists. Add, change, remove or comment out items. Anything the form cannot
  represent is shown as code and editable as code. Every edit is a surgical
  text change that keeps comments and formatting, and is rejected if it would
  break the file's syntax.
- **Build & apply**: replicates the owner's `sync.sh` flow (preserve
  `flake.lock`, copy `hardware-configuration.nix` back, resolve a
  `configuration.nix` conflict, copy the working copy to `/etc/nixos`) and then
  runs `nixos-rebuild switch|boot|test|build`, streaming the log. Your sudo
  password is asked once per build and never stored.

## Architecture

| Part | Where | Notes |
|---|---|---|
| Core library | `crates/core` | rnix-based parsing/editing, discovery, deploy jobs. No UI or HTTP. |
| Daemon | `crates/daemon` | axum HTTP + WebSocket API, serves the built UI from one binary. |
| CLI | `crates/cli` | `nixos-manager info|overview|file|hosts|suggest|generations` |
| UI | `ui/` | Angular 22 + Material. Types are generated from Rust (`scripts/gen-types.sh`). |
| Nix | `flake.nix`, `nix/module.nix` | dev shell, package, `services.nixos-manager` user service |

The daemon runs as **your user**, listens on `127.0.0.1:7788`, and only the
sync + rebuild step is privileged (through `sudo -S` with the password you
type in the UI). Do not expose the port without TLS.

The app's own state is one small JSON file
(`~/.local/state/nixos-manager/state.json`): recent folders and preferences,
never configuration contents.

## Run it

The quickest way on any machine with Nix:

```bash
./run.sh            # nix-shell with the toolchains, builds what is stale, opens the browser
./run.sh --dev      # same, but with the Angular dev server and live reload on :4200
./run.sh --watch    # rebuild and restart automatically whenever Rust or UI sources change
./run.sh --tui      # terminal UI instead of the browser: pick a folder, validate, build/deploy (to this machine, to a target, on a target, or compiled on a builder), manage targets (add, check, SSH key, pull hardware), generations / disk / garbage collection here or on a target, free memory before a build (for SSH sessions and headless machines)
./run.sh --help     # all options
```

## Run it by hand (development)

```bash
# terminal 1: API + serves ui/dist if built
cargo run -p nixos-manager-daemon

# terminal 2: Angular dev server with live reload, proxies /api to :7788
cd ui && npm install && npm start      # http://localhost:4200

# or build the UI once and use the daemon alone on http://127.0.0.1:7788
cd ui && npm run build
```

With Nix: `nix develop` gives you cargo, node and helpers.

## Tests

```bash
cargo test            # core: parser, edit engine, deploy script generation
scripts/gen-types.sh  # regenerate TypeScript API types after changing crates/core/src/types.rs
```

## Project notes

Design decisions, the owner's config layout, and the roadmap live in
[.claude/memories/](.claude/memories/). Start with `README.md` there.
