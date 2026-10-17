# Remote: deploy targets and remote builders

Added 2026-10-15 after Vincent asked how NixOS builds for / deploys to other
machines. Two mechanisms, both implemented:

## Build here, activate there (`nixos-rebuild --target-host`)
- `DeployTarget { name, ssh_target, flake_host, build_host, use_remote_sudo }`
  stored in `AppState.deploy_targets` (state.json); routes
  `GET/PUT /api/remote/targets`, `POST /api/remote/test` (job: `ssh -o
  BatchMode=yes target 'nixos-version; nix --version; sudo -n true'`).
- `DeployRequest.target: Option<DeployTarget>`; `deploy::run` hands off to
  `remote::run_remote`: runs **as the user, no sudo, no sync** from the
  working copy: `nixos-rebuild <action> --flake '<workdir>#<host>'
  --target-host <ssh> [--build-host <bh>] [--use-remote-sudo]` (classic:
  `--no-flake -I nixos-config=<workdir>/configuration.nix`), with
  `NIX_SSHOPTS="-o BatchMode=yes -o ConnectTimeout=20"` so it fails instead
  of hanging on prompts. `--upgrade` runs `nix flake update` in the working
  copy first. Logs warnings: no hardware-configuration.nix in the working
  copy (not borrowed for remote), no SSH agent visible (`SSH_AUTH_SOCK`),
  managed secrets not installed remotely. Preflight carries `ssh_agent` and
  `targets`.
- Build dialog: "Deploy to" select (this machine / targets). For a target the
  sync-related controls, conflict note, secrets and sudo password are hidden.
