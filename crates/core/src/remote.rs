//! Remote machines: "build here, activate there" deployments with
//! `nixos-rebuild --target-host`, SSH connection tests, and the root SSH key
//! that remote builders (`nix.buildMachines`) need because the nix daemon
//! connects as root.

use crate::deploy::run_root_script;
use crate::jobs::JobHandle;
use crate::process;
use crate::types::{ConfigKind, DeployRequest, DeployTarget, LogLine, LogStream, RebuildAction, RemoteCheck};
use std::path::Path;

fn sh(s: &str) -> String {
    format!("'{}'", s.replace('\'', "'\\''"))
}

fn action_arg(a: RebuildAction) -> &'static str {
    match a {
        RebuildAction::Switch => "switch",
        RebuildAction::Boot => "boot",
        RebuildAction::Test => "test",
        RebuildAction::Build => "build",
        RebuildAction::DryBuild => "dry-build",
    }
}

/// The user-side script for a remote deployment. `staged` is a plain copy of
/// the working folder: a `path:` flake reference sees every file in it,
/// whereas a git checkout would hide untracked files such as a gitignored
/// hardware-configuration.nix.
pub fn remote_script(req: &DeployRequest, target: &DeployTarget, staged: &Path, kind: ConfigKind, sudo_flag: &str, stop_file: &Path) -> String {
    let wd = sh(&staged.to_string_lossy());
    let action = action_arg(req.action);
    let mut s = String::new();
    s.push_str("set -euo pipefail\n");
    // Fail fast instead of hanging on an interactive prompt (no TTY here).
    s.push_str("export NIX_SSHOPTS=\"${NIX_SSHOPTS:-} -o BatchMode=yes -o ConnectTimeout=20\"\n");
    s.push_str(&format!("cd {wd}\n"));
    s.push_str(&format!("echo \"[nixos-manager] deploying to {} as $(id -un) from a staged copy of the working folder\"\n", target.ssh_target));
    if req.upgrade && kind == ConfigKind::Flake {
        s.push_str("echo \"[nixos-manager] --upgrade: updating flake inputs\"\nnix flake update\n");
    }
    let (_, user) = split_target(&target.ssh_target);
    let needs_trust_step = user.as_deref().map(|u| u != "root").unwrap_or(true);
    if needs_trust_step {
        s.push_str(&trust_snippet(target, true));
    }
    s.push_str(&crate::deploy::after_prelude(req.after, req.after_even_on_failure, None, &remote_after_body(target, req.after)));
    let mut cmd = format!("nixos-rebuild {action}");
    match kind {
        ConfigKind::Flake => {
            let host = target.flake_host.clone().or_else(|| req.host.clone()).unwrap_or_default();
            let flake_ref = format!("path:{}", staged.to_string_lossy());
            if host.is_empty() {
                cmd.push_str(&format!(" --flake {}", sh(&flake_ref)));
            } else {
                cmd.push_str(&format!(" --flake {}", sh(&format!("{flake_ref}#{host}"))));
            }
        }
        _ => {
            cmd.push_str(&format!(" --no-flake -I {}", sh(&format!("nixos-config={}/configuration.nix", staged.to_string_lossy()))));
        }
    }
    cmd.push_str(&format!(" --target-host {}", sh(&target.ssh_target)));
    if let Some(bh) = target.build_host.as_deref().filter(|b| !b.trim().is_empty()) {
        cmd.push_str(&format!(" --build-host {}", sh(bh)));
    }
    // nixos-rebuild's stderr is progress, not errors, and its final message is
    // split across stderr ("Done. The new configuration is ") and stdout (the
    // path): merged (2>&1) so the log stays in order and in one colour.
    // Activation: with a target sudo password nixos-rebuild asks for it on
    // stdin (--ask-sudo-password, getpass falls back to stdin without a tty);
    // otherwise plain --sudo relies on passwordless sudo.
    // nixos-rebuild reads the sudo password with python's getpass, which
    // opens /dev/tty whenever the process has a controlling terminal (the
    // daemon started from a terminal by run.sh passes one down) and only
    // falls back to stdin without one. setsid detaches the terminal so the
    // piped password is what gets read.
    // The rebuild runs through `stoppable` (own session, polled stop file).
    // A new session has no controlling terminal, so with a target sudo
    // password nixos-rebuild's getpass reads it from stdin.
    let with_pw = format!("printf '%s\\n' \"$NM_TARGET_PW\" | {cmd} --ask-sudo-password 2>&1");
    let without = if target.use_remote_sudo { format!("{cmd} {sudo_flag} 2>&1") } else { format!("{cmd} 2>&1") };
    let run_line = if target.use_remote_sudo {
        format!(
            "if [ -n \"${{NM_TARGET_PW:-}}\" ]; then\necho \"[nixos-manager] $ {cmd} --ask-sudo-password  (password supplied on stdin)\"\n{}else\necho \"[nixos-manager] $ {cmd} {sudo_flag}\"\n{}fi\n",
            crate::deploy::stoppable(&with_pw, stop_file),
            crate::deploy::stoppable(&without, stop_file)
        )
    } else {
        format!("echo \"[nixos-manager] $ {cmd}\"\n{}", crate::deploy::stoppable(&without, stop_file))
    };
    s.push_str(&run_line);
    if needs_trust_step {
        // Always undo the temporary trust, even when the rebuild fails or is stopped.
        s.push_str(&trust_snippet(target, false));
    }
    s.push_str("if [ \"$RC\" -ne 0 ]; then exit \"$RC\"; fi\n");
    s.push_str(&format!("echo \"[nixos-manager] nixos-rebuild {action} on {} finished successfully.\"\n", target.ssh_target));
    s.push_str(&crate::deploy::after_snippet(req.after, None));
    s
}

