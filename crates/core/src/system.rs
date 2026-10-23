//! System-level helpers behind the "System" page: store usage, rollback to a
//! generation and garbage collection (the last two run as root through the
//! job system, like a build).

use crate::error::{CoreError, Result};
use crate::jobs::JobHandle;
use crate::process;
use crate::types::StoreUsage;
use std::time::Duration;

/// Disk usage of the filesystem that holds /nix/store (bytes).
pub async fn store_usage() -> Result<StoreUsage> {
    let out = process::run("df", &["-B1", "--output=size,used,avail", "/nix/store"], None, Duration::from_secs(10)).await?;
    let line = out.stdout.lines().nth(1).ok_or_else(|| CoreError::Other("df gave no data".into()))?;
    let mut it = line.split_whitespace().map(|v| v.parse::<u64>().unwrap_or(0));
    let (size, used, avail) = (it.next().unwrap_or(0), it.next().unwrap_or(0), it.next().unwrap_or(0));
    let generations = crate::nix::generations().await.map(|g| g.len() as u32).unwrap_or(0);
    Ok(StoreUsage { size, used, avail, generations })
}

/// Make generation `number` the current one and activate it (the undo story).
pub async fn rollback(job: JobHandle, number: u32, password: String) {
    job.running().await;
    let script = format!(
        "set -euo pipefail\nP=/nix/var/nix/profiles/system\nif [ ! -e \"$P-{n}-link\" ]; then echo \"generation {n} does not exist\" >&2; exit 2; fi\necho \"[nixos-manager] switching the system profile to generation {n}...\"\nnix-env -p \"$P\" --switch-generation {n} 2>&1\necho \"[nixos-manager] activating it (switch-to-configuration switch)...\"\n\"$P/bin/switch-to-configuration\" switch 2>&1\necho \"[nixos-manager] generation {n} is active and will be used at the next boot.\"\n",
        n = number
    );
    match crate::deploy::run_root_script(&job, &password, &script, None).await {
        Ok(0) => job.succeeded().await,
        Ok(2) => job.failed(format!("generation {number} does not exist")).await,
        Ok(c) => job.failed(format!("rollback exited with status {c}; see the log")).await,
        Err(e) => job.failed(e.to_string()).await,
    }
}

/// `nix-collect-garbage` flags: keep generations → none (only unreferenced
/// paths go), 0 days → `-d`, otherwise `--delete-older-than Nd`.
fn gc_flag(older_than_days: u32, keep_generations: bool) -> String {
    if keep_generations { String::new() } else if older_than_days == 0 { "-d".to_string() } else { format!("--delete-older-than {older_than_days}d") }
}

/// Delete old generations and unreferenced store paths.
/// `older_than_days` = 0 deletes every generation but the current one.
pub async fn collect_garbage(job: JobHandle, older_than_days: u32, password: String, keep_generations: bool) {
    job.running().await;
    let flag = gc_flag(older_than_days, keep_generations);
    let script = format!(
        "set -euo pipefail\necho \"[nixos-manager] before:\"; df -h /nix/store | tail -1\necho \"[nixos-manager] $ nix-collect-garbage {flag}\"\nnix-collect-garbage {flag} 2>&1\necho \"[nixos-manager] after:\"; df -h /nix/store | tail -1\n"
    );
    match crate::deploy::run_root_script(&job, &password, &script, None).await {
        Ok(0) => job.succeeded().await,
        Ok(c) => job.failed(format!("garbage collection exited with status {c}; see the log")).await,
        Err(e) => job.failed(e.to_string()).await,
    }
}

// ───────────────────────── remote targets ─────────────────────────

/// Shell that prints the target's generations and store usage as simple
/// `key=value` lines (no root needed).
const REMOTE_INFO_SCRIPT: &str = r#"set -u
echo "host=$(uname -n)"
cur=$(readlink /nix/var/nix/profiles/system 2>/dev/null || true)
for l in /nix/var/nix/profiles/system-*-link; do
  [ -e "$l" ] || continue
  n=${l##*/system-}; n=${n%-link}
  m=$(stat -c %Y "$l" 2>/dev/null || echo 0)
  v=$(cat "$l/nixos-version" 2>/dev/null || true)
  k=$(readlink "$l/kernel" 2>/dev/null || true)
  c=0; case "$cur" in *"system-$n-link"*) c=1;; esac
  echo "gen=$n|$m|$c|$v|$k"
