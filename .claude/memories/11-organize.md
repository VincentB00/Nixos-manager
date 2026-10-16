# Organize (auto-group settings)

Added 2026-10-15 at Vincent's request. "Organize" button in the file view
(form and code mode) → preview dialog with a unified diff → apply.

## Algorithm (`crates/core/src/organize.rs`)
1. Find the module body attrset (also under `let ... in`). `rec` body → error.
2. Flatten: every binding becomes a leaf (path segments, value source lines,
   leading comments, trailing comment). Plain `{ ... }` values are flattened
   recursively so existing blocks are re-flowed; a block's own comments move
   to its first child. Not flattened (kept as opaque values): `rec` sets,
   sets with `inherit`, empty `{ }`, sets whose comment tokens are not all
   attached to an entry. `inherit` entries are emitted verbatim in place.
3. Group leaves by first segment in order of first appearance. A group with
   one leaf keeps its dotted path. Otherwise the longest common prefix of
   the group becomes `prefix = { ... }` (so `fileSystems."/mnt/x" = { ... }`
   rather than nested singletons), recursing into the remainder. If a leaf's
   whole path equals the prefix (`a = v; a.b = w;`) the group is emitted
   unchanged (would be a Nix error to merge).
4. Emit with 2-space indents; multi-line values re-indented relative to
   their original line indentation; blank lines around multi-line blocks at
   the top level only. Re-parse; refuse to write if the result is invalid.
Idempotent (organizing twice yields the same text). Tests cover Vincent's
example, multi-layer services, unsafe sets, let-in bodies.

Route: `POST /api/config/organize {path, file, version, preview}` →
`OrganizeResult {changed, groups_created, source, unified_diff, view}`.
