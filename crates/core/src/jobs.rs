//! Registry of long-running jobs (deploys, evaluations) with log streaming.

use crate::types::{JobDetail, JobEvent, JobInfo, JobStatus, LogLine, LogStream};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::{broadcast, RwLock};

const MAX_JOBS: usize = 50;

struct JobRecord {
    info: JobInfo,
    log: Vec<LogLine>,
    cancel: Arc<Cancel>,
    file: Option<std::fs::File>,
}

/// Directory where every job log is also written as a text file
/// (~/.local/state/nixos-manager/logs). Lets the user read a build log
/// after a reboot or a daemon restart.
pub fn logs_dir() -> std::path::PathBuf {
    let base = directories::ProjectDirs::from("", "", "nixos-manager")
        .and_then(|d| d.state_dir().map(|p| p.to_path_buf()))
        .unwrap_or_else(|| std::env::temp_dir().join("nixos-manager"));
    base.join("logs")
}

const MAX_LOG_FILES: usize = 60;

fn open_log_file(id: &str, kind: &str, title: &str, started: &str) -> Option<(std::fs::File, std::path::PathBuf)> {
    use std::io::Write;
    let dir = logs_dir();
    std::fs::create_dir_all(&dir).ok()?;
    // prune the oldest files beyond the limit
    if let Ok(rd) = std::fs::read_dir(&dir) {
        let mut files: Vec<std::path::PathBuf> = rd.flatten().map(|e| e.path()).filter(|p| p.extension().map(|e| e == "log").unwrap_or(false)).collect();
        files.sort();
        while files.len() >= MAX_LOG_FILES {
            let _ = std::fs::remove_file(files.remove(0));
        }
    }
    let stamp = chrono::Local::now().format("%Y%m%d-%H%M%S");
    let path = dir.join(format!("{stamp}-{kind}-{}.log", &id[..8.min(id.len())]));
    let mut f = std::fs::OpenOptions::new().create_new(true).append(true).open(&path).ok()?;
    let _ = writeln!(f, "# title: {title}\n# kind: {kind}\n# started: {started}\n# id: {id}");
    Some((f, path))
}

pub fn parse_log_header(text: &str) -> (Option<String>, Option<String>, Option<String>) {
    let mut title = None;
    let mut started = None;
    let mut status = None;
    for l in text.lines().take(6) {
        if let Some(t) = l.strip_prefix("# title: ") { title = Some(t.to_string()); }
        if let Some(t) = l.strip_prefix("# started: ") { started = Some(t.to_string()); }
    }
    for l in text.lines().rev().take(3) {
        if let Some(t) = l.strip_prefix("# status: ") { status = Some(t.to_string()); }
    }
    (title, started, status)
}

/// Stop support for one job. Two mechanisms, because the local build runs
/// as root under sudo where the daemon (a plain user) cannot signal it:
///   * `stop_file`: deploy scripts poll for this file around nixos-rebuild
///     and terminate its process group themselves (works for root scripts);
///   * `pgid`: user-level jobs run in their own process group, which the
///     daemon signals directly (SIGTERM, then SIGKILL after a grace period).
pub struct Cancel {
    pub requested: std::sync::atomic::AtomicBool,
    pub pgid: std::sync::Mutex<Option<i32>>,
    pub stop_file: std::path::PathBuf,
    /// The job's script polls `stop_file` and terminates its own children.
    /// Then the daemon must NOT signal the process group right away: that
    /// would kill the polling wrapper first and orphan the real work (which
    /// also keeps the log pipe open). The group is signalled only as a
    /// fallback after a grace period.
    pub polls_stop_file: std::sync::atomic::AtomicBool,
}

impl Cancel {
    fn new(id: &str) -> Self {
        Self { requested: Default::default(), pgid: Default::default(), stop_file: std::env::temp_dir().join(format!("nixos-manager-stop-{id}")), polls_stop_file: Default::default() }
    }
    pub fn is_requested(&self) -> bool {
        self.requested.load(std::sync::atomic::Ordering::SeqCst)
    }
}

#[derive(Clone)]
pub struct JobManager {
    inner: Arc<RwLock<Inner>>,
    tx: broadcast::Sender<JobEvent>,
}

struct Inner {
    jobs: HashMap<String, JobRecord>,
    order: Vec<String>,
}

impl Default for JobManager {
    fn default() -> Self {
        Self::new()
    }
}

