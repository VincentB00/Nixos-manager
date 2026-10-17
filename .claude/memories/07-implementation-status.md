# Implementation status

Last updated: 2026-10-02 (first implementation session)

## What exists and works

Everything below was built and verified against a scratch copy of Vincent's
real config (64 files) on 2026-10-02. `cargo test` passes (53 tests), the
Angular app builds, and the API was exercised end to end with curl.

### Rust workspace (`Cargo.toml`, crates/)
- `crates/core` (lib `nixos_manager_core`)
  - `types.rs`: all API types, `#[derive(TS)]`; `scripts/gen-types.sh`
    regenerates `ui/src/app/api/generated/`. i64 fields need
    `#[ts(type = "number")]` or TS gets `bigint`.
  - `state.rs`: JSON state file (`$NIXOS_MANAGER_STATE` or
    `~/.local/state/nixos-manager/state.json`), recent locations, prefs.
  - `fsbrowse.rs`: directory listing, `~` expansion, suggestions
    (/etc/nixos, common home folders, shallow home scan).
  - `nixparse.rs`: rnix 0.14 (rowan 0.16) → generic `Node` tree with byte
    spans; node id = "<start>-<end>" of the entry span; comments (leading
    on own line, trailing same line); commented-out list items/imports
    detected by `looks_like_simple_expr` heuristic; `with pkgs;` recorded as
    `with_namespace`; `++` → Concat, `//` → Merge; imports/references
    extraction; static `nixosConfigurations.<host>` detection.
  - `discover.rs`: file classification, import graph, roots, reachability,
    `overview_with`, `read_file_view`, path-escape guard.
  - `edit.rs`: span-based lossless edits (`EditOp`): set_scalar, set_raw,
    list_append (inserts above the closing bracket so it lands after trailing
    comments), list_remove, toggle_disabled, add_binding, remove_binding,
    replace_file; `add_import`; `create_file`. Re-parses and rejects edits
    that add parse errors; version (content hash) must match → 409 otherwise.
  - `deploy.rs`: `preflight` (conflict diff, /etc/nixos state) and `run`:
    validates sudo password (`sudo -S -k -v`), writes a root bash script that
    mirrors sync.sh exactly, runs it with `sudo -S bash script`, streams
    output into a job. Unit tests assert the script contents.
  - `jobs.rs`: job registry + broadcast events; `process.rs`: run/stream.
  - `nix.rs`: machine info, `nix flake show` hosts, generations.
- `crates/daemon`: axum 0.8, routes under `/api` (see `api.rs`), WebSocket
  `/api/ws`, UI embedded with rust-embed from `ui/dist/nixos-manager-ui/browser`
  (build.rs creates a placeholder so cargo builds without the UI). Default
  listen 127.0.0.1:7788. `--cors-origin` for ng serve if not using the proxy.
- `crates/cli`: `nixos-manager info|overview|file|hosts|suggest|generations`.

### Angular UI (`ui/`, Angular 22, Material, signals, standalone)
- `api/api.service.ts` (HttpClient wrappers, `ApiFailure`),
  `api/jobs.service.ts` (WebSocket mirror of jobs), `core/workspace.service.ts`.
- Pages: `setup` (location prompt: recent, suggestions, browser),
  `workspace` (toolbar, host menu, file list, Build button),
  `overview` (cards + import map tree), `file-view` + `node` (recursive form
  editor with code fallback, whole-file code mode), `build` dialog
  (preflight, action, upgrade, skip sync, conflict choice, password),
  `jobs` + `job-log` (xterm.js), `new-file` dialog.
- Sidebar view modes (added 2026-10-07): `list` (flat .nix list, default) and
  `tree` (`workspace/file-tree.component.ts`, recursive filesystem tree of
  every file with folder collapse). Persisted in localStorage key
  `nixos-manager.fileView`; `?view=tree|list` query param also sets it.
  Tree rows have a context menu (right-click or hover ⋮; 2026-10-07):
  folders → new module / new empty file / new folder / rename-move / delete
  (checkbox for recursive); files → import from configuration.nix (when
  nothing imports it), rename-move, duplicate, delete. Root-level buttons
  above the tree. Backend: `crates/core/src/fsops.rs` (mkdir, rename,
  duplicate, delete) + routes `/api/config/fs/{mkdir,rename,duplicate,delete}`.
  Rename rewrites every path literal (imports, references, commented-out
  imports) that resolves to the moved file/folder, and references *from*
  files that move with a folder; returns `updated_files`. Shared
  `shared/prompt-dialog.component.ts` handles prompt/confirm dialogs.
  Empty folders: `ConfigOverview.directories` (from `discover::list_dirs`)
  feeds the tree so folders without files still show (bug fix 2026-10-07).
  Non-.nix files open in the file view as plain text (code mode only);
  the backend allows only `replace_file` on them, without Nix validation.
- File view header (form mode) has Expand all / Collapse all buttons
  (2026-10-15): `expandState` signal `{open, seq}` is passed down through
  every `app-node`; an `effect` sets each node's `open`. `seq` re-triggers
  repeats.
- Dev: `ng serve` proxies `/api` to :7788 (`proxy.conf.json`).
- Icons come from Google Fonts (Material Symbols) → no icons offline. TODO:
  self-host.

### Nix
- `flake.nix`: devShell, `packages.{default,daemon,ui}`
  (buildNpmPackage with npmDepsHash computed 2026-10-02; cargo via
  Cargo.lock), `nixosModules.default` = `nix/module.nix` providing
  `services.nixos-manager` as a systemd *user* service. `nix build` not yet
  run end to end.

