//! Git support for a working folder that is a repository: branches, status,
//! diffs, commits with a generated message, push / fetch / pull.
//!
//! Everything shells out to the `git` CLI (it respects the user's own config,
//! credential helpers and hooks). Rules:
//!   * git must never block on a hidden prompt: terminal prompts are off and
//!     ssh runs in BatchMode unless a password was supplied; then it reaches
//!     git/ssh through an askpass helper that prints an environment variable
//!     (never a file or an argument).
//!   * nothing destructive: no force push, no reset, no branch deletion, pull
//!     is fast-forward only, switching with changes to tracked files is
//!     refused.
//!   * credentials embedded in remote URLs are redacted in every answer.

use crate::error::{CoreError, Result};
use crate::types::{GitBranch, GitCommit, GitFileState, GitFileStatus, GitInfo, GitRemote, GitSuggestion};
use std::collections::BTreeMap;
use std::path::Path;
use std::process::Stdio;
use std::time::Duration;
use tokio::process::Command;

const T: Duration = Duration::from_secs(30);
const T_NET: Duration = Duration::from_secs(240);

pub struct Out {
    pub status: i32,
    pub stdout: String,
    pub stderr: String,
}

/// Run git in `workdir`. With a password, git and ssh can ask for it through
/// an askpass helper; without one every prompt fails immediately instead of
/// hanging the daemon.
async fn git(workdir: &Path, args: &[&str], password: Option<&str>, timeout: Duration) -> Result<Out> {
    let mut cmd = Command::new("git");
    cmd.args(args).current_dir(workdir).stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::piped()).kill_on_drop(true);
    cmd.env("GIT_TERMINAL_PROMPT", "0").env("LC_ALL", "C").env_remove("GIT_DIR").env_remove("GIT_WORK_TREE");
    let mut askpass: Option<std::path::PathBuf> = None;
    match password {
        Some(pw) if !pw.is_empty() => {
            use std::io::Write;
            use std::os::unix::fs::OpenOptionsExt;
            let p = std::env::temp_dir().join(format!("nixos-manager-askpass-{}.sh", uuid::Uuid::new_v4()));
            let mut f = std::fs::OpenOptions::new().write(true).create_new(true).mode(0o700).open(&p)?;
            f.write_all(b"#!/bin/sh\nprintf '%s\\n' \"$NM_GIT_PASSWORD\"\n")?;
            drop(f);
            cmd.env("GIT_ASKPASS", &p).env("SSH_ASKPASS", &p).env("SSH_ASKPASS_REQUIRE", "force").env("NM_GIT_PASSWORD", pw);
            cmd.env("GIT_SSH_COMMAND", "ssh -o BatchMode=no -o ConnectTimeout=20");
            askpass = Some(p);
        }
        _ => {
            cmd.env("GIT_SSH_COMMAND", "ssh -o BatchMode=yes -o ConnectTimeout=20");
        }
    }
    let child = cmd.spawn().map_err(|e| CoreError::Other(format!("failed to start git: {e}")));
    let res = match child {
        Ok(child) => tokio::time::timeout(timeout, child.wait_with_output())
            .await
            .map_err(|_| CoreError::Other(format!("git timed out after {}s", timeout.as_secs())))
            .and_then(|r| r.map_err(|e| CoreError::Other(format!("git failed: {e}")))),
        Err(e) => Err(e),
    };
    if let Some(p) = askpass {
        let _ = std::fs::remove_file(p);
    }
    let out = res?;
    Ok(Out {
        status: out.status.code().unwrap_or(-1),
        stdout: String::from_utf8_lossy(&out.stdout).to_string(),
        stderr: redact(&String::from_utf8_lossy(&out.stderr)),
    })
}

fn ok(out: Out) -> Result<String> {
    if out.status == 0 {
        Ok(out.stdout)
    } else {
        let msg = if out.stderr.trim().is_empty() { out.stdout } else { out.stderr };
        Err(CoreError::Invalid(format!("git: {}", msg.trim())))
    }
}

/// Hide `user:secret@` in URLs (tokens are often embedded in HTTPS remotes).
pub fn redact(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(i) = rest.find("://") {
        let (head, tail) = rest.split_at(i + 3);
        out.push_str(head);
        // credentials end at the first '@' before any '/' or whitespace
        let stop = tail.find(|c: char| c == '/' || c.is_whitespace()).unwrap_or(tail.len());
        if let Some(at) = tail[..stop].rfind('@') {
            out.push_str("***@");
            rest = &tail[at + 1..];
        } else {
            rest = tail;
        }
    }
    out.push_str(rest);
    out
}

