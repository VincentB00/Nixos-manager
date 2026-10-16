//! Registry of long-running jobs (deploys, evaluations) with log streaming.

use crate::types::{JobDetail, JobEvent, JobInfo, JobStatus, LogLine, LogStream};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::{broadcast, RwLock};

const MAX_JOBS: usize = 50;

struct JobRecord {
    info: JobInfo,
    log: Vec<LogLine>,
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
        let info = JobInfo {
            id: id.clone(),
            kind: kind.to_string(),
            title: title.to_string(),
            status: JobStatus::Pending,
            started_at: chrono::Utc::now().to_rfc3339(),
            finished_at: None,
            error: None,
            log_lines: 0,
        };
        {
            let mut inner = self.inner.write().await;
            inner.jobs.insert(id.clone(), JobRecord { info: info.clone(), log: Vec::new() });
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

    async fn push_log(&self, id: &str, line: LogLine) {
        {
            let mut inner = self.inner.write().await;
            if let Some(j) = inner.jobs.get_mut(id) {
                j.log.push(line.clone());
                j.info.log_lines = j.log.len() as u32;
            }
        }
        let _ = self.tx.send(JobEvent::Log { job_id: id.to_string(), line });
    }

    async fn set_status(&self, id: &str, status: JobStatus, error: Option<String>) {
        {
            let mut inner = self.inner.write().await;
            if let Some(j) = inner.jobs.get_mut(id) {
                j.info.status = status;
                j.info.error = error.clone();
                if matches!(status, JobStatus::Succeeded | JobStatus::Failed | JobStatus::Cancelled) {
                    j.info.finished_at = Some(chrono::Utc::now().to_rfc3339());
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
