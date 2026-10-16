//! Nix channels for classic (non-flake) configurations: list the system and
//! user channels, discover which channels exist upstream, and add / remove /
//! update them (system scope runs through sudo with the UI-provided password).

use crate::deploy::run_root_script;
use crate::jobs::JobHandle;
use crate::process;
use crate::types::{ChannelAction, ChannelActionRequest, ChannelEntry, ChannelScope, ChannelsInfo, LogStream};
use crate::{CoreError, Result};
use std::path::Path;
use std::time::Duration;

const ROOT_CHANNELS_DIR: &str = "/nix/var/nix/profiles/per-user/root/channels";
const FALLBACK_AVAILABLE: &[&str] = &["nixos-unstable", "nixos-26.05", "nixos-25.11", "nixos-25.05"];

pub fn channel_url(name: &str) -> String {
    format!("https://nixos.org/channels/{}", name.trim())
}

/// System channels without root: names and versions from the root profile.
fn system_channels() -> Vec<ChannelEntry> {
    let mut out = Vec::new();
    let Ok(rd) = std::fs::read_dir(ROOT_CHANNELS_DIR) else { return out };
    for e in rd.flatten() {
        let name = e.file_name().to_string_lossy().to_string();
        if name == "manifest.nix" || name == "binary-caches" || name.starts_with('.') {
            continue;
        }
        if !e.path().is_dir() {
            continue;
        }
        let version = std::fs::read_to_string(e.path().join(".version")).ok().map(|v| {
            let suffix = std::fs::read_to_string(e.path().join(".version-suffix")).unwrap_or_default();
            format!("{}{}", v.trim(), suffix.trim())
        });
        out.push(ChannelEntry { name, url: None, version, scope: ChannelScope::System });
    }
    out.sort_by(|a, b| a.name.cmp(&b.name));
    out
}

/// Version of the system `nixos` channel, if present.
pub fn system_nixos_version() -> Option<String> {
    system_channels().into_iter().find(|c| c.name == "nixos").and_then(|c| c.version)
}

async fn user_channels() -> Vec<ChannelEntry> {
    let Ok(out) = process::run("nix-channel", &["--list"], None, Duration::from_secs(10)).await else { return vec![] };
    out.stdout
        .lines()
        .filter_map(|l| {
            let mut it = l.split_whitespace();
            let name = it.next()?.to_string();
            let url = it.next().map(|s| s.to_string());
            Some(ChannelEntry { name, url, version: None, scope: ChannelScope::User })
        })
        .collect()
}

/// Channel names published upstream (nixos-*), newest first; falls back to a
/// static list when offline.
pub async fn available() -> Vec<String> {
    let fetched = tokio::time::timeout(Duration::from_secs(8), async {
        process::run("curl", &["-fsSL", "--max-time", "6", "https://nix-channels.s3.amazonaws.com/?delimiter=/"], None, Duration::from_secs(8)).await
    })
    .await;
    let mut names: Vec<String> = match fetched {
        Ok(Ok(o)) if o.status == 0 => o
            .stdout
            .split("<Key>")
            .skip(1)
            .filter_map(|s| s.split("</Key>").next())
            .filter(|k| k.starts_with("nixos-") && !k.contains("small") && !k.contains("aarch64"))
            .map(|s| s.to_string())
            .collect(),
        _ => vec![],
    };
    if names.is_empty() {
        names = FALLBACK_AVAILABLE.iter().map(|s| s.to_string()).collect();
    }
    names.sort_by(|a, b| version_key(b).cmp(&version_key(a)));
    names.dedup();
    names
}

fn version_key(name: &str) -> (u32, u32, u32) {
    if name == "nixos-unstable" {
        return (9999, 0, 0);
    }
    let v = name.trim_start_matches("nixos-");
    let mut it = v.split('.');
    let a = it.next().and_then(|x| x.parse().ok()).unwrap_or(0);
    let b = it.next().and_then(|x| x.parse().ok()).unwrap_or(0);
    (a, b, 0)
}

pub async fn info() -> ChannelsInfo {
    let (user, avail, machine) = tokio::join!(user_channels(), available(), crate::nix::machine_info());
    ChannelsInfo {
        system: system_channels(),
        user,
        available: avail,
        nixos_version: machine.nixos_version,
        system_urls_known: false,
    }
}

/// Run a channel action. System scope needs the sudo password.
pub async fn run(job: JobHandle, req: ChannelActionRequest) {
    job.running().await;
    let cmd = match (&req.action, &req.name) {
        (ChannelAction::Add, Some(name)) => {
            let url = req.url.clone().filter(|u| !u.trim().is_empty()).unwrap_or_else(|| channel_url(name));
            format!("nix-channel --add {} {}", sh(&url), sh(name))
        }
        (ChannelAction::Remove, Some(name)) => format!("nix-channel --remove {}", sh(name)),
        (ChannelAction::Update, name) => match name {
            Some(n) if !n.is_empty() => format!("nix-channel --update {}", sh(n)),
            _ => "nix-channel --update".to_string(),
        },
        _ => {
            job.failed("channel name is required").await;
            return;
        }
    };
    let script = format!("set -euo pipefail\necho \"[nixos-manager] $ {cmd}\"\n{cmd}\necho \"[nixos-manager] done; current channels:\"\nnix-channel --list\n");
    let result = match req.scope {
        ChannelScope::System => {
            let Some(pw) = req.password.filter(|p| !p.is_empty()) else {
                job.failed("sudo password is required for system channels").await;
                return;
            };
            run_root_script(&job, &pw, &script, None).await
        }
        ChannelScope::User => {
            let sender = job.line_sender();
            process::stream("bash", &["-c", &script], None, None, move |is_err, line| {
                let _ = sender.send(crate::types::LogLine { stream: if is_err { LogStream::Stderr } else { LogStream::Stdout }, text: line });
            })
            .await
        }
    };
    match result {
        Ok(0) => job.succeeded().await,
        Ok(code) => job.failed(format!("nix-channel exited with status {code}")).await,
        Err(e) => job.failed(e.to_string()).await,
    }
}

fn sh(s: &str) -> String {
    format!("'{}'", s.replace('\'', "'\\''"))
}

#[allow(dead_code)]
fn _unused(_: &Path) -> Result<()> {
    Err(CoreError::Other(String::new()))
}