async fn is_repo(workdir: &Path) -> Option<String> {
    let out = git(workdir, &["rev-parse", "--show-toplevel"], None, T).await.ok()?;
    if out.status == 0 { Some(out.stdout.trim().to_string()) } else { None }
}

fn parse_status(porcelain: &str) -> (Option<String>, Option<String>, i32, i32, Vec<GitFileStatus>, bool) {
    let mut head = None;
    let mut upstream = None;
    let (mut ahead, mut behind) = (0, 0);
    let mut files = Vec::new();
    let mut detached = false;
    for line in porcelain.lines() {
        if let Some(h) = line.strip_prefix("# branch.head ") {
            if h == "(detached)" { detached = true; } else { head = Some(h.to_string()); }
        } else if let Some(u) = line.strip_prefix("# branch.upstream ") {
            upstream = Some(u.to_string());
        } else if let Some(ab) = line.strip_prefix("# branch.ab ") {
            for part in ab.split_whitespace() {
                if let Some(a) = part.strip_prefix('+') { ahead = a.parse().unwrap_or(0); }
                if let Some(b) = part.strip_prefix('-') { behind = b.parse().unwrap_or(0); }
            }
        } else if let Some(rest) = line.strip_prefix("? ") {
            files.push(GitFileStatus { path: rest.to_string(), state: GitFileState::Untracked, staged: false, old_path: None });
        } else if line.starts_with("1 ") || line.starts_with("2 ") || line.starts_with("u ") {
            let fields: Vec<&str> = line.splitn(if line.starts_with("1 ") { 9 } else if line.starts_with("2 ") { 10 } else { 11 }, ' ').collect();
            let xy = fields.get(1).copied().unwrap_or("..");
            let last = fields.last().copied().unwrap_or("");
            let (path, old_path) = if line.starts_with("2 ") {
                let mut it = last.split('\t');
                let p = it.next().unwrap_or("").to_string();
                (p, it.next().map(|s| s.to_string()))
            } else {
                (last.to_string(), None)
            };
            let x = xy.chars().next().unwrap_or('.');
            let y = xy.chars().nth(1).unwrap_or('.');
            let state = if line.starts_with("u ") {
                GitFileState::Conflict
            } else if x == 'R' || y == 'R' {
                GitFileState::Renamed
            } else if x == 'A' || y == 'A' {
                GitFileState::Added
            } else if x == 'D' || y == 'D' {
                GitFileState::Deleted
            } else {
                GitFileState::Modified
            };
            files.push(GitFileStatus { path, state, staged: x != '.', old_path });
        }
    }
    files.sort_by(|a, b| a.path.cmp(&b.path));
    (head, upstream, ahead, behind, files, detached)
}

pub async fn info(workdir: &Path) -> Result<GitInfo> {
    let Some(root) = is_repo(workdir).await else {
        return Ok(GitInfo { is_repo: false, root: None, branch: None, detached: false, upstream: None, ahead: 0, behind: 0, remotes: vec![], user_name: None, user_email: None, files: vec![], last_commit: None });
    };
    let st = ok(git(workdir, &["status", "--porcelain=v2", "--branch", "--untracked-files=all"], None, T).await?)?;
    let (branch, upstream, ahead, behind, files, detached) = parse_status(&st);
    let cfg = |k: &'static str| async move {
        git(workdir, &["config", "--get", k], None, T).await.ok().filter(|o| o.status == 0).map(|o| o.stdout.trim().to_string()).filter(|s| !s.is_empty())
    };
    let user_name = cfg("user.name").await;
    let user_email = cfg("user.email").await;
    let mut remotes: Vec<GitRemote> = Vec::new();
    if let Ok(o) = git(workdir, &["remote", "-v"], None, T).await {
        for l in o.stdout.lines() {
            let mut it = l.split_whitespace();
            if let (Some(name), Some(url)) = (it.next(), it.next()) {
                if !remotes.iter().any(|r| r.name == name) {
                    remotes.push(GitRemote { name: name.to_string(), url: redact(url) });
                }
            }
        }
    }
    let last_commit = git(workdir, &["log", "-1", "--format=%H%x00%s%x00%cI%x00%an"], None, T).await.ok().filter(|o| o.status == 0).and_then(|o| {
        let parts: Vec<&str> = o.stdout.trim_end().split('\0').collect();
        (parts.len() >= 4).then(|| GitCommit { hash: parts[0].to_string(), subject: parts[1].to_string(), date: parts[2].to_string(), author: parts[3].to_string() })
    });
    Ok(GitInfo { is_repo: true, root: Some(root), branch, detached, upstream, ahead, behind, remotes, user_name, user_email, files, last_commit })
}