impl JobManager {
    pub fn new() -> Self {
        let (tx, _) = broadcast::channel(4096);
        Self { inner: Arc::new(RwLock::new(Inner { jobs: HashMap::new(), order: Vec::new() })), tx }
    }

    pub fn subscribe(&self) -> broadcast::Receiver<JobEvent> {
        self.tx.subscribe()
    }

    pub async fn create(&self, kind: &str, title: &str) -> JobHandle {
        let id = uuid::Uuid::new_v4().to_string();
        let started = chrono::Utc::now().to_rfc3339();
        let file = open_log_file(&id, kind, title, &started);
        let info = JobInfo {
            id: id.clone(),
            kind: kind.to_string(),
            title: title.to_string(),
            status: JobStatus::Pending,
            started_at: started,
            finished_at: None,
            error: None,
            log_lines: 0,
            log_file: file.as_ref().map(|(_, p)| p.to_string_lossy().to_string()),
        };
        {
            let mut inner = self.inner.write().await;
            inner.jobs.insert(id.clone(), JobRecord { info: info.clone(), log: Vec::new(), cancel: Arc::new(Cancel::new(&id)), file: file.map(|(f, _)| f) });
            inner.order.push(id.clone());
            while inner.order.len() > MAX_JOBS {
                let old = inner.order.remove(0);
                inner.jobs.remove(&old);
            }
        }
        let _ = self.tx.send(JobEvent::Created { job: info });
        JobHandle { id, mgr: self.clone() }
    }

    pub async fn list(&self) -> Vec<JobInfo> {
        let inner = self.inner.read().await;
        inner.order.iter().rev().filter_map(|id| inner.jobs.get(id).map(|j| j.info.clone())).collect()
    }

    pub async fn get(&self, id: &str) -> Option<JobDetail> {
        let inner = self.inner.read().await;
        inner.jobs.get(id).map(|j| JobDetail { info: j.info.clone(), log: j.log.clone() })
    }

    /// Logs kept on disk, newest first.
    pub fn history() -> Vec<crate::types::JobLogFile> {
        let dir = logs_dir();
        let mut out = Vec::new();
        if let Ok(rd) = std::fs::read_dir(&dir) {
            for e in rd.flatten() {
                let p = e.path();
                if p.extension().map(|x| x != "log").unwrap_or(true) { continue; }
                let name = p.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
                let size = e.metadata().map(|m| m.len()).unwrap_or(0);
                // header lines + last lines only
                let text = std::fs::read_to_string(&p).unwrap_or_default();
                let (title, started_at, status) = parse_log_header(&text);
                out.push(crate::types::JobLogFile { title: title.unwrap_or_else(|| name.clone()), name, status, started_at, size });
            }
        }
        out.sort_by(|a, b| b.name.cmp(&a.name));
        out
    }

    pub fn history_read(name: &str) -> crate::error::Result<String> {
        if name.is_empty() || name.contains('/') || name.contains("..") || !name.ends_with(".log") {
            return Err(crate::error::CoreError::Invalid("bad log name".into()));
        }
        std::fs::read_to_string(logs_dir().join(name)).map_err(|_| crate::error::CoreError::NotFound(format!("log {name}")))
    }

    /// Last successful build/deploy of `workdir` (from the on-disk log
    /// headers) and the .nix files modified after it.
    pub fn build_status(workdir: &std::path::Path) -> crate::types::BuildStatus {
        let needle = format!("({})", workdir.display());
        let mut best: Option<(String, String)> = None;
        for h in Self::history() {
            if h.status.as_deref().map(|s| s.starts_with("succeeded")).unwrap_or(false) && h.title.contains(&needle) && h.title.starts_with("nixos-rebuild") {
                if let Some(at) = h.started_at.clone() {
                    if best.as_ref().map(|(b, _)| at > *b).unwrap_or(true) {
                        best = Some((at, h.title.clone()));
                    }
                }
            }
        }
        let Some((at, title)) = best else {
            return crate::types::BuildStatus { last_build_at: None, last_build_title: None, changed_files: vec![] };
        };
        let since = chrono::DateTime::parse_from_rfc3339(&at).map(|d| d.with_timezone(&chrono::Utc)).ok();
        let mut changed = Vec::new();
        if let Some(since) = since {
            for (rel, _) in crate::discover::list_files(workdir) {
                if !rel.ends_with(".nix") || rel.ends_with("hardware-configuration.nix") {
                    continue;
                }
                if let Ok(m) = std::fs::metadata(workdir.join(&rel)).and_then(|m| m.modified()) {
                    if chrono::DateTime::<chrono::Utc>::from(m) > since {
                        changed.push(rel);
                    }
                }
            }
        }
        crate::types::BuildStatus { last_build_at: Some(at), last_build_title: Some(title), changed_files: changed }
    }