done
df -B1 --output=size,used,avail /nix/store 2>/dev/null | tail -1 | sed 's/^ *//; s/ \+/ /g' | sed 's/^/df=/'
"#;

/// Generations + store usage of a deploy target, over SSH as the SSH user.
pub async fn remote_info(target: &crate::types::DeployTarget) -> Result<crate::types::RemoteSystemInfo> {
    let local = format!(
        "{}ssh ${{NM_SSH_OPTS:-}} -o BatchMode=yes -o ConnectTimeout=20 {} bash -s <<'NM_REMOTE_EOF'\n{}NM_REMOTE_EOF\n",
        crate::remote::port_prelude(target.port),
        crate::remote::sh_pub(&target.ssh_target),
        REMOTE_INFO_SCRIPT
    );
    let out = process::run("bash", &["-c", &local], None, Duration::from_secs(60)).await?;
    if out.status != 0 && out.stdout.trim().is_empty() {
        return Err(CoreError::Invalid(format!("cannot reach {}: {}", target.ssh_target, out.stderr.trim().chars().take(200).collect::<String>())));
    }
    let mut info = crate::types::RemoteSystemInfo { target: target.name.clone(), hostname: None, generations: vec![], store: None };
    for line in out.stdout.lines() {
        if let Some(h) = line.strip_prefix("host=") {
            info.hostname = Some(h.to_string());
        } else if let Some(g) = line.strip_prefix("gen=") {
            let p: Vec<&str> = g.splitn(5, '|').collect();
            if p.len() < 5 { continue; }
            let number: u32 = p[0].parse().unwrap_or(0);
            let mtime: i64 = p[1].parse().unwrap_or(0);
            let date = chrono::DateTime::<chrono::Utc>::from_timestamp(mtime, 0).map(|d| d.with_timezone(&chrono::Local).format("%Y-%m-%d %H:%M:%S").to_string()).unwrap_or_default();
            let kernel = if p[4].is_empty() { None } else { std::path::Path::new(p[4]).components().rev().nth(1).map(|c| c.as_os_str().to_string_lossy().to_string()).map(|n| n.splitn(2, '-').nth(1).unwrap_or(&n).to_string()) };
            info.generations.push(crate::types::Generation { number, date, current: p[2] == "1", nixos_version: if p[3].is_empty() { None } else { Some(p[3].to_string()) }, kernel });
        } else if let Some(d) = line.strip_prefix("df=") {
            let mut it = d.split_whitespace().map(|v| v.parse::<u64>().unwrap_or(0));
            let (size, used, avail) = (it.next().unwrap_or(0), it.next().unwrap_or(0), it.next().unwrap_or(0));
            if size > 0 {
                info.store = Some(StoreUsage { size, used, avail, generations: 0 });
            }
        }
    }
    info.generations.sort_by(|a, b| b.number.cmp(&a.number));
    if let Some(st) = info.store.as_mut() {
        st.generations = info.generations.len() as u32;
    }
    Ok(info)
}

/// Run a root script on the target: the sudo password goes first on ssh's
/// stdin (`sudo -S` reads one line), the script follows (`bash -s`). For a
/// root login no sudo is involved.
async fn run_remote_root(job: &JobHandle, target: &crate::types::DeployTarget, password: &str, script: &str) -> Result<i32> {
    let (_, user) = crate::remote::split_target_pub(&target.ssh_target);
    let is_root = user.as_deref() == Some("root");
    let t = crate::remote::sh_pub(&target.ssh_target);
    let pre = crate::remote::port_prelude(target.port);
    let local = if is_root {
        format!("{pre}ssh ${{NM_SSH_OPTS:-}} -o BatchMode=yes -o ConnectTimeout=20 {t} bash -s <<'NM_REMOTE_EOF'\n{script}NM_REMOTE_EOF\n")
    } else {
        format!("{pre}{{ printf '%s\\n' \"${{NM_TARGET_PW:-}}\"; cat <<'NM_REMOTE_EOF'\n{script}NM_REMOTE_EOF\n}} | ssh ${{NM_SSH_OPTS:-}} -o BatchMode=yes -o ConnectTimeout=20 {t} \"sudo -S -p '' bash -s\"\n")
    };
    let mut cmd = tokio::process::Command::new("bash");
    cmd.arg("-c").arg(&local);
    if !password.is_empty() {
        cmd.env("NM_TARGET_PW", password);
    }
    let sender = job.line_sender();
    process::stream_cmd_job(job, cmd, None, move |is_err, line| {
        let _ = sender.send(crate::types::LogLine { stream: if is_err { crate::types::LogStream::Stderr } else { crate::types::LogStream::Stdout }, text: line });
    })
    .await
}

