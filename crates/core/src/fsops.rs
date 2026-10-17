//! File and folder operations inside a configuration root: create folders,
//! rename/move, duplicate, delete. Renaming a .nix file rewrites the path
//! literals in files that import or reference it.

use crate::discover::{list_files, resolve_inside};
use crate::nixparse::{self, Parsed};
use crate::types::{ImportVia, RenameResult};
use crate::{CoreError, Result};
use std::path::{Component, Path, PathBuf};

fn check_name(rel: &str) -> Result<()> {
    let t = rel.trim();
    if t.is_empty() || t == "." || t.ends_with('/') {
        return Err(CoreError::Invalid("name cannot be empty".into()));
    }
    if Path::new(t).components().any(|c| matches!(c, Component::ParentDir | Component::RootDir)) {
        return Err(CoreError::Invalid(format!("{rel} escapes the configuration folder")));
    }
    Ok(())
}

pub fn mkdir(root: &Path, rel: &str) -> Result<()> {
    check_name(rel)?;
    let full = resolve_inside(root, rel)?;
    if full.exists() {
        return Err(CoreError::Conflict(format!("{rel} already exists")));
    }
    std::fs::create_dir_all(&full)?;
    Ok(())
}

/// Delete a file or folder. With `remove_references`, every enabled
/// `imports = [ … ]` entry pointing at the deleted file (or into the deleted
/// folder) is removed from the importing files first, so the configuration
/// keeps evaluating. Returns the files that were edited.
pub fn delete(root: &Path, rel: &str, recursive: bool, remove_references: bool) -> Result<Vec<String>> {
    check_name(rel)?;
    let full = resolve_inside(root, rel)?;
    let meta = std::fs::symlink_metadata(&full).map_err(|_| CoreError::NotFound(format!("{rel} not found")))?;
    let mut updated = Vec::new();
    if remove_references {
        let gone = nixparse::normalize(Path::new(rel));
        for (irel, _) in list_files(root) {
            if !irel.ends_with(".nix") || nixparse::normalize(Path::new(&irel)).starts_with(&gone) {
                continue;
            }
            let Ok(text) = std::fs::read_to_string(root.join(&irel)) else { continue };
            let parsed = Parsed::parse(text.clone());
            let mut edits: Vec<(usize, usize)> = Vec::new();
            for imp in nixparse::imports(&parsed, &irel, root) {
                if !imp.enabled || imp.via != ImportVia::Imports {
                    continue;
                }
                let t = nixparse::normalize(Path::new(&imp.target));
                if t == gone || (meta.is_dir() && t.starts_with(&gone)) {
                    if let Some((s, e)) = nixparse::parse_node_id(&imp.node_id) {
                        edits.push((s, e));
                    }
                }
            }
            if edits.is_empty() {
                continue;
            }
            edits.sort_by(|a, b| b.0.cmp(&a.0));
            let mut out = text;
            for (s, e) in edits {
                let ed = crate::edit::remove_item_edit(&out, s, e);
                let (rs, re): (usize, usize) = (ed.range.start().into(), ed.range.end().into());
                out.replace_range(rs..re, &ed.text);
            }
            crate::edit::write_atomic(&root.join(&irel), &out)?;
            updated.push(irel);
        }
    }
    if meta.is_dir() {
        if recursive {
            std::fs::remove_dir_all(&full)?;
        } else {
            std::fs::remove_dir(&full).map_err(|e| match e.kind() {
                std::io::ErrorKind::DirectoryNotEmpty => CoreError::Conflict(format!("{rel} is not empty")),
                _ => CoreError::Io(e),
            })?;
        }
    } else {
        std::fs::remove_file(&full)?;
    }
    Ok(updated)
}

pub fn duplicate(root: &Path, rel: &str, to: &str) -> Result<()> {
    check_name(rel)?;
    check_name(to)?;
    let src = resolve_inside(root, rel)?;
    let dst = resolve_inside(root, to)?;
    if !src.is_file() {
        return Err(CoreError::Invalid(format!("{rel} is not a file")));
    }
    if dst.exists() {
        return Err(CoreError::Conflict(format!("{to} already exists")));
    }
    if let Some(p) = dst.parent() {
        std::fs::create_dir_all(p)?;
    }
    std::fs::copy(&src, &dst)?;
    Ok(())
}