/// Shell snippet (remote side) that answers "is the current user trusted by
/// the nix daemon?" Prefers nix's own answer (`nix store info` prints
/// `Trusted: 1`), falls back to the configured list.
pub const REMOTE_IS_TRUSTED_FN: &str = r#"is_trusted() {
  # "Trusted: 1/0" is printed on STDERR (nix's notice() channel), so keep it
  out=$(nix store info 2>&1 || nix store ping 2>&1 || true)
  case "$out" in *"Trusted: 1"*) return 0;; *"Trusted: 0"*) return 1;; esac
  me=$(id -un); [ "$me" = root ] && return 0
  tu=$(nix config show trusted-users 2>/dev/null || nix show-config 2>/dev/null | sed -n "s/^trusted-users = //p" || true)
  echo " $tu " | grep -q " $me " && return 0
  echo " $tu " | grep -q " @wheel " && id -nG | grep -qw wheel && return 0
  return 1
}
as_root() {
  if [ -n "${NM_PW:-}" ]; then printf "%s\n" "$NM_PW" | sudo -S -p "" -k "$@"; else sudo -n "$@"; fi
}
daemon_idle() {
  # child processes are named "nix-daemon <uid>" while serving a client
  # pgrep -c prints 0 AND exits 1 when nothing matches, so never append a fallback to its output
  n=$(pgrep -c -f "^nix-daemon [0-9]" 2>/dev/null || true); case "$n" in ''|*[!0-9]*) n=0;; esac; [ "$n" -eq 0 ]
}
wait_idle() {
  for i in $(seq 1 30); do daemon_idle && return 0; [ "$i" -eq 1 ] && echo "[nixos-manager] nix-daemon is busy on the target; waiting up to 60s for it to be idle before restarting it..."; sleep 2; done
  return 1
}"#;

/// Remote script, run as the SSH user with `bash -s`: grants temporary trust
/// only when needed, verifies it, and reverts on any failure.
pub fn remote_grant_script(user: &str) -> String {
    format!(
        r#"set -u
{fns}
revert_as_root='f=/root/.config/nix/nix.conf; [ -f "$f" ] || exit 0; tmp=$(mktemp); awk -v u="{user}" '"'"'skip{{skip=0; if ($0 == "extra-trusted-users = " u) next}} /^# nixos-manager temporary trust$/{{skip=1; next}} {{print}}'"'"' "$f" > "$tmp" && cat "$tmp" > "$f"; rm -f "$tmp"'
if is_trusted; then echo "[nixos-manager] {user} is already a trusted user on $(uname -n); nothing to change"; exit 0; fi
command -v systemctl >/dev/null 2>&1 || {{ echo "[nixos-manager] ERROR: the target has no systemctl; cannot restart nix-daemon safely, nothing changed" >&2; exit 2; }}
as_root true 2>/dev/null || {{ echo "[nixos-manager] ERROR: sudo on the target refused ({user}): wrong password, or no sudo rights; nothing changed" >&2; exit 2; }}
wait_idle || {{ echo "[nixos-manager] ERROR: nix-daemon stayed busy for 60s; not restarting it (that would abort running builds). Nothing changed; try again later." >&2; exit 3; }}
as_root sh -c 'f=/root/.config/nix/nix.conf; mkdir -p /root/.config/nix; touch "$f"; if grep -q "^# nixos-manager temporary trust$" "$f"; then echo "[nixos-manager] a temporary trust entry was already present (leftover from an earlier run)"; else printf "\n# nixos-manager temporary trust\nextra-trusted-users = {user}\n" >> "$f"; echo "[nixos-manager] added extra-trusted-users = {user} to the nix.conf of root"; fi' || {{ echo "[nixos-manager] ERROR: could not write the nix.conf of root" >&2; exit 2; }}
if ! as_root systemctl restart nix-daemon; then
  echo "[nixos-manager] ERROR: nix-daemon failed to restart; reverting the change" >&2
  as_root sh -c "$revert_as_root"; as_root systemctl restart nix-daemon || true
  exit 4
fi
sleep 1
if systemctl is-active --quiet nix-daemon || systemctl is-active --quiet nix-daemon.socket; then :; else
  echo "[nixos-manager] ERROR: nix-daemon is not active after the restart; reverting" >&2
  as_root sh -c "$revert_as_root"; as_root systemctl restart nix-daemon || true
  exit 4
fi
if is_trusted; then
  echo "[nixos-manager] temporary trust granted to {user} on $(uname -n)"
  echo NM_TRUST_GRANTED
else
  echo "[nixos-manager] ERROR: the daemon still does not trust {user} after the change (is the home of root /root?); reverting" >&2
  as_root sh -c "$revert_as_root"; as_root systemctl restart nix-daemon || true
  exit 5
fi
"#,
        fns = REMOTE_IS_TRUSTED_FN,
        user = user
    )
}

/// Remote script that removes exactly the two lines the grant wrote and
/// restarts the daemon when it is idle.
pub fn remote_revoke_script(user: &str) -> String {
    format!(
        r#"set -u
{fns}
as_root sh -c 'f=/root/.config/nix/nix.conf; [ -f "$f" ] || exit 0; grep -q "^# nixos-manager temporary trust$" "$f" || exit 0; tmp=$(mktemp); awk -v u="{user}" '"'"'skip{{skip=0; if ($0 == "extra-trusted-users = " u) next}} /^# nixos-manager temporary trust$/{{skip=1; next}} {{print}}'"'"' "$f" > "$tmp" && cat "$tmp" > "$f"; rm -f "$tmp"; echo "[nixos-manager] temporary trust entry removed from the nix.conf of root"' || {{ echo "[nixos-manager] WARNING: could not edit the nix.conf of root; remove the marker line # nixos-manager temporary trust and the extra-trusted-users line after it by hand" >&2; exit 1; }}
if wait_idle; then
  as_root systemctl restart nix-daemon && echo "[nixos-manager] nix-daemon restarted; temporary trust is gone"
else
  echo "[nixos-manager] WARNING: nix-daemon is busy; the config lines are removed but the daemon keeps trusting {user} until it next restarts (it will not be interrupted now)." >&2
fi
"#,
        fns = REMOTE_IS_TRUSTED_FN,
        user = user
    )
}

pub(crate) fn valid_unix_user(u: &str) -> bool {
    let mut chars = u.chars();
    matches!(chars.next(), Some(c) if c.is_ascii_lowercase() || c == '_') && u.len() <= 32 && u.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_' || c == '-')
}

/// Local-side wrapper: feeds the remote script to `bash -s` over ssh (no
/// quoting games), records whether anything was granted, and revokes only
/// in that case.
/// Dev-helper access to the trust wrapper (examples/dump_scripts.rs).
pub fn trust_snippet_pub(target: &DeployTarget, grant: bool) -> String {
    trust_snippet(target, grant)
}