pub async fn remote_rollback(job: JobHandle, target: crate::types::DeployTarget, number: u32, password: String) {
    job.running().await;
    job.system(format!("Rolling {} back to generation {number} over SSH...", target.ssh_target)).await;
    let script = format!(
        "set -euo pipefail\nP=/nix/var/nix/profiles/system\nif [ ! -e \"$P-{n}-link\" ]; then echo \"generation {n} does not exist on $(uname -n)\" >&2; exit 2; fi\necho \"[nixos-manager] $(uname -n): switching the system profile to generation {n}...\"\nnix-env -p \"$P\" --switch-generation {n} 2>&1\necho \"[nixos-manager] activating it (switch-to-configuration switch)...\"\n\"$P/bin/switch-to-configuration\" switch 2>&1\necho \"[nixos-manager] generation {n} is active on $(uname -n) and will be used at its next boot.\"\n",
        n = number
    );
    match run_remote_root(&job, &target, &password, &script).await {
        Ok(0) => job.succeeded().await,
        Ok(2) => job.failed(format!("generation {number} does not exist on the target")).await,
        Ok(c) => { sudo_hint(&job, c).await; job.failed(format!("remote rollback exited with status {c}; see the log")).await }
        Err(e) => job.failed(e.to_string()).await,
    }
}

pub async fn remote_gc(job: JobHandle, target: crate::types::DeployTarget, older_than_days: u32, password: String, keep_generations: bool) {
    job.running().await;
    let flag = gc_flag(older_than_days, keep_generations);
    job.system(format!("Collecting garbage on {} over SSH ({flag})...", target.ssh_target)).await;
    let script = format!(
        "set -euo pipefail\necho \"[nixos-manager] $(uname -n) before:\"; df -h /nix/store | tail -1\necho \"[nixos-manager] $ nix-collect-garbage {flag}\"\nnix-collect-garbage {flag} 2>&1\necho \"[nixos-manager] after:\"; df -h /nix/store | tail -1\n"
    );
    match run_remote_root(&job, &target, &password, &script).await {
        Ok(0) => job.succeeded().await,
        Ok(c) => { sudo_hint(&job, c).await; job.failed(format!("remote garbage collection exited with status {c}; see the log")).await }
        Err(e) => job.failed(e.to_string()).await,
    }
}

async fn sudo_hint(job: &JobHandle, code: i32) {
    if code == 1 {
        job.system("If the log shows a sudo password error: enter the target's sudo password in the dialog (needed unless the SSH user is root or has passwordless sudo).").await;
    }
}

// ───────────────────────── builds for other machines ─────────────────────────

fn parse_system_name(name: &str) -> Option<(String, String)> {
    // <hash>-nixos-system-<host>-<version>
    let rest = name.splitn(2, "-nixos-system-").nth(1)?;
    // the version starts at the last "-<digit>" run that looks like 26.05…
    let idx = rest.rmatch_indices('-').map(|(i, _)| i).find(|&i| rest[i + 1..].chars().next().map(|c| c.is_ascii_digit()).unwrap_or(false) && rest[i + 1..].contains('.'))?;
    Some((rest[..idx].to_string(), rest[idx + 1..].to_string()))
}