- Remote deploys run from a **staged plain copy** (`validate::stage_plain_copy`,
  no .git / secrets.age, nothing borrowed from /etc/nixos) referenced as
  `path:<staged>#<host>`: a git checkout as flake ref would hide untracked /
  gitignored files (Vincent's hardware-configuration.nix is gitignored). The
  flake.lock nix writes/updates in the staged copy is copied back to the
  working copy afterwards so the next deploy is reproducible.

## Remote builders (`nix.buildMachines`)
- Recipes (category Builds): `remote_builder_use` (client: distributedBuilds,
  builders-use-substitutes, buildMachines entry with sshKey default
  /root/.ssh/id_ed25519, optional max-jobs=0) and `remote_builder_serve`
  (builder: user remotebuild, no home, group, authorized keys, trusted-users,
  openssh enabled).
- `remote::root_builder_key` (route `POST /api/remote/root-key`, sudo
  password): creates /root/.ssh/id_ed25519 if missing, prints `PUBKEY <key>`
  into the job log, optionally `ssh-keyscan`s the builder into root's
  known_hosts. The Remote page polls the job and shows/copies the key.

## UI
`pages/remote/remote.component.ts` (targets list with test/edit/remove,
builder steps + key), `target-dialog.component.ts`; sidebar entry "Remote".

## Key access setup (2026-10-16)
Per-target "Set up key access" button (vpn_key icon) → PromptDialog asks for
the target's SSH password once → `POST /api/remote/setup-key {target,
password}` → `remote::setup_key_access` job (runs as the user, password
passed only via the `NM_SSH_PASSWORD` env var to an askpass helper with
`SSH_ASKPASS_REQUIRE=force`). Steps: dedicated key
`~/.ssh/nixos-manager_ed25519` (created only if missing; other keys
untouched) → probe `ssh -i key -o IdentitiesOnly=yes -o BatchMode=yes` →
if it fails, password login (`PreferredAuthentications=password,
keyboard-interactive`, `PubkeyAuthentication=no`) appends the pubkey to the
target's authorized_keys unless present → managed `Host <host>` block in
`~/.ssh/config` (marker comment, IdentityFile + IdentitiesOnly yes) → verify
key login and `sudo -n`. Exit codes: 4 host key changed (hint: ssh-keygen
-R), 5 unreachable, 6 login refused (wrong password, or target is
publickey-only: pubkey printed for manual install), 8 key installed but
login still failing. `StrictHostKeyChecking=accept-new` records new hosts.
Tested against a throwaway local sshd on port 2222 (key-only) with HOME
pointed at a fake home: refusal path, already-authorized path, idempotence,
unreachable host.

## Failure diagnosis (2026-10-16)
Vincent's first real remote deploy (lg-laptop) failed with `cannot add path
... lacks a signature by a trusted key`: the SSH user is not in the target's
`nix.settings.trusted-users`, so the target's daemon refuses unsigned paths
copied by a non-root user (happens before sudo). Fix is on the target:
`nix.settings.trusted-users = [ "<user>" ]`, applied once locally or via a
root@ deploy. `run_remote` now watches the stream for that message, for
`sudo: a password is required`, and for `Permission denied`, and appends a
plain-language "Why it failed" line with the fix. `test_connection` now also
reports "trusted by nix: yes/NO" (checks `nix config show trusted-users`,
with `@wheel` + group membership handling). `NM_SSH_OPTS` hook added there too.

## Temporary trust bootstrap (2026-10-16)
(Originally behind a `DeployTarget.bootstrap_trust` checkbox; Vincent had it
off and the deploy failed again, so the checkbox was removed the same day:
see "Automatic trust check + target sudo password" below. The mechanics here
still apply, but the step now always runs for non-root SSH users.)
`remote_script` wraps the rebuild: before it, the user's trust is checked
without sudo (same logic as the connection test: root, listed, or @wheel +
wheel membership); only when NOT trusted (Vincent's requirement 2026-10-16), `ssh target sudo -n sh -c ...`
appends `# nixos-manager temporary trust` + `extra-trusted-users = <user>`
to `/root/.config/nix/nix.conf` (the daemon reads root's user config;
`extra-` appends rather than replacing) and restarts nix-daemon. The grant
step's output sets `NM_GRANTED=1` only when something was actually written;
after the rebuild (success or failure, via set +e / RC) the revoke step runs
only if NM_GRANTED=1, removes the two marked lines with sed and restarts the
daemon (failure to revoke is a WARNING with manual instructions, not a job
failure). Already-trusted users: log says "nothing to change", no sudo used. Grant failure → exit 9 → job error
"could not grant temporary trust". Needs passwordless sudo on the target,
which deployments need anyway. Intended for the first deployment of a target
whose configuration does not set nix.settings.trusted-users yet.

### Hardening (2026-10-16, Vincent: "don't mess up the target daemon")
Remote grant/revoke are now separate bash scripts fed over ssh with
`bash -s <<'NM_REMOTE_EOF'` (no quoting games). Grant: trust check prefers
nix's own answer (`nix store info` → `Trusted: 1/0`, fallback to config
parsing); refuses with nothing changed when systemctl is missing or
passwordless sudo is not available; waits up to 60 s for an idle daemon
(no `nix-daemon <uid>` children) and refuses rather than restart a busy one;
appends marker + `extra-trusted-users = <user>`; restarts; checks the daemon
is active and that the user is now trusted, otherwise reverts the lines and
restarts again (exit 4/5). Prints `NM_TRUST_GRANTED` so the local wrapper
sets `NM_GRANTED=1`. Revoke: awk removes the marker and ONLY the line
`extra-trusted-users = <user>` directly after it (foreign lines kept, handles
duplicates), then restarts only when idle (else WARNING: lines removed, trust
stays in memory until the daemon next restarts). Username validated
(`valid_unix_user`) before any script is built. `generated_scripts_parse`
test runs `bash -n` on every generated script incl. inner `sh -c` bodies.
Connection test reports leftover temporary-trust entries when sudo works.

