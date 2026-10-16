# Tech stack decisions

All decided 2026-10-01 in a planning conversation. No code exists yet.

## Final stack

| Layer | Choice | Notes |
|---|---|---|
| Backend daemon | **Rust**, axum, tokio | Runs as a systemd service per machine |
| Frontend | **Angular** (TypeScript), Angular Material, RxJS, xterm.js | Built by Angular CLI, embedded in the Rust binary via rust-embed |
| Rust to TS types | ts-rs | Generate TS interfaces from Rust API structs; may move to utoipa/OpenAPI later |
| Transport | REST (JSON) for queries/commands, one WebSocket for job events and logs | Unix socket by default, optional TCP with token auth |
| Packaging | Nix flake: `packages.default`, `nixosModules.default`, `devShells.default` | Cargo.lock and package-lock.json give reproducible builds |

## Alternatives considered and why they lost

- **Java + JavaFX** (Vincent's original plan). Dropped because: JavaFX runs
  only through XWayland on Linux (Project Wakefield native Wayland is not
  shipped), nixpkgs pins OpenJFX at 21, Maven/Gradle packaging in Nix needs a
  dependency hash bumped per change, JVM footprint is heavy on small servers,
  and above all a toolkit window cannot run on headless machines (requirement
  3). Vincent has a strong Java background, which is why Angular was chosen
  for the UI: its classes, decorators and DI feel familiar.
- **Node (Fastify) backend.** Viable fallback; Vincent knows it from his
  lo-vee project. Lost to Rust on footprint (~10 MB vs 50-80 MB), cleaner Nix
  packaging, and because the Nix tooling ecosystem (nh, nvd, nix-editor,
  rnix-parser) is Rust.
- **Java backend (Quarkus/Spring) with web UI.** Possible once the UI became a
  web page, but heaviest footprint and most packaging friction.
- **React frontend.** Was the first proposal (Vincent knows React from
  lo-vee). Angular chosen instead on 2026-10-01 at Vincent's request. Note:
  Vincent asked whether to use "Angular instead of TypeScript"; Angular is a
  TypeScript framework, so the choice was Angular vs React.
- **Tauri desktop shell.** Not needed for v1; the web UI covers all machines.
  Could be added later as a thin wrapper around the same UI.
- **Cockpit plugin.** Would give login, privilege and remote hosts for free
  but ties the UI to Cockpit's frame and plugin API. Set aside.
- **Qt/PySide6, GTK4/libadwaita, Electron, Compose Multiplatform.** All
  require a display or are heavy; ruled out by requirement 3.

## Toolchain facts checked on 2026-10-01

- Machine has rustc/cargo 1.95, node 24, python 3.13, no Go, no bun.
- nixpkgs: openjfx 21.0.3, jdk 21.0.12, cargo-tauri 2.11.0,
  `maven.buildMavenPackage` available.
