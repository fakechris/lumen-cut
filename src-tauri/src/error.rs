//! Crate-wide error and Project types.

use std::path::PathBuf;

/// All cross-module errors. Keeps the surface narrow so callers can match on
/// discriminant without unwrapping nested context chains.
#[derive(Debug, thiserror::Error)]
pub enum AppError {
    #[error("io: {0}")]
    Io(#[from] std::io::Error),

    #[error("json: {0}")]
    Json(#[from] serde_json::Error),

    #[error("ffmpeg failed: {0}")]
    Ffmpeg(String),

    #[error("yt-dlp failed: {0}")]
    YtDlp(String),

    #[error("sidecar failed: {sidecar}: {message}")]
    Sidecar {
        sidecar: &'static str,
        message: String,
    },

    #[error("project not found at {0}")]
    ProjectNotFound(PathBuf),

    #[error("doc.json schema mismatch: {0}")]
    Schema(String),

    #[error("operation cancelled")]
    Cancelled,
}

pub type AppResult<T> = Result<T, AppError>;

/// Bridge to Tauri's IPC error type so `#[tauri::command]` functions
/// returning `AppResult<T>` work directly.
impl From<AppError> for tauri::ipc::InvokeError {
    fn from(e: AppError) -> Self {
        tauri::ipc::InvokeError::from(e.to_string())
    }
}
