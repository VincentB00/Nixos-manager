//! Sync the working copy to /etc/nixos (replicating Vincent's sync.sh step by
//! step) and run nixos-rebuild there, as one sudo job with a password prompted
//! by the UI. See .claude/memories/04-architecture.md "Deploy flow".

use crate::jobs::JobHandle;
use crate::nix;
use crate::process;
use crate::types::{ConfigConflict, ConfigKind, DeployPreflight, DeployRequest, DeployTarget, HardwareChoice, KeepConfiguration, LogLine, LogStream, RebuildAction};
use crate::{CoreError, Result};
use std::path::{Path, PathBuf};
use std::time::Duration;

pub const ETC_NIXOS: &str = "/etc/nixos";

pub async fn preflight(workdir: &Path, pull: Option<&crate::types::HardwarePull>) -> Result<DeployPreflight> {
    let loc = crate::discover::location_info(workdir)?;
    let etc = Path::new(ETC_NIXOS);
    let etc_exists = etc.is_dir();
    let etc_readable = etc_exists && std::fs::read_dir(etc).is_ok();
    let local_cfg = workdir.join("configuration.nix");
    let system_cfg = etc.join("configuration.nix");
    let configuration_conflict = match (std::fs::read_to_string(&system_cfg), std::fs::read_to_string(&local_cfg)) {
        (Ok(sys), Ok(local)) if sys != local => {
            let diff = similar::TextDiff::from_lines(&sys, &local)
                .unified_diff()
                .context_radius(3)
                .header("/etc/nixos/configuration.nix", "working copy configuration.nix")
                .to_string();
            Some(ConfigConflict { system_source: sys, local_source: local, unified_diff: diff })
        }
        _ => None,
    };
    let machine = nix::machine_info().await;
    Ok(DeployPreflight {
        etc_nixos_exists: etc_exists,
        etc_nixos_readable: etc_readable,
        source_is_etc_nixos: loc.is_etc_nixos,
        etc_has_flake_lock: etc.join("flake.lock").is_file(),
        etc_has_hardware_config: etc.join("hardware-configuration.nix").is_file(),
        local_has_hardware_config: local_cfg.parent().map(|p| p.join("hardware-configuration.nix").is_file()).unwrap_or(false),
        hardware: crate::hardware::status(workdir, pull),
        configuration_conflict,
        hosts: loc.hosts,
        current_hostname: machine.hostname,
        kind: loc.kind,
        secrets: crate::secrets::deploy_info(workdir),
        ssh_agent: std::env::var("SSH_AUTH_SOCK").is_ok(),
        app_key_exists: crate::remote::app_key_path().is_file(),
        app_key_hosts: crate::remote::managed_hosts(),
        targets: vec![],
    })
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

/// Root-script fragment for "build there, activate here": root must reach
/// the builder over SSH with the user's key and known hosts, the user must be
/// trusted by nix on the builder (temporary trust otherwise, password read
/// from stdin after the sudo password), and the rebuild runs under `set +e`
/// so the trust can be revoked afterwards (the caller closes that).
pub fn build_host_block(t: &DeployTarget, app_key: &Path) -> String {
    let home = std::env::var("HOME").unwrap_or_else(|_| "/root".into());
    let mut opts = format!("-F {h}/.ssh/config -o UserKnownHostsFile={h}/.ssh/known_hosts", h = sh_quote(&home).trim_matches('\'').to_string());
    if app_key.is_file() {
        opts.push_str(&format!(" -i {}", app_key.display()));
    }
    let mut s = String::new();
    s.push_str("# builder sudo password: the line after the sudo password on stdin (never argv, env or a file)\nNM_TARGET_PW=\"\"\n");
    s.push_str("while IFS= read -r NM_L; do case \"$NM_L\" in NM_TARGET_PW=*) NM_TARGET_PW=${NM_L#NM_TARGET_PW=}; break;; esac; done\n");
    s.push_str(&format!("NM_SSH_OPTS={}\nexport NIX_SSHOPTS=\"$NM_SSH_OPTS -o BatchMode=yes -o ConnectTimeout=20\"\n", sh_quote(&opts)));
    s.push_str(&format!("echo \"[nixos-manager] compiling on {} (evaluated here, built there, result copied back and activated on this machine)\"\n", t.ssh_target));
    s.push_str(&crate::remote::trust_snippet(t, true));
    s.push_str("set +e\n");
    s
}

fn sh_quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', "'\\''"))
}