pub(crate) fn trust_snippet(target: &DeployTarget, grant: bool) -> String {
    let (_, user) = split_target(&target.ssh_target);
    let user = user.unwrap_or_else(|| "root".into());
    if grant {
        format!(
            "echo \"[nixos-manager] checking whether the user is trusted by nix on the target (temporary trust is granted only if not)...\"\nNM_GRANTED=0\nNM_RC=0\nNM_OUT=$({{ printf 'NM_PW=%q\\n' \"${{NM_TARGET_PW:-}}\"; cat <<'NM_REMOTE_EOF'\n{script}NM_REMOTE_EOF\n}} | ssh ${{NM_SSH_OPTS:-}} -o BatchMode=yes -o ConnectTimeout=20 {t} bash -s 2>&1) || NM_RC=$?\necho \"$NM_OUT\" | grep -v '^NM_TRUST_GRANTED$' || true\ncase \"$NM_OUT\" in *NM_TRUST_GRANTED*) NM_GRANTED=1;; esac\nif [ \"$NM_RC\" -ne 0 ]; then echo \"[nixos-manager] ERROR: could not grant temporary trust (see above); nothing was deployed\" >&2; exit 9; fi\n",
            t = sh(&target.ssh_target),
            script = remote_grant_script(&user),
        )
    } else {
        format!(
            "if [ \"${{NM_GRANTED:-0}}\" = 1 ]; then echo \"[nixos-manager] removing the temporary nix trust granted earlier...\"; {{ printf 'NM_PW=%q\\n' \"${{NM_TARGET_PW:-}}\"; cat <<'NM_REMOTE_EOF'\n{script}NM_REMOTE_EOF\n}} | ssh ${{NM_SSH_OPTS:-}} -o BatchMode=yes -o ConnectTimeout=20 {t} bash -s || true; fi\n",
            t = sh(&target.ssh_target),
            script = remote_revoke_script(&user),
        )
    }
}

pub async fn run_remote(job: JobHandle, req: DeployRequest, target: DeployTarget) {
    job.running().await;
    let workdir = std::path::PathBuf::from(&req.path);
    let kind = match crate::discover::location_info(&workdir) {
        Ok(l) => l.kind,
        Err(e) => {
            job.failed(e.to_string()).await;
            return;
        }
    };
    if kind == ConfigKind::Unknown {
        job.failed("the working copy has neither flake.nix nor configuration.nix").await;
        return;
    }
    if std::env::var("SSH_AUTH_SOCK").is_err() {
        job.system("Note: no SSH agent is visible to the daemon; the SSH key for the target must have no passphrase.").await;
    }
    if crate::secrets::deploy_info(&workdir).present {
        job.system("Note: managed secrets (secrets.age) are not installed on remote targets; only this machine's builds do that.").await;
    }
    // Newer nixos-rebuild spells it --sudo; older ones only know --use-remote-sudo.
    let sudo_flag = match process::run("nixos-rebuild", &["--help"], None, std::time::Duration::from_secs(10)).await {
        Ok(o) if o.stdout.contains("--sudo") || o.stderr.contains("--sudo") => "--sudo",
        _ => "--use-remote-sudo",
    };
    {
        let (_, user) = split_target(&target.ssh_target);
        let user = user.unwrap_or_else(|| "root".into());
        if !valid_unix_user(&user) {
            job.failed(format!("'{user}' is not a plain unix user name; use user@host with a normal account")).await;
            return;
        }
    }
    // Stage a plain copy so untracked/gitignored files are included.
    let staged = match crate::validate::stage_plain_copy(&workdir) {
        Ok(p) => p,
        Err(e) => {
            job.failed(format!("cannot stage a copy of the working folder: {e}")).await;
            return;
        }
    };
    // The target's own hardware-configuration.nix is the source of truth:
    // the working copy's file (possibly another machine's disks) is ignored.
    let hw_rel = crate::hardware::rel_path_in(&workdir);
    if req.hardware == Some(crate::types::HardwareChoice::Workspace) {
        if !staged.join(&hw_rel).is_file() {
            job.failed(format!("the working copy has no {} but \"use the working copy's hardware-configuration.nix\" was chosen", hw_rel.display())).await;
            let _ = std::fs::remove_dir_all(&staged);
            return;
        }
        job.system(format!("Using the working copy's {} as requested; the target's own /etc/nixos/hardware-configuration.nix is NOT used. Make sure it describes the target's disks.", hw_rel.display())).await;
    } else {
    job.system(format!("Fetching the target's own /etc/nixos/hardware-configuration.nix (the working copy's {} is ignored for remote targets)...", hw_rel.display())).await;
    match fetch_target_hardware(&target).await {
        Ok((source, text)) => {
            let dest = staged.join(&hw_rel);
            if let Some(p) = dest.parent() {
                let _ = std::fs::create_dir_all(p);
            }
            if let Err(e) = std::fs::write(&dest, text) {
                job.failed(format!("could not write the target's hardware configuration into the staged copy: {e}")).await;
                let _ = std::fs::remove_dir_all(&staged);
                return;
            }
            job.system(match source.as_str() {
                "etc" => "Using the target's /etc/nixos/hardware-configuration.nix.".to_string(),
                _ => "The target has no /etc/nixos/hardware-configuration.nix; using one generated on the target with nixos-generate-config --show-hardware-config.".to_string(),
            }).await;
        }
        Err(e) => {
            job.failed(format!("cannot determine the target's hardware configuration: {e}")).await;
            let _ = std::fs::remove_dir_all(&staged);
            return;
        }
    }
    }
    // flake.lock lives in /etc/nixos only. Pin the deployment to the target's
    // own lock when it has one; otherwise fall back to this machine's; an
    // upgrade (or no lock anywhere) resolves the inputs fresh.
    if kind == ConfigKind::Flake {
        let _ = std::fs::remove_file(staged.join("flake.lock"));
        if req.upgrade {
            job.system("Upgrade: flake inputs are resolved fresh (no flake.lock is used).").await;
        } else if let Some(lock) = fetch_target_file(&target, "/etc/nixos/flake.lock").await {
            let _ = std::fs::write(staged.join("flake.lock"), lock);
            job.system("Using the target's /etc/nixos/flake.lock (pinned inputs).").await;
        } else if let Ok(lock) = std::fs::read(std::path::Path::new(crate::deploy::ETC_NIXOS).join("flake.lock")) {
            let _ = std::fs::write(staged.join("flake.lock"), lock);
            job.system("The target has no /etc/nixos/flake.lock; using this machine's /etc/nixos/flake.lock. Missing inputs are resolved fresh.").await;
        } else {
            job.system("No flake.lock on the target or on this machine; flake inputs are resolved fresh.").await;
        }
    }
    job.uses_stop_file().await;
    let stop_file = job.stop_file().await;
    let script = remote_script(&req, &target, &staged, kind, sudo_flag, &stop_file);
    let sender = job.line_sender();
    let seen = std::sync::Arc::new(std::sync::Mutex::new(Vec::<String>::new()));
    let seen_w = seen.clone();
    let mut cmd = tokio::process::Command::new("bash");
    cmd.arg("-c").arg(&script).current_dir(&staged);
    if let Some(pw) = req.target_sudo_password.as_deref().filter(|p| !p.is_empty()) {
        cmd.env("NM_TARGET_PW", pw);
    }
    let result = process::stream_cmd_job(&job, cmd, None, move |is_err, line| {
        // python's getpass complains when it has to read from a pipe; expected here
        if line.contains("GetPassWarning") || line.contains("fallback_getpass") || line.contains("Password input may be echoed") {
            return;
        }
        if line.contains("lacks a signature by a trusted key") || line.contains("Permission denied") || line.contains("sudo: a password is required") || line.contains("incorrect password attempt") {
            seen_w.lock().unwrap().push(line.clone());
        }
        let _ = sender.send(LogLine { stream: if is_err { LogStream::Stderr } else { LogStream::Stdout }, text: line });
    })
    .await;
    let seen_lines = seen.lock().unwrap().clone();
    let user = target.ssh_target.split('@').next().filter(|_| target.ssh_target.contains('@')).unwrap_or("your user").to_string();
    if seen_lines.iter().any(|l| l.contains("lacks a signature by a trusted key")) {
        job.system(format!("Why it failed: the target's nix daemon only accepts unsigned store paths from trusted users, and '{user}' is not one on the target; the temporary trust step above should have handled this, check its messages. Permanent fix on the target's configuration:  nix.settings.trusted-users = [ \"{user}\" ];  (the recipe \"Nix store and build settings\" writes it).")).await;
    } else if seen_lines.iter().any(|l| l.contains("incorrect password attempt")) {
        job.system(format!("Why it failed: the build and the copy to the target succeeded, but sudo on the target rejected the password for {user} during activation. Check the target's sudo password in the build dialog and deploy again (the already-copied paths are reused, so the retry is quick).")).await;
    } else if seen_lines.iter().any(|l| l.contains("sudo: a password is required")) {
        job.system(format!("Why it failed: sudo on the target asked for a password. Enter the target's sudo password for {user} in the build dialog, or use root@ as the SSH address.")).await;
    } else if seen_lines.iter().any(|l| l.contains("Permission denied")) {
        job.system("Why it failed: SSH login was refused. Use \"Set up key access\" on the Remote page for this target.").await;
    }
    // The lock is deliberately NOT written back to the working folder.
    let _ = std::fs::remove_dir_all(&staged);
    match result {
        Ok(0) => job.succeeded().await,
        Ok(9) => job.failed("could not grant temporary trust on the target; see the log").await,
        Ok(130) => job.failed("stopped by the user").await,
        Ok(code) => job.failed(format!("remote deployment exited with status {code}; see the log")).await,
        Err(e) => job.failed(e.to_string()).await,
    }
}