pub async fn branches(workdir: &Path) -> Result<Vec<GitBranch>> {
    let out = ok(git(workdir, &["branch", "-a", "--format=%(refname:short)%00%(HEAD)%00%(upstream:short)%00%(refname)"], None, T).await?)?;
    let mut locals: Vec<GitBranch> = Vec::new();
    let mut remotes: Vec<GitBranch> = Vec::new();
    for l in out.lines() {
        let p: Vec<&str> = l.split('\0').collect();
        if p.len() < 4 { continue; }
        let (short, head, upstream, full) = (p[0], p[1], p[2], p[3]);
        if full.starts_with("refs/remotes/") {
            if short.ends_with("/HEAD") { continue; }
            // "origin/x" → remote-only candidate "x"
            let name = short.splitn(2, '/').nth(1).unwrap_or(short).to_string();
            remotes.push(GitBranch { name, current: false, upstream: Some(short.to_string()), remote_only: true });
        } else {
            locals.push(GitBranch { name: short.to_string(), current: head == "*", upstream: (!upstream.is_empty()).then(|| upstream.to_string()), remote_only: false });
        }
    }
    for r in remotes {
        if !locals.iter().any(|l| l.name == r.name) {
            locals.push(r);
        }
    }
    locals.sort_by(|a, b| b.current.cmp(&a.current).then(a.remote_only.cmp(&b.remote_only)).then(a.name.cmp(&b.name)));
    Ok(locals)
}

fn tracked_changes(files: &[GitFileStatus]) -> usize {
    files.iter().filter(|f| f.state != GitFileState::Untracked).count()
}

/// Switch to a local branch, or check out a remote branch as a tracking branch.
pub async fn switch(workdir: &Path, name: &str) -> Result<GitInfo> {
    check_branch_name(workdir, name).await?;
    let cur = info(workdir).await?;
    let n = tracked_changes(&cur.files);
    if n > 0 {
        return Err(CoreError::Conflict(format!("{n} changed file(s) are not committed; commit them first (or switch from a terminal with stash)")));
    }
    ok(git(workdir, &["switch", name], None, T).await?)?;
    info(workdir).await
}

pub async fn create_branch(workdir: &Path, name: &str) -> Result<GitInfo> {
    check_branch_name(workdir, name).await?;
    ok(git(workdir, &["switch", "-c", name], None, T).await?)?;
    info(workdir).await
}

async fn check_branch_name(workdir: &Path, name: &str) -> Result<()> {
    if name.trim().is_empty() || name.starts_with('-') {
        return Err(CoreError::Invalid("branch name is empty or starts with '-'".into()));
    }
    let o = git(workdir, &["check-ref-format", "--branch", name], None, T).await?;
    if o.status != 0 {
        return Err(CoreError::Invalid(format!("'{name}' is not a valid branch name")));
    }
    Ok(())
}

/// Unified diff of one file against HEAD (untracked files show as all-added).
pub async fn diff(workdir: &Path, file: &str) -> Result<String> {
    if file.starts_with('-') || file.contains("..") && file.split('/').any(|c| c == "..") {
        return Err(CoreError::Invalid("bad file path".into()));
    }
    let tracked = git(workdir, &["ls-files", "--error-unmatch", "--", file], None, T).await?.status == 0;
    let o = if tracked {
        git(workdir, &["diff", "HEAD", "--", file], None, T).await?
    } else {
        git(workdir, &["diff", "--no-index", "--", "/dev/null", file], None, T).await?
    };
    if o.status == 0 || o.status == 1 {
        Ok(o.stdout)
    } else {
        Err(CoreError::Invalid(format!("git: {}", o.stderr.trim())))
    }
}

// ───────────────────────── commit message suggestion ─────────────────────────