## Automatic trust check + target sudo password (2026-10-16, Vincent's design)
Vincent: "the app doesn't need the checkbox; at build deployment check trust
on the target first, if it fails prompt for the sudo password and grant
temporary trust for that build". Implemented:
- `POST /api/remote/check {target}` → `RemoteCheck { reachable, user,
  is_root, trusted, passwordless_sudo, needs_sudo_password, hostname,
  nixos_version, message }` (`remote::check`, one ssh with BatchMode, 30 s
  timeout, same `REMOTE_IS_TRUSTED_FN` logic + `sudo -n true`).
  `needs_sudo_password` = not root AND (not trusted OR sudo needs a password).
- Build dialog: selecting a target runs the check (progress bar), shows
  "trusted by nix / passwordless sudo" status, an amber key note, and a
  required "sudo password on the target (<user>)" field only when needed.
  Start is disabled while checking, when unreachable, or when the password
  is required but empty. Password sent as `DeployRequest.target_sudo_password`
  (null otherwise), cleared after start, never stored.
- `run_remote` passes it to the local bash as env `NM_TARGET_PW` (never on a
  command line). The local wrapper prefixes the remote `bash -s` stream with
  `printf 'NM_PW=%q\n' "$NM_TARGET_PW"` so the grant/revoke scripts get it;
  they use `as_root()` = `printf '%s\n' "$NM_PW" | sudo -S -p '' -k …` when a
  password is set, else `sudo -n`. The trust step runs for every non-root
  user; already-trusted users never touch sudo ("nothing to change").
- Activation: with a password `printf '%s\n' "$NM_TARGET_PW" | nixos-rebuild
  … --ask-sudo-password` (nixos-rebuild-ng reads it via getpass, which falls
  back to stdin without a tty; implies --sudo); without, `--sudo` (or the
  deprecated `--use-remote-sudo` on old nixos-rebuild). So passwordless sudo
  on the target is no longer required.
- Bug found while testing with a fake target: `pgrep -c` prints "0" AND
  exits 1 when nothing matches, so the old `|| echo 0` fallback made
  `daemon_idle` fail forever ("busy for 60s", grant always refused). Fixed
  with `|| true` + numeric sanitising; regression test
  `idle_check_survives_pgrep_zero_exit_one`.
- Dev helper `crates/core/examples/dump_scripts.rs` (`cargo run -p
  nixos-manager-core --example dump_scripts -- grant|revoke|fns|deploy
  <staged>`) prints the generated scripts; `remote_script`, grant/revoke
  builders and `REMOTE_IS_TRUSTED_FN` are `pub` for it. Tested end-to-end
  with fake `ssh`/`sudo` (password check, /root bind-mounted via
  `unshare -rm`)/`nix`/`systemctl`/`pgrep`/`nixos-rebuild` on PATH: wrong
  password → exit 9 nothing changed; right password → grant, rebuild gets the
  password on stdin, revoke; failing rebuild still revokes; failing daemon
  restart reverts; already trusted → no sudo, plain `--sudo` or
  `--ask-sudo-password` depending on whether a password was supplied.
- Old saved targets with `bootstrap_trust` in state.json still load (serde
  ignores the unknown field).

### Real-target failure #3 (2026-10-16): "daemon still does not trust … reverting"
Vincent's lg-laptop deploy with the new flow: password accepted, line added,
daemon restarted, then the verify step said not trusted and reverted. Root
cause: `nix store info` prints `Trusted: 1/0` through `notice()`, i.e. on
**stderr**; `is_trusted` ran it with `2>/dev/null`, so it always fell back to
`nix config show trusted-users`, which is the *client's* config view and can
never see root's `/root/.config/nix/nix.conf`. Fixed with `2>&1`; regression
test `trust_check_reads_nix_stderr`; the fake `nix` used for script tests now
prints on stderr like the real one. `test_connection` also prefers the daemon
answer now. Verified in Nix 2.34.8 source that the daemon does read
`$HOME/.config/nix/nix.conf` for root (`loadConfFile` → `nixUserConfFiles`,
`getHome()` falls back to passwd when systemd leaves HOME unset; NixOS's
nix-daemon.service has no ProtectHome). Lesson: when faking a tool for a
test, copy its real stdout/stderr behaviour first.

