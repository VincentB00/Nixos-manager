# Status and next steps

Last updated: 2026-10-01

## Status

2026-10-02: first implementation done. Rust workspace (core/daemon/cli),
Angular UI, flake and NixOS module exist; Feature 1 (location prompt, state
file, discovery, map, form editing with code fallback) and the build job
(sync.sh semantics + nixos-rebuild with sudo password) are implemented and
verified against a copy of Vincent's config. See 07-implementation-status.md.
Vincent will now try it and change things as we go.

## Decisions made

- Goal and requirements: see 01-project-goal.md.
- Stack: Rust (axum) daemon + Angular UI, ts-rs for shared types, Nix flake
  packaging. See 03-stack-decisions.md.
- Architecture: daemon + web UI, layered flexibility, unprivileged daemon with
  one escalation point. See 04-architecture.md.
- Features: startup location prompt, JSON state file, Feature 1 = discover,
  map and edit the config structure with lossless CST edits and validation;
  guided actions for beginners. Editing principle revised to allow surgical
  edits of user files. See 06-features.md.

## Open decisions (defaults will be taken unless Vincent objects)

1. Multi-host: one daemon per machine, no SSH deploy in v1. (default: yes)
2. Editing convention: tool-owned `nixos-manager/<hostname>.nix` per host.
   Less central now that surgical edits of user files are allowed; may only be
   needed for generated lists. Not yet confirmed.
3. Should Vincent migrate his own config from branch-per-host to hosts in one
   flake? The tool must read both; migration unlocks multi-host views. Not yet
   answered.
4. (closed 2026-10-01) Non-flake supported; daemon runs as the user with a
   sudo password prompt at build time; git ignored for now; sync replicates
   sync.sh exactly with no added safeguards. See 06-features.md and
   04-architecture.md deploy flow.

## Next steps, in order

1. Vincent tries the app on his real config and reports what to change.
2. Run a real build job once (test action first) to validate the sudo flow
   and the sync script on a live machine.
3. `nix build` the flake end to end and try `services.nixos-manager` on a
   machine.
4. (done 2026-10-14) Guided wizard with recipes, option search with docs, package search. See 08-wizard.md.
5. Option docs on hover in the form view (index exists now); evaluated values.
6. Generations list + rollback in the UI (API exists: /api/generations).
7. Self-host Material Symbols so icons work offline.

## Working agreements with Vincent

- Keep these notes in `.claude/memories/` updated whenever something is
  understood or completed, as a knowledge transfer for future sessions
  (requested 2026-10-01).
- Plan and consult first; do not implement without a go-ahead.