/// Rename or move a file or folder. For every .nix file in the root, path
/// literals that resolve to the old location (or inside it, for folders) are
/// rewritten to point at the new one.
pub fn rename(root: &Path, from: &str, to: &str, update_references: bool) -> Result<RenameResult> {
    check_name(from)?;
    check_name(to)?;
    let src = resolve_inside(root, from)?;
    let dst = resolve_inside(root, to)?;
    if !src.exists() {
        return Err(CoreError::NotFound(format!("{from} not found")));
    }
    if dst.exists() {
        return Err(CoreError::Conflict(format!("{to} already exists")));
    }
    let from_norm = nixparse::normalize(Path::new(from));
    let to_norm = nixparse::normalize(Path::new(to));
    if from_norm == to_norm {
        return Err(CoreError::Invalid("old and new names are the same".into()));
    }
    let is_dir = src.is_dir();
    if is_dir && to_norm.starts_with(&from_norm) {
        return Err(CoreError::Invalid("cannot move a folder into itself".into()));
    }

    // Plan reference rewrites before touching the filesystem (targets must
    // still exist for resolution). For every path literal: compute where its
    // target ends up and where the referencing file ends up, and rewrite the
    // literal if the relative path between them changes.
    let moved = |p: &Path| -> PathBuf {
        if p == from_norm {
            to_norm.clone()
        } else if is_dir && p.starts_with(&from_norm) {
            to_norm.join(p.strip_prefix(&from_norm).unwrap())
        } else {
            p.to_path_buf()
        }
    };
    let mut rewrites: Vec<(String, Vec<(usize, usize, String)>)> = Vec::new();
    if update_references {
        for (rel, _) in list_files(root) {
            if !rel.ends_with(".nix") {
                continue;
            }
            let Ok(src_text) = std::fs::read_to_string(root.join(&rel)) else { continue };
            let parsed = Parsed::parse(src_text);
            let importer_new = moved(Path::new(&rel));
            let importer_new_dir = importer_new.parent().unwrap_or(Path::new("")).to_path_buf();
            let mut edits = Vec::new();
            for imp in nixparse::imports(&parsed, &rel, root) {
                if !imp.enabled && imp.via != ImportVia::Imports {
                    continue;
                }
                let new_target = moved(Path::new(&imp.target));
                let new_literal = relative_literal(&new_target, &importer_new_dir);
                if new_literal == imp.raw {
                    continue;
                }
                let Some((s, e)) = nixparse::parse_node_id(&imp.node_id) else { continue };
                // Disabled imports live inside a comment token: replace only the path text.
                let (s, e) = if imp.enabled {
                    (s, e)
                } else {
                    let tok = &parsed.source[s..e];
                    match tok.find(&imp.raw) {
                        Some(off) => (s + off, s + off + imp.raw.len()),
                        None => continue,
                    }
                };
                edits.push((s, e, new_literal));
            }
            if !edits.is_empty() {
                rewrites.push((rel, edits));
            }
        }
    }

    if let Some(p) = dst.parent() {
        std::fs::create_dir_all(p)?;
    }
    std::fs::rename(&src, &dst)?;

    let mut updated = Vec::new();
    for (rel, mut edits) in rewrites {
        // Apply from the end so earlier offsets stay valid.
        edits.sort_by(|a, b| b.0.cmp(&a.0));
        let on_disk = moved(Path::new(&rel)).to_string_lossy().to_string();
        let full = root.join(&on_disk);
        let mut text = std::fs::read_to_string(&full)?;
        for (s, e, lit) in edits {
            text.replace_range(s..e, &lit);
        }
        crate::edit::write_atomic(&full, &text)?;
        updated.push(on_disk);
    }
    Ok(RenameResult { from: from.to_string(), to: to.to_string(), updated_files: updated })
}

