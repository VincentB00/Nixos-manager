//! Server-side directory browsing. The browser cannot see the daemon's
//! filesystem, so the location prompt lists directories through here.

use crate::types::{BrowseResult, ConfigKind, DirEntryInfo, LocationSuggestion};
use crate::{CoreError, Result};
use std::path::{Path, PathBuf};
use walkdir::WalkDir;

const SKIP_DIRS: &[&str] = &[
    ".git", ".claude", "node_modules", "target", "result", ".cache", ".npm",
    ".cargo", ".rustup", ".local", ".mozilla", ".steam", ".var", "dist",
];

pub fn home_dir() -> Option<PathBuf> {
    directories::BaseDirs::new().map(|b| b.home_dir().to_path_buf())
}

/// Expand a leading `~` and make the path absolute.
pub fn normalize_input(input: &str) -> PathBuf {
    let trimmed = input.trim();
    if let Some(rest) = trimmed.strip_prefix("~") {
        if let Some(home) = home_dir() {
            return home.join(rest.trim_start_matches('/'));
        }
    }
    let p = PathBuf::from(trimmed);
    if p.is_absolute() {
        p
    } else {
        std::env::current_dir().map(|c| c.join(p)).unwrap_or_else(|_| PathBuf::from(trimmed))
    }
}

pub fn classify_dir(dir: &Path) -> ConfigKind {
    if dir.join("flake.nix").is_file() {
        ConfigKind::Flake
    } else if dir.join("configuration.nix").is_file() {
        ConfigKind::Legacy
    } else {
        ConfigKind::Unknown
    }
}

pub fn browse(input: &str) -> Result<BrowseResult> {
    let path = normalize_input(input);
    let meta = std::fs::metadata(&path)
        .map_err(|_| CoreError::NotFound(format!("{} does not exist", path.display())))?;
    if !meta.is_dir() {
        return Err(CoreError::Invalid(format!("{} is not a directory", path.display())));
    }
    let mut entries = Vec::new();
    let rd = std::fs::read_dir(&path)
        .map_err(|e| CoreError::Forbidden(format!("cannot read {}: {e}", path.display())))?;
    for entry in rd.flatten() {
        let name = entry.file_name().to_string_lossy().to_string();
        let ft = match entry.file_type() {
            Ok(ft) => ft,
            Err(_) => continue,
        };
        let full = entry.path();
        let is_symlink = ft.is_symlink();
        let is_dir = if is_symlink { full.is_dir() } else { ft.is_dir() };
        let is_nix = name.ends_with(".nix");
        if !is_dir && !is_nix && name != "flake.lock" {
            continue;
        }
        if name.starts_with('.') && name != ".config" {
            continue;
        }
        entries.push(DirEntryInfo { name, path: full.to_string_lossy().to_string(), is_dir, is_symlink, is_nix });
    }
    entries.sort_by(|a, b| b.is_dir.cmp(&a.is_dir).then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase())));
    Ok(BrowseResult {
        path: path.to_string_lossy().to_string(),
        parent: path.parent().map(|p| p.to_string_lossy().to_string()),
        looks_like_config: classify_dir(&path) != ConfigKind::Unknown,
        entries,
    })
}

/// Places a NixOS configuration is likely to live.
pub fn suggestions() -> Vec<LocationSuggestion> {
    let mut out = Vec::new();
    let mut seen = std::collections::HashSet::new();
    let mut push = |path: PathBuf, reason: &str| {
        let kind = classify_dir(&path);
        if kind == ConfigKind::Unknown {
            return;
        }
        let s = path.to_string_lossy().to_string();
        if seen.insert(s.clone()) {
            out.push(LocationSuggestion { path: s, kind, reason: reason.to_string() });
        }
    };

    push(PathBuf::from("/etc/nixos"), "system configuration directory");

    if let Some(home) = home_dir() {
        for candidate in [".config/nixos", "nixos", "nixos-config", ".nixos", "dotfiles", "nix"] {
            push(home.join(candidate), "common location in home");
        }
        // Shallow scan of the home directory for flake.nix / configuration.nix.
        let walker = WalkDir::new(&home)
            .max_depth(5)
            .follow_links(false)
            .into_iter()
            .filter_entry(|e| {
                let name = e.file_name().to_string_lossy();
                !(e.depth() > 0 && (SKIP_DIRS.contains(&name.as_ref()) || (name.starts_with('.') && name != ".config")))
            });
        let mut count = 0usize;
        for entry in walker.flatten() {
            count += 1;
            if count > 60_000 {
                break;
            }
            if !entry.file_type().is_file() {
                continue;
            }
            let name = entry.file_name().to_string_lossy();
            if name == "flake.nix" || name == "configuration.nix" {
                if let Some(dir) = entry.path().parent() {
                    push(dir.to_path_buf(), "found while scanning home");
                }
            }
        }
    }
    out
}