### Real-target run #4 (2026-10-16): hang at the sudo prompt, then "incorrect password"
With the stderr fix the whole chain worked on lg-laptop (trust granted, build,
every path copied, trust revoked) but nixos-rebuild first sat idle for 8+
minutes with zero CPU, then activation failed with "Sorry, try again / sudo:
no password was provided / 1 incorrect password attempt". Cause: nixos-rebuild-ng
reads the sudo password with python `getpass`, which ALWAYS opens /dev/tty
first and only falls back to stdin when the process has no controlling
terminal. The daemon is started by run.sh in a terminal (pts/0), so the
prompt went to that terminal and whatever was typed there was sent to sudo;
the password piped on stdin was never read. Fix: the password branch runs
`printf '%s\n' "$NM_TARGET_PW" | setsid -w nixos-rebuild … --ask-sudo-password`
(setsid = no controlling tty → getpass reads stdin; fds stay as they are).
Falls back with a WARNING if setsid is missing. getpass's "GetPassWarning /
Password input may be echoed" noise is filtered out of the job log;
"incorrect password attempt" gets a "Why it failed" hint. Fake
`nixos-rebuild` for script tests now reads the password through real python
getpass, and the e2e run is done under `script -q -e -c … /dev/null` so the
test has a controlling pty like the daemon does (without setsid this hangs).
Diagnosis tip that found it: `ps -o etime,time -p <nixos-rebuild pid>` showed
0 CPU with no children → blocked on input, and `ps -o tty=` showed pts/0.
Also renamed the staging dir prefix `nixos-manager-validate-` →
`nixos-manager-staged-` (it is used by Validate AND remote deploy; Vincent
asked what the tmp path in the log meant). Under run.sh it lives in
`$TMPDIR=/tmp/nix-shell-<pid>-<rand>/` because nix-shell sets TMPDIR.

### Log tail "Done. The new configuration is" (2026-10-16)
First successful lg-laptop deploy ended with the store path on its own line,
then our success echo, then a dangling "Done. The new configuration is".
nixos-rebuild-ng's `print_result` writes the message WITHOUT newline to
stderr and the path to stdout; our line reader only releases the stderr
fragment at EOF. Fix: both deploy scripts run nixos-rebuild with `2>&1`
(remote_script and build_script), so the fragment and the path arrive as one
ordered line. Side effect, intended: nix progress is no longer amber in the
job log (it is stderr chatter, not a warning; Vincent wants amber = warning).

## Build there, activate here (one-off `--build-host`, 2026-10-16, Vincent)
Vincent did not want the recipe-based distributed builds as the only way; a
deploy target is now usable in both directions. Build dialog, "Deploy to:
This machine" → new "Build on" select (This machine | any target). Choosing a
target runs the same `/remote/check` (shown as "Builder: … trusted by nix")
and asks for the builder's sudo password ONLY when the user is not trusted
there (builders never need sudo for activation). Request:
`DeployRequest.build_host: Option<DeployTarget>` (+ `target_sudo_password`).
Local flow unchanged otherwise: sync to /etc/nixos as root, then
`nixos-rebuild <action> --flake /etc/nixos#host --build-host user@host`.
Root-script specifics (`deploy::build_host_block`):
- root must ssh to the builder → `NM_SSH_OPTS="-F $HOME/.ssh/config -o
  UserKnownHostsFile=$HOME/.ssh/known_hosts -i <app key>"` (the user's
  managed config/known hosts/key; ssh does not object to a key owned by
  another user) and `export NIX_SSHOPTS="$NM_SSH_OPTS -o BatchMode=yes …"`
  (nixos-rebuild-ng and `nix copy` both honour it).
- the builder's sudo password travels on the root script's STDIN as the line
  `NM_TARGET_PW=…` after the sudo password (`sudo -S` reads exactly one
  line, byte by byte); the script loops `read` until it sees that marker, so
  it also works when sudo did not consume a line (NOPASSWD/cached).