/// Reboot / power off the TARGET after a successful remote deployment, as
/// root there (sudo password through NM_PW like the trust scripts).
/// Body of `nm_after` for a remote target (see deploy::after_prelude).
fn remote_after_body(target: &DeployTarget, after: Option<crate::types::AfterBuild>) -> String {
    use crate::types::AfterBuild;
    let (verb, cmd) = match after {
        Some(AfterBuild::Reboot) => ("rebooting", "shutdown -r now"),
        Some(AfterBuild::Poweroff) => ("powering off", "shutdown now"),
        _ => return String::new(),
    };
    let t = sh(&target.ssh_target);
    let script = format!("{REMOTE_IS_TRUSTED_FN}\nif as_root {cmd} 2>/dev/null; then echo \"[nixos-manager] target is {verb} now\"; else echo \"[nixos-manager] WARNING: could not run {cmd} on the target (sudo refused?)\"; fi\n");
    format!(
        "  {{ printf 'NM_PW=%q\\n' \"${{NM_TARGET_PW:-}}\"; cat <<'NM_REMOTE_EOF'\n{script}NM_REMOTE_EOF\n  }} | ssh ${{NM_SSH_OPTS:-}} -o BatchMode=yes -o ConnectTimeout=20 {t} bash -s || true\n"
    )
}

/// Remote script printing the target's hardware configuration: first line
/// `NM_HW=etc|generate|none`, then the file.
pub fn target_hardware_script() -> &'static str {
    "if [ -f /etc/nixos/hardware-configuration.nix ]; then echo NM_HW=etc; cat /etc/nixos/hardware-configuration.nix; elif command -v nixos-generate-config >/dev/null 2>&1; then echo NM_HW=generate; nixos-generate-config --show-hardware-config 2>/dev/null; else echo NM_HW=none; fi\n"
}

/// Read one file from the target over SSH (None when absent or unreachable).
async fn fetch_target_file(target: &DeployTarget, remote_path: &str) -> Option<String> {
    let local = format!("ssh ${{NM_SSH_OPTS:-}} -o BatchMode=yes -o ConnectTimeout=20 {} cat {}", sh(&target.ssh_target), sh(remote_path));
    let out = process::run("bash", &["-c", &local], None, std::time::Duration::from_secs(60)).await.ok()?;
    (out.status == 0 && !out.stdout.trim().is_empty()).then_some(out.stdout)
}

