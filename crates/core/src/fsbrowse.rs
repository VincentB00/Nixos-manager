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
/// `~` expands to the home directory; a relative path is taken relative to
/// the home directory as well (never the daemon's working directory, which
/// depends on how it was started); trailing slashes are dropped.
pub fn normalize_input(input: &str) -> PathBuf {
    let trimmed = input.trim().trim_end_matches('/');
    let trimmed = if trimmed.is_empty() && input.trim().starts_with('/') { "/" } else { trimmed };
    if let Some(rest) = trimmed.strip_prefix('~') {
        if let Some(home) = home_dir() {
            return home.join(rest.trim_start_matches('/'));
        }
    }
    let p = PathBuf::from(trimmed);
    if p.is_absolute() {
        p
    } else {
        home_dir().map(|h| h.join(&p)).unwrap_or(p)
    }
}

/// The canonical path of an existing configuration folder, for every API
/// call that works inside one: it must exist, be a directory and hold a
/// flake.nix or configuration.nix. Nothing is ever created here.
pub fn config_root(input: &str) -> crate::error::Result<PathBuf> {
    let p = normalize_input(input);
    let canon = std::fs::canonicalize(&p).map_err(|_| crate::error::CoreError::NotFound(format!("{} does not exist", p.display())))?;
    if !canon.is_dir() {
        return Err(crate::error::CoreError::Invalid(format!("{} is not a folder", canon.display())));
    }
    if classify_dir(&canon) == ConfigKind::Unknown {
        return Err(crate::error::CoreError::Invalid(format!("{} has neither flake.nix nor configuration.nix", canon.display())));
    }
    Ok(canon)
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
        // any project can have a flake.nix; only system flakes belong here
        if kind == ConfigKind::Flake && reason != "system configuration directory" {
            let is_system = std::fs::read_to_string(path.join("flake.nix")).map(|t| t.contains("nixosConfigurations")).unwrap_or(false);
            if !is_system {
                return;
            }
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

/// "Start from this machine's configuration": copy /etc/nixos into `dest`
/// (flake.lock left out — it lives in /etc/nixos only), optionally make it a
/// git repository. `dest` must not exist or must be an empty folder.
pub fn start_from_system(dest: &Path) -> Result<Vec<String>> {
    let src = Path::new(crate::deploy::ETC_NIXOS);
    if classify_dir(src) == ConfigKind::Unknown {
        return Err(CoreError::Invalid("/etc/nixos has no configuration.nix or flake.nix to start from".into()));
    }
    if dest.exists() {
        if !dest.is_dir() {
            return Err(CoreError::Invalid(format!("{} exists and is not a folder", dest.display())));
        }
        if std::fs::read_dir(dest)?.next().is_some() {
            return Err(CoreError::Conflict(format!("{} already exists and is not empty; choose another folder or open it directly", dest.display())));
        }
    }
    let mut copied = Vec::new();
    for entry in WalkDir::new(src).min_depth(1).follow_links(false).into_iter().filter_entry(|e| !(e.file_type().is_dir() && (e.file_name() == ".git" || e.file_name() == "result"))) {
        let entry = entry.map_err(|e| CoreError::Other(format!("cannot read /etc/nixos: {e}")))?;
        let rel = entry.path().strip_prefix(src).unwrap();
        if rel == Path::new("flake.lock") || rel == Path::new("result") {
            continue;
        }
        let to = dest.join(rel);
        if entry.file_type().is_dir() {
            std::fs::create_dir_all(&to)?;
        } else if entry.file_type().is_file() {
            if let Some(p) = to.parent() {
                std::fs::create_dir_all(p)?;
            }
            std::fs::copy(entry.path(), &to).map_err(|e| CoreError::Other(format!("cannot copy {}: {e}", rel.display())))?;
            // the copy belongs to the user now
            let _ = std::fs::set_permissions(&to, std::fs::Permissions::from(std::os::unix::fs::PermissionsExt::from_mode(0o644)));
            copied.push(rel.to_string_lossy().to_string());
        }
    }
    Ok(copied)
}
