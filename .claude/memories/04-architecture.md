# Architecture

Last updated: 2026-10-01. Agreed design, not yet implemented.

## Shape

One daemon per NixOS machine, installed through a NixOS module
(`services.nixos-manager.*`). It serves the Angular UI and exposes an API.
Any browser, local or remote, is the client. There is no separate desktop app.

```
browser (Angular) --HTTP/WS--> nixos-manager daemon (Rust) --subprocess--> nix / nixos-rebuild
                                        |
                                        +--sudo/polkit (one rule)--> switch-to-configuration
```

## Layers of capability

1. **Introspection (works on any flake, no conventions).** Hosts from
   `nix flake show --json`; inputs and lock state from `nix flake metadata
   --json`; option values from `nix eval --json`; option schema from the
   generated options.json (`config.system.build.manual.optionsJSON`);
   generations from `/nix/var/nix/profiles/system`.
2. **Operations (no conventions).** build, test, switch, boot, diff against
   the running system (nvd or own implementation), `nix flake update <input>`,
   rollback / switch-generation, garbage collection. Long operations are jobs
   with IDs, streamed over the WebSocket, reattachable after page reload.
3. **Editing (needs an opt-in convention).** The tool owns one file per host,
   proposed `nixos-manager/<hostname>.nix`, which the host's module list
   imports. Module toggles and option edits write only there. User modules are
   never rewritten. If the convention is absent, editing panels are hidden.
4. **Later.** Module enable-list convention plus migration helper from the
   "comment out the import" pattern; option editor with typed widgets from
   options.json; package search; secrets via sops-nix; SSH deploy to other hosts.

## Privilege model (revised 2026-10-01 per Vincent's decision)

The daemon runs **as the logged-in user** (systemd user service, or launched
directly). It therefore reads and writes the user's settings folder with no
privileges, and builds/evaluates through the Nix daemon as that user.

Privileged steps (sync to /etc/nixos and nixos-rebuild switch/boot/test) run
via `sudo -S`, with the **sudo password prompted in the UI at build time**.
The password is held in memory for that job only, fed to sudo over stdin, and
never written to disk or to the JSON state file. The API listens on a Unix
socket or localhost by default; if TCP is ever exposed it must be behind TLS
and token auth because the password crosses it.

Earlier plan (dedicated system user plus a narrow sudoers/polkit rule) was
dropped: it could not write configs in the user's home, and Vincent prefers an
explicit password prompt.

## Deploy flow (decided 2026-10-01: mirror Vincent's sync.sh exactly)

Vincent's existing workflow copies the working folder into /etc/nixos and
builds there, so /etc/nixos always mirrors the running system and plain
`nixos-rebuild` keeps working. He confirmed on 2026-10-01 that the app's sync
must behave like `sync.sh` in his config repo (`/home/vincent/Desktop/code/
private/nixos/sync.sh`). Proposed extras (timestamped backup of /etc/nixos,
building as the user before asking for the password) were declined; do not
add them unless he asks.

Sequence when the user clicks build/switch:

1. Prompt for the sudo password in the UI (see privilege model).
2. **Sync**, as root, replicating sync.sh step by step:
   a. Options: `upgrade` flag (default off).
   b. Refuse if the working copy *is* /etc/nixos.
   c. If `/etc/nixos/flake.lock` exists and `upgrade` is off, stash a copy of
      it (sync.sh uses /tmp/flake.lock.bak; a temp file is fine).
   d. Copy `/etc/nixos/hardware-configuration.nix` **into the working copy**,
      overwriting. (Direction: system to working copy. hardware-configuration
      lives on the machine and is gitignored in the repo.)
   e. If `configuration.nix` differs between /etc/nixos and the working copy,
      the working copy version always wins (decided 2026-10-15; the UI only
      shows a note with the diff, no choice). The backend still accepts
      `keep_configuration: system` for sync.sh parity, but the UI never sends it.
   f. `rm -rf /etc/nixos/*` then `cp -r <workdir>/* /etc/nixos/`. Note the
      glob skips dotfiles (.git, .gitignore, .claude), which is intended.
   g. If `upgrade`: delete `/etc/nixos/flake.lock` so inputs re-resolve.
      Otherwise restore the stashed flake.lock.
3. `nixos-rebuild switch|test|boot` in /etc/nixos (`--flake /etc/nixos#<host>`
   for flakes, default invocation for a configuration.nix setup), log streamed
   to the UI. If `upgrade` was set this is where new inputs get locked.

Consequences the implementation must respect:
- flake.lock is owned by /etc/nixos, not the working copy. After a rebuild the
  app may offer to copy the new lock back to the working copy, but that is an
  extra, not part of sync.
- hardware-configuration.nix in the working copy is always overwritten from
  the system during sync.
- For a beginner whose only config is /etc/nixos, step 2b refuses; the first-
  run import (copy /etc/nixos to a user folder) is what makes sync possible.

## Repo layout (planned)

```
Nixos-manager/
  flake.nix              # packages.default, nixosModules.default, devShell
  Cargo.toml             # workspace
  crates/core/           # flake introspection, options, jobs, nix wrappers (no HTTP)
  crates/daemon/         # axum server, embeds ui/dist
  crates/cli/            # thin CLI over core, for scripting and tests
  ui/                    # Angular workspace
  nix/module.nix         # services.nixos-manager.*
  nix/package.nix
  .claude/memories/      # these notes
```

`core` must have no UI or HTTP dependencies so the UI layer can be swapped.

## Multi-host model (default, pending confirmation)

Each daemon manages only its own machine. To manage a server, point the
browser at that server's daemon (user service with `loginctl enable-linger`
on headless hosts). SSH-based deploy from one daemon to another is deferred.

### Spans are UTF-8 byte offsets (bug fixed 2026-10-16)
`Span.start/end` come from rnix `TextRange` = bytes. The UI must never slice
the source string with them directly (JS strings are UTF-16): "Edit as Nix
code" on a value after a non-ASCII character (an "→" in a comment in
Vincent's users.nix) showed a shifted fragment. Use
`ui/src/app/shared/byte-offsets.ts` (`sliceBytes`, `byteToIndex`) for any
span → string work. Node ids "start-end" and edit ops stay in bytes; the
daemon does all slicing on the Rust side, which is correct.
