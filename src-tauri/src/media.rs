//! `ffmpeg`-driven media extraction: any input → 16 kHz mono WAV suitable for
//! most ASR engines (faster-whisper, whisper.cpp, …).
//! The 16 kHz mono convention keeps local speech models and project metadata
//! consistent across import paths.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use tracing::info;

use crate::error::{AppError, AppResult};
use crate::proc;

/// Default sample rate expected by speech-to-text engines.
pub const ASR_SAMPLE_RATE: u32 = 16_000;

/// Probe a media file with `ffprobe` and return its duration in seconds,
/// codec + container info.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MediaInfo {
    pub path: PathBuf,
    pub duration_seconds: f64,
    pub sample_rate: Option<u32>,
    pub channels: Option<u32>,
    pub codec_name: Option<String>,
    pub codec_type: Option<String>,
}

/// Extract one frame at the given timestamp; useful for B-roll thumbnail prep
/// downstream (Stage 4). Returns the path of the written PNG.
pub async fn extract_frame(video: &Path, at_seconds: f64, out: &Path) -> AppResult<PathBuf> {
    if !tokio::fs::try_exists(out).await.unwrap_or(false) {
        let _ = tokio::fs::create_dir_all(out.parent().unwrap_or(Path::new("."))).await;
    }
    let at = format!("{:.3}", at_seconds);
    proc::run(
        "ffmpeg",
        &[
            "-nostdin",
            "-y",
            "-ss",
            &at,
            "-i",
            &video.display().to_string(),
            "-frames:v",
            "1",
            &out.display().to_string(),
        ],
    )
    .await?;
    Ok(out.to_path_buf())
}

/// Convert any audio/video file to a 16 kHz mono WAV for ASR.
pub async fn extract_audio_wav(input: &Path, output: &Path) -> AppResult<PathBuf> {
    if let Some(parent) = output.parent() {
        tokio::fs::create_dir_all(parent).await?;
    }
    info!(input = %input.display(), output = %output.display(), "extracting audio");
    proc::run(
        "ffmpeg",
        &[
            "-nostdin",
            "-y",
            "-i",
            &input.display().to_string(),
            "-vn", // drop video
            "-ac",
            "1", // mono
            "-ar",
            &ASR_SAMPLE_RATE.to_string(),
            "-f",
            "wav",
            &output.display().to_string(),
        ],
    )
    .await?;
    Ok(output.to_path_buf())
}

/// Probe media metadata via `ffprobe`. We parse just enough for stage-3 use;
/// full `MediaInfo` is populated later (Stage 4 may add chapter detection).
pub async fn probe(input: &Path) -> AppResult<MediaInfo> {
    let raw = proc::run(
        "ffprobe",
        &[
            "-v",
            "error",
            "-print_format",
            "json",
            "-show_format",
            "-show_streams",
            &input.display().to_string(),
        ],
    )
    .await?;

    #[derive(Deserialize)]
    struct FfProbeOut {
        format: Option<FormatBlock>,
        streams: Vec<StreamBlock>,
    }
    #[derive(Deserialize)]
    struct FormatBlock {
        duration: Option<String>,
    }
    #[derive(Deserialize)]
    struct StreamBlock {
        codec_name: Option<String>,
        codec_type: Option<String>,
        sample_rate: Option<String>,
        channels: Option<u32>,
    }

    let parsed: FfProbeOut =
        serde_json::from_str(&raw).map_err(|e| AppError::Ffmpeg(format!("ffprobe json: {e}")))?;

    let duration_seconds: f64 = parsed
        .format
        .and_then(|f| f.duration)
        .and_then(|s| s.parse().ok())
        .unwrap_or(0.0);

    // Prefer the FIRST audio stream for sample rate / channels.
    let audio = parsed
        .streams
        .iter()
        .find(|s| s.codec_type.as_deref() == Some("audio"));

    Ok(MediaInfo {
        path: input.to_path_buf(),
        duration_seconds,
        sample_rate: audio
            .and_then(|s| s.sample_rate.as_ref())
            .and_then(|s| s.parse().ok()),
        channels: audio.and_then(|s| s.channels),
        codec_name: audio.and_then(|s| s.codec_name.clone()),
        codec_type: audio.and_then(|s| s.codec_type.clone()),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// ffmpeg can manufacture a 1-second mono tone — useful when no real file
    /// is available. We just check the helpers don't panic on missing inputs.
    #[tokio::test]
    async fn probe_missing_file_errors() {
        let path = Path::new("/tmp/lumen-cut-no-such-file.mp4");
        let err = probe(path).await.unwrap_err();
        assert!(matches!(err, AppError::Sidecar { .. }));
    }
}