/// Flatten a Nix file into "attribute path → value text" for comparison.
fn settings_of(source: &str) -> BTreeMap<String, String> {
    let view = crate::nixparse::analyze_file("x", source.to_string());
    let mut out = BTreeMap::new();
    fn walk(node: &crate::types::Node, prefix: &str, out: &mut BTreeMap<String, String>) {
        for c in &node.children {
            let path = match (&c.label, prefix) {
                (Some(l), "") => l.clone(),
                (Some(l), p) => format!("{p}.{l}"),
                (None, p) => p.to_string(),
            };
            let is_set = matches!(c.kind, crate::types::NodeKind::Attrset) && !c.children.is_empty();
            if is_set && c.label.is_some() {
                walk(c, &path, out);
            } else if c.label.is_some() {
                let v = c.value.clone().or_else(|| c.source.clone()).unwrap_or_else(|| {
                    // lists and attrsets without raw source: summarise by children
                    c.children.iter().map(|x| x.value.clone().or_else(|| x.source.clone()).unwrap_or_default()).collect::<Vec<_>>().join(" ")
                });
                out.insert(path, v);
            }
        }
    }
    walk(&view.root, "", &mut out);
    out
}

fn describe_nix_change(old: &str, new: &str) -> Vec<String> {
    let a = settings_of(old);
    let b = settings_of(new);
    let mut added = Vec::new();
    let mut removed = Vec::new();
    let mut changed = Vec::new();
    for (k, v) in &b {
        match a.get(k) {
            None => added.push(k.clone()),
            Some(ov) if ov != v => changed.push(k.clone()),
            _ => {}
        }
    }
    for k in a.keys() {
        if !b.contains_key(k) {
            removed.push(k.clone());
        }
    }
    // collapse children of an added/removed set to the shortest common prefixes
    let mut parts = Vec::new();
    let cap = |v: Vec<String>, verb: &str| -> Option<String> {
        if v.is_empty() { return None; }
        let shown: Vec<String> = v.iter().take(4).cloned().collect();
        let more = v.len().saturating_sub(4);
        Some(format!("{verb} {}{}", shown.join(", "), if more > 0 { format!(" (+{more} more)") } else { String::new() }))
    };
    if let Some(s) = cap(added, "add") { parts.push(s); }
    if let Some(s) = cap(changed, "change") { parts.push(s); }
    if let Some(s) = cap(removed, "remove") { parts.push(s); }
    parts
}

pub async fn suggest_message(workdir: &Path) -> Result<GitSuggestion> {
    let cur = info(workdir).await?;
    if !cur.is_repo {
        return Err(CoreError::Invalid("not a git repository".into()));
    }
    let mut lines: Vec<String> = Vec::new();
    let mut names: Vec<String> = Vec::new();
    for f in &cur.files {
        let name = f.path.clone();
        names.push(name.clone());
        let is_nix = name.ends_with(".nix");
        let detail = match f.state {
            GitFileState::Modified | GitFileState::Renamed if is_nix => {
                let old_path = f.old_path.clone().unwrap_or_else(|| name.clone());
                let old = git(workdir, &["show", &format!("HEAD:{old_path}")], None, T).await.ok().filter(|o| o.status == 0).map(|o| o.stdout).unwrap_or_default();
                let new = std::fs::read_to_string(workdir.join(&name)).unwrap_or_default();
                let parts = describe_nix_change(&old, &new);
                if parts.is_empty() { "update (formatting or comments)".to_string() } else { parts.join("; ") }
            }
            GitFileState::Added | GitFileState::Untracked if is_nix => {
                let new = std::fs::read_to_string(workdir.join(&name)).unwrap_or_default();
                let n = settings_of(&new).len();
                if n > 0 { format!("add ({n} settings)") } else { "add".to_string() }
            }
            GitFileState::Added | GitFileState::Untracked => "add".to_string(),
            GitFileState::Deleted => "remove".to_string(),
            GitFileState::Renamed => format!("rename from {}", f.old_path.clone().unwrap_or_default()),
            GitFileState::Conflict => "resolve conflict".to_string(),
            GitFileState::Modified => "update".to_string(),
        };
        lines.push(format!("- {name}: {detail}"));
    }
    let subject = match names.len() {
        0 => String::new(),
        1 => {
            let d = lines[0].splitn(2, ": ").nth(1).unwrap_or("").to_string();
            let short = if d.len() > 60 { format!("{}…", &d[..57]) } else { d };
            let verb = match cur.files[0].state { GitFileState::Added | GitFileState::Untracked => "Add", GitFileState::Deleted => "Remove", _ => "Update" };
            if short.is_empty() || short.starts_with("add") && matches!(cur.files[0].state, GitFileState::Added | GitFileState::Untracked) {
                format!("{verb} {}", names[0])
            } else {
                format!("{verb} {}: {short}", names[0])
            }
        }
        n => {
            let shown: Vec<String> = names.iter().take(3).cloned().collect();
            format!("Update {}{}", shown.join(", "), if n > 3 { format!(" (+{} more)", n - 3) } else { String::new() })
        }
    };
    let body = if names.len() > 1 || (names.len() == 1 && subject.len() >= 60) { lines.join("\n") } else { String::new() };
    Ok(GitSuggestion { subject, body })
}

