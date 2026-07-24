//! Diarization sidecar adapter + speaker assignment.
//!
//! `sidecars/diarize/main.py` (pyannote.audio) turns a 16 kHz mono WAV into
//! raw speaker segments (`diarize_out.v1`); [`assign`] maps them onto
//! `doc.json` paragraphs by maximum time overlap. Wired into the CLI as
//! `lumen-cut diarize <pid>`.

pub mod assign;

use std::path::{Path, PathBuf};
use std::sync::Arc;

use serde::{Deserialize, Serialize};
use tracing::info;

use crate::error::{AppError, AppResult};
use crate::proc;

pub use assign::{
    assign_speakers, match_paragraph, normalize_speaker_paragraphs, reliable_speaker_match,
    SpeakerMatch,
};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DiarizeOutV1 {
    pub schema_version: u32,
    pub segments: Vec<DiarSegment>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DiarSegment {
    pub speaker: String,
    pub start: f64,
    pub end: f64,
}

const PROGRESS_PREFIX: &str = "LUMEN_CUT_PROGRESS ";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DiarizeProgress {
    pub phase: String,
    pub progress: u8,
    pub current: Option<u32>,
    pub total: Option<u32>,
    #[serde(default)]
    pub device: Option<String>,
    #[serde(default)]
    pub elapsed_seconds: Option<f64>,
    #[serde(default)]
    pub cpu_percent: Option<u32>,
    #[serde(default)]
    pub peak_memory_mb: Option<u64>,
    #[serde(default)]
    pub memory_limit_mb: Option<u64>,
}

pub type DiarizeProgressCallback = Arc<dyn Fn(DiarizeProgress) + Send + Sync>;

fn parse_sidecar_progress(line: &str) -> Option<DiarizeProgress> {
    let payload = line.strip_prefix(PROGRESS_PREFIX)?;
    serde_json::from_str(payload).ok()
}

/// Run the diarization sidecar against an audio file.  Returns raw segments;
/// the caller is responsible for aligning them with ASR sentences (see
/// [`assign_speakers`]).
pub async fn diarize_file(wav: &Path) -> AppResult<DiarizeOutV1> {
    let model = crate::data::modelconfig::load().diarize_model;
    diarize_file_with_model(wav, &model).await
}

pub async fn diarize_file_with_model(wav: &Path, model: &str) -> AppResult<DiarizeOutV1> {
    diarize_file_with_model_progress(wav, model, None).await
}

pub async fn diarize_file_with_model_progress(
    wav: &Path,
    model: &str,
    on_progress: Option<DiarizeProgressCallback>,
) -> AppResult<DiarizeOutV1> {
    let sidecar = locate_sidecar("sidecars/diarize/main.py").ok_or_else(|| AppError::Sidecar {
        sidecar: "lumen_cut_diarize",
        message: "sidecar script not found — set LUMEN_CUT_DIARIZE_SCRIPT, place it at \
             sidecars/diarize/main.py, or install a bundle containing Resources/sidecars"
            .into(),
    })?;
    let py = std::env::var("LUMEN_CUT_PYTHON")
        .unwrap_or_else(|_| crate::asr::resolve_python().to_string_lossy().into_owned());

    info!(bin = %py, path = %sidecar.display(), "spawning diarize sidecar");

    let args = build_sidecar_args(wav, model, &sidecar);
    let arg_refs: Vec<&str> = args.iter().map(String::as_str).collect();
    let config_token = crate::data::modelconfig::load().hf_token;
    let token = (!config_token.trim().is_empty())
        .then_some(config_token)
        .or_else(|| std::env::var("HF_TOKEN").ok())
        .or_else(|| std::env::var("HUGGING_FACE_HUB_TOKEN").ok());
    let environment = token
        .as_deref()
        .map(|token| vec![("HF_TOKEN", token)])
        .unwrap_or_default();
    let raw = if let Some(callback) = on_progress {
        proc::run_with_env_progress(
            &py,
            &arg_refs,
            &environment,
            Arc::new(move |line| {
                if let Some(progress) = parse_sidecar_progress(&line) {
                    callback(progress);
                }
            }),
        )
        .await?
    } else {
        proc::run_with_env(&py, &arg_refs, &environment).await?
    };
    let parsed: DiarizeOutV1 = serde_json::from_str(&raw).map_err(|e| AppError::Sidecar {
        sidecar: "lumen_cut_diarize",
        message: format!("json: {e}"),
    })?;
    Ok(parsed)
}

fn build_sidecar_args(wav: &Path, model: &str, sidecar: &Path) -> Vec<String> {
    vec![
        sidecar.display().to_string(),
        "--audio".into(),
        wav.display().to_string(),
        "--model".into(),
        model.to_string(),
        "--out".into(),
        "-".into(),
    ]
}

fn locate_sidecar(rel: &str) -> Option<PathBuf> {
    // The explicit override is authoritative, but only when the file exists:
    // a stale path must surface as the clean "not found" error above, not as
    // an opaque spawn failure from the interpreter.
    if let Ok(p) = std::env::var("LUMEN_CUT_DIARIZE_SCRIPT") {
        let p = PathBuf::from(p);
        return p.exists().then_some(p);
    }
    if let Ok(exe) = std::env::current_exe() {
        if let Some(bin_dir) = exe.parent() {
            let adjacent = bin_dir.join(rel);
            if adjacent.exists() {
                return Some(adjacent);
            }
            if let Some(contents_dir) = bin_dir.parent() {
                let bundled = contents_dir.join("Resources").join(rel);
                if bundled.exists() {
                    return Some(bundled);
                }
            }
        }
    }
    if let Some(source_root) = Path::new(env!("CARGO_MANIFEST_DIR")).parent() {
        let source_tree = source_root.join(rel);
        if source_tree.exists() {
            return Some(source_tree);
        }
    }
    let mut here = std::env::current_dir().ok()?;
    for _ in 0..6 {
        let candidate = here.join(rel);
        if candidate.exists() {
            return Some(candidate);
        }
        if !here.pop() {
            break;
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    // Tests below mutate process env; serialise them so they cannot race.
    static ENV_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

    async fn lock_env() -> tokio::sync::MutexGuard<'static, ()> {
        ENV_LOCK.lock().await
    }

    fn clear_env() {
        std::env::remove_var("LUMEN_CUT_PYTHON");
        std::env::remove_var("LUMEN_CUT_DIARIZE_SCRIPT");
    }

    /// A stand-in for the python interpreter: runs `body`, ignoring args.
    fn write_stub(dir: &Path, body: &str) -> PathBuf {
        use std::os::unix::fs::PermissionsExt;
        let path = dir.join("stub_python.sh");
        std::fs::write(&path, body).unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
        path
    }

    #[tokio::test]
    async fn parses_diarize_out_v1() {
        let _env = lock_env().await;
        clear_env();
        let tmp = tempfile::tempdir().unwrap();
        let stub = write_stub(
            tmp.path(),
            "#!/bin/sh\nprintf '%s' '{\"schema_version\":1,\"segments\":[{\"speaker\":\"SPEAKER_00\",\"start\":0.0,\"end\":1.5},{\"speaker\":\"SPEAKER_01\",\"start\":1.5,\"end\":3.0}]}'\n",
        );
        std::env::set_var("LUMEN_CUT_PYTHON", &stub);
        std::env::set_var("LUMEN_CUT_DIARIZE_SCRIPT", &stub);
        let out = diarize_file(Path::new("ignored.wav")).await.unwrap();
        assert_eq!(out.schema_version, 1);
        assert_eq!(out.segments.len(), 2);
        assert_eq!(out.segments[0].speaker, "SPEAKER_00");
        assert_eq!(out.segments[1].end, 3.0);
        clear_env();
    }

    #[test]
    fn sidecar_args_include_configured_model() {
        let args = build_sidecar_args(
            Path::new("/tmp/audio.wav"),
            "pyannote/speaker-diarization-3.1",
            Path::new("/tmp/main.py"),
        );
        assert!(args
            .windows(2)
            .any(|pair| pair == ["--model", "pyannote/speaker-diarization-3.1"]));
    }

    #[test]
    fn parses_structured_progress_without_accepting_log_noise() {
        let progress = parse_sidecar_progress(
            r#"LUMEN_CUT_PROGRESS {"phase":"embedding","progress":81,"current":3,"total":5,"device":"mps","elapsed_seconds":12.4,"cpu_percent":87,"peak_memory_mb":2431,"memory_limit_mb":6144}"#,
        )
        .unwrap();
        assert_eq!(progress.phase, "embedding");
        assert_eq!(progress.progress, 81);
        assert_eq!(progress.current, Some(3));
        assert_eq!(progress.total, Some(5));
        assert_eq!(progress.device.as_deref(), Some("mps"));
        assert_eq!(progress.elapsed_seconds, Some(12.4));
        assert_eq!(progress.cpu_percent, Some(87));
        assert_eq!(progress.peak_memory_mb, Some(2431));
        assert_eq!(progress.memory_limit_mb, Some(6144));
        assert!(parse_sidecar_progress("Downloading model files").is_none());
    }

    #[tokio::test]
    async fn sidecar_failure_surfaces_stderr_tail() {
        let _env = lock_env().await;
        clear_env();
        let tmp = tempfile::tempdir().unwrap();
        // Mimics main.py's pyannote/HF-token-missing path: a clear stderr
        // message plus a non-zero exit — the user must see the guidance,
        // not a panic.
        let stub = write_stub(
            tmp.path(),
            "#!/bin/sh\necho 'lumen_cut_diarize: requires a HuggingFace token' >&2\nexit 2\n",
        );
        std::env::set_var("LUMEN_CUT_PYTHON", &stub);
        std::env::set_var("LUMEN_CUT_DIARIZE_SCRIPT", &stub);
        let err = diarize_file(Path::new("x.wav")).await.unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("exit 2"), "unexpected: {msg}");
        assert!(msg.contains("HuggingFace token"), "unexpected: {msg}");
        clear_env();
    }

    #[tokio::test]
    async fn missing_sidecar_script_is_a_clean_error() {
        let _env = lock_env().await;
        clear_env();
        std::env::set_var("LUMEN_CUT_DIARIZE_SCRIPT", "/definitely/not/here/main.py");
        let err = diarize_file(Path::new("x.wav")).await.unwrap_err();
        assert!(
            err.to_string().contains("sidecar script not found"),
            "unexpected: {err}"
        );
        clear_env();
    }

    #[tokio::test]
    async fn invalid_sidecar_json_is_an_error_not_a_panic() {
        let _env = lock_env().await;
        clear_env();
        let tmp = tempfile::tempdir().unwrap();
        let stub = write_stub(tmp.path(), "#!/bin/sh\necho 'not json'\n");
        std::env::set_var("LUMEN_CUT_PYTHON", &stub);
        std::env::set_var("LUMEN_CUT_DIARIZE_SCRIPT", &stub);
        let err = diarize_file(Path::new("x.wav")).await.unwrap_err();
        assert!(err.to_string().contains("json"), "unexpected: {err}");
        clear_env();
    }
}
