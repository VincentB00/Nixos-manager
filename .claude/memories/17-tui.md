# Terminal UI (2026-10-22, Vincent's request)

`crates/tui` → binary `nixos-manager-tui` (ratatui 0.29 + crossterm 0.28,
event-stream). `./run.sh --tui [FOLDER]` builds it when stale and execs it
(no daemon, no browser): it links `nixos-manager-core` directly and runs
jobs in-process (`JobManager::new()`, `jobs.subscribe()` broadcast →
mpsc `Msg::Job` into the UI loop). Same housekeeping as the web UI on open
(`fsbrowse::config_root`, hardware sync, stray flake.lock removal,
`touch_location`, `build_status`).

Screens / keys:
- Pick: recent (from state.json) + `fsbrowse::suggestions`; `/` or Tab to
  type a path; Enter opens; q quits. A folder given on the command line
  opens directly.
- Workspace: nix files (syntax error / not imported marks), summary, host
  (`h` cycles and persists via touch_location), targets, pending-changes
  line; `b` build, `v` validate, `g` generations, `j` jobs, `r` rescan.
- Build form: Deploy to (this machine | targets; selecting a target runs
  `remote::check` and shows trust/sudo state), Mode (push | on-target),
  Host, Action, Upgrade, Skip sync, Afterwards (stay/reboot/poweroff),
  masked password field (local sudo, or target sudo), Start. Builds the
  same `DeployRequest` as the web dialog (keep_configuration Local,
  hardware System, allow_unfree_env for on-target).
- Generations: list + Enter → popup asking the sudo password → `system::
  rollback` job.
- Job: live log (colours per stream, red for error lines), `s` stop
  (`jobs.stop`), scroll keys, `f` follow. Jobs: this session's jobs.
Debug: `NM_TUI_LOG=<file>` appends every message the loop receives.
Testing: drive it in tmux (`tmux new-session -d -x 130 -y 34 '…tui <dir>'`,
`send-keys`, `capture-pane -p`) — that is how it was verified (pick,
workspace, form navigation, a Validate job streaming, generations, jobs).
Not in the TUI: file editing, wizard, git, secrets, remote-target
management (use the web UI for those).