// ───────────────────────── commit / identity / network ─────────────────────────

/// Throw away uncommitted work. `files` = None: `git reset --hard` (every
/// tracked file back to HEAD; with `clean_untracked` also `git clean -fd`).
/// `files` = Some: only those paths — tracked ones are restored from HEAD,
/// untracked ones are deleted. Returns the refreshed status.
pub async fn discard(workdir: &Path, files: Option<&[String]>, clean_untracked: bool) -> Result<GitInfo> {
    let cur = info(workdir).await?;
    if !cur.is_repo {
        return Err(CoreError::Invalid("not a git repository".into()));
    }
    match files {
        None => {
            ok(git(workdir, &["reset", "--hard", "-q"], None, T).await?)?;
            if clean_untracked {
                ok(git(workdir, &["clean", "-fd", "-q"], None, T).await?)?;
            }
        }
        Some(list) => {
            if list.is_empty() {
                return Err(CoreError::Invalid("no files selected".into()));
            }
            for f in list {
                if f.starts_with('-') || f.starts_with('/') || f.split('/').any(|c| c == "..") {
                    return Err(CoreError::Invalid(format!("bad file path: {f}")));
                }
            }
            let known: std::collections::HashMap<&str, GitFileState> = cur.files.iter().map(|f| (f.path.as_str(), f.state)).collect();
            let mut tracked: Vec<&str> = Vec::new();
            for f in list {
                match known.get(f.as_str()) {
                    Some(GitFileState::Untracked) => {
                        let full = workdir.join(f);
                        if full.is_dir() { std::fs::remove_dir_all(&full)?; } else { std::fs::remove_file(&full)?; }
                    }
                    Some(_) => tracked.push(f.as_str()),
                    None => return Err(CoreError::NotFound(format!("{f} has no changes"))),
                }
            }
            if !tracked.is_empty() {
                let mut args = vec!["restore", "--source=HEAD", "--staged", "--worktree", "--"];
                args.extend(tracked);
                ok(git(workdir, &args, None, T).await?)?;
            }
        }
    }
    info(workdir).await
}

/// `git init` + a .gitignore for the machine-owned files + a first commit
/// (when an identity exists; otherwise the files are only staged).
pub async fn init_repo(workdir: &Path) -> Result<GitInfo> {
    if is_repo(workdir).await.is_some() {
        return Err(CoreError::Conflict("this folder is already a git repository".into()));
    }
    ok(git(workdir, &["init", "-q", "--initial-branch=main"], None, T).await?)?;
    let gi = workdir.join(".gitignore");
    if !gi.exists() {
        std::fs::write(&gi, "# machine-owned files managed by Nixos-manager\nhardware-configuration.nix\nflake.lock\nresult\n")?;
    }
    ok(git(workdir, &["add", "-A", "--", "."], None, T).await?)?;
    let cur = info(workdir).await?;
    if cur.user_name.is_some() && cur.user_email.is_some() {
        let _ = git(workdir, &["commit", "-q", "-m", "Initial configuration"], None, T).await;
    }
    info(workdir).await
}

/// `git remote add <name> <url>` (the URL may embed credentials; it is never echoed back unredacted).
pub async fn add_remote(workdir: &Path, name: &str, url: &str) -> Result<GitInfo> {
    let name = name.trim();
    let url = url.trim();
    if name.is_empty() || name.starts_with('-') || name.contains(char::is_whitespace) {
        return Err(CoreError::Invalid("remote name must be a single word, e.g. origin".into()));
    }
    if url.is_empty() || url.starts_with('-') {
        return Err(CoreError::Invalid("remote URL is required".into()));
    }
    ok(git(workdir, &["remote", "add", name, url], None, T).await?)?;
    info(workdir).await
}