/// NixOS system closures in /nix/store that are not a local generation.
pub async fn other_systems() -> Result<Vec<crate::types::OtherSystem>> {
    let store = std::path::Path::new("/nix/store");
    let mut local: std::collections::HashSet<String> = std::collections::HashSet::new();
    if let Ok(rd) = std::fs::read_dir("/nix/var/nix/profiles") {
        for e in rd.flatten() {
            if let Ok(t) = std::fs::read_link(e.path()) {
                local.insert(t.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default());
            }
        }
    }
    for p in ["/run/current-system", "/run/booted-system"] {
        if let Ok(t) = std::fs::canonicalize(p) {
            local.insert(t.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default());
        }
    }
    // everything the local generations need: those paths stay whatever we delete
    let mut local_closure: std::collections::HashSet<String> = std::collections::HashSet::new();
    for name in &local {
        if name.is_empty() { continue; }
        let p = store.join(name).to_string_lossy().to_string();
        if let Ok(o) = process::run("nix-store", &["-qR", &p], None, Duration::from_secs(120)).await {
            local_closure.extend(o.stdout.lines().map(|l| l.to_string()));
        }
    }
    let mut out = Vec::new();
    let rd = std::fs::read_dir(store).map_err(|e| CoreError::Other(format!("cannot read /nix/store: {e}")))?;
    for e in rd.flatten() {
        let name = e.file_name().to_string_lossy().to_string();
        if !name.contains("-nixos-system-") || name.ends_with(".drv") || local.contains(&name) {
            continue;
        }
        let Some((host, version)) = parse_system_name(&name) else { continue };
        let path = store.join(&name);
        let path_s = path.to_string_lossy().to_string();
        // GC roots (stderr carries "removing stale link" noise; ignore it)
        let roots_out = process::run("nix-store", &["--query", "--roots", &path_s], None, Duration::from_secs(60)).await?;
        let roots: Vec<String> = roots_out.stdout.lines().map(|l| l.trim().to_string()).filter(|l| !l.is_empty()).collect();
        // closure size: requisites, then sizes in batches
        let req = process::run("nix-store", &["-qR", &path_s], None, Duration::from_secs(120)).await?;
        let paths: Vec<String> = req.stdout.lines().map(|l| l.to_string()).filter(|l| !l.is_empty()).collect();
        let mut closure_size: u64 = 0;
        let mut exclusive_size: u64 = 0;
        for chunk in paths.chunks(400) {
            let mut args: Vec<&str> = vec!["-q", "--size"];
            args.extend(chunk.iter().map(|s| s.as_str()));
            if let Ok(o) = process::run("nix-store", &args, None, Duration::from_secs(120)).await {
                for (p, sz) in chunk.iter().zip(o.stdout.lines().filter_map(|l| l.trim().parse::<u64>().ok())) {
                    closure_size += sz;
                    if !local_closure.contains(p) {
                        exclusive_size += sz;
                    }
                }
            }
        }
        out.push(crate::types::OtherSystem { path: path_s, host, nixos_version: version, closure_size, exclusive_size, roots });
    }
    out.sort_by(|a, b| b.exclusive_size.cmp(&a.exclusive_size).then(a.host.cmp(&b.host)));
    Ok(out)
}