/// The root-side script. Mirrors sync.sh:
///   guard (not inside /etc/nixos) → stash flake.lock (unless upgrade) →
///   copy hardware-configuration.nix system→workdir → configuration.nix
///   conflict choice → rm -rf /etc/nixos/* → cp -r workdir/* /etc/nixos/ →
///   drop or restore flake.lock → nixos-rebuild.
fn build_script(req: &DeployRequest, workdir: &Path, kind: ConfigKind, invoking_uid: u32, invoking_gid: u32) -> String {
    let wd = sh_quote(&workdir.to_string_lossy());
    let action = action_arg(req.action);
    let mut s = String::new();
    s.push_str("set -euo pipefail\n");
    s.push_str(&format!("NIXOS_DIR={ETC_NIXOS}\nWORKDIR={wd}\n"));
    s.push_str("echo \"[nixos-manager] running as $(id -un) for workdir $WORKDIR\"\n");

    if !req.skip_sync {
        s.push_str(r#"
if [[ "$(realpath "$WORKDIR")" == "$(realpath "$NIXOS_DIR")" ]]; then
  echo "Error: the working copy is $NIXOS_DIR itself; refusing to sync." >&2
  exit 2
fi
mkdir -p "$NIXOS_DIR"
LOCK_TMP="$(mktemp /tmp/flake.lock.XXXXXX)"
HAVE_LOCK=0
"#);
        if req.upgrade {
            s.push_str("echo \"--upgrade set: flake.lock will not be preserved.\"\n");
        } else {
            s.push_str(r#"if [[ -f "$NIXOS_DIR/flake.lock" ]]; then
  echo "Preserving $NIXOS_DIR/flake.lock"
  cp "$NIXOS_DIR/flake.lock" "$LOCK_TMP"
  HAVE_LOCK=1
fi
"#);
        }
        let hw_rel = sh_quote(&crate::hardware::rel_path_in(workdir).to_string_lossy());
        s.push_str(&format!("HW_REL={hw_rel}\n"));
        if req.hardware == Some(HardwareChoice::Workspace) {
            s.push_str(r#"echo "Applying the working copy's hardware-configuration.nix ($HW_REL) as confirmed; /etc/nixos's copy is NOT used."
"#);
        } else {
            s.push_str(&format!(r#"
if [[ -f "$NIXOS_DIR/hardware-configuration.nix" ]]; then
  if [[ -f "$WORKDIR/$HW_REL" ]] && ! diff -q "$NIXOS_DIR/hardware-configuration.nix" "$WORKDIR/$HW_REL" &>/dev/null; then
    echo "hardware-configuration.nix: the working copy's version differed; the system copy (source of truth) replaces it."
  fi
  mkdir -p "$(dirname "$WORKDIR/$HW_REL")"
  cp "$NIXOS_DIR/hardware-configuration.nix" "$WORKDIR/$HW_REL"
  chown {invoking_uid}:{invoking_gid} "$WORKDIR/$HW_REL" || true
  echo "Copied hardware-configuration.nix into the working copy ($HW_REL)."
else
  echo "Note: $NIXOS_DIR/hardware-configuration.nix not found; nothing copied."
fi
"#));
        }
        match req.keep_configuration {
            Some(KeepConfiguration::System) => s.push_str(r#"
if [[ -f "$NIXOS_DIR/configuration.nix" ]] && ! diff -q "$NIXOS_DIR/configuration.nix" "$WORKDIR/configuration.nix" &>/dev/null; then
  cp "$NIXOS_DIR/configuration.nix" "$WORKDIR/configuration.nix"
  echo "Conflict: kept the system configuration.nix (copied into the working copy)."
fi
"#),
            Some(KeepConfiguration::Local) => s.push_str("echo \"Conflict: keeping the working copy configuration.nix.\"\n"),
            None => s.push_str(r#"
if [[ -f "$NIXOS_DIR/configuration.nix" && -f "$WORKDIR/configuration.nix" ]] && ! diff -q "$NIXOS_DIR/configuration.nix" "$WORKDIR/configuration.nix" &>/dev/null; then
  echo "Error: configuration.nix differs between $NIXOS_DIR and the working copy; choose which to keep." >&2
  exit 3
fi
"#),
        }
        s.push_str(r#"
echo "Clearing $NIXOS_DIR..."
rm -rf "${NIXOS_DIR:?}"/*
echo "Copying working copy to $NIXOS_DIR..."
cp -r "$WORKDIR"/* "$NIXOS_DIR/"
"#);
        if req.upgrade {
            s.push_str("rm -f \"$NIXOS_DIR/flake.lock\"\necho \"Removed $NIXOS_DIR/flake.lock for upgrade.\"\n");
        } else {
            s.push_str(r#"if [[ "$HAVE_LOCK" == 1 ]]; then
  cp "$LOCK_TMP" "$NIXOS_DIR/flake.lock"
  echo "Restored flake.lock."
fi
"#);
        }
        s.push_str("rm -f \"$LOCK_TMP\"\necho \"Sync complete.\"\n");
    } else {
        s.push_str("echo \"Skipping sync; rebuilding from $NIXOS_DIR as-is.\"\n");
    }

    // nixos-rebuild's stderr is progress, not errors, and its final message is
    // split across stderr ("Done. The new configuration is ") and stdout (the
    // path); merging the streams keeps the log in order and in one colour.
    s.push_str(&format!("cd \"$NIXOS_DIR\"\necho \"Running nixos-rebuild {action}...\"\n"));
    let build_host_flag = match &req.build_host {
        Some(t) => {
            s.push_str(&build_host_block(t, &crate::remote::app_key_path()));
            format!(" --build-host {}", sh_quote(&t.ssh_target))
        }
        None => String::new(),
    };
    match kind {
        ConfigKind::Flake => {
            let host = req.host.clone().unwrap_or_default();
            if host.is_empty() {
                s.push_str(&format!("nixos-rebuild {action} --flake \"$NIXOS_DIR\"{build_host_flag} 2>&1\n"));
            } else {
                s.push_str(&format!("nixos-rebuild {action} --flake \"$NIXOS_DIR#{}\"{build_host_flag} 2>&1\n", host.replace('"', "")));
            }
        }
        _ => s.push_str(&format!("nixos-rebuild {action}{build_host_flag} 2>&1\n")),
    }
    if req.build_host.is_some() {
        // the nixos-rebuild line above ran under `set +e` (see build_host_block)
        s.push_str("RC=$?\nset -e\n");
        s.push_str(&crate::remote::trust_snippet(req.build_host.as_ref().unwrap(), false));
        s.push_str("if [ \"$RC\" -ne 0 ]; then exit \"$RC\"; fi\n");
    }
    s.push_str(&format!("echo \"[nixos-manager] nixos-rebuild {action} finished successfully.\"\n"));
    s
}

/// Validate the sudo password without running anything.
async fn check_sudo(password: &str) -> Result<()> {
    let mut lines: Vec<String> = Vec::new();
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<String>();
    let code = process::stream(
        "sudo",
        &["-S", "-k", "-p", "", "-v"],
        None,
        Some(format!("{password}\n").into_bytes()),
        move |_, l| {
            let _ = tx.send(l);
        },
    )
    .await?;
    while let Ok(l) = rx.try_recv() {
        lines.push(l);
    }
    if code != 0 {
        let joined = lines.join(" ").to_lowercase();
        if joined.contains("try again") || joined.contains("incorrect") || joined.contains("sorry") {
            return Err(CoreError::Forbidden("sudo rejected the password".into()));
        }
        return Err(CoreError::Forbidden(format!("sudo validation failed (exit {code}): {}", lines.join(" "))));
    }
    Ok(())
}

/// Run the whole deploy as a job. Resolves when the job has finished.
pub async fn run(job: JobHandle, req: DeployRequest) {
    if let Some(target) = req.target.clone() {
        return crate::remote::run_remote(job, req, target).await;
    }
    let workdir = PathBuf::from(&req.path);
    job.running().await;
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
    if let Some(b) = &req.build_host {
        let (_, user) = crate::remote::split_target(&b.ssh_target);
        let user = user.unwrap_or_else(|| "root".into());
        if !crate::remote::valid_unix_user(&user) {
            job.failed(format!("'{user}' is not a plain unix user name; use user@host for the builder")).await;
            return;
        }
        job.system(format!("Build on {}: the system is evaluated here, compiled on the builder, copied back and activated on this machine.", b.ssh_target)).await;
    }
    job.system(format!("Validating sudo password for {}...", req.path)).await;
    if let Err(e) = check_sudo(&req.password).await {
        job.failed(e.to_string()).await;
        return;
    }
    job.system("Password accepted.").await;

    // Managed secrets: decrypt as the user into a private temp dir; the root
    // script installs them before nixos-rebuild.
    let mut secrets_prelude = String::new();
    if let Some(pw) = req.secrets_password.as_deref().filter(|p| !p.is_empty()) {
        match crate::secrets::stage_for_deploy(&workdir, pw) {
            Ok((tmp, hash)) => {
                job.system("Secrets password accepted; managed secrets will be installed.").await;
                secrets_prelude = crate::secrets::install_script(&tmp, &hash);
            }
            Err(e) => {
                job.failed(format!("managed secrets: {e}")).await;
                return;
            }
        }
    } else if crate::secrets::deploy_info(&workdir).present && !crate::secrets::deploy_info(&workdir).deployed_up_to_date {
        job.system("Note: secrets.age changed since the last deploy but no secrets password was given; keeping previously installed secrets.").await;
    }
    let uid = unsafe_uid();
    let mut script = build_script(&req, &workdir, kind, uid.0, uid.1);
    if !secrets_prelude.is_empty() {
        let marker = "cd \"$NIXOS_DIR\"";
        script = script.replacen(marker, &format!("{secrets_prelude}{marker}"), 1);
    }
    let extra_stdin = req.build_host.as_ref().map(|_| format!("NM_TARGET_PW={}\n", req.target_sudo_password.clone().unwrap_or_default()));
    let seen = std::sync::Arc::new(std::sync::Mutex::new(Vec::<String>::new()));
    let result = run_root_script_inner(&job, &req.password, &script, Some(&workdir), extra_stdin, Some(seen.clone())).await;
    if let Some(b) = &req.build_host {
        let lines = seen.lock().unwrap().clone();
        let (_, user) = crate::remote::split_target(&b.ssh_target);
        let user = user.unwrap_or_else(|| "root".into());
        if lines.iter().any(|l| l.contains("lacks a signature by a trusted key")) {
            job.system(format!("Why it failed: the builder's nix daemon only accepts unsigned paths from trusted users and '{user}' is not one there; the temporary trust step above should have handled this, check its messages. Permanent fix on the builder:  nix.settings.trusted-users = [ \"{user}\" ];  (wizard \"Trust a user for nix\").")).await;
        } else if lines.iter().any(|l| l.contains("Permission denied") || l.contains("Host key verification failed")) {
            job.system("Why it failed: SSH to the builder was refused as root on this machine. Use \"Set up key access\" and \"Test connection\" on the Remote page for this target first (root reuses your key and known hosts).").await;
        }
    }
    match result {
        Ok(0) => job.succeeded().await,
        Ok(9) => job.failed("could not grant temporary trust on the builder; see the log").await,
        Ok(2) => job.failed("refused: the working copy is /etc/nixos itself").await,
        Ok(3) => job.failed("configuration.nix conflict: choose which version to keep and retry").await,
        Ok(code) => job.failed(format!("nixos-rebuild/sync exited with status {code}")).await,
        Err(e) => job.failed(e.to_string()).await,
    }
}

/// Validate the password, then run `script` as root with `sudo -S bash`,
/// streaming output into the job. Returns the exit status.
pub async fn run_root_script(job: &JobHandle, password: &str, script: &str, cwd: Option<&Path>) -> Result<i32> {
    job.system("Validating sudo password...").await;
    check_sudo(password).await?;
    job.system("Password accepted.").await;
    run_root_script_streamed(job, password, script, cwd).await
}

async fn run_root_script_streamed(job: &JobHandle, password: &str, script: &str, cwd: Option<&Path>) -> Result<i32> {
    run_root_script_inner(job, password, script, cwd, None, None).await
}

/// `extra_stdin` is written after the sudo password line (sudo -S reads one
/// line; the script can read the rest). `watch` collects lines mentioning
/// known failure signatures for the "Why it failed" hints.
async fn run_root_script_inner(
    job: &JobHandle,
    password: &str,
    script: &str,
    cwd: Option<&Path>,
    extra_stdin: Option<String>,
    watch: Option<std::sync::Arc<std::sync::Mutex<Vec<String>>>>,
) -> Result<i32> {
    let script_path = write_script(script).map_err(|e| CoreError::Other(format!("cannot write script: {e}")))?;
    let sender = job.line_sender();
    let script_str = script_path.to_string_lossy().to_string();
    let args_ref: Vec<&str> = vec!["-S", "-p", "", "bash", script_str.as_str()];
    let result = process::stream(
        "sudo",
        &args_ref,
        cwd,
        Some(format!("{password}\n{}", extra_stdin.unwrap_or_default()).into_bytes()),
        move |is_err, line| {
            if let Some(w) = &watch {
                if line.contains("lacks a signature by a trusted key") || line.contains("Permission denied") || line.contains("Host key verification failed") {
                    w.lock().unwrap().push(line.clone());
                }
            }
            let _ = sender.send(LogLine { stream: if is_err { LogStream::Stderr } else { LogStream::Stdout }, text: line });
        },
    )
    .await;
    let _ = std::fs::remove_file(&script_path);
    result
}

fn write_script(content: &str) -> std::io::Result<PathBuf> {
    use std::io::Write;
    use std::os::unix::fs::OpenOptionsExt;
    let dir = std::env::temp_dir();
    let path = dir.join(format!("nixos-manager-deploy-{}.sh", uuid::Uuid::new_v4()));
    let mut f = std::fs::OpenOptions::new().write(true).create_new(true).mode(0o700).open(&path)?;
    f.write_all(content.as_bytes())?;
    Ok(path)
}

fn unsafe_uid() -> (u32, u32) {
    // Read from /proc to avoid a libc dependency.
    let status = std::fs::read_to_string("/proc/self/status").unwrap_or_default();
    let mut uid = 1000;
    let mut gid = 100;
    for line in status.lines() {
        if let Some(rest) = line.strip_prefix("Uid:") {
            uid = rest.split_whitespace().next().and_then(|s| s.parse().ok()).unwrap_or(uid);
        }
        if let Some(rest) = line.strip_prefix("Gid:") {
            gid = rest.split_whitespace().next().and_then(|s| s.parse().ok()).unwrap_or(gid);
        }
    }
    (uid, gid)
}

/// Quick, unprivileged syntax/eval check of the working copy: `nix flake check`
/// is too slow, so we only run `nix-instantiate --parse` on every .nix file.
pub async fn parse_check(workdir: &Path) -> Vec<(String, String)> {
    let mut problems = Vec::new();
    for (rel, _) in crate::discover::list_files(workdir) {
        if !rel.ends_with(".nix") {
            continue;
        }
        let full = workdir.join(&rel);
        if let Ok(out) = process::run("nix-instantiate", &["--parse", &full.to_string_lossy()], Some(workdir), Duration::from_secs(20)).await {
            if out.status != 0 {
                problems.push((rel, out.stderr.trim().to_string()));
            }
        }
    }
    problems
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn script_mirrors_sync_sh() {
        let req = DeployRequest {
            path: "/home/u/nixos".into(),
            host: Some("main".into()),
            action: RebuildAction::Switch,
            upgrade: false,
            keep_configuration: Some(KeepConfiguration::Local),
            hardware: None,
            build_host: None,
            skip_sync: false,
            password: "x".into(),
            secrets_password: None,
            target: None,
            target_sudo_password: None,
        };
        let s = build_script(&req, Path::new("/home/u/nixos"), ConfigKind::Flake, 1000, 100);
        assert!(s.contains("refusing to sync"));
        assert!(s.contains("Preserving $NIXOS_DIR/flake.lock"));
        assert!(s.contains("cp \"$NIXOS_DIR/hardware-configuration.nix\" \"$WORKDIR/$HW_REL\""), "{s}");
        assert!(s.contains("HW_REL='hardware-configuration.nix'"), "{s}");
        let req_ws = DeployRequest { hardware: Some(HardwareChoice::Workspace), ..req.clone() };
        let s_ws = build_script(&req_ws, Path::new("/home/u/nixos"), ConfigKind::Flake, 1000, 100);
        assert!(!s_ws.contains("cp \"$NIXOS_DIR/hardware-configuration.nix\""), "workspace choice must not copy the system file: {s_ws}");
        assert!(s_ws.contains("as confirmed"));
        assert!(s.contains("rm -rf \"${NIXOS_DIR:?}\"/*"));
        assert!(s.contains("cp -r \"$WORKDIR\"/* \"$NIXOS_DIR/\""));
        assert!(s.contains("Restored flake.lock"));
        assert!(s.contains("nixos-rebuild switch --flake \"$NIXOS_DIR#main\""));
        assert!(!s.contains("password"));
    }

    #[test]
    fn build_host_script_shape() {
        let t = DeployTarget { name: "laptop".into(), ssh_target: "vincent@lg".into(), flake_host: None, build_host: None, use_remote_sudo: true };
        let req = DeployRequest {
            path: "/home/u/nixos".into(),
            host: Some("main".into()),
            action: RebuildAction::Switch,
            upgrade: false,
            keep_configuration: Some(KeepConfiguration::Local),
            hardware: None,
            skip_sync: false,
            password: "x".into(),
            secrets_password: None,
            target: None,
            target_sudo_password: Some("pw".into()),
            build_host: Some(t),
        };
        let s = build_script(&req, Path::new("/home/u/nixos"), ConfigKind::Flake, 1000, 100);
        assert!(s.contains("nixos-rebuild switch --flake \"$NIXOS_DIR#main\" --build-host 'vincent@lg' 2>&1"), "{s}");
        assert!(s.contains("NM_TARGET_PW=*)"), "password must be read from stdin: {s}");
        assert!(!s.contains("pw"), "password must not be in the script");
        assert!(s.contains("UserKnownHostsFile=") && s.contains("export NIX_SSHOPTS="));
        assert!(s.contains("temporary trust") && s.contains("set +e\n") && s.contains("RC=$?\nset -e\n") && s.contains("exit \"$RC\""), "{s}");
        // the sync still happens (local activation)
        assert!(s.contains("rm -rf \"${NIXOS_DIR:?}\"/*"));
        let st = std::process::Command::new("bash").arg("-n").arg("-c").arg(&s).status().unwrap();
        assert!(st.success(), "script must parse");
    }

    #[test]
    fn upgrade_drops_lock_and_legacy_has_no_flake_flag() {
        let req = DeployRequest {
            path: "/w".into(),
            host: None,
            action: RebuildAction::Boot,
            upgrade: true,
            keep_configuration: None,
            hardware: None,
            build_host: None,
            skip_sync: false,
            password: String::new(),
            secrets_password: None,
            target: None,
            target_sudo_password: None,
        };
        let s = build_script(&req, Path::new("/w"), ConfigKind::Legacy, 1000, 100);
        assert!(s.contains("rm -f \"$NIXOS_DIR/flake.lock\""));
        assert!(!s.contains("Preserving"));
        assert!(s.contains("nixos-rebuild boot 2>&1\n"));
    }
}