pub async fn set_identity(workdir: &Path, name: &str, email: &str) -> Result<GitInfo> {
    if name.trim().is_empty() || email.trim().is_empty() || !email.contains('@') {
        return Err(CoreError::Invalid("name and a valid e-mail are needed".into()));
    }
    ok(git(workdir, &["config", "user.name", name.trim()], None, T).await?)?;
    ok(git(workdir, &["config", "user.email", email.trim()], None, T).await?)?;
    info(workdir).await
}

/// Stage the given paths (all changes when None) and commit.
pub async fn commit(workdir: &Path, message: &str, paths: Option<&[String]>) -> Result<GitInfo> {
    let cur = info(workdir).await?;
    if !cur.is_repo {
        return Err(CoreError::Invalid("not a git repository".into()));
    }
    if cur.user_name.is_none() || cur.user_email.is_none() {
        return Err(CoreError::Conflict("git needs your name and e-mail for commits".into()));
    }
    if message.trim().is_empty() {
        return Err(CoreError::Invalid("the commit message is empty".into()));
    }
    let mut add: Vec<String> = vec!["add".into(), "-A".into(), "--".into()];
    match paths {
        Some(p) if !p.is_empty() => {
            for x in p {
                if x.starts_with('-') { return Err(CoreError::Invalid("bad file path".into())); }
                add.push(x.clone());
            }
        }
        Some(_) => return Err(CoreError::Invalid("no files selected".into())),
        None => add.push(".".into()),
    }
    let add_ref: Vec<&str> = add.iter().map(|s| s.as_str()).collect();
    ok(git(workdir, &add_ref, None, T).await?)?;
    let o = git(workdir, &["commit", "-m", message], None, T).await?;
    if o.status != 0 {
        let m = format!("{}{}", o.stdout, o.stderr);
        if m.contains("nothing to commit") || m.contains("no changes added") {
            return Err(CoreError::Invalid("nothing to commit".into()));
        }
        return Err(CoreError::Invalid(format!("git: {}", m.trim())));
    }
    info(workdir).await
}

fn needs_credentials(msg: &str) -> bool {
    let m = msg.to_ascii_lowercase();
    m.contains("authentication failed") || m.contains("permission denied") || m.contains("could not read username") || m.contains("could not read password") || m.contains("terminal prompts disabled") || m.contains("passphrase") || m.contains("host key verification failed")
}

async fn net(workdir: &Path, args: &[&str], password: Option<&str>) -> Result<String> {
    let o = git(workdir, args, password, T_NET).await?;
    if o.status == 0 {
        Ok(format!("{}{}", o.stdout, o.stderr).trim().to_string())
    } else {
        let m = format!("{}{}", o.stdout, o.stderr).trim().to_string();
        if needs_credentials(&m) {
            Err(CoreError::Forbidden(format!("git needs a password or key passphrase: {m}")))
        } else {
            Err(CoreError::Invalid(format!("git: {m}")))
        }
    }
}

pub async fn push(workdir: &Path, password: Option<&str>) -> Result<String> {
    let cur = info(workdir).await?;
    if cur.detached || cur.branch.is_none() {
        return Err(CoreError::Invalid("not on a branch (detached HEAD); create a branch first".into()));
    }
    if cur.remotes.is_empty() {
        return Err(CoreError::Invalid("this repository has no remote; add one with  git remote add origin <url>".into()));
    }
    if cur.upstream.is_some() {
        net(workdir, &["push"], password).await
    } else {
        let remote = cur.remotes.iter().find(|r| r.name == "origin").map(|r| r.name.clone()).unwrap_or_else(|| cur.remotes[0].name.clone());
        net(workdir, &["push", "-u", &remote, "HEAD"], password).await
    }
}

pub async fn fetch(workdir: &Path, password: Option<&str>) -> Result<String> {
    net(workdir, &["fetch", "--prune"], password).await
}

