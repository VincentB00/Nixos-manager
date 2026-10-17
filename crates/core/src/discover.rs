//! Whole-configuration discovery: walk the folder, classify files, parse every
//! .nix file, and build the import graph.

use crate::fsbrowse::classify_dir;
use crate::nixparse::{self, Parsed};
use crate::types::{ConfigKind, ConfigOverview, FileKind, FileSummary, ImportEdge, LocationInfo, MachineInfo};
use crate::{CoreError, Result};
use std::collections::{HashMap, HashSet, VecDeque};
use std::path::{Path, PathBuf};
use walkdir::WalkDir;

const SKIP_DIRS: &[&str] = &[".git", ".claude", "node_modules", "result", "target", ".direnv", "dist"];
/// Never descended into, whatever the root (a walk of `/` must not touch them).
const SKIP_ABS: &[&str] = &["/proc", "/sys", "/dev", "/run", "/nix/store", "/nix/var"];
/// Hard cap on entries per walk (a configuration has dozens of files, not thousands).
const MAX_ENTRIES: usize = 20_000;

fn skip_entry(e: &walkdir::DirEntry) -> bool {
    if e.depth() == 0 || !e.file_type().is_dir() {
        return false;
    }
    let name = e.file_name().to_string_lossy();
    if SKIP_DIRS.contains(&name.as_ref()) || name.starts_with('.') {
        return true;
    }
    let p = e.path().to_string_lossy();
    SKIP_ABS.iter().any(|a| p == *a || p.starts_with(&format!("{a}/")))
}
const MAX_FILE_BYTES: u64 = 2 * 1024 * 1024;

pub fn location_info(dir: &Path) -> Result<LocationInfo> {
    let meta = std::fs::metadata(dir).map_err(|_| CoreError::NotFound(format!("{} does not exist", dir.display())))?;
    if !meta.is_dir() {
        return Err(CoreError::Invalid(format!("{} is not a directory", dir.display())));
    }
    let kind = classify_dir(dir);
    let entry_file = match kind {
        ConfigKind::Flake => Some("flake.nix".to_string()),
        ConfigKind::Legacy => Some("configuration.nix".to_string()),
        ConfigKind::Unknown => None,
    };
    let hosts = if kind == ConfigKind::Flake {
        std::fs::read_to_string(dir.join("flake.nix"))
            .map(|s| nixparse::flake_hosts_static(&Parsed::parse(s)))
            .unwrap_or_default()
    } else {
        vec![]
    };
    let canonical = std::fs::canonicalize(dir).unwrap_or_else(|_| dir.to_path_buf());
    let etc = std::fs::canonicalize("/etc/nixos").unwrap_or_else(|_| PathBuf::from("/etc/nixos"));
    Ok(LocationInfo {
        path: dir.to_string_lossy().to_string(),
        kind,
        entry_file,
        has_hardware_config: dir.join("hardware-configuration.nix").is_file(),
        has_flake_lock: dir.join("flake.lock").is_file(),
        is_etc_nixos: canonical == etc,
        is_git: dir.join(".git").exists(),
        writable: is_writable(dir),
        hosts,
    })
}

fn is_writable(dir: &Path) -> bool {
    let probe = dir.join(".nixos-manager-write-test");
    match std::fs::OpenOptions::new().write(true).create_new(true).open(&probe) {
        Ok(_) => {
            let _ = std::fs::remove_file(&probe);
            true
        }
        Err(_) => false,
    }
}

pub fn classify_file(rel: &str) -> FileKind {
    let name = Path::new(rel).file_name().map(|s| s.to_string_lossy().to_lowercase()).unwrap_or_default();
    match name.as_str() {
        "flake.nix" => return FileKind::Flake,
        "flake.lock" => return FileKind::FlakeLock,
        "configuration.nix" => return FileKind::Configuration,
        "hardware-configuration.nix" => return FileKind::HardwareConfiguration,
        _ => {}
    }
    if name.contains("secret") || name.contains("password") || name.contains("credential") || name.ends_with(".age") || name.ends_with(".key") {
        return FileKind::Secret;
    }
    if name.ends_with(".nix") {
        return FileKind::Module;
    }
    if name.ends_with(".sh") || name.ends_with(".bash") || name.ends_with(".py") {
        return FileKind::Script;
    }
    if name.ends_with(".json") || name.ends_with(".yaml") || name.ends_with(".yml") || name.ends_with(".toml") || name.ends_with(".conf") {
        return FileKind::Data;
    }
    FileKind::Other
}

/// Relative paths of all tracked files under `dir`.
pub fn list_files(dir: &Path) -> Vec<(String, u64)> {
    let mut out = Vec::new();
    let walker = WalkDir::new(dir).max_depth(8).follow_links(false).same_file_system(true).into_iter().filter_entry(|e| !skip_entry(e));
    for entry in walker.flatten() {
        if out.len() >= MAX_ENTRIES {
            break;
        }
        if !entry.file_type().is_file() {
            continue;
        }
        let name = entry.file_name().to_string_lossy();
        if name.starts_with('.') {
            continue;
        }
        let rel = entry.path().strip_prefix(dir).unwrap_or(entry.path()).to_string_lossy().to_string();
        let size = entry.metadata().map(|m| m.len()).unwrap_or(0);
        out.push((rel, size));
    }
    out.sort();
    out
}

