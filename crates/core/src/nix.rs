//! Wrappers around the system's `nix`, `nixos-version`, `hostname`, and the
//! system profile. We deliberately use whatever nix the host has installed.

use crate::process::run;
use crate::types::{Generation, MachineInfo};
use crate::Result;
use std::path::Path;
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
    MachineInfo { hostname, nixos_version, nix_version }
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

/// System generations from `nix-env --list-generations -p /nix/var/nix/profiles/system`.
pub async fn generations() -> Result<Vec<Generation>> {
    let out = run(
        "nix-env",
        &["--list-generations", "-p", "/nix/var/nix/profiles/system"],
        None,
        Duration::from_secs(30),
    )
    .await?;
    let mut gens = Vec::new();
    for line in out.stdout.lines() {
        let parts: Vec<&str> = line.split_whitespace().collect();
        if parts.len() < 3 {
            continue;
        }
        let number: u32 = match parts[0].parse() {
            Ok(n) => n,
            Err(_) => continue,
        };
        let date = format!("{} {}", parts[1], parts[2]);
        let current = line.contains("(current)");
        let link = format!("/nix/var/nix/profiles/system-{number}-link");
        let nixos_version = std::fs::read_to_string(format!("{link}/nixos-version")).ok().map(|s| s.trim().to_string());
        let kernel = std::fs::read_link(format!("{link}/kernel"))
            .ok()
            .and_then(|p| p.to_string_lossy().split('/').nth(3).map(|s| s.to_string()));
        gens.push(Generation { number, date, current, nixos_version, kernel });
    }
    gens.reverse();
    Ok(gens)
}
