# Package sets (pkgs / pkgs-<channel>) in wizard and form

Added 2026-10-15 at Vincent's request: let users choose and change which
nixpkgs a package or package-valued setting comes from.

## Model
`flake::package_sets(dir)` → `PackageSet { name, label, input, flake_ref,
is_default, bits32 }`. Classic config: just `pkgs` (label shows the nixos
channel version). Flake: `pkgs` from input `nixpkgs`, then for every name in
`channels`: `pkgs-<name>` and `pkgs-<name>-32` from input `nixpkgs-<name>`.
Label = branch (from `?ref=` or github path) + locked date. `flake_ref` =
`github:owner/repo/<locked rev>` when the lock matches the declared branch,
else the declared url. Lock entries whose original source is a different
repo/branch than the flake declares are ignored (`lock_matches`) because the
/etc/nixos fallback lock may belong to another config.

## Backend
- `packages.rs`: `search(query, installable, set)`; installable is the set's
  `flake_ref`, so results come from the exact pinned nixpkgs (first search per
  ref ~13 s, then cached). Route `/api/packages/search?q=&set=&path=`.
- `ConfigChange::AddPackage { attr, set }`; `edit::add_package(src, attr, set)`
  writes `set.attr`, or the bare attr when the list is `with <set>;`.
- `RenderRequest.package_set`: recipes replace `pkgs.` with the chosen set in
  values/items/packages (e.g. `users.defaultUserShell = pkgs-25_11.zsh`).
- `edit::package_sets_in(text)` finds `pkgs`/`pkgs-*` identifiers; both
  `apply_changes` and the form `apply()` call `ensure_module_arg` for each so a
  module that starts using `pkgs-unstable` gets it in its argument pattern.
- Route `/api/config/package-sets?path=`.

## UI
- `WorkspaceService.packageSets` loaded with the overview.
- Wizard: "Take packages from" selector (only when >1 set) on the programs
  step (search uses it; selections remember their set; chips show
  `pkgs-25_11.foo`) and under recipe fields.
- Form (`node.component.ts`): package refs (`pkgs-x.attr`, or bare names
  inside a `with pkgs-x;` list via the new `withNamespace` input) get a small
  dropdown of sets; choosing the list's own set writes the bare name,
  otherwise `set.attr`. Unknown sets are listed as "(not in this flake)".

Verified on a converted copy: htop from pkgs-25_11 (3.4.1) vs pkgs (3.5.1),
module args updated automatically, form switch `git` → `pkgs-unstable.git`.

## Flake vars picker in the form (added 2026-10-15)
`WorkspaceService.flakeVars` (from `/api/flake/info`) feeds a `data_object`
menu button next to every inline input in `node.component.ts`:
- "Use as the value" → `set_scalar` ref `vars.<name>` (value becomes a reference).
- "Insert into the text" (string nodes only) → appends `${vars.<name>}` to
  the draft; on save, drafts containing `${` go through `set_raw` with manual
  quoting because the scalar path escapes `${`.
`edit::module_args_in(text)` (package sets + `vars`) replaces
`package_sets_in` in `apply()`/`apply_changes` so a module that starts using
`vars` gets it added to its argument pattern. Not yet in the wizard (recipe
text fields render as quoted strings, so a var there would need raw/`${}`
handling).

### Code-mode insertion (2026-10-15)
Both code editors (whole-file textarea in `file-view`, per-value textarea in
`node.component`) have an "Insert global value" menu that inserts
`vars.<name>` or `${vars.<name>}` at the caret (`shared/insert-at-cursor.ts`).
Raw saves bypass the engine's argument detection on purpose (comments could
trigger false positives for package sets), so after a `replace_file` the UI
checks the saved source for `vars.` and, if the module's pattern lacks
`vars`, sends the new `EditOp::EnsureArg { name }` (→ `ensure_module_arg`).
