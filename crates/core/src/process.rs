//! Helpers to run external commands with captured or streamed output.

use crate::{CoreError, Result};
use std::process::Stdio;
use std::time::Duration;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::Command;

pub struct Output {
    pub status: i32,
    pub stdout: String,
    pub stderr: String,
}

/// Run a command to completion with a timeout, capturing output.
pub async fn run(program: &str, args: &[&str], cwd: Option<&std::path::Path>, timeout: Duration) -> Result<Output> {
    let mut cmd = Command::new(program);
    cmd.args(args).stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::piped()).kill_on_drop(true);
    if let Some(c) = cwd {
        cmd.current_dir(c);
    }
    let child = cmd.spawn().map_err(|e| CoreError::Other(format!("failed to start {program}: {e}")))?;
    let out = tokio::time::timeout(timeout, child.wait_with_output())
        .await
        .map_err(|_| CoreError::Other(format!("{program} timed out after {}s", timeout.as_secs())))?
        .map_err(|e| CoreError::Other(format!("{program} failed: {e}")))?;
    Ok(Output {
        status: out.status.code().unwrap_or(-1),
        stdout: String::from_utf8_lossy(&out.stdout).to_string(),
        stderr: String::from_utf8_lossy(&out.stderr).to_string(),
    })
}

/// Like [`stream`] but for a caller-prepared `Command` (custom env, args).
pub async fn stream_cmd<F>(mut cmd: Command, mut on_line: F) -> Result<i32>
where
    F: FnMut(bool, String) + Send + 'static,
{
    cmd.stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::piped()).kill_on_drop(true);
    let mut child = cmd.spawn().map_err(|e| CoreError::Other(format!("failed to start command: {e}")))?;
    let stdout = child.stdout.take().expect("piped stdout");
    let stderr = child.stderr.take().expect("piped stderr");
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<(bool, String)>();
    let tx1 = tx.clone();
    tokio::spawn(async move {
        let mut lines = BufReader::new(stdout).lines();
        while let Ok(Some(l)) = lines.next_line().await {
            if tx1.send((false, l)).is_err() {
                break;
            }
        }
    });
    tokio::spawn(async move {
        let mut lines = BufReader::new(stderr).lines();
        while let Ok(Some(l)) = lines.next_line().await {
            if tx.send((true, l)).is_err() {
                break;
            }
        }
    });
    let waiter = tokio::spawn(async move { child.wait().await });
    while let Some((is_err, line)) = rx.recv().await {
        on_line(is_err, line);
    }
    let status = waiter.await.map_err(|e| CoreError::Other(e.to_string()))?.map_err(|e| CoreError::Other(e.to_string()))?;
    Ok(status.code().unwrap_or(-1))
}

/// Run a command, streaming each stdout/stderr line to `on_line(is_stderr, line)`.
/// `stdin_data` is written to the child's stdin and then stdin is closed.
pub async fn stream<F>(
    program: &str,
    args: &[&str],
    cwd: Option<&std::path::Path>,
    stdin_data: Option<Vec<u8>>,
    mut on_line: F,
) -> Result<i32>
where
    F: FnMut(bool, String) + Send + 'static,
{
    let mut cmd = Command::new(program);
    cmd.args(args)
        .stdin(if stdin_data.is_some() { Stdio::piped() } else { Stdio::null() })
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    if let Some(c) = cwd {
        cmd.current_dir(c);
    }
    let mut child = cmd.spawn().map_err(|e| CoreError::Other(format!("failed to start {program}: {e}")))?;

    if let Some(data) = stdin_data {
        if let Some(mut stdin) = child.stdin.take() {
            let _ = stdin.write_all(&data).await;
            let _ = stdin.shutdown().await;
            drop(stdin);
        }
    }

    let stdout = child.stdout.take().expect("piped stdout");
    let stderr = child.stderr.take().expect("piped stderr");
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<(bool, String)>();

    let tx1 = tx.clone();
    tokio::spawn(async move {
        let mut lines = BufReader::new(stdout).lines();
        while let Ok(Some(l)) = lines.next_line().await {
            if tx1.send((false, l)).is_err() {
                break;
            }
        }
    });
    tokio::spawn(async move {
        let mut lines = BufReader::new(stderr).lines();
        while let Ok(Some(l)) = lines.next_line().await {
            if tx.send((true, l)).is_err() {
                break;
            }
        }
    });

    let waiter = tokio::spawn(async move { child.wait().await });
    while let Some((is_err, line)) = rx.recv().await {
        on_line(is_err, line);
    }
    let status = waiter
        .await
        .map_err(|e| CoreError::Other(e.to_string()))?
        .map_err(|e| CoreError::Other(e.to_string()))?;
    Ok(status.code().unwrap_or(-1))
}