- unsigned .drv/source paths copied to the builder hit the same
  trusted-users rule → `trust_snippet` grant before, `set +e` rebuild,
  revoke after, exit 9 → "could not grant temporary trust on the builder".
- "Why it failed" hints for "lacks a signature" and SSH refusals as root.
Tests: `build_host_script_shape` (flags, stdin marker, no password in the
script, bash -n); fragment e2e via `dump_scripts builder` under the fakes.
The recipe pair (`remote_builder_use`/`_serve`) remains for permanent,
automatic distributed builds.

## Stop button for running jobs (2026-10-16, Vincent)
`POST /api/jobs/{id}/stop` → `JobManager::stop`. Two mechanisms because the
local build runs as root under `sudo -S` and the daemon (plain user) cannot
signal root processes:
1. **Stop file** `$TMPDIR/nixos-manager-stop-<job id>` (JobHandle::stop_file).
   Both deploy scripts wrap nixos-rebuild in `deploy::stoppable(cmd,
   stop_file)`: `setsid bash -c "$NM_CMD" &` (own session = own process
   group, and no controlling tty → the target-sudo-password path no longer
   needs its own setsid), then a 1 s poll loop: zombie check via
   /proc/$pid/stat (kill -0 is true for zombies), on stop file → `kill -TERM
   -- -$NM_PID`, SIGKILL after 20 s; `wait || RC=$?`; stopped → RC=130.
   The temporary-trust revoke still runs after (RC!=0 → exit RC after it).
   Stopping is only honoured around nixos-rebuild, never during the
   rm -rf/cp sync phase (which is short and must not be interrupted).
2. **Process group**: `process::stream_job/stream_cmd_job` spawn with
   `process_group(0)` and register the pgid on the job. Jobs whose script
   polls the stop file declare it (`JobHandle::uses_stop_file`); for them
   `stop()` only writes the file and signals the group as a FALLBACK after
   30 s (then SIGKILL 10 s later). Lesson from the first attempt: signalling
   the group immediately killed the polling bash wrapper first, the real
   work (own session) survived and kept the log pipe open, so the job hung
   as "running" forever. Non-script jobs are signalled immediately (SIGKILL
   after 20 s). For sudo jobs the signal is a harmless EPERM anyway.
`set_status`: Failed + stop requested → `Cancelled` with "stopped by the
user"; stop file removed on any terminal status. Exit 130 maps to failed →
cancelled. UI: Stop button (confirm dialog warning that stopping during
activation can leave the system half-switched) on the job log page.
Tested: fake remote deploy with a sleeping nixos-rebuild: stop file after
3 s → SIGTERM reaches the fake, RC=130, trust revoked, no leftovers.

## After the build: reboot / power off + logs on disk (2026-10-16, Vincent)
"Sometimes I just want to run the build and leave." `DeployRequest.after:
Option<AfterBuild>` (none|poweroff|reboot). Only after SUCCESS (the script
reaches the line after the RC check): local root script runs
`setsid bash -c 'sleep 5; shutdown [-r] now' &` detached so the script
exits, the job is marked succeeded and the log file is flushed first
(`deploy::after_snippet`). Remote target: `remote_after_snippet` → ssh +
`as_root shutdown …` with NM_PW (same mechanism as the trust scripts),
WARNING if sudo refuses. Nothing happens on failure or stop.
Every job log is now also written to
`~/.local/state/nixos-manager/logs/<stamp>-<kind>-<id8>.log` (header
lines `# title/kind/started/id`, `[app]` prefix for app lines, trailing
`# status: …`), pruned to 60 files; `JobInfo.log_file`; `GET /api/jobs/
history` + `/history/{name}` and an "Earlier logs on disk" section on the
Jobs page, so the log can be read after the reboot.
- Logs cleanup (same day): `DELETE /api/jobs/history/{name}` and
  `DELETE /api/jobs/history` (all); logs of running jobs are never deleted
  (`is_log_in_use`). Jobs page: per-log Delete button + "Clear all logs"
  with confirm and total size.