/// Delete store paths (refused by nix when something still roots them) and
/// optionally sweep their dependencies with a garbage collection.
pub async fn delete_paths(job: JobHandle, paths: Vec<String>, password: String, gc_after: bool) {
    job.running().await;
    for p in &paths {
        if !p.starts_with("/nix/store/") || p.contains("..") || p.contains(' ') || p.contains('\'') {
            job.failed(format!("refusing to delete {p}: not a store path")).await;
            return;
        }
    }
    let quoted: Vec<String> = paths.iter().map(|p| format!("'{p}'")).collect();
    let mut script = format!("set -euo pipefail\necho \"[nixos-manager] deleting {} store path(s)...\"\nnix-store --delete {} 2>&1\n", paths.len(), quoted.join(" "));
    if gc_after {
        script.push_str("echo \"[nixos-manager] collecting garbage so the dependencies that are no longer referenced go too...\"\nnix-collect-garbage 2>&1\necho \"[nixos-manager] after:\"; df -h /nix/store | tail -1\n");
    }
    match crate::deploy::run_root_script(&job, &password, &script, None).await {
        Ok(0) => job.succeeded().await,
        Ok(c) => {
            job.system("If nix refused because the path is still alive, something still points at it (a `result` link or a profile); remove that first.").await;
            job.failed(format!("deletion exited with status {c}; see the log")).await
        }
        Err(e) => job.failed(e.to_string()).await,
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn gc_flags() {
        assert_eq!(super::gc_flag(30, true), "");
        assert_eq!(super::gc_flag(0, false), "-d");
        assert_eq!(super::gc_flag(30, false), "--delete-older-than 30d");
    }

    #[test]
    fn parses_system_store_names() {
        let (h, v) = super::parse_system_name("3zq4rgvgkhh1ihl5g5asgnagia9q50vk-nixos-system-lg-16z90s-h_au87u1-26.05.20261015.26f4107").unwrap();
        assert_eq!(h, "lg-16z90s-h_au87u1");
        assert_eq!(v, "26.05.20261015.26f4107");
        let (h, v) = super::parse_system_name("abc-nixos-system-test-box-26.05.20260820.5880666").unwrap();
        assert_eq!((h.as_str(), v.as_str()), ("test-box", "26.05.20260820.5880666"));
        assert!(super::parse_system_name("abc-firefox-1.0").is_none());
    }
}

// ───────────────────────── builds for other machines, on a target ─────────────────────────

/// Same scan as `other_systems`, but computed on the target by a shell
/// script (one ssh, no root): prints `other=<path>|<closure>|<exclusive>|<roots;…>`.
const REMOTE_OTHERS_SCRIPT: &str = r#"set -u
loc=$(mktemp); gens=$(mktemp)
for l in /nix/var/nix/profiles/system-*-link /run/current-system /run/booted-system; do
  [ -e "$l" ] || continue
  readlink -f "$l" >> "$gens"
  nix-store -qR "$l" 2>/dev/null
done | sort -u > "$loc"
for p in /nix/store/*-nixos-system-*; do
  case "$p" in *.drv) continue;; esac
  grep -qxF "$p" "$gens" && continue
  req=$(nix-store -qR "$p" 2>/dev/null) || continue
  total=$(printf '%s
' "$req" | xargs -r nix-store -q --size 2>/dev/null | awk '{s+=$1} END{print s+0}')
  excl=$(printf '%s
' "$req" | grep -vxF -f "$loc" | xargs -r nix-store -q --size 2>/dev/null | awk '{s+=$1} END{print s+0}')
  roots=$(nix-store --query --roots "$p" 2>/dev/null | tr '
' ';')
  echo "other=$p|$total|$excl|$roots"
done
rm -f "$loc" "$gens"
"#;

pub async fn remote_other_systems(target: &crate::types::DeployTarget) -> Result<Vec<crate::types::OtherSystem>> {
    let local = format!(
        "{}ssh ${{NM_SSH_OPTS:-}} -o BatchMode=yes -o ConnectTimeout=20 {} bash -s <<'NM_REMOTE_EOF'\n{}NM_REMOTE_EOF\n",
        crate::remote::port_prelude(target.port),
        crate::remote::sh_pub(&target.ssh_target),
        REMOTE_OTHERS_SCRIPT
    );
    let out = process::run("bash", &["-c", &local], None, Duration::from_secs(600)).await?;
    if out.status != 0 && out.stdout.trim().is_empty() {
        return Err(CoreError::Invalid(format!("cannot reach {}: {}", target.ssh_target, out.stderr.trim().chars().take(200).collect::<String>())));
    }
    let mut res = Vec::new();
    for line in out.stdout.lines() {
        let Some(rest) = line.strip_prefix("other=") else { continue };
        let f: Vec<&str> = rest.splitn(4, '|').collect();
        if f.len() < 4 { continue; }
        let name = f[0].rsplit('/').next().unwrap_or("");
        let Some((host, nixos_version)) = parse_system_name(name) else { continue };
        res.push(crate::types::OtherSystem {
            path: f[0].to_string(),
            host,
            nixos_version,
            closure_size: f[1].parse().unwrap_or(0),
            exclusive_size: f[2].parse().unwrap_or(0),
            roots: f[3].split(';').filter(|r| !r.trim().is_empty()).map(|r| r.to_string()).collect(),
        });
    }
    res.sort_by(|a, b| b.exclusive_size.cmp(&a.exclusive_size).then(a.host.cmp(&b.host)));
    Ok(res)
}

pub async fn remote_delete_paths(job: JobHandle, target: crate::types::DeployTarget, paths: Vec<String>, password: String, gc_after: bool) {
    job.running().await;
    for p in &paths {
        if !p.starts_with("/nix/store/") || p.contains("..") || p.contains(' ') || p.contains('\'') {
            job.failed(format!("refusing to delete {p}: not a store path")).await;
            return;
        }
    }
    let quoted: Vec<String> = paths.iter().map(|p| format!("'{p}'")).collect();
    let mut script = format!("set -euo pipefail\necho \"[nixos-manager] $(uname -n): deleting {} store path(s)...\"\nnix-store --delete {} 2>&1\n", paths.len(), quoted.join(" "));
    if gc_after {
        script.push_str("echo \"[nixos-manager] collecting garbage so the dependencies that are no longer referenced go too...\"\nnix-collect-garbage 2>&1\necho \"[nixos-manager] after:\"; df -h /nix/store | tail -1\n");
    }
    job.system(format!("Deleting {} build(s) on {} over SSH...", paths.len(), target.ssh_target)).await;
    match run_remote_root(&job, &target, &password, &script).await {
        Ok(0) => job.succeeded().await,
        Ok(c) => {
            sudo_hint(&job, c).await;
            job.system("If nix refused because the path is still alive, something on the target still points at it (a `result` link or a profile); remove that first.").await;
            job.failed(format!("remote deletion exited with status {c}; see the log")).await
        }
        Err(e) => job.failed(e.to_string()).await,
    }
}

// ───────────────────────── free memory before a build (TUI) ─────────────────────────

/// Vincent's clear-sessions.sh as a root job: terminate every login session
/// except `my_session`, terminate lingering user managers except `my_user`
/// and root, then drop the page cache. Frees the RAM a desktop holds so a
/// big build can run from a bare TTY or an SSH session.
pub fn free_memory_script(my_session: &str, my_user: &str) -> String {
    let sess = my_session.replace(|c: char| !c.is_ascii_alphanumeric(), "");
    let user = my_user.replace(|c: char| !(c.is_ascii_alphanumeric() || c == '-' || c == '_' || c == '.'), "");
    format!(
        r#"set -uo pipefail
MY_SESSION='{sess}'
MY_USER='{user}'
echo "[nixos-manager] protecting session ${{MY_SESSION:-unknown}} (user $MY_USER)"
echo "[nixos-manager] memory before:"; free -h | head -2
killed=0
for sid in $(loginctl list-sessions --no-legend | awk '{{print $1}}'); do
  if [ "$sid" = "$MY_SESSION" ]; then echo "  skip      session $sid (this one)"; continue; fi
  suser=$(loginctl show-session "$sid" -p Name --value 2>/dev/null || echo '?')
  echo "  terminate session $sid (user $suser)"
  loginctl terminate-session "$sid" || echo "    could not terminate session $sid"
  killed=$((killed + 1))
done
for u in $(loginctl list-users --no-legend | awk '{{print $2}}'); do
  if [ "$u" = "$MY_USER" ] || [ "$u" = root ]; then continue; fi
  echo "  terminate user manager of $u"
  loginctl terminate-user "$u" || echo "    could not terminate user $u"
done
sleep 2
sync
echo 3 > /proc/sys/vm/drop_caches 2>/dev/null || true
echo "[nixos-manager] terminated $killed other session(s); page cache dropped"
echo "[nixos-manager] memory now:"; free -h | head -2
"#
    )
}

pub async fn free_memory(job: JobHandle, password: String, my_session: String, my_user: String) {
    job.running().await;
    let script = free_memory_script(&my_session, &my_user);
    match crate::deploy::run_root_script(&job, &password, &script, None).await {
        Ok(0) => job.succeeded().await,
        Ok(c) => job.failed(format!("free-memory exited with status {c}; see the log")).await,
        Err(e) => job.failed(e.to_string()).await,
    }
}

#[cfg(test)]
mod free_memory_tests {
    #[test]
    fn script_parses_and_protects_the_caller() {
        let s = super::free_memory_script("3", "vincent");
        assert!(s.contains("MY_SESSION='3'") && s.contains("MY_USER='vincent'"));
        assert!(s.contains("loginctl terminate-session") && s.contains("drop_caches"));
        let st = std::process::Command::new("bash").arg("-n").arg("-c").arg(&s).status().unwrap();
        assert!(st.success());
        // odd characters cannot break out of the quotes
        let s2 = super::free_memory_script("3'; rm -rf /", "a b");
        assert!(s2.contains("MY_SESSION='3rmrf'") && s2.contains("MY_USER='ab'"));
    }
}
