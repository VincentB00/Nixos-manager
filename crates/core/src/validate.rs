//! "Validate": check a configuration without building or touching the
//! system. Runs as the user, no sudo:
//! 1. syntax check of every .nix file (`nix-instantiate --parse`),
//! 2. a full evaluation + dry build of the system closure
//!    (`nix build --dry-run` for flakes, `nixos-rebuild dry-build` for
//!    classic configs) on a temporary copy of the working folder that has
//!    flake.lock and hardware-configuration.nix borrowed from /etc/nixos when
//!    missing, i.e. what sync.sh would produce.

use crate::deploy::ETC_NIXOS;
use crate::discover::{list_files, location_info};
use crate::jobs::JobHandle;
use crate::process;
use crate::types::{ConfigKind, LogLine, LogStream};
use std::path::{Path, PathBuf};
use std::time::Duration;

const SKIP: &[&str] = &[".git", ".claude", "node_modules", "result", "target", "secrets.age"];

/// Plain copy of the working folder (no .git, no secrets.age) so a flake
/// reference sees every file, tracked or not. Nothing is borrowed from
/// /etc/nixos. Used for remote deployments.
pub fn stage_plain_copy(workdir: &Path) -> std::io::Result<PathBuf> {
    stage_copy_inner(workdir, &[])
}

fn stage_copy(workdir: &Path, is_flake: bool) -> std::io::Result<PathBuf> {
    let borrow: &[&str] = if is_flake { &["flake.lock", "hardware-configuration.nix"] } else { &["hardware-configuration.nix"] };
    stage_copy_inner(workdir, borrow)
}

fn stage_copy_inner(workdir: &Path, borrow: &[&str]) -> std::io::Result<PathBuf> {
    use std::os::unix::fs::PermissionsExt;
    let tmp = std::env::temp_dir().join(format!("nixos-manager-staged-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir(&tmp)?;
    std::fs::set_permissions(&tmp, std::fs::Permissions::from_mode(0o700))?;
    for entry in walkdir::WalkDir::new(workdir).min_depth(1).into_iter().filter_entry(|e| !SKIP.contains(&e.file_name().to_string_lossy().as_ref())) {
        let entry = entry?;
        let rel = entry.path().strip_prefix(workdir).unwrap();
        let dest = tmp.join(rel);
        if entry.file_type().is_dir() {
            std::fs::create_dir_all(&dest)?;
        } else if entry.file_type().is_file() {
            if let Some(p) = dest.parent() {
                std::fs::create_dir_all(p)?;
            }
            std::fs::copy(entry.path(), &dest)?;
        }
    }
    for borrowed in borrow {
        if !tmp.join(borrowed).exists() {
            let src = Path::new(ETC_NIXOS).join(borrowed);
            if src.is_file() {
                let _ = std::fs::copy(&src, tmp.join(borrowed));
            }
        }
    }
    Ok(tmp)
}

pub async fn run(job: JobHandle, workdir: PathBuf, host: Option<String>) {
    job.running().await;
    let loc = match location_info(&workdir) {
        Ok(l) => l,
        Err(e) => {
            job.failed(e.to_string()).await;
            return;
        }
    };

    // 1. Syntax.
    let nix_files: Vec<String> = list_files(&workdir).into_iter().map(|(r, _)| r).filter(|r| r.ends_with(".nix")).collect();
    job.system(format!("Checking syntax of {} files...", nix_files.len())).await;
    let mut syntax_errors = 0;
    for rel in &nix_files {
        let full = workdir.join(rel);
        match process::run("nix-instantiate", &["--parse", &full.to_string_lossy()], Some(&workdir), Duration::from_secs(30)).await {
            Ok(out) if out.status != 0 => {
                syntax_errors += 1;
                job.log(LogStream::Stderr, format!("{rel}: {}", out.stderr.trim())).await;
            }
            Err(e) => {
                job.log(LogStream::Stderr, format!("{rel}: could not run nix-instantiate: {e}")).await;
            }
            _ => {}
        }
    }
    if syntax_errors > 0 {
        job.failed(format!("{syntax_errors} file(s) have syntax errors")).await;
        return;
    }
    job.system("Syntax OK.").await;

    // 2. Evaluation + dry build on a staged copy.
    let staged = match stage_copy(&workdir, loc.kind == ConfigKind::Flake) {
        Ok(p) => p,
        Err(e) => {
            job.failed(format!("cannot stage a copy for validation: {e}")).await;
            return;
        }
    };
    let borrowed: Vec<&str> = ["flake.lock", "hardware-configuration.nix"].into_iter().filter(|f| !workdir.join(f).exists() && staged.join(f).exists()).collect();
    if !borrowed.is_empty() {
        job.system(format!("Using {} from {ETC_NIXOS} (not in the working copy), as a build would.", borrowed.join(" and "))).await;
    }
    let sender = job.line_sender();
    let on_line = move |is_err: bool, line: String| {
        let _ = sender.send(LogLine { stream: if is_err { LogStream::Stderr } else { LogStream::Stdout }, text: line });
    };
    let result = match loc.kind {
        ConfigKind::Flake => {
            let host = host.filter(|h| !h.is_empty()).or_else(|| loc.hosts.first().cloned()).unwrap_or_else(|| "main".into());
            let installable = format!("path:{}#nixosConfigurations.{host}.config.system.build.toplevel", staged.to_string_lossy());
            job.system(format!("Evaluating host '{host}' and computing what a build would do (nix build --dry-run)...")).await;
            process::stream("nix", &["build", "--dry-run", "--no-write-lock-file", "--no-link", &installable], Some(&staged), None, on_line).await
        }
        ConfigKind::Legacy => {
            job.system("Evaluating configuration.nix and computing what a build would do (nixos-rebuild dry-build)...").await;
            let cfg = staged.join("configuration.nix").to_string_lossy().to_string();
            // --no-flake: otherwise nixos-rebuild silently prefers /etc/nixos/flake.nix.
            let nixos_config = format!("nixos-config={cfg}");
            process::stream("nixos-rebuild", &["dry-build", "--no-flake", "-I", &nixos_config], Some(&staged), None, on_line).await
        }
        ConfigKind::Unknown => {
            let _ = std::fs::remove_dir_all(&staged);
            job.failed("neither flake.nix nor configuration.nix found").await;
            return;
        }
    };
    let _ = std::fs::remove_dir_all(&staged);
    match result {
        Ok(0) => {
            job.system("Validation passed: the configuration evaluates and can be built.").await;
            job.succeeded().await;
        }
        Ok(code) => job.failed(format!("evaluation failed (exit {code}); see the log above for the error")).await,
        Err(e) => job.failed(e.to_string()).await,
    }
}

