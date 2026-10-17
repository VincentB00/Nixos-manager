//! The app's single JSON state file.
//!
//! Location: `$NIXOS_MANAGER_STATE` if set, else
//! `$XDG_STATE_HOME/nixos-manager/state.json` (usually
//! `~/.local/state/nixos-manager/state.json`). It only remembers *where*
//! configs are and a few preferences, never config contents or secrets.

use crate::types::{AppState, RecentLocation};
use crate::Result;
use std::path::{Path, PathBuf};
use tokio::sync::Mutex;

const STATE_VERSION: u32 = 1;
const MAX_RECENT: usize = 10;

pub struct StateStore {
    path: PathBuf,
    state: Mutex<AppState>,
}

impl StateStore {
    pub fn default_path() -> PathBuf {
        if let Ok(p) = std::env::var("NIXOS_MANAGER_STATE") {
            return PathBuf::from(p);
        }
        directories::ProjectDirs::from("", "", "nixos-manager")
            .map(|d| {
                d.state_dir()
                    .map(|p| p.to_path_buf())
                    .unwrap_or_else(|| d.data_local_dir().to_path_buf())
            })
            .unwrap_or_else(|| PathBuf::from(".nixos-manager"))
            .join("state.json")
    }

    pub fn load(path: PathBuf) -> Result<Self> {
        let state = match std::fs::read(&path) {
            Ok(bytes) => serde_json::from_slice::<AppState>(&bytes).unwrap_or_else(|e| {
                tracing::warn!("state file {} unreadable ({e}); starting fresh", path.display());
                AppState::default()
            }),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => AppState::default(),
            Err(e) => return Err(e.into()),
        };
        Ok(Self { path, state: Mutex::new(state) })
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub async fn get(&self) -> AppState {
        self.state.lock().await.clone()
    }

    pub async fn update<F: FnOnce(&mut AppState)>(&self, f: F) -> Result<AppState> {
        let mut guard = self.state.lock().await;
        f(&mut guard);
        guard.version = STATE_VERSION;
        self.persist(&guard)?;
        Ok(guard.clone())
    }

    /// Record that `path` was opened (moves it to the top of recents).
    pub async fn touch_location(&self, path: &str, host: Option<String>) -> Result<AppState> {
        let now = chrono::Utc::now().to_rfc3339();
        // one entry per folder whatever the spelling (trailing slash, symlinks)
        let canon = std::fs::canonicalize(path).map(|p| p.to_string_lossy().to_string()).unwrap_or_else(|_| path.trim_end_matches('/').to_string());
        let path = canon.as_str();
        self.update(|s| {
            s.recent_locations.retain(|r| r.path.trim_end_matches('/') != path && std::fs::canonicalize(&r.path).map(|p| p.to_string_lossy() != path).unwrap_or(true));
            s.recent_locations.insert(
                0,
                RecentLocation { path: path.to_string(), last_opened: now, host },
            );
            s.recent_locations.truncate(MAX_RECENT);
            s.last_location = Some(path.to_string());
        })
        .await
    }

    pub async fn set_targets(&self, targets: Vec<crate::types::DeployTarget>) -> Result<AppState> {
        self.update(|s| s.deploy_targets = targets).await
    }

    pub async fn set_hardware_pull(&self, path: &str, pull: crate::types::HardwarePull) -> Result<AppState> {
        let path = path.to_string();
        self.update(|s| {
            s.hardware_pulls.insert(path, pull);
        })
        .await
    }

    pub async fn forget_location(&self, path: &str) -> Result<AppState> {
        self.update(|s| {
            s.recent_locations.retain(|r| r.path != path);
            if s.last_location.as_deref() == Some(path) {
                s.last_location = s.recent_locations.first().map(|r| r.path.clone());
            }
        })
        .await
    }

    fn persist(&self, state: &AppState) -> Result<()> {
        if let Some(parent) = self.path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let tmp = self.path.with_extension("json.tmp");
        std::fs::write(&tmp, serde_json::to_vec_pretty(state)?)?;
        std::fs::rename(&tmp, &self.path)?;
        Ok(())
    }
}