pub async fn pull(workdir: &Path, password: Option<&str>) -> Result<String> {
    let cur = info(workdir).await?;
    if tracked_changes(&cur.files) > 0 {
        return Err(CoreError::Conflict("commit your changes before pulling".into()));
    }
    if cur.upstream.is_none() {
        return Err(CoreError::Invalid("this branch has no upstream yet; push it first".into()));
    }
    match net(workdir, &["pull", "--ff-only"], password).await {
        Err(CoreError::Invalid(m)) if m.contains("fast-forward") || m.contains("diverg") => Err(CoreError::Invalid("the branch and its remote have diverged; this app only fast-forwards. Resolve it from a terminal (git pull --rebase)".into())),
        r => r,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn redacts_embedded_credentials() {
        assert_eq!(redact("https://ghp_abc123@github.com/u/r.git"), "https://***@github.com/u/r.git");
        assert_eq!(redact("https://user:pw@host/x y https://host/z"), "https://***@host/x y https://host/z");
        assert_eq!(redact("git@github.com:u/r.git"), "git@github.com:u/r.git");
        assert_eq!(redact("fatal: unable to access 'https://tok@gh.com/a/b/': 403"), "fatal: unable to access 'https://***@gh.com/a/b/': 403");
    }

    #[test]
    fn parses_porcelain_v2() {
        let s = "# branch.oid abc\n# branch.head main\n# branch.upstream origin/main\n# branch.ab +2 -1\n1 .M N... 100644 100644 100644 aaa bbb users.nix\n1 A. N... 000000 100644 100644 000 111 new file.nix\n2 R. N... 100644 100644 100644 a b R100 renamed.nix\told.nix\n? untracked.txt\nu UU N... 100644 100644 100644 100644 a b c conflict.nix\n";
        let (head, up, a, b, files, det) = parse_status(s);
        assert_eq!(head.as_deref(), Some("main"));
        assert_eq!(up.as_deref(), Some("origin/main"));
        assert_eq!((a, b), (2, 1));
        assert!(!det);
        let get = |p: &str| files.iter().find(|f| f.path == p).unwrap();
        assert_eq!(get("users.nix").state, GitFileState::Modified);
        assert!(!get("users.nix").staged);
        assert_eq!(get("new file.nix").state, GitFileState::Added);
        assert!(get("new file.nix").staged);
        assert_eq!(get("renamed.nix").old_path.as_deref(), Some("old.nix"));
        assert_eq!(get("untracked.txt").state, GitFileState::Untracked);
        assert_eq!(get("conflict.nix").state, GitFileState::Conflict);
        let (_, _, _, _, _, det) = parse_status("# branch.head (detached)\n");
        assert!(det);
    }

    #[test]
    fn describes_nix_changes_by_option_path() {
        let old = "{ config, pkgs, ... }: {\n  services.openssh.enable = true;\n  users.users.vincent = { shell = pkgs.bash; isNormalUser = true; };\n  environment.systemPackages = [ pkgs.htop ];\n}\n";
        let new = "{ config, pkgs, ... }: {\n  services.openssh.enable = true;\n  users.users.vincent = { shell = pkgs.zsh; isNormalUser = true; };\n  nix.settings.trusted-users = [ \"vincent\" ];\n}\n";
        let parts = describe_nix_change(old, new);
        let joined = parts.join("; ");
        assert!(joined.contains("add nix.settings.trusted-users"), "{joined}");
        assert!(joined.contains("change users.users.vincent.shell"), "{joined}");
        assert!(joined.contains("remove environment.systemPackages"), "{joined}");
        assert!(!joined.contains("openssh"), "{joined}");
        assert!(describe_nix_change(old, old).is_empty());
    }

    #[tokio::test]
    async fn end_to_end_on_a_scratch_repo() {
        // isolate from the developer's global git config (identity, signing, hooks)
        std::env::set_var("GIT_CONFIG_GLOBAL", "/dev/null");
        std::env::set_var("GIT_CONFIG_NOSYSTEM", "1");
        let d = std::env::temp_dir().join(format!("nm-git-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        let work = d.join("work");
        let bare = d.join("remote.git");
        std::fs::create_dir_all(&work).unwrap();
        std::fs::create_dir_all(&bare).unwrap();
        let sh = |args: &[&str], cwd: &Path| {
            let st = std::process::Command::new("git").args(args).current_dir(cwd).env("GIT_TERMINAL_PROMPT", "0").output().unwrap();
            assert!(st.status.success(), "git {:?}: {}", args, String::from_utf8_lossy(&st.stderr));
        };
        sh(&["init", "--bare", "-q", "--initial-branch=main"], &bare);
        sh(&["init", "-q", "--initial-branch=main"], &work);
        sh(&["config", "commit.gpgsign", "false"], &work);
        assert!(info(&work).await.unwrap().is_repo);
        std::fs::write(work.join("configuration.nix"), "{ ... }: {\n  services.openssh.enable = true;\n}\n").unwrap();
        // identity missing → Conflict
        let e = commit(&work, "x", None).await.unwrap_err();
        assert!(matches!(e, CoreError::Conflict(_)), "{e:?}");
        set_identity(&work, "Test User", "t@example.com").await.unwrap();
        let sug = suggest_message(&work).await.unwrap();
        assert_eq!(sug.subject, "Add configuration.nix", "{sug:?}");
        let i = commit(&work, &sug.subject, None).await.unwrap();
        assert!(i.files.is_empty() && i.last_commit.is_some());
        // no remote → clear error
        let e = push(&work, None).await.unwrap_err();
        assert!(e.to_string().contains("no remote"), "{e}");
        sh(&["remote", "add", "origin", bare.to_str().unwrap()], &work);
        push(&work, None).await.unwrap();
        assert_eq!(info(&work).await.unwrap().upstream.as_deref(), Some("origin/main"));
        // change + suggestion with option paths
        std::fs::write(work.join("configuration.nix"), "{ ... }: {\n  services.openssh.enable = true;\n  nix.settings.trusted-users = [ \"vincent\" ];\n}\n").unwrap();
        std::fs::write(work.join("notes.md"), "hi\n").unwrap();
        let i = info(&work).await.unwrap();
        assert_eq!(i.files.len(), 2);
        let sug = suggest_message(&work).await.unwrap();
        assert!(sug.subject.starts_with("Update configuration.nix, notes.md"), "{sug:?}");
        assert!(sug.body.contains("add nix.settings.trusted-users"), "{sug:?}");
        let dif = diff(&work, "configuration.nix").await.unwrap();
        assert!(dif.contains("+  nix.settings.trusted-users"));
        let dif = diff(&work, "notes.md").await.unwrap();
        assert!(dif.contains("+hi"));
        // switching with tracked changes is refused; creating a branch is fine
        let e = switch(&work, "main").await.unwrap_err();
        assert!(matches!(e, CoreError::Conflict(_)));
        let i = create_branch(&work, "laptop").await.unwrap();
        assert_eq!(i.branch.as_deref(), Some("laptop"));
        // partial commit
        let i = commit(&work, "Only the config", Some(&["configuration.nix".to_string()])).await.unwrap();
        assert_eq!(i.files.len(), 1, "{:?}", i.files);
        assert_eq!(i.files[0].path, "notes.md");
        push(&work, None).await.unwrap();
        let br = branches(&work).await.unwrap();
        assert!(br.iter().any(|b| b.name == "laptop" && b.current && b.upstream.as_deref() == Some("origin/laptop")), "{br:?}");
        assert!(br.iter().any(|b| b.name == "main" && !b.current));
        // discard: a modified tracked file and an untracked one
        std::fs::write(work.join("configuration.nix"), "{ ... }: { }\n").unwrap();
        std::fs::write(work.join("junk.txt"), "x\n").unwrap();
        let i = discard(&work, Some(&["configuration.nix".to_string()]), false).await.unwrap();
        assert!(std::fs::read_to_string(work.join("configuration.nix")).unwrap().contains("trusted-users"), "restored from HEAD");
        assert!(i.files.iter().any(|f| f.path == "junk.txt"), "untracked file untouched by a per-file discard of another file");
        let i = discard(&work, None, false).await.unwrap();
        assert!(i.files.iter().any(|f| f.path == "junk.txt"), "reset --hard leaves untracked files");
        let i = discard(&work, None, true).await.unwrap();
        assert!(!i.files.iter().any(|f| f.path == "junk.txt"), "clean removes them");
        // untracked files do not block switching
        std::fs::write(work.join("notes.md"), "hi\n").unwrap();
        let i = switch(&work, "main").await.unwrap();
        assert_eq!(i.branch.as_deref(), Some("main"));
        fetch(&work, None).await.unwrap();
        pull(&work, None).await.unwrap();
        assert!(!create_branch(&work, "bad name").await.is_ok());
        let _ = std::fs::remove_dir_all(&d);
    }
}