/// `./x/y.nix`-style literal for `target` as seen from `from_dir` (both relative to root).
fn relative_literal(target: &Path, from_dir: &Path) -> String {
    let t: Vec<_> = target.components().collect();
    let f: Vec<_> = from_dir.components().collect();
    let common = t.iter().zip(f.iter()).take_while(|(a, b)| a == b).count();
    let mut out = String::new();
    for _ in common..f.len() {
        out.push_str("../");
    }
    if out.is_empty() {
        out.push_str("./");
    }
    let rest: Vec<String> = t[common..].iter().map(|c| c.as_os_str().to_string_lossy().to_string()).collect();
    out.push_str(&rest.join("/"));
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn setup() -> tempdir::TempDir {
        let d = tempdir::TempDir::new("fsops").unwrap();
        let r = d.path();
        std::fs::create_dir_all(r.join("modules")).unwrap();
        std::fs::write(r.join("configuration.nix"), "{ ... }:\n{\n  imports = [\n    ./modules/a.nix\n    # ./modules/b.nix\n  ];\n}\n").unwrap();
        std::fs::write(r.join("modules/a.nix"), "{ ... }:\n{ imports = [ ./b.nix ]; x = builtins.readFile ../data.txt; }\n").unwrap();
        std::fs::write(r.join("modules/b.nix"), "{ }\n").unwrap();
        std::fs::write(r.join("data.txt"), "hi").unwrap();
        d
    }

    #[test]
    fn delete_can_remove_imports() {
        let d = std::env::temp_dir().join(format!("nm-del-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(d.join("modules")).unwrap();
        std::fs::write(d.join("configuration.nix"), "{ ... }: {\n  imports = [\n    ./modules/a.nix\n    ./modules/b.nix # keep\n  ];\n}\n").unwrap();
        std::fs::write(d.join("modules/a.nix"), "{ }\n").unwrap();
        std::fs::write(d.join("modules/b.nix"), "{ }\n").unwrap();
        let updated = delete(&d, "modules/a.nix", false, true).unwrap();
        assert_eq!(updated, vec!["configuration.nix".to_string()]);
        let cfg = std::fs::read_to_string(d.join("configuration.nix")).unwrap();
        assert!(!cfg.contains("a.nix") && cfg.contains("./modules/b.nix # keep"), "{cfg}");
        assert!(!d.join("modules/a.nix").exists());
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn rename_file_updates_importers() {
        let d = setup();
        let r = d.path();
        let res = rename(r, "modules/b.nix", "modules/bluetooth.nix", true).unwrap();
        assert!(r.join("modules/bluetooth.nix").is_file());
        let cfg = std::fs::read_to_string(r.join("configuration.nix")).unwrap();
        assert!(cfg.contains("# ./modules/bluetooth.nix"), "{cfg}");
        let a = std::fs::read_to_string(r.join("modules/a.nix")).unwrap();
        assert!(a.contains("imports = [ ./bluetooth.nix ]"), "{a}");
        assert_eq!(res.updated_files.len(), 2);
    }

    #[test]
    fn rename_folder_updates_both_directions() {
        let d = setup();
        let r = d.path();
        rename(r, "modules", "mods/system", true).unwrap();
        let cfg = std::fs::read_to_string(r.join("configuration.nix")).unwrap();
        assert!(cfg.contains("./mods/system/a.nix"), "{cfg}");
        assert!(cfg.contains("# ./mods/system/b.nix"), "{cfg}");
        let a = std::fs::read_to_string(r.join("mods/system/a.nix")).unwrap();
        assert!(a.contains("[ ./b.nix ]"), "{a}");
        assert!(a.contains("../../data.txt"), "{a}");
    }

    #[test]
    fn mkdir_delete_duplicate() {
        let d = setup();
        let r = d.path();
        mkdir(r, "new/dir").unwrap();
        assert!(r.join("new/dir").is_dir());
        assert!(delete(r, "new", false, false).is_err());
        delete(r, "new", true, false).unwrap();
        duplicate(r, "modules/b.nix", "modules/c.nix").unwrap();
        assert!(r.join("modules/c.nix").is_file());
        assert!(mkdir(r, "../x").is_err());
    }
}