    /// Delete one log file (never the one of a job that is still running).
    pub async fn history_delete(&self, name: &str) -> crate::error::Result<()> {
        if name.is_empty() || name.contains('/') || name.contains("..") || !name.ends_with(".log") {
            return Err(crate::error::CoreError::Invalid("bad log name".into()));
        }
        let path = logs_dir().join(name);
        if self.is_log_in_use(&path).await {
            return Err(crate::error::CoreError::Conflict("that log belongs to a job that is still running".into()));
        }
        std::fs::remove_file(&path).map_err(|_| crate::error::CoreError::NotFound(format!("log {name}")))
    }

    /// Delete every log file except those of running jobs. Returns (deleted, bytes freed).
    pub async fn history_clear(&self) -> (usize, u64) {
        let mut n = 0;
        let mut bytes = 0;
        if let Ok(rd) = std::fs::read_dir(logs_dir()) {
            for e in rd.flatten() {
                let p = e.path();
                if p.extension().map(|x| x != "log").unwrap_or(true) || self.is_log_in_use(&p).await {
                    continue;
                }
                let size = e.metadata().map(|m| m.len()).unwrap_or(0);
                if std::fs::remove_file(&p).is_ok() {
                    n += 1;
                    bytes += size;
                }
            }
        }
        (n, bytes)
    }

    async fn is_log_in_use(&self, path: &std::path::Path) -> bool {
        let p = path.to_string_lossy().to_string();
        self.inner.read().await.jobs.values().any(|j| matches!(j.info.status, JobStatus::Running | JobStatus::Pending) && j.info.log_file.as_deref() == Some(p.as_str()))
    }

    async fn is_active(&self, id: &str) -> bool {
        matches!(self.inner.read().await.jobs.get(id).map(|j| j.info.status), Some(JobStatus::Running | JobStatus::Pending))
    }

    pub async fn cancel_of(&self, id: &str) -> Option<Arc<Cancel>> {
        self.inner.read().await.jobs.get(id).map(|j| j.cancel.clone())
    }

    /// Ask a running job to stop. Creates the stop file (root scripts poll
    /// it) and signals the job's process group when the daemon owns it.
    pub async fn stop(&self, id: &str) -> crate::error::Result<JobInfo> {
        let (info, cancel) = {
            let inner = self.inner.read().await;
            let j = inner.jobs.get(id).ok_or_else(|| crate::error::CoreError::NotFound(format!("job {id}")))?;
            (j.info.clone(), j.cancel.clone())
        };
        if !matches!(info.status, JobStatus::Running | JobStatus::Pending) {
            return Err(crate::error::CoreError::Conflict("the job is not running".into()));
        }
        let first = !cancel.requested.swap(true, std::sync::atomic::Ordering::SeqCst);
        let _ = std::fs::write(&cancel.stop_file, b"stop\n");
        if first {
            self.push_log(id, LogLine { stream: LogStream::System, text: "Stop requested by the user; terminating the running step (it may take a few seconds)...".into() }).await;
        }
        let pgid = *cancel.pgid.lock().unwrap();
        let polls = cancel.polls_stop_file.load(std::sync::atomic::Ordering::SeqCst);
        if let Some(pg) = pgid {
            let mgr = self.clone();
            let id = id.to_string();
            // script-driven jobs get 30 s to stop themselves (they SIGKILL their
            // own children after 20 s); others are signalled immediately.
            let (first_delay, second_delay) = if polls { (30u64, 10u64) } else { (0u64, 20u64) };
            tokio::spawn(async move {
                tokio::time::sleep(std::time::Duration::from_secs(first_delay)).await;
                if !mgr.is_active(&id).await { return; }
                // SAFETY: plain libc call with a process group we spawned ourselves.
                unsafe { libc::killpg(pg, libc::SIGTERM); }
                tokio::time::sleep(std::time::Duration::from_secs(second_delay)).await;
                if mgr.is_active(&id).await {
                    unsafe { libc::killpg(pg, libc::SIGKILL); }
                }
            });
        }
        Ok(info)
    }

