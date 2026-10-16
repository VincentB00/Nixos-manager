//! NixOS option documentation index. Built from the `options.json` that
//! nixpkgs generates for the NixOS manual (every option with description,
//! type, default and example). On a NixOS machine it is usually already in
//! the store, so loading takes about a second; otherwise `nix-build`
//! evaluates it (a few minutes the first time).

use crate::jobs::JobHandle;
use crate::process;
use crate::types::{LogLine, LogStream, OptionDoc, OptionsStatus};
use crate::{CoreError, Result};
use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::RwLock;

const GENERIC_EXPR: &str = "(import <nixpkgs/nixos> { configuration = {}; }).config.system.build.manual.optionsJSON";

#[derive(Default)]
struct Inner {
    docs: Vec<OptionDoc>,
    by_name: HashMap<String, usize>,
    source: Option<String>,
    building: bool,
    error: Option<String>,
    job_id: Option<String>,
}

#[derive(Clone, Default)]
pub struct OptionsIndex {
    inner: Arc<RwLock<Inner>>,
}

impl OptionsIndex {
    pub async fn status(&self) -> OptionsStatus {
        let i = self.inner.read().await;
        OptionsStatus {
            ready: !i.docs.is_empty(),
            building: i.building,
            count: i.docs.len() as u32,
            error: i.error.clone(),
            source: i.source.clone(),
            job_id: i.job_id.clone(),
        }
    }

    pub async fn is_ready(&self) -> bool {
        !self.inner.read().await.docs.is_empty()
    }

    /// Try to find an already-built options.json without building anything
    /// (fast path: `nix-build` with the generic expression hits the store).
    pub async fn try_load_cached(&self) -> Result<bool> {
        if self.is_ready().await {
            return Ok(true);
        }
        let out = process::run("nix-build", &["--no-out-link", "-E", GENERIC_EXPR], None, Duration::from_secs(20)).await?;
        if out.status != 0 {
            return Ok(false);
        }
        let path = out.stdout.trim().to_string();
        if path.is_empty() {
            return Ok(false);
        }
        self.load_from_store_path(&path).await?;
        Ok(true)
    }

    /// Build (or fetch) options.json as a job and load it.
    pub async fn build(&self, job: JobHandle) {
        {
            let mut i = self.inner.write().await;
            if i.building {
                return;
            }
            i.building = true;
            i.error = None;
            i.job_id = Some(job.id.clone());
        }
        job.running().await;
        job.system("Building the NixOS option index (options.json). The first time this evaluates every NixOS module and can take a few minutes.").await;
        let sender = job.line_sender();
        let mut stdout = String::new();
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<String>();
        let result = process::stream(
            "nix-build",
            &["--no-out-link", "-E", GENERIC_EXPR],
            None,
            None,
            move |is_err, line| {
                if is_err {
                    let _ = sender.send(LogLine { stream: LogStream::Stderr, text: line });
                } else {
                    let _ = tx.send(line);
                }
            },
        )
        .await;
        while let Ok(l) = rx.try_recv() {
            stdout.push_str(&l);
            stdout.push('\n');
        }
        let outcome = match result {
            Ok(0) => {
                let path = stdout.trim().to_string();
                match self.load_from_store_path(&path).await {
                    Ok(()) => {
                        let n = self.inner.read().await.docs.len();
                        job.system(format!("Loaded {n} options from {path}")).await;
                        Ok(())
                    }
                    Err(e) => Err(e.to_string()),
                }
            }
            Ok(code) => Err(format!("nix-build exited with status {code}")),
            Err(e) => Err(e.to_string()),
        };
        let mut i = self.inner.write().await;
        i.building = false;
        match outcome {
            Ok(()) => {
                drop(i);
                job.succeeded().await;
            }
            Err(e) => {
                i.error = Some(e.clone());
                drop(i);
                job.failed(e).await;
            }
        }
    }

    pub async fn load_from_store_path(&self, path: &str) -> Result<()> {
        let json_path = format!("{path}/share/doc/nixos/options.json");
        let text = tokio::fs::read(&json_path).await.map_err(|e| CoreError::Other(format!("cannot read {json_path}: {e}")))?;
        let parsed: HashMap<String, RawOption> = serde_json::from_slice(&text)?;
        let mut docs: Vec<OptionDoc> = parsed
            .into_iter()
            .filter(|(name, _)| !name.starts_with('_'))
            .map(|(name, raw)| OptionDoc {
                name,
                description: raw.description.unwrap_or_default(),
                type_: raw.r#type.unwrap_or_default(),
                default: raw.default.and_then(|v| v.text),
                example: raw.example.and_then(|v| v.text),
                declarations: raw.declarations.unwrap_or_default(),
                read_only: raw.read_only.unwrap_or(false),
            })
            .collect();
        docs.sort_by(|a, b| a.name.cmp(&b.name));
        let by_name = docs.iter().enumerate().map(|(i, d)| (d.name.clone(), i)).collect();
        let mut i = self.inner.write().await;
        i.docs = docs;
        i.by_name = by_name;
        i.source = Some(path.to_string());
        Ok(())
    }

    pub async fn get(&self, name: &str) -> Option<OptionDoc> {
        let i = self.inner.read().await;
        i.by_name.get(name).map(|&idx| i.docs[idx].clone())
    }

    /// Ranked substring search over names and descriptions.
    pub async fn search(&self, query: &str, limit: usize) -> Vec<OptionDoc> {
        let q = query.trim().to_lowercase();
        if q.is_empty() {
            return vec![];
        }
        let words: Vec<&str> = q.split_whitespace().collect();
        let i = self.inner.read().await;
        let mut scored: Vec<(i32, &OptionDoc)> = i
            .docs
            .iter()
            .filter_map(|d| {
                let name = d.name.to_lowercase();
                let desc = d.description.to_lowercase();
                let mut score = 0;
                for w in &words {
                    if name == *w {
                        score += 100;
                    } else if name.starts_with(w) {
                        score += 40;
                    } else if name.split('.').any(|seg| seg == *w) {
                        score += 30;
                    } else if name.contains(w) {
                        score += 20;
                    } else if desc.contains(w) {
                        score += 5;
                    } else {
                        return None;
                    }
                }
                // Prefer shorter names (more general options) and `enable` switches.
                score -= (name.matches('.').count() as i32).min(6);
                if name.ends_with(".enable") {
                    score += 8;
                }
                Some((score, d))
            })
            .collect();
        scored.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| a.1.name.cmp(&b.1.name)));
        scored.into_iter().take(limit).map(|(_, d)| d.clone()).collect()
    }
}

#[derive(serde::Deserialize)]
struct RawOption {
    description: Option<String>,
    r#type: Option<String>,
    default: Option<RawValue>,
    example: Option<RawValue>,
    declarations: Option<Vec<String>>,
    #[serde(rename = "readOnly")]
    read_only: Option<bool>,
}

#[derive(serde::Deserialize)]
struct RawValue {
    text: Option<String>,
}