/// Relative paths of all directories under `dir` (same skip rules as files).
pub fn list_dirs(dir: &Path) -> Vec<String> {
    let mut out = Vec::new();
    let walker = WalkDir::new(dir).max_depth(8).follow_links(false).same_file_system(true).into_iter().filter_entry(|e| !skip_entry(e));
    for entry in walker.flatten() {
        if out.len() >= MAX_ENTRIES {
            break;
        }
        if entry.depth() == 0 || !entry.file_type().is_dir() {
            continue;
        }
        out.push(entry.path().strip_prefix(dir).unwrap_or(entry.path()).to_string_lossy().to_string());
    }
    out.sort();
    out
}

pub async fn overview(dir: &Path) -> Result<ConfigOverview> {
    let location = location_info(dir)?;
    if location.kind == ConfigKind::Unknown {
        return Err(CoreError::Invalid(format!("{} has neither flake.nix nor configuration.nix", dir.display())));
    }
    let machine: MachineInfo = crate::nix::machine_info().await;
    // the walk and the parsing are synchronous file work: keep them off the async runtime
    tokio::task::spawn_blocking(move || overview_with(location, machine)).await.map_err(|e| CoreError::Other(e.to_string()))
}

pub fn overview_with(location: LocationInfo, machine: MachineInfo) -> ConfigOverview {
    let dir = PathBuf::from(&location.path);
    let mut files: Vec<FileSummary> = Vec::new();
    for (rel, size) in list_files(&dir) {
        let kind = classify_file(&rel);
        let mut summary = FileSummary {
            rel_path: rel.clone(),
            kind,
            size,
            parse_ok: true,
            parse_errors: vec![],
            imports: vec![],
            imported_by: vec![],
            namespaces: vec![],
            description: None,
            args: vec![],
        };
        if rel.ends_with(".nix") && size <= MAX_FILE_BYTES {
            if let Ok(src) = std::fs::read_to_string(dir.join(&rel)) {
                let parsed = Parsed::parse(src);
                summary.parse_ok = parsed.errors.is_empty();
                summary.parse_errors = parsed.parse_errors().into_iter().map(|e| e.message).collect();
                summary.imports = nixparse::imports(&parsed, &rel, &dir);
                summary.namespaces = nixparse::namespaces(&parsed);
                summary.description = nixparse::file_description(&parsed);
                summary.args = parsed.module_body().0;
            }
        }
        files.push(summary);
    }

    // Edges and reverse index.
    let mut edges = Vec::new();
    let mut imported_by: HashMap<String, Vec<String>> = HashMap::new();
    for f in &files {
        for i in &f.imports {
            edges.push(ImportEdge { from: f.rel_path.clone(), to: i.target.clone(), via: i.via, enabled: i.enabled });
            if i.enabled {
                imported_by.entry(i.target.clone()).or_default().push(f.rel_path.clone());
            }
        }
    }
    for f in &mut files {
        if let Some(v) = imported_by.get(&f.rel_path) {
            let mut v = v.clone();
            v.sort();
            v.dedup();
            f.imported_by = v;
        }
    }

    // Roots: nix files nobody imports (enabled). Entry file first.
    let nix_files: Vec<String> = files.iter().filter(|f| f.rel_path.ends_with(".nix")).map(|f| f.rel_path.clone()).collect();
    let mut roots: Vec<String> = nix_files.iter().filter(|f| !imported_by.contains_key(*f)).cloned().collect();
    if let Some(entry) = &location.entry_file {
        if let Some(pos) = roots.iter().position(|r| r == entry) {
            let e = roots.remove(pos);
            roots.insert(0, e);
        }
    }

    // Reachability from the entry file through enabled imports.
    let mut reachable: HashSet<String> = HashSet::new();
    if let Some(entry) = &location.entry_file {
        let adj: HashMap<&str, Vec<&str>> = files
            .iter()
            .map(|f| (f.rel_path.as_str(), f.imports.iter().filter(|i| i.enabled).map(|i| i.target.as_str()).collect()))
            .collect();
        let mut q = VecDeque::from([entry.as_str()]);
        while let Some(cur) = q.pop_front() {
            if !reachable.insert(cur.to_string()) {
                continue;
            }
            if let Some(next) = adj.get(cur) {
                for n in next {
                    q.push_back(n);
                }
            }
        }
    }
    let unreachable: Vec<String> = if location.entry_file.is_some() {
        nix_files.iter().filter(|f| !reachable.contains(*f)).cloned().collect()
    } else {
        vec![]
    };

    let directories = list_dirs(&dir);
    ConfigOverview { location, files, directories, edges, roots, unreachable, machine }
}

/// Read a file inside the config root, rejecting path escapes.
pub fn resolve_inside(root: &Path, rel: &str) -> Result<PathBuf> {
    let rel_path = nixparse::normalize(Path::new(rel));
    if rel_path.is_absolute() || rel_path.starts_with("..") {
        return Err(CoreError::Invalid(format!("{rel} escapes the configuration folder")));
    }
    Ok(root.join(rel_path))
}

pub fn read_file_view(root: &Path, rel: &str) -> Result<crate::types::NixFileView> {
    let full = resolve_inside(root, rel)?;
    let src = std::fs::read_to_string(&full).map_err(|e| match e.kind() {
        std::io::ErrorKind::NotFound => CoreError::NotFound(format!("{rel} not found")),
        std::io::ErrorKind::InvalidData => {
            let size = std::fs::metadata(&full).map(|m| m.len()).unwrap_or(0);
            CoreError::Invalid(format!("{rel} is a binary file ({size} bytes); it cannot be shown or edited here"))
        }
        _ => CoreError::Io(e),
    })?;
    Ok(nixparse::analyze_file(rel, src))
}
