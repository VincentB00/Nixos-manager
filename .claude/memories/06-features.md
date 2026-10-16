# Features

Last updated: 2026-10-01. Agreed direction from a feature discussion with
Vincent; not yet implemented.

## Target audience (restated by Vincent on 2026-10-01)

The end goal is to make NixOS usable by people who do not know NixOS at all.
Power-user features (structure view, raw option editing) are still wanted, but
beginner-friendliness is the measure of success.

## Startup flow

1. On first open, the UI prompts for the location of the NixOS configuration.
   The browser cannot see the server's filesystem, so the daemon exposes a
   directory browser endpoint plus suggestions: `/etc/nixos`, any `flake.nix`
   with `nixosConfigurations` found under the user's home, recent locations.
2. The daemon validates the location: flake or not, has configuration.nix,
   is a git repo, which hosts it defines.
3. The location is saved to the app's JSON state file so it is auto-filled
   next time.

## App state: one JSON file

Small and flat, stored in the daemon's state directory. Holds: recent config
locations, last-opened location, per-location preferences (selected host),
UI preferences. No secrets. No config content (the flake is the source of
truth, the JSON only remembers where it is).

## Feature 1 (first big feature): discover, map, and edit the config

### Discovery levels

| Level | What | How | Cost |
|---|---|---|---|
| 1 | Files and folders | walk the tree; classify flake.nix, configuration.nix, hardware-configuration.nix, flake.lock, scripts, secret-looking files | instant |
| 2 | Import graph | parse each .nix with rnix-parser, follow literal `./x.nix` imports, note commented-out imports as disabled modules | instant |
| 3 | Per-file attribute tree | rnix CST to a JSON tree of nodes: attr path, value kind (literal, list, attrset, code/opaque), source span | instant |
| 4 | Evaluated values and docs | `nix eval --json` for merged option values; options.json for docs/types of every option | slow, runs in background after 1-3 are shown |

### Map view

Tree view plus import graph, per-file summary of which option namespaces it
touches (services.*, environment.systemPackages, ...), option documentation
on hover from options.json, host selector when the flake defines several.

### Editing engine

- **Structured edits** via lossless CST manipulation (rnix-parser; nix-editor
  as reference): change a literal, add/remove a list item, add an attribute,
  add/remove/toggle an import, create a module file from a template.
- **Code fallback**: inline Nix editor for any expression the form cannot
  represent (let/in, functions, interpolation, `with`, conditionals). Never a
  dead end.
- **Validation before apply**: `nix eval` / dry build in the background;
  errors shown next to the field that caused them, with plain-language hints.
  This is the key beginner feature.
- **Git safety net**: when the folder is a git repo, show a diff for every
  change; commit policy is an open decision (see below).

### Principle change (2026-10-01)

Earlier notes said the tool never rewrites hand-written Nix and only edits a
tool-owned file. For the beginner audience that is too conservative. New
principle: the tool makes surgical, lossless edits to existing files, always
shows the diff, and falls back to code view when it cannot be surgical. A
tool-owned file may still be used for things like generated package lists.
04-architecture.md layer 3 should be read with this update.

### UI rendering

The daemon emits a generic JSON node tree (kind, path, value, children, span,
editable flag, option doc ref). Angular renders it with a recursive component
per node kind. Adding a new value kind means one Rust enum variant and one
Angular component.

## Beginner layer: guided actions (built on the same engine)

"Install a program" (search nixpkgs, append to a package list), "Turn on a
service" (browse options.json with docs, write the enable line), "Add a user",
"Mount a drive", "Convert to flake". Each is a recipe that produces structured
edits plus validation. This is the front door for non-NixOS users; the
structure view is for understanding and power users.

## Decisions made 2026-10-01 (Vincent)

1. **Non-flake configs are supported.** The daemon detects whether flake.nix
   or configuration.nix is the entry point and picks the matching
   nixos-rebuild invocation. A guided "convert to flake" action is a later
   nicety, not a requirement.
2. **Daemon runs as the user; sudo password prompted at build time.** See
   04-architecture.md, privilege model and deploy flow.
3. **Git is out of scope for now.** No commits, no diffs from git. The
   NixOS generations are the undo story.
   Remove the "Git safety net" bullet above from consideration until revisited.
4. **Sync mirrors sync.sh exactly** (confirmed 2026-10-01). No extra backups,
   no build-before-sync. Exact steps in 04-architecture.md, deploy flow.
