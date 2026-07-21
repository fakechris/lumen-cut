//! Diarization sidecar adapter + speaker assignment.
//!
//! `sidecars/diarize/main.py` (pyannote.audio) turns a 16 kHz mono WAV into
//! raw speaker segments (`diarize_out.v1`); [`assign`] maps them onto
//! `doc.json` paragraphs by maximum time overlap. Wired into the CLI as
//! `lumen-cut diarize <pid>`.

pub mod assign;

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use tracing::info;

use crate::error::{AppError, AppResult};
use crate::proc;

pub use assign::assign_speakers;

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

/// Run the diarization sidecar against an audio file.  Returns raw segments;
/// the caller is responsible for aligning them with ASR sentences (see
/// [`assign_speakers`]).
pub async fn diarize_file(wav: &Path) -> AppResult<DiarizeOutV1> {
    let model = crate::data::modelconfig::load().diarize_model;
    diarize_file_with_model(wav, &model).await
}

pub async fn diarize_file_with_model(wav: &Path, model: &str) -> AppResult<DiarizeOutV1> {
    let sidecar = locate_sidecar("sidecars/diarize/main.py").ok_or_else(|| AppError::Sidecar {
        sidecar: "lumen_cut_diarize",
        message: "sidecar script not found — set LUMEN_CUT_DIARIZE_SCRIPT, place it at \
             sidecars/diarize/main.py, or install a bundle containing Resources/sidecars"
            .into(),
    })?;
    let py = std::env::var("LUMEN_CUT_PYTHON").unwrap_or_else(|_| "python3".to_string());

    info!(bin = %py, path = %sidecar.display(), "spawning diarize sidecar");

    let args = build_sidecar_args(wav, model, &sidecar);
    let arg_refs: Vec<&str> = args.iter().map(String::as_str).collect();
    let raw = proc::run(&py, &arg_refs).await?;
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