/// Fetch the target's hardware-configuration.nix over SSH → (source, text).
pub async fn fetch_target_hardware(target: &DeployTarget) -> std::result::Result<(String, String), String> {
    let local = format!(
        "ssh ${{NM_SSH_OPTS:-}} -o BatchMode=yes -o ConnectTimeout=20 {} bash -s <<'NM_REMOTE_EOF'\n{}NM_REMOTE_EOF\n",
        sh(&target.ssh_target),
        target_hardware_script()
    );
    let out = process::run("bash", &["-c", &local], None, std::time::Duration::from_secs(60)).await.map_err(|e| e.to_string())?;
    let stdout = out.stdout;
    let mut lines = stdout.splitn(2, '\n');
    let first = lines.next().unwrap_or("").trim();
    let body = lines.next().unwrap_or("");
    if out.status != 0 && !first.starts_with("NM_HW=") {
        return Err(format!("ssh failed (exit {}): {}", out.status, out.stderr.trim().chars().take(300).collect::<String>()));
    }
    match first.strip_prefix("NM_HW=") {
        Some("etc") if !body.trim().is_empty() => Ok(("etc".into(), body.to_string())),
        Some("generate") if body.contains("fileSystems") || body.contains("boot.") => Ok(("generate".into(), body.to_string())),
        Some("generate") => Err("nixos-generate-config on the target produced nothing usable; create /etc/nixos/hardware-configuration.nix there with  sudo nixos-generate-config".into()),
        Some("none") => Err("the target has no /etc/nixos/hardware-configuration.nix and no nixos-generate-config; create it there with  sudo nixos-generate-config".into()),
        _ => Err(format!("unexpected answer from the target: {}", first.chars().take(120).collect::<String>())),
    }
}

/// Synchronous pre-deployment check of a target (reachable, trusted, sudo).
pub async fn check(target: &DeployTarget) -> RemoteCheck {
    let (_, user) = split_target(&target.ssh_target);
    let user = user.unwrap_or_else(|| "root".into());
    let script = format!(
        "{fns}\necho \"NM_HOST=$(uname -n)\"; echo \"NM_NIXOS=$(nixos-version 2>/dev/null || true)\"; if is_trusted; then echo NM_TRUSTED=1; else echo NM_TRUSTED=0; fi; if sudo -n true 2>/dev/null; then echo NM_SUDO=1; else echo NM_SUDO=0; fi; if [ -f /etc/nixos/hardware-configuration.nix ]; then echo NM_HW=etc; elif command -v nixos-generate-config >/dev/null 2>&1; then echo NM_HW=generate; else echo NM_HW=none; fi\n",
        fns = REMOTE_IS_TRUSTED_FN
    );
    let mut cmd = tokio::process::Command::new("bash");
    let local = format!("ssh ${{NM_SSH_OPTS:-}} -o BatchMode=yes -o ConnectTimeout=15 {} bash -s <<'NM_REMOTE_EOF'\n{script}NM_REMOTE_EOF\n", sh(&target.ssh_target));
    cmd.arg("-c").arg(&local);
    let lines = std::sync::Arc::new(std::sync::Mutex::new(Vec::<String>::new()));
    let l2 = lines.clone();
    let rc = tokio::time::timeout(std::time::Duration::from_secs(30), process::stream_cmd(cmd, move |_, line| l2.lock().unwrap().push(line))).await;
    let lines = lines.lock().unwrap().clone();
    let get = |k: &str| lines.iter().find_map(|l| l.strip_prefix(k).map(|v| v.to_string()));
    let ok = matches!(rc, Ok(Ok(0)));
    let is_root = user == "root";
    let trusted = get("NM_TRUSTED=").map(|v| v == "1");
    let pw_sudo = get("NM_SUDO=").map(|v| v == "1");
    let needs = !is_root && (trusted != Some(true) || pw_sudo != Some(true));
    RemoteCheck {
        reachable: ok,
        user,
        is_root,
        trusted,
        passwordless_sudo: pw_sudo,
        needs_sudo_password: ok && needs,
        hostname: get("NM_HOST=").filter(|h| !h.is_empty()),
        nixos_version: get("NM_NIXOS=").filter(|h| !h.is_empty()),
        hardware_config: get("NM_HW="),
        message: if ok { None } else { Some(lines.iter().filter(|l| !l.starts_with("NM_")).cloned().collect::<Vec<_>>().join(" ").chars().take(300).collect()) },
    }
}

/// `ssh target 'nixos-version'` with BatchMode, as the user.
pub async fn test_connection(job: JobHandle, ssh_target: String) {
    job.running().await;
    let script = format!(
        "set -o pipefail\necho \"[nixos-manager] ssh {t} ...\"\nssh ${{NM_SSH_OPTS:-}} -o BatchMode=yes -o ConnectTimeout=15 {t} 'echo connected as $(id -un) on $(uname -n); nixos-version; nix --version; sudo -n true 2>/dev/null && echo \"passwordless sudo: yes\" || echo \"passwordless sudo: no (use root@ or configure NOPASSWD)\"; TU=$(nix config show trusted-users 2>/dev/null || nix show-config 2>/dev/null | sed -n \"s/^trusted-users = //p\"); ME=$(id -un); TR=$(nix store info 2>&1 | sed -n \"s/^Trusted: //p\"); if [ \"$ME\" = root ]; then echo \"trusted by nix: yes (root)\"; elif [ \"$TR\" = 1 ]; then echo \"trusted by nix: yes (daemon answer)\"; elif [ \"$TR\" = 0 ]; then echo \"trusted by nix: NO - deployments need the temporary trust step (the build dialog asks for your sudo password on this target). Permanent fix: nix.settings.trusted-users = [ \\\"$ME\\\" ] on this target.\"; elif echo \" $TU \" | grep -q \" $ME \"; then echo \"trusted by nix: yes\"; elif echo \" $TU \" | grep -q \" @wheel \" && id -nG | grep -qw wheel; then echo \"trusted by nix: yes (via @wheel)\"; else echo \"trusted by nix: NO - deployments will be refused (cannot add path ... lacks a signature). Add nix.settings.trusted-users = [ \\\"$ME\\\" ] on this target.\"; fi; if sudo -n grep -q \"^# nixos-manager temporary trust\" /root/.config/nix/nix.conf 2>/dev/null; then echo \"note: a leftover temporary trust entry from an interrupted deployment is still in /root/.config/nix/nix.conf; the next deployment with temporary trust enabled removes it\"; fi'\n",
        t = sh(&ssh_target)
    );
    let sender = job.line_sender();
    let result = process::stream("bash", &["-c", &script], None, None, move |is_err, line| {
        let _ = sender.send(LogLine { stream: if is_err { LogStream::Stderr } else { LogStream::Stdout }, text: line });
    })
    .await;
    match result {
        Ok(0) => job.succeeded().await,
        Ok(code) => job.failed(format!("ssh exited with status {code}. Check the address, that your key is authorized there, and that the host key is known.")).await,
        Err(e) => job.failed(e.to_string()).await,
    }
}

