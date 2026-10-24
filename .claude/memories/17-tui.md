# Terminal UI (2026-10-22, Vincent's request)

`crates/tui` → binary `nixos-manager-tui` (ratatui 0.29 + crossterm 0.28,
event-stream). `./run.sh --tui [FOLDER]` builds it when stale and execs it
(no daemon, no browser): it links `nixos-manager-core` directly and runs
jobs in-process (`JobManager::new()`, `jobs.subscribe()` broadcast →
mpsc `Msg::Job` into the UI loop). Same housekeeping as the web UI on open
(`fsbrowse::config_root`, hardware sync, stray flake.lock removal,
`touch_location`, `build_status`).

Screens / keys (remote support completed 2026-10-22):
- Pick: recent (from state.json) + `fsbrowse::suggestions`; `/` or Tab to
  type a path; Enter opens; `t` targets; q quits. A folder given on the
  command line opens directly.
- Workspace: nix files (syntax error / not imported marks), summary, host
  (`h` cycles and persists via touch_location), hardware line
  (`hardware::status`: pulled from target X / matches / differs), targets
  with reachability once checked, pending-changes line; `b` build,
  `v` validate, `g` system, `t` targets, `j` jobs, `m` free memory, `r` rescan.
- Build form (same `DeployRequest` as the web dialog): Deploy to (this
  machine | targets; selecting one runs `remote::check`), Mode (push |
  on-target), Build on (only for this machine: a target as `build_host`,
  "build there, activate here"), Host, Action, Upgrade, Skip sync (local),
  Hardware (target: its own file | working copy's file, with the warning
  only when the working copy is chosen; refuses to start when the check
  says `hardware_config == "none"` and the target's file was chosen),
  Target afterwards / This machine afterwards (after, after_local),
  Even on failure (after_even_on_failure, shown once an action is set),
  password fields (local sudo; target or builder sudo — required only when
  the check says the user is not root and has no passwordless sudo), Start.
- Targets (`t`): list + detail panel. `a` add / `e` edit (form: Name, Host,
  SSH user, Port, nixosConfigurations name, Use sudo; stored as user@host,
  port only when ≠ 22, root ⇒ no sudo; names unique) → `store.set_targets`,
  `d` remove (plain confirm), `c`/Enter `remote::check` (cached per name in
  `App::checks`), `t` `remote::test_connection` job, `k` `remote::
  setup_key_access` (target password popup), `p` pull hardware
  (`remote::fetch_target_hardware` → `hardware::install_pulled` →
  `store.set_hardware_pull`; needs an open workspace; status line reports
  it), `s` remote System screen, `K` `remote::root_builder_key` with the
  target's host (root key for using it as a builder).
- System (`g` local, or `s` on a target): header (machine, `store_usage` /
  `remote_info` disk + generation count, access rule for targets),
  generations list (Enter → rollback; `c` GC keep generations, `o` older
  than 30 d, `D` all old = `-d`), "Builds for other machines" list
  (`other_systems` / `remote_other_systems`; Tab focuses it, Enter deletes
  the selected path + GC unless it still has roots). All root actions go
  through one `Prompt` popup: local ⇒ password required; target ⇒ not
  asked when the check says root/passwordless sudo, required when it says
  sudo asks, optional (hint) when unchecked. Finished rollback/GC/delete
  jobs reload the screen.
- Job: live log (colours per stream, red for error lines), `s` stop
  (`jobs.stop`), scroll keys, `f` follow; Esc returns to the screen the
  job was started from (`job_back`). Jobs: this session's jobs.
Internals: `Msg` enum carries typed results (`Check(name, RemoteCheck)`,
`SystemInfo`, `OtherSystems`, `OpenJob`, `JobDetail`, `Info`, `Error`) —
no string sentinels any more. `App::run_job(kind, title, |job| fut)`
creates a job, spawns the future and opens the log.
Debug: `NM_TUI_LOG=<file>` appends every message the loop receives.
Testing: drive it in tmux (`tmux new-session -d -x 150 -y 40 '…tui <dir>'`,
`send-keys`, `capture-pane -p`) with `NIXOS_MANAGER_STATE=<scratch>/state.json`
so Vincent's real targets are untouched. Verified 2026-10-22: add/edit/
remove a target, check (unreachable), SSH test job, key/root-key prompts,
pull hardware error path, remote System error path, build form field sets
(target, after fields, build on), local System with disk + generations +
GC prompt, free-memory prompt. Note: in tmux send Escape and the next key
in separate `send-keys` calls (otherwise it becomes Alt+key).
Not in the TUI: file editing, wizard, git, secrets (use the web UI).

## Free memory (m on the workspace screen) — added 2026-10-22, reworked 2026-10-23
TUI-only port of Vincent's `clear-sessions.sh` (`/home/vincent/Desktop/code/private/nixos/script/clear-sessions.sh`).
- `core::system::free_memory_script(my_session, my_user)` builds the root script: `loginctl terminate-session` for every
  session except the caller's (prints user + class), `loginctl terminate-user` for every user except the caller and root,
  then — new on 10-23 — **waits up to 60 s until the terminated sessions are really gone** (`loginctl show-session` fails),
  sends `loginctl kill-session --signal=SIGKILL` to whatever is still alive, then `sync`, `echo 3 > drop_caches`, `free -h`,
  the remaining sessions and the 8 largest processes by RSS. Inputs are sanitised (alnum only) so they cannot break the quotes.
  Why: Vincent's first run showed `used 10Gi → 10Gi` because terminate-session only queues the stop; the desktop processes
  were still exiting when the 2 s sleep ended, so the numbers (and possibly the cleanup, when something ignores SIGTERM)
  were not comparable with the script run from a TTY. The log is now a true picture of what is left.
- `core::system::free_memory(job, password, my_session, my_user)` runs it through `deploy::run_root_script` as a `system` job.
- TUI: `m` opens the shared Prompt popup (password only when not root). Own session = `XDG_SESSION_ID` or
  `loginctl show-session self -p Id --value` (the script also falls back to `self` when the value is empty); own user = `$USER`.
- Not exposed in the web UI on purpose (it would kill the desktop session the browser lives in). Must be run from the TTY/SSH
  session the build will run from. Never run for real by me — verified with a fake `loginctl` in PATH
  (one session dying after 7 polls, one never → SIGKILL branch), `bash -n`, and the unit test `script_parses_and_protects_the_caller`.

## Running as root (run.sh / TUI started with sudo) — 2026-10-23
`deploy::running_as_root()` (uid 0 from /proc/self/status). When true: `check_sudo` is a no-op, `run_root_script_inner`
runs `bash <script>` directly (stdin = only the extra lines, no password line), `deploy::run` and `run_root_script` log
"Running as root: no sudo password needed." instead of validating. Daemon handlers that required a non-empty local
password (deploy start without target, rollback, gc, delete-paths, root-key) accept an empty one when root.
Exposed as `MachineInfo.running_as_root` (in `ConfigOverview.machine` and `/api/machine`) and `DeployPreflight.running_as_root`.
Web UI: `WorkspaceService.isRoot` computed; build dialog hides both local password fields and the Start hint; System page
`needPw` for this machine = `!isRoot`; Channels skips the sudo prompt; Remote root-key prompt drops the password input.
TUI: no LocalPassword field, local Prompt popups have no password line, workspace shows a green "Running as root" line.
Remote targets are unaffected (their own sudo rule still applies). Verified with `unshare -Ur` (fake uid 0): `/api/machine`
→ `running_as_root: true`, root-key with empty password ran bash directly (then failed on /root as expected in a userns),
TUI showed the root line and no password fields. Note: inside `unshare -Ur` the scratchpad /tmp is not visible — test
folders must live under $HOME (used `~/.cache/nm-tui-test`, deleted afterwards).
