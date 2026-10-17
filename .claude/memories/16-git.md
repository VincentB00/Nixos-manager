# Git support (2026-10-16, Vincent's request)

Scope agreed: branches (view/switch/create), status with diffs, commit with
an auto-filled message (editable), push, plus fetch and pull (ff-only).
Nothing destructive yet (no discard, no reset, no force, no branch delete).

## Backend `crates/core/src/git.rs`
- Shells out to `git` (user's config/credential helpers/hooks apply).
  `git()` helper: `GIT_TERMINAL_PROMPT=0`, `LC_ALL=C`; without a password
  `GIT_SSH_COMMAND="ssh -o BatchMode=yes"` so nothing can hang; with a
  password a temp askpass script (mode 700, deleted afterwards) prints
  `$NM_GIT_PASSWORD` for both `GIT_ASKPASS` and `SSH_ASKPASS`
  (`SSH_ASKPASS_REQUIRE=force`). The password never touches argv or disk.
- `redact()` hides `user:token@` in URLs everywhere (Vincent's real remote
  embeds a GitHub token; never print it — also not in these notes).
- `info` parses `status --porcelain=v2 --branch --untracked-files=all`
  (`parse_status`: head/upstream/ahead/behind/detached + files with state
  modified|added|deleted|renamed|untracked|conflict, staged flag, old_path).
- `branches`: `branch -a --format` → locals + remote-only candidates
  (origin/x without a local x → `remote_only`, `switch` auto-tracks them).
- `switch` refuses when tracked files changed (Conflict 409); untracked ok.
  `create_branch` = `switch -c` (carries changes). Names validated with
  `check-ref-format --branch`.
- `diff`: `git diff HEAD -- file`; untracked → `diff --no-index /dev/null`.
- `suggest_message`: per changed .nix file, `settings_of()` flattens the
  parsed tree (nixparse::analyze_file) into attr-path → value and diffs
  HEAD vs working copy → "add a.b, change c.d, remove e.f" (cap 4 each).
  Subject: 1 file → "Update users.nix: add …" / "Add x.nix" / "Remove …";
  several → "Update a, b, c (+N more)" with a body line per file.
- `commit(message, paths?)`: identity required (Conflict 409 → UI prompts
  name+email → `set_identity` writes repo-local config), `add -A -- paths|.`
  then `commit -m`. "nothing to commit" → Invalid.
- `push`: `git push`, or `push -u <origin|first remote> HEAD` without
  upstream; refuses detached/no remote. `fetch --prune`; `pull --ff-only`
  (refuses with changes or no upstream; diverged → explains `git pull
  --rebase` in a terminal).
- Auth failures ("Authentication failed", "Permission denied", "terminal
  prompts disabled", "passphrase", "Host key verification failed") →
  Forbidden 403 → the UI asks for a password/token/passphrase and retries
  the same operation with it. For "Commit & push" the commit stays done and
  only the push is retried.
- Tests: redaction, porcelain parsing, nix change description, and an
  end-to-end on a scratch repo with a bare remote (isolated via
  `GIT_CONFIG_GLOBAL=/dev/null`, `GIT_CONFIG_NOSYSTEM=1`).

## API (`/api/git/*`)
GET info|branches|diff(file)|suggest (query path), POST switch|branch
(`{path,name}`), commit (`{path,message,paths?,push,password?}`), push|
fetch|pull (`{path,password?}`), identity (`{path,name,email}`).

## UI
`pages/git/git.component.ts` (route `git`): branch menu (locals + remote-
only with cloud icon), New branch, fetch/pull/push icon buttons with
ahead/behind; Changes card with per-file checkbox, state tag, expandable
diff; Commit card with identity warning, message textarea (auto-suggested
on open, "Suggest message" button), Commit / Commit & push. Sidebar entry
"Git" only when `ws.gitInfo().is_repo` (WorkspaceService loads git info on
reload); header chip shows the branch + uncommitted count and links to the
page. Branch switch / pull call `ws.reload()` (files changed on disk).
Non-repo folders get a short "git init" how-to instead.

## Interplay with other features
- hardware-configuration.nix is gitignored in Vincent's repos, so the
  sync-on-open never shows up as a change. If it is tracked elsewhere, the
  sync would show it as modified — intended (system copy is the truth).
- Branch-per-host: switching branch swaps the whole config; the workspace
  reloads, hardware sync runs again on the new tree.

## Discard (2026-10-17, Vincent)
`POST /api/git/discard {path, files: null | [..], clean_untracked}` →
`git::discard`: null = `git reset --hard -q` (+ `git clean -fd -q` when
asked; untracked files are otherwise left alone), a list = `git restore
--source=HEAD --staged --worktree -- <tracked…>` and plain deletion for
untracked entries; paths validated (no `-`, `/`, `..`); unknown paths 404.
UI: "Discard all changes" button above the file list (confirm dialog with a
checkbox for the untracked files, says there is no undo) and a per-file
"Discard changes" / "Delete file" button inside the expanded diff. Covered
by the scratch-repo test (`end_to_end_on_a_scratch_repo`).
