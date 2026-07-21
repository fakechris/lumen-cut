//! Subprocess plumbing. Centralised so the sidecar pattern is uniform across
//! ffmpeg / yt-dlp / faster-whisper / pyannote.

use std::future::Future;
use std::process::Stdio;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use tokio::io::AsyncReadExt;
use tokio::process::Command as TokioCommand;

use crate::error::{AppError, AppResult};

tokio::task_local! {
    static CANCEL_FLAG: Arc<AtomicBool>;
}

/// Run a future so every subprocess it spawns observes the same cancellation
/// flag. This keeps ffmpeg, yt-dlp and Python ASR cancellation consistent
/// without a process-global kill switch.
pub async fn with_cancellation<F>(flag: Arc<AtomicBool>, future: F) -> F::Output
where
    F: Future,
{
    CANCEL_FLAG.scope(flag, future).await
}

pub fn cancellation_requested() -> bool {
    CANCEL_FLAG
        .try_with(|flag| flag.load(Ordering::Relaxed))
        .unwrap_or(false)
}

/// Async variant — runs the subprocess to completion, capturing stdout, and
/// annotates failures with `sidecar` and the stderr tail. Used by code paths
/// that already live inside a tokio runtime.
pub async fn run(bin: &str, args: &[&str]) -> AppResult<String> {
    let mut command = TokioCommand::new(bin);
    command
        .args(args)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    #[cfg(unix)]
    command.process_group(0);
    let mut child = command.spawn().map_err(|e| AppError::Sidecar {
        sidecar: label(bin),
        message: format!("spawn: {e}"),
    })?;

    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| AppError::Schema("subprocess stdout was not captured".into()))?;
    let stderr = child
        .stderr
        .take()
        .ok_or_else(|| AppError::Schema("subprocess stderr was not captured".into()))?;
    let stdout_task = tokio::spawn(async move {
        let mut reader = stdout;
        let mut bytes = Vec::new();
        reader.read_to_end(&mut bytes).await.map(|_| bytes)
    });
    let stderr_task = tokio::spawn(async move {
        let mut reader = stderr;
        let mut bytes = Vec::new();
        reader.read_to_end(&mut bytes).await.map(|_| bytes)
    });

    let mut was_cancelled = false;
    let status = loop {
        if cancellation_requested() {
            was_cancelled = true;
            break terminate_process_tree(&mut child).await?;
        }
        if let Some(status) = child.try_wait()? {
            break status;
        }
        tokio::time::sleep(std::time::Duration::from_millis(60)).await;
    };
    let out = stdout_task
        .await
        .map_err(|error| AppError::Schema(format!("stdout reader failed: {error}")))??;
    let err = stderr_task
        .await
        .map_err(|error| AppError::Schema(format!("stderr reader failed: {error}")))??;

    if was_cancelled {
        return Err(AppError::Cancelled);
    }
    if !status.success() {
        let tail = String::from_utf8_lossy(&err);
        return Err(AppError::Sidecar {
            sidecar: label(bin),
            message: format!("exit {}: {}", status.code().unwrap_or(-1), tail.trim()),
        });
    }

    Ok(String::from_utf8(out).unwrap_or_default())
}

async fn terminate_process_tree(
    child: &mut tokio::process::Child,
) -> std::io::Result<std::process::ExitStatus> {
    #[cfg(unix)]
    {
        if let Some(pid) = child.id() {
            let group = format!("-{pid}");
            let _ = std::process::Command::new("/bin/kill")
                .args(["-TERM", &group])
                .status();
            let deadline = tokio::time::Instant::now() + std::time::Duration::from_millis(800);
            loop {
                if let Some(status) = child.try_wait()? {
                    return Ok(status);
                }
                if tokio::time::Instant::now() >= deadline {
                    let _ = std::process::Command::new("/bin/kill")
                        .args(["-KILL", &group])
                        .status();
                    break;
                }
                tokio::time::sleep(std::time::Duration::from_millis(30)).await;
            }
        }
    }
    let _ = child.kill().await;
    child.wait().await
}

/// Convert a binary name to the canonical sidecar label we surface to users.
fn label(bin: &str) -> &'static str {
    match bin {
        "ffmpeg" => "ffmpeg",
        "yt-dlp" => "yt-dlp",
        b if b.ends_with("lumen_cut_asr") => "lumen_cut_asr",
        b if b.ends_with("lumen_cut_diarize") => "lumen_cut_diarize",
        _ => "sidecar",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn run_captures_stdout() {
        let out = run("/bin/echo", &["hello"]).await.unwrap();
        assert_eq!(out.trim(), "hello");
    }

    #[tokio::test]
    async fn run_surfaces_failure() {
        let err = run("/usr/bin/false", &[]).await.unwrap_err();
        match err {
            AppError::Sidecar { message, .. } => assert!(message.contains("exit")),
            other => panic!("unexpected error: {other:?}"),
        }
    }

    #[tokio::test]
    async fn scoped_cancellation_stops_a_running_process() {
        let flag = Arc::new(AtomicBool::new(false));
        let trigger = flag.clone();
        tokio::spawn(async move {
            tokio::time::sleep(std::time::Duration::from_millis(80)).await;
            trigger.store(true, Ordering::Relaxed);
        });
        let started = std::time::Instant::now();
        let error = with_cancellation(flag, run("/bin/sleep", &["10"]))
            .await
            .unwrap_err();
        assert!(matches!(error, AppError::Cancelled));
        assert!(started.elapsed() < std::time::Duration::from_secs(2));
    }
}