- "Also when the build fails" (same day): `DeployRequest.after_even_on_failure`.
  Scripts define `nm_after <outcome>` early (`deploy::after_prelude`, body =
  `after_local_body` or `remote::remote_after_body`); the success path calls
  `nm_after finished`; with the flag an EXIT trap calls `nm_after "failed
  (exit N)"` for any non-zero exit EXCEPT 130 (manual Stop: the user is
  there). Covers failures in the sync phase too (set -e → trap). Verified
  with fakes: failure → shutdown called, success → called, stop → not called.

## Third mode: build AND activate on the target (2026-10-16, Vincent)
`DeployRequest.on_target = true` (+ `allow_unfree_env`). `remote::run_on_target`:
stage plain copy (flake.lock skipped as always) → write the LOCAL root
script (`deploy::build_script_with`, i.e. the sync.sh mirror: preserve
/etc/nixos/flake.lock, copy /etc/nixos/hardware-configuration.nix over the
working copy unless hardware=workspace, rm -rf /etc/nixos/*, cp, restore
lock, stoppable nixos-rebuild, after-action) into
`staged/.nixos-manager-deploy.sh` (0700; dotfile so `cp -r $WORKDIR/*`
never copies it into /etc/nixos) → local wrapper `on_target_wrapper`:
`rsync -az --delete -e "$SSH"` (tar over ssh fallback) to
`/tmp/nixos-manager-deploy-<id8>` on the target, a background watcher
relays the local stop file as `<dir>.stop` on the target (the uploaded
script polls that path), then `printf pw | ssh T "sudo -S -p '' bash
<dir>/.nixos-manager-deploy.sh"` (password on ssh stdin, never argv).
**Cleanup always** (Vincent asked explicitly): `trap cleanup EXIT` +
`trap 'exit 143' TERM INT` → `ssh T rm -rf <dir> <dir>.stop` on success,
failure and stop. ScriptEnv: flake → `export NIX_CONFIG='experimental-
features = nix-command flakes'`; allow_unfree_env → `NIXPKGS_ALLOW_UNFREE=1`
+ `--impure` on the flake rebuild (env var only counts with --impure);
legacy configs just get the env var. keep_configuration forced to Local
(working copy wins), build_host cleared, no nix trust needed (root there).
Password rule in the dialog: sudo password unless root or passwordless
sudo (`onTargetNeedsPassword`). Hints for sudo/ssh/flakes/unfree failures.
Job title "nixos-rebuild Switch on <target>". Fakes for e2e: fake ssh runs
single-arg commands via `bash -c`, fake sudo bind-mounts fake /root AND a
fake /etc/nixos (unshare -rm) so the real sync script runs end to end.
- Bug caught by the on-target e2e (2026-10-16): the stop wrapper runs the
  rebuild in a CHILD bash (`setsid bash -c "$NM_CMD"`), so unexported
  script variables are empty there: `--flake "$NIXOS_DIR#main"` became
  `--flake "#main"`. Local builds since the Stop feature would have been
  broken. Fix: `export NIXOS_DIR` / `export WORKDIR` in build_script (+ a
  test that evaluates the expansion through a child bash). Rule: anything
  referenced inside NM_CMD must be exported or literal.
- Target deployments: second selector "After the deployment, this machine"
  (`DeployRequest.after_local`; `after` keeps acting on the target).
  `deploy::local_after_action` (runs as the daemon user, after the job's
  script finished): detached `setsid bash -c 'sleep 5; systemctl
  --no-ask-password poweroff|reboot || printf pw | sudo -S shutdown …'`
  — logind first (works for an active desktop session via polkit), sudo
  fallback with the optional local password (`password` field, validated
  with check_sudo at job start when given). Respects "also on failure";
  never after a manual stop (130). The dialog shows the optional local
  sudo password field only when a local action is chosen.