## Colour convention (Vincent, 2026-10-16)
Red (`.error-text`, `.errors`, `.warn` in build dialog) only for failures
that stop the user: API errors, syntax errors, refused actions, failed jobs.
Amber (`.warning-text`, `.warning-box`, `.tag.warning` in styles.scss, tokens
`--nm-warning*`) for non-blocking notices: SSH agent/key hints, recipe
warnings, secrets out of date, missing import targets, unwritable folder,
non-template flake, channel/input mismatch.

## Testing etiquette
When Vincent is trying the app, port 7788 is his. Run test daemons on
another port (e.g. `--listen 127.0.0.1:7799`) and stop only that pid; never
`kill $(pidof nixos-manager-daemon)` blindly (did that once on 2026-10-07 and
killed his instance).

## Not done / known gaps
- No automated UI tests; Angular spec files were removed.
- The build job has never been run for real (would modify /etc/nixos and
  rebuild the machine). Only the generated script is unit-tested.
- Option documentation (options.json), evaluated values (`nix eval`),
  package search, guided actions: not started (planned, see 06-features.md).
- Password crosses plain HTTP on localhost; fine locally, document before
  any remote exposure.
- rsync/cp quirks: /etc/nixos files are owned nobody:nogroup on Vincent's
  machine; the deploy script chowns hardware-configuration.nix back to the
  invoking user after copying it into the working copy.

## How to run

`./run.sh` (added 2026-10-02 at Vincent's request): re-execs itself inside
`nix-shell shell.nix` (cargo, rustc, nodejs_24 only; deliberately no sudo/nix
from nixpkgs because nixpkgs' sudo is not setuid and would shadow the host's),
runs `npm ci` if node_modules is stale, `ng build` if ui/src is newer than the
bundle, `cargo build --release` if crates are newer than the binary, then
starts the daemon and opens the browser. `--dev` runs cargo run + ng serve.
`--watch` (added 2026-10-07) wraps the script in `watchexec --restart`
watching crates/, ui/src, Cargo.toml: any change rebuilds what is stale and
restarts the daemon; watchexec comes from shell.nix.
Falls back to `-I nixpkgs=channel:nixos-unstable` when `<nixpkgs>` is missing.
Verified 2026-10-02: serves on :7788 from a clean shell.
2026-10-07: Vincent hit `Error: write EPIPE` from sass-embedded (Angular's
native Dart Sass helper died) during `run.sh` UI build. Not reproducible;
likely a collision with a concurrent build by Claude in the same folder.
run.sh now retries a failed UI build with `NG_BUILD_SASS_EMBEDDED=0` (pure-JS
sass), and `--js-sass` / `NIXOS_MANAGER_JS_SASS=1` force that mode. Avoid
running `ng build` in the repo while Vincent is running run.sh.

Manual (dev):
```
cargo run -p nixos-manager-daemon          # API + built UI on :7788
cd ui && npm start                         # live UI on :4200
cd ui && npm run build                     # then the daemon serves it
```

## 2026-10-17: Monaco editor + comment out groups (Vincent's QoL requests)
- `ui/src/app/shared/code-editor.component.ts` (`<app-code-editor>`): Monaco
  (`monaco-editor@0.52.2`, ESM import, vs-dark, automaticLayout) with a
  Monarch tokenizer for Nix (keywords, strings incl. `${}` interpolation and
  `''` indented strings, paths, comments, `key =` highlighting). Used for the
  whole-file Code view (file-view), the per-node "Edit as Nix code" editor
  (node.component) and the new-file dialog. API: `[value]`, `(valueChange)`,
  `height`, `language`, `insertAtCursor(text)` (the "Insert global value"
  menus use it). No web worker is bundled (Angular's esbuild cannot resolve
  monaco's worker URL); a no-op blob worker silences the console error and
  Monaco runs its services on the main thread — fine for config files.
  angular.json: `loader: {".ttf": "file"}` for the codicon font, budgets
  raised, AND `node_modules/monaco-editor/min/vs/editor/editor.main.css` in
  `styles` — the CSS that monaco's ESM modules import from JS is emitted as
  a chunk the Angular bundler never injects (first version: find widget
  floating outside the editor, broken icon glyphs, no line numbers). Keep
  the full `monaco-editor` import (`editor.api` drops find/fold/etc.).
  Debug hook: the host div exposes `__monaco` (used by screenshot scripts).
  `shared/insert-at-cursor.ts` is no longer used by those pages.
- Form editor: "Comment out" now also on bindings/groups (node.component
  `isBinding`). Backend: `edit::toggle_edit` comments every line of a
  multi-line span with `# ` at the GROUP's indentation (inner indentation
  kept after the marker, empty lines → `#`), and uncomments blocks (strip
  after indentation, bare `#` → empty line) — exact round trip. Parser:
  `nixparse::comment_groups` collects runs of consecutive `#` lines inside a
  set; `entries()` greedily finds the longest run that parses as exactly one
  binding (`{ text }`) and shows it as a disabled node (label = key, kind by
  value, `source` = the block) — so a commented-out group shows as
  "GROUP · commented out (N lines)" with Enable and the block preview; prose
  comment lines in between are ignored. Leading code-like comments are no
  longer attached as the entry's comment text (`leading_comments(.., true)`).
  Tests: `comment_out_a_group_and_restore_it`,
  `commented_out_group_and_setting_show_as_disabled`.
