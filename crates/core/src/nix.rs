//! Wrappers around the system's `nix`, `nixos-version`, `hostname`, and the
//! system profile. We deliberately use whatever nix the host has installed.

use crate::process::run;
use crate::types::{Generation, MachineInfo};
use crate::Result;
use std::path::Path;
use crate::error::CoreError;
use std::time::Duration;

pub async fn machine_info() -> MachineInfo {
    let hostname = run("hostname", &[], None, Duration::from_secs(5))
        .await
        .ok()
        .map(|o| o.stdout.trim().to_string())
        .filter(|s| !s.is_empty());
    let nixos_version = run("nixos-version", &[], None, Duration::from_secs(10))
        .await
        .ok()
        .filter(|o| o.status == 0)
        .map(|o| o.stdout.trim().to_string());
    let nix_version = run("nix", &["--version"], None, Duration::from_secs(10))
        .await
        .ok()
        .filter(|o| o.status == 0)
        .map(|o| o.stdout.trim().to_string());
    MachineInfo { hostname, nixos_version, nix_version, running_as_root: crate::deploy::running_as_root() }
}

/// Hosts according to `nix flake show --json`. Slow (evaluates the flake and
/// may fetch inputs); only call on explicit request.
pub async fn flake_hosts(flake_dir: &Path) -> Result<Vec<String>> {
    let out = run(
        "nix",
        &["flake", "show", "--json", "--no-write-lock-file", &flake_dir.to_string_lossy()],
        Some(flake_dir),
        Duration::from_secs(600),
    )
    .await?;
    if out.status != 0 {
        return Err(crate::CoreError::Other(format!("nix flake show failed:\n{}", out.stderr)));
    }
    let v: serde_json::Value = serde_json::from_str(&out.stdout)?;
    let mut hosts: Vec<String> = v
        .get("nixosConfigurations")
        .and_then(|c| c.as_object())
        .map(|o| o.keys().cloned().collect())
        .unwrap_or_default();
    hosts.sort();
    Ok(hosts)
}

/// System generations, read from the profile symlinks
/// `/nix/var/nix/profiles/system-<n>-link` (no lock file, works for any user;
/// `nix-env --list-generations` needs write access to the profile directory).
pub async fn generations() -> Result<Vec<Generation>> {
    let dir = Path::new("/nix/var/nix/profiles");
    let current_target = std::fs::read_link(dir.join("system")).ok();
    let mut gens = Vec::new();
    let rd = std::fs::read_dir(dir).map_err(|e| CoreError::Other(format!("cannot read {}: {e}", dir.display())))?;
    for e in rd.flatten() {
        let name = e.file_name().to_string_lossy().to_string();
        let Some(num) = name.strip_prefix("system-").and_then(|r| r.strip_suffix("-link")) else { continue };
        let Ok(number) = num.parse::<u32>() else { continue };
        let link = e.path();
        let meta = std::fs::symlink_metadata(&link).ok();
        let date = meta
            .and_then(|m| m.modified().ok())
            .map(|t| chrono::DateTime::<chrono::Local>::from(t).format("%Y-%m-%d %H:%M:%S").to_string())
            .unwrap_or_default();
        let current = current_target.as_deref().map(|t| t == Path::new(&name) || t == link.as_path()).unwrap_or(false);
        let nixos_version = std::fs::read_to_string(link.join("nixos-version")).ok().map(|s| s.trim().to_string());
        let kernel = std::fs::read_link(link.join("kernel")).ok().and_then(|k| {
            // /nix/store/<hash>-linux-6.12.3/bzImage → "linux-6.12.3"
            k.components().rev().nth(1).map(|c| c.as_os_str().to_string_lossy().to_string()).map(|n| n.splitn(2, '-').nth(1).unwrap_or(&n).to_string())
        });
        gens.push(Generation { number, date, current, nixos_version, kernel });
    }
    gens.sort_by(|a, b| b.number.cmp(&a.number));
    Ok(gens)
}