/// Make sure root has an SSH key (the nix daemon connects to builders as
/// root), print its public key, and optionally pre-trust a builder's host
/// key. Needs the sudo password.
pub async fn root_builder_key(job: JobHandle, password: String, builder_host: Option<String>) {
    job.running().await;
    let mut script = String::from(
        r#"set -euo pipefail
install -d -m 700 /root/.ssh
if [ ! -f /root/.ssh/id_ed25519 ]; then
  ssh-keygen -t ed25519 -N '' -f /root/.ssh/id_ed25519 -C "root@$(uname -n) nix-builder" >/dev/null
  echo "[nixos-manager] generated /root/.ssh/id_ed25519"
else
  echo "[nixos-manager] /root/.ssh/id_ed25519 already exists"
fi
echo "[nixos-manager] public key:"
echo "PUBKEY $(cat /root/.ssh/id_ed25519.pub)"
"#,
    );
    if let Some(h) = builder_host.as_deref().filter(|h| !h.trim().is_empty()) {
        let host = h.split('@').next_back().unwrap_or(h).split(':').next().unwrap_or(h);
        script.push_str(&format!(
            "touch /root/.ssh/known_hosts\nif ssh-keyscan -T 8 {h} 2>/dev/null | grep -v '^#' > /tmp/nm-keyscan; then\n  while read -r line; do grep -qF \"$line\" /root/.ssh/known_hosts || echo \"$line\" >> /root/.ssh/known_hosts; done < /tmp/nm-keyscan\n  rm -f /tmp/nm-keyscan\n  echo \"[nixos-manager] host key of {h} added to /root/.ssh/known_hosts\"\nelse\n  echo \"[nixos-manager] warning: could not reach {h} to record its host key\"\nfi\n",
            h = sh(host)
        ));
    }
    match run_root_script(&job, &password, &script, None).await {
        Ok(0) => job.succeeded().await,
        Ok(code) => job.failed(format!("exit status {code}")).await,
        Err(e) => job.failed(e.to_string()).await,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn remote_script_shape() {
        let req = DeployRequest {
            path: "/home/u/nixos".into(),
            host: Some("main".into()),
            action: RebuildAction::Switch,
            upgrade: false,
            keep_configuration: None,
            hardware: None,
            skip_sync: false,
            password: String::new(),
            secrets_password: None,
            target: None,
            target_sudo_password: None,
            build_host: None,
            after: None,
            after_even_on_failure: false,
        };
        let t = DeployTarget { name: "server".into(), ssh_target: "vincent@cyber-server-1".into(), flake_host: Some("main".into()), build_host: None, use_remote_sudo: true };
        let s = remote_script(&req, &t, Path::new("/tmp/staged"), ConfigKind::Flake, "--sudo", Path::new("/tmp/nm-stop"));
        assert!(s.contains("nixos-rebuild switch --flake 'path:/tmp/staged#main' --target-host 'vincent@cyber-server-1' --sudo"), "{s}");
        assert!(s.contains("--ask-sudo-password 2>&1'") && s.contains("setsid bash -c \"$NM_CMD\" &"), "password path must run in its own session: {s}");
        assert!(s.contains("BatchMode=yes"));
        assert!(!s.contains("rm -rf"));
        assert!(!s.contains("bootstrap"));
        let s2 = remote_script(&req, &t, Path::new("/w"), ConfigKind::Legacy, "--use-remote-sudo", Path::new("/tmp/nm-stop"));
        assert!(s2.contains("--no-flake -I 'nixos-config=/w/configuration.nix' --target-host"), "{s2}");
        let req_after = DeployRequest { after: Some(crate::types::AfterBuild::Reboot), ..req.clone() };
        let sa = remote_script(&req_after, &t, Path::new("/tmp/staged"), ConfigKind::Flake, "--sudo", Path::new("/tmp/nm-stop"));
        assert!(sa.find("nm_after finished").unwrap() > sa.find("finished successfully").unwrap(), "{sa}");
        assert!(sa.contains("shutdown -r now") && !sa.contains("trap 'NM_RC"), "{sa}");
        let req_always = DeployRequest { after_even_on_failure: true, ..req_after.clone() };
        let sb = remote_script(&req_always, &t, Path::new("/tmp/staged"), ConfigKind::Flake, "--sudo", Path::new("/tmp/nm-stop"));
        assert!(sb.contains("trap 'NM_RC=$?"), "{sb}");
        assert!(!s.contains("shutdown"));
        // root targets skip the trust step entirely
        let root_t = DeployTarget { ssh_target: "root@h".into(), use_remote_sudo: false, ..t.clone() };
        let sr = remote_script(&req, &root_t, Path::new("/w"), ConfigKind::Flake, "--sudo", Path::new("/tmp/nm-stop"));
        assert!(!sr.contains("temporary"), "{sr}");
        let s3 = remote_script(&req, &t, Path::new("/tmp/staged"), ConfigKind::Flake, "--sudo", Path::new("/tmp/nm-stop"));
        let grant = s3.find("checking whether the user is trusted").unwrap();
        let rebuild = s3.find("nixos-rebuild switch").unwrap();
        let revoke = s3.find("removing the temporary nix trust").unwrap();
        assert!(grant < rebuild && rebuild < revoke, "{s3}");
        assert!(s3.contains("extra-trusted-users = vincent"));
        assert!(s3.contains("if [ \"$RC\" -ne 0 ]; then exit \"$RC\"; fi"));
        assert!(s3.contains("wait_idle"), "must wait for an idle daemon");
        assert!(s3.contains("Trusted: 1"), "must prefer nix's own trust answer");
        assert!(s3.contains("reverting"), "must revert on failure");
        assert!(s3.contains("NM_TRUST_GRANTED"));
        assert!(s3.contains(") || NM_RC=$?"), "grant failure must be captured despite set -e");
        assert!(valid_unix_user("vincent") && valid_unix_user("_svc-1") && !valid_unix_user("Bad User") && !valid_unix_user("a;rm") && !valid_unix_user(""));
    }

    /// Every generated script must at least parse (`bash -n`): the local
    /// wrapper with heredocs, and both remote scripts on their own.
    #[test]
    fn trust_check_reads_nix_stderr() {
        // `nix store info` prints "Trusted: 1" via notice(), i.e. on stderr;
        // discarding stderr made the grant step always conclude "not trusted".
        assert!(!REMOTE_IS_TRUSTED_FN.contains("nix store info 2>/dev/null"));
        let dir = std::env::temp_dir().join(format!("nm-nix-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let fake = dir.join("nix");
        std::fs::write(&fake, "#!/usr/bin/env bash\necho 'Store URL: daemon' >&2; echo 'Trusted: 1' >&2\n").unwrap();
        std::fs::set_permissions(&fake, std::os::unix::fs::PermissionsExt::from_mode(0o755)).unwrap();
        let script = format!("{REMOTE_IS_TRUSTED_FN}\nif is_trusted; then echo YES; else echo NO; fi\n");
        let out = std::process::Command::new("bash")
            .arg("-c").arg(&script)
            .env("PATH", format!("{}:{}", dir.display(), std::env::var("PATH").unwrap_or_default()))
            .output().unwrap();
        let _ = std::fs::remove_dir_all(&dir);
        let text = String::from_utf8_lossy(&out.stdout).to_string();
        assert!(text.contains("YES"), "{text}");
    }

    #[test]
    fn idle_check_survives_pgrep_zero_exit_one() {
        // real `pgrep -c` prints "0" AND exits 1 when nothing matches; the old
        // `|| echo 0` fallback produced "0\n0" and the grant always timed out.
        assert!(!REMOTE_IS_TRUSTED_FN.contains("|| echo 0"));
        let dir = std::env::temp_dir().join(format!("nm-pgrep-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let fake = dir.join("pgrep");
        std::fs::write(&fake, "#!/usr/bin/env bash\necho 0; exit 1\n").unwrap();
        std::fs::set_permissions(&fake, std::os::unix::fs::PermissionsExt::from_mode(0o755)).unwrap();
        let script = format!("{REMOTE_IS_TRUSTED_FN}\ndaemon_idle && echo IDLE\n");
        let out = std::process::Command::new("bash")
            .arg("-c").arg(&script)
            .env("PATH", format!("{}:{}", dir.display(), std::env::var("PATH").unwrap_or_default()))
            .output().unwrap();
        let _ = std::fs::remove_dir_all(&dir);
        let text = String::from_utf8_lossy(&out.stdout).to_string() + &String::from_utf8_lossy(&out.stderr);
        assert!(text.contains("IDLE"), "{text}");
        assert!(!text.contains("integer expected"), "{text}");
    }

    #[test]
    fn target_hardware_script_parses() {
        let st = std::process::Command::new("bash").arg("-n").arg("-c").arg(target_hardware_script()).status().unwrap();
        assert!(st.success());
    }

    #[test]
    fn generated_scripts_parse() {
        let check = |label: &str, text: &str| {
            let dir = tempdir::TempDir::new("nm-script").unwrap();
            let f = dir.path().join("s.sh");
            std::fs::write(&f, text).unwrap();
            let out = std::process::Command::new("bash").arg("-n").arg(&f).output().expect("bash");
            assert!(out.status.success(), "{label} does not parse:\n{}\n---\n{text}", String::from_utf8_lossy(&out.stderr));
        };
        let req = DeployRequest { path: "/w".into(), host: Some("main".into()), action: RebuildAction::Switch, upgrade: true, keep_configuration: None, hardware: None, skip_sync: false, password: String::new(), secrets_password: None, target: None, target_sudo_password: None, build_host: None, after: None, after_even_on_failure: false };
        let t = DeployTarget { name: "x".into(), ssh_target: "vincent@h".into(), flake_host: None, build_host: Some("b".into()), use_remote_sudo: true };
        check("local deploy script", &remote_script(&req, &t, Path::new("/tmp/st"), ConfigKind::Flake, "--sudo", Path::new("/tmp/nm-stop")));
        check("local deploy script (legacy)", &remote_script(&req, &t, Path::new("/tmp/st"), ConfigKind::Legacy, "--use-remote-sudo", Path::new("/tmp/nm-stop")));
        check("remote grant script", &remote_grant_script("vincent"));
        check("remote revoke script", &remote_revoke_script("vincent"));
        // The inner `sudo -n sh -c '...'` bodies must parse as sh too.
        for script in [remote_grant_script("vincent"), remote_revoke_script("vincent")] {
            for line in script.lines().filter(|l| l.starts_with("sudo -n sh -c '")) {
                let body = line.trim_start_matches("sudo -n sh -c '");
                let end = body.rfind('\'').unwrap();
                let body = body[..end].replace("'\"'\"'", "'");
                check("inner sh -c body", &body);
            }
        }
    }
}

// ───────────────────────── key access setup ─────────────────────────

pub const APP_KEY_NAME: &str = "nixos-manager_ed25519";

fn home() -> std::path::PathBuf {
    directories::BaseDirs::new().map(|b| b.home_dir().to_path_buf()).unwrap_or_else(|| std::path::PathBuf::from("/root"))
}

pub fn app_key_path() -> std::path::PathBuf {
    home().join(".ssh").join(APP_KEY_NAME)
}

/// Hosts with a "# nixos-manager target: <host>" block in ~/.ssh/config.
pub fn managed_hosts() -> Vec<String> {
    std::fs::read_to_string(home().join(".ssh").join("config"))
        .map(|c| c.lines().filter_map(|l| l.strip_prefix("# nixos-manager target: ")).map(|h| h.trim().to_string()).collect())
        .unwrap_or_default()
}

/// "user@host" → ("host", Some("user")). A bare host is allowed.
pub(crate) fn split_target(ssh_target: &str) -> (String, Option<String>) {
    match ssh_target.split_once('@') {
        Some((u, h)) => (h.trim().to_string(), Some(u.trim().to_string())),
        None => (ssh_target.trim().to_string(), None),
    }
}

/// Shell script run as the user: see the doc of [`setup_key_access`].
/// The password reaches ssh through an askpass helper that prints the
/// `NM_SSH_PASSWORD` environment variable, so it never lands in a file or
/// on a command line.
fn key_access_script(ssh_target: &str, key_path: &str, target_name: &str) -> String {
    let (host, _user) = split_target(ssh_target);
    format!(
        r##"set -uo pipefail
# NM_SSH_OPTS: extra ssh options (used by tests to point at a throwaway sshd)
KEY={key}
PUB="$KEY.pub"
TARGET={t}
HOST={h}
SSH_DIR="$HOME/.ssh"
mkdir -p "$SSH_DIR" && chmod 700 "$SSH_DIR"

# 1. A key dedicated to this app (never touches your other keys).
if [ ! -f "$KEY" ]; then
  ssh-keygen -q -t ed25519 -N '' -f "$KEY" -C "nixos-manager@$(uname -n)"
  echo "[nixos-manager] created $KEY"
else
  echo "[nixos-manager] using existing $KEY"
fi
PUBKEY="$(cat "$PUB")"
echo "PUBKEY $PUBKEY"

# 2. Does the key already open the target?
if ssh ${{NM_SSH_OPTS:-}} -i "$KEY" -o IdentitiesOnly=yes -o BatchMode=yes -o ConnectTimeout=15 -o StrictHostKeyChecking=accept-new "$TARGET" true 2>/tmp/nm-ssh-err; then
  echo "[nixos-manager] the key already opens $TARGET; nothing to install"
  INSTALLED=1
else
  ERR="$(cat /tmp/nm-ssh-err)"
  if echo "$ERR" | grep -qi "REMOTE HOST IDENTIFICATION HAS CHANGED"; then
    echo "[nixos-manager] ERROR: the host key of $HOST changed since it was last seen. If the machine was reinstalled, run: ssh-keygen -R $HOST  and try again." >&2
    exit 4
  fi
  if echo "$ERR" | grep -qi "Connection refused\|Could not resolve\|No route to host\|timed out\|Connection timed out"; then
    echo "[nixos-manager] ERROR: cannot reach $HOST over SSH: $(echo "$ERR" | tail -1)" >&2
    exit 5
  fi
  INSTALLED=0
fi

# 3. Install the public key with a one-time password login.
if [ "$INSTALLED" = 0 ]; then
  ASKPASS="$(mktemp)"
  printf '%s\n' '#!/bin/sh' 'printf "%s\n" "$NM_SSH_PASSWORD"' > "$ASKPASS"
  chmod 700 "$ASKPASS"
  export SSH_ASKPASS="$ASKPASS" SSH_ASKPASS_REQUIRE=force
  echo "[nixos-manager] logging in to $TARGET with the password to install the key..."
  REMOTE='umask 077; mkdir -p ~/.ssh; touch ~/.ssh/authorized_keys; if grep -qxF "'"$PUBKEY"'" ~/.ssh/authorized_keys; then echo "already-present"; else echo "'"$PUBKEY"'" >> ~/.ssh/authorized_keys; echo "added"; fi'
  OUT="$(ssh ${{NM_SSH_OPTS:-}} -o PreferredAuthentications=password,keyboard-interactive -o PubkeyAuthentication=no -o NumberOfPasswordPrompts=1 -o ConnectTimeout=15 -o StrictHostKeyChecking=accept-new "$TARGET" "$REMOTE" 2>/tmp/nm-ssh-err)"
  RC=$?
  rm -f "$ASKPASS"
  if [ $RC -ne 0 ]; then
    ERR="$(cat /tmp/nm-ssh-err)"
    if echo "$ERR" | grep -qi "Permission denied"; then
      if echo "$ERR" | grep -q "password"; then
        echo "[nixos-manager] ERROR: the target rejected the password." >&2
      else
        echo "[nixos-manager] ERROR: the target does not accept password logins (publickey only). Add this public key to ~/.ssh/authorized_keys of the user on the target by other means:" >&2
        echo "$PUBKEY" >&2
      fi
      exit 6
    fi
    echo "[nixos-manager] ERROR: ssh failed: $(echo "$ERR" | tail -2 | tr '\n' ' ')" >&2
    exit 7
  fi
  case "$OUT" in
    *already-present*) echo "[nixos-manager] the public key was already in authorized_keys on the target" ;;
    *) echo "[nixos-manager] public key added to authorized_keys on the target" ;;
  esac
fi

# 4. Make deployments use this key for the host (one managed block in ~/.ssh/config).
CFG="$SSH_DIR/config"
MARK="# nixos-manager target: $HOST"
touch "$CFG" && chmod 600 "$CFG"
if ! grep -qxF "$MARK" "$CFG"; then
  printf '\n%s\nHost %s\n  IdentityFile %s\n  IdentitiesOnly yes\n' "$MARK" "$HOST" "$KEY" >> "$CFG"
  echo "[nixos-manager] added a Host block for $HOST to ~/.ssh/config"
else
  echo "[nixos-manager] ~/.ssh/config already has a block for $HOST"
fi

# 5. Verify key login and sudo.
if ssh ${{NM_SSH_OPTS:-}} -o BatchMode=yes -o ConnectTimeout=15 "$TARGET" 'echo "[nixos-manager] key login works as $(id -un) on $(uname -n)"; if sudo -n true 2>/dev/null; then echo "[nixos-manager] passwordless sudo: yes"; else echo "[nixos-manager] passwordless sudo: NO - deployments need it unless the user is root (set security.sudo.wheelNeedsPassword = false on the target)"; fi'; then
  echo "[nixos-manager] key access to $TARGET is ready ({name})"
else
  echo "[nixos-manager] ERROR: the key was installed but a key login still fails; check the target's sshd settings (PubkeyAuthentication) and ~/.ssh permissions there." >&2
  exit 8
fi
"##,
        key = sh(key_path),
        t = sh(ssh_target),
        h = sh(&host),
        name = target_name
    )
}

/// Install this app's SSH key on a deploy target using a one-time password.
pub async fn setup_key_access(job: JobHandle, target: DeployTarget, password: String) {
    job.running().await;
    let key_path = home().join(".ssh").join(APP_KEY_NAME).to_string_lossy().to_string();
    let script = key_access_script(&target.ssh_target, &key_path, &target.name);
    let sender = job.line_sender();
    let mut cmd = tokio::process::Command::new("bash");
    cmd.arg("-c").arg(&script).env("NM_SSH_PASSWORD", &password);
    let result = process::stream_cmd(cmd, move |is_err, line| {
        let _ = sender.send(LogLine { stream: if is_err { LogStream::Stderr } else { LogStream::Stdout }, text: line });
    })
    .await;
    match result {
        Ok(0) => job.succeeded().await,
        Ok(4) => job.failed("host key changed; see the log").await,
        Ok(5) => job.failed("target unreachable").await,
        Ok(6) => job.failed("login refused; see the log").await,
        Ok(code) => job.failed(format!("setup failed (exit {code}); see the log")).await,
        Err(e) => job.failed(e.to_string()).await,
    }
}