    pub async fn set_pgid(&self, id: &str, pgid: i32) {
        if let Some(c) = self.cancel_of(id).await {
            *c.pgid.lock().unwrap() = Some(pgid);
            // a stop that arrived before the process existed: apply it now
            // (script-driven jobs will see the stop file by themselves)
            if c.is_requested() && !c.polls_stop_file.load(std::sync::atomic::Ordering::SeqCst) {
                unsafe { libc::killpg(pgid, libc::SIGTERM); }
            }
        }
    }

    async fn push_log(&self, id: &str, line: LogLine) {
        {
            let mut inner = self.inner.write().await;
            if let Some(j) = inner.jobs.get_mut(id) {
                j.log.push(line.clone());
                j.info.log_lines = j.log.len() as u32;
                if let Some(f) = j.file.as_mut() {
                    use std::io::Write;
                    let prefix = match line.stream { LogStream::System => "[app] ", LogStream::Stderr => "", LogStream::Stdout => "" };
                    let _ = writeln!(f, "{prefix}{}", line.text);
                }
            }
        }
        let _ = self.tx.send(JobEvent::Log { job_id: id.to_string(), line });
    }

    async fn set_status(&self, id: &str, status: JobStatus, error: Option<String>) {
        let mut status = status;
        let mut error = error;
        {
            let mut inner = self.inner.write().await;
            if let Some(j) = inner.jobs.get_mut(id) {
                if status == JobStatus::Failed && j.cancel.is_requested() {
                    status = JobStatus::Cancelled;
                    error = Some("stopped by the user".into());
                }
                j.info.status = status;
                j.info.error = error.clone();
                if matches!(status, JobStatus::Succeeded | JobStatus::Failed | JobStatus::Cancelled) {
                    j.info.finished_at = Some(chrono::Utc::now().to_rfc3339());
                    let _ = std::fs::remove_file(&j.cancel.stop_file);
                    if let Some(f) = j.file.as_mut() {
                        use std::io::Write;
                        let _ = writeln!(f, "# status: {}{}", format!("{status:?}").to_lowercase(), error.as_ref().map(|e| format!(" ({e})")).unwrap_or_default());
                        let _ = f.flush();
                        let _ = f.sync_all();
                    }
                }
            }
        }
        let _ = self.tx.send(JobEvent::Status { job_id: id.to_string(), status, error });
    }
}

#[derive(Clone)]
pub struct JobHandle {
    pub id: String,
    mgr: JobManager,
}

impl JobHandle {
    pub async fn log(&self, stream: LogStream, text: impl Into<String>) {
        self.mgr.push_log(&self.id, LogLine { stream, text: text.into() }).await;
    }
    pub async fn system(&self, text: impl Into<String>) {
        self.log(LogStream::System, text).await;
    }
    pub async fn running(&self) {
        self.mgr.set_status(&self.id, JobStatus::Running, None).await;
    }
    pub async fn succeeded(&self) {
        self.mgr.set_status(&self.id, JobStatus::Succeeded, None).await;
    }
    pub async fn failed(&self, error: impl Into<String>) {
        self.mgr.set_status(&self.id, JobStatus::Failed, Some(error.into())).await;
    }
    /// Path the deploy scripts poll to learn about a stop request.
    pub async fn stop_file(&self) -> std::path::PathBuf {
        self.mgr.cancel_of(&self.id).await.map(|c| c.stop_file.clone()).unwrap_or_else(|| std::env::temp_dir().join(format!("nixos-manager-stop-{}", self.id)))
    }
    pub async fn set_pgid(&self, pgid: i32) {
        self.mgr.set_pgid(&self.id, pgid).await;
    }
    pub async fn log_file(&self) -> Option<std::path::PathBuf> {
        self.mgr.get(&self.id).await.and_then(|j| j.info.log_file.map(std::path::PathBuf::from))
    }
    /// Declare that this job's script polls the stop file and stops itself.
    pub async fn uses_stop_file(&self) {
        if let Some(c) = self.mgr.cancel_of(&self.id).await {
            c.polls_stop_file.store(true, std::sync::atomic::Ordering::SeqCst);
        }
    }
    pub async fn stop_requested(&self) -> bool {
        self.mgr.cancel_of(&self.id).await.map(|c| c.is_requested()).unwrap_or(false)
    }
    /// A sender usable from blocking/sync callbacks; lines are forwarded to the job.
    pub fn line_sender(&self) -> tokio::sync::mpsc::UnboundedSender<LogLine> {
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<LogLine>();
        let mgr = self.mgr.clone();
        let id = self.id.clone();
        tokio::spawn(async move {
            while let Some(line) = rx.recv().await {
                mgr.push_log(&id, line).await;
            }
        });
        tx
    }
}
