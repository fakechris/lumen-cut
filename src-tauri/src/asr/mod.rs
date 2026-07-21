//! Stage-3 ASR sidecar adapter.
//!
//! The actual model lives in a small Python package (see `sidecars/asr/`). This
//! module spawns it via `crate::proc::run` and parses its JSON output
//! (`asr_out.v1`) into our `Doc` shape.

use std::path::Path;
use std::process::{Command, Stdio};

use serde::{Deserialize, Serialize};
use tracing::info;

use crate::data::{Doc, MediaRef, Meta, Paragraph, Sentence, Word};
use crate::error::{AppError, AppResult};
use crate::proc;

const ASR_PACKAGE: &str = "mlx-qwen3-asr[aligner]==0.3.5";

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RuntimeStatus {
    pub python_path: Option<String>,
    pub runtime_ready: bool,
    pub runtime_detail: String,
    pub model_id: String,
    pub model_cached: bool,
    pub aligner_id: String,
    pub aligner_cached: bool,
    pub ready: bool,
}

pub fn managed_python(home: &Path) -> PathBuf {
    home.join(".lumen-cut/runtime/bin/python3")
}

fn python_candidates(home: &Path) -> Vec<PathBuf> {
    let mut candidates = Vec::new();
    if let Some(path) = std::env::var_os("LUMEN_CUT_PYTHON").filter(|value| !value.is_empty()) {
        candidates.push(PathBuf::from(path));
    }
    candidates.push(managed_python(home));
    candidates.extend([
        PathBuf::from("/opt/homebrew/bin/python3.13"),
        PathBuf::from("/opt/homebrew/bin/python3.12"),
        PathBuf::from("/usr/local/bin/python3.13"),
        PathBuf::from("/usr/local/bin/python3.12"),
        PathBuf::from("python3"),
    ]);
    candidates.dedup();
    candidates
}

fn package_version(python: &Path) -> Option<String> {
    let output = Command::new(python)
        .args([
            "-c",
            "import importlib.metadata as m; import mlx_qwen3_asr; print(m.version('mlx-qwen3-asr'))",
        ])
        .stdin(Stdio::null())
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    Some(String::from_utf8_lossy(&output.stdout).trim().to_string())
}

pub fn resolve_python() -> PathBuf {
    let home = std::env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_default();
    python_candidates(&home)
        .into_iter()
        .find(|candidate| package_version(candidate).is_some())
        .unwrap_or_else(|| managed_python(&home))
}

pub fn runtime_status() -> RuntimeStatus {
    let home = std::env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_default();
    let config = crate::data::modelconfig::load();
    let runtime = python_candidates(&home)
        .into_iter()
        .find_map(|candidate| package_version(&candidate).map(|version| (candidate, version)));
    let model_cached = crate::data::modelconfig::model_cached(&home, &config.asr_model);
    let aligner_cached = crate::data::modelconfig::model_cached(&home, &config.asr_aligner);
    let runtime_ready = runtime.is_some();
    RuntimeStatus {
        python_path: runtime
            .as_ref()
            .map(|(path, _)| path.to_string_lossy().into_owned()),
        runtime_ready,
        runtime_detail: runtime
            .map(|(_, version)| format!("mlx-qwen3-asr {version}"))
            .unwrap_or_else(|| {
                "mlx-qwen3-asr is not installed in a supported Python 3.10–3.13 runtime".into()
            }),
        model_id: config.asr_model,
        model_cached,
        aligner_id: config.asr_aligner,
        aligner_cached,
        ready: runtime_ready && model_cached && aligner_cached,
    }
}

fn find_uv(home: &Path) -> Option<PathBuf> {
    let candidates = [
        home.join(".local/bin/uv"),
        PathBuf::from("/opt/homebrew/bin/uv"),
        PathBuf::from("/usr/local/bin/uv"),
        PathBuf::from("uv"),
    ];
    candidates.into_iter().find(|candidate| {
        Command::new(candidate)
            .arg("--version")
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .map(|status| status.success())
            .unwrap_or(false)
    })
}

pub async fn install_runtime() -> AppResult<RuntimeStatus> {
    let home = std::env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_default();
    let uv = find_uv(&home).ok_or_else(|| AppError::Sidecar {
        sidecar: "lumen_cut_asr",
        message: "the `uv` installer was not found; install uv from https://docs.astral.sh/uv/ and try again"
            .into(),
    })?;
    let runtime_dir = home.join(".lumen-cut/runtime");
    tokio::fs::create_dir_all(home.join(".lumen-cut")).await?;
    let python = managed_python(&home);
    if !python.is_file() {
        let output = tokio::process::Command::new(&uv)
            .args(["venv", "--python", "3.12"])
            .arg(&runtime_dir)
            .output()
            .await?;
        if !output.status.success() {
            return Err(AppError::Sidecar {
                sidecar: "lumen_cut_asr",
                message: String::from_utf8_lossy(&output.stderr).trim().to_string(),
            });
        }
    }
    let output = tokio::process::Command::new(&uv)
        .arg("pip")
        .arg("install")
        .arg("--python")
        .arg(&python)
        .arg(ASR_PACKAGE)
        .output()
        .await?;
    if !output.status.success() {
        return Err(AppError::Sidecar {
            sidecar: "lumen_cut_asr",
            message: String::from_utf8_lossy(&output.stderr).trim().to_string(),
        });
    }
    Ok(runtime_status())
}

pub async fn download_models() -> AppResult<RuntimeStatus> {
    let status = runtime_status();
    let python = status.python_path.ok_or_else(|| AppError::Sidecar {
        sidecar: "lumen_cut_asr",
        message: "install the local transcription runtime before downloading models".into(),
    })?;
    let script =
        "from huggingface_hub import snapshot_download; import sys; snapshot_download(sys.argv[1])";
    for model in [&status.model_id, &status.aligner_id] {
        let output = tokio::process::Command::new(&python)
            .args(["-c", script, model])
            .output()
            .await?;
        if !output.status.success() {
            return Err(AppError::Sidecar {
                sidecar: "lumen_cut_asr",
                message: String::from_utf8_lossy(&output.stderr).trim().to_string(),
            });
        }
    }
    Ok(runtime_status())
}

/// Output schema written by `sidecars/asr/main.py`. Stable; bump the
/// `schema_version` and add a v2 deserializer if fields change.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AsrOutV1 {
    pub schema_version: u32,
    pub language: Option<String>,
    pub duration_seconds: f64,
    pub paragraphs: Vec<AsrParagraph>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AsrParagraph {
    pub speaker: Option<String>,
    pub sentences: Vec<AsrSentence>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AsrSentence {
    pub text: String,
    pub words: Vec<AsrWord>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AsrWord {
    pub text: String,
    pub start: f64,
    pub end: f64,
}

impl From<AsrOutV1> for Doc {
    fn from(asr: AsrOutV1) -> Self {
        let paragraphs = asr
            .paragraphs
            .into_iter()
            .enumerate()
            .map(|(pi, p)| Paragraph {
                id: pi as u32 + 1,
                speaker: p.speaker,
                sentences: p
                    .sentences
                    .into_iter()
                    .enumerate()
                    .map(|(si, s)| Sentence {
                        id: format!("p{}s{}", pi + 1, si + 1),
                        text: s.text,
                        words: s
                            .words
                            .into_iter()
                            .enumerate()
                            .map(|(wi, w)| Word {
                                id: format!("w{}", wi),
                                text: w.text,
                                start: w.start,
                                end: w.end,
                            })
                            .collect(),
                    })
                    .collect(),
            })
            .collect();

        Doc {
            id: uuid::Uuid::new_v4().to_string(),
            schema: 1,
            media: MediaRef {
                path: PathBuf::from(""),
                duration_seconds: asr.duration_seconds,
                sample_rate: Some(16_000),
                channels: Some(1),
            },
            meta: Meta {
                title: String::new(),
                description: String::new(),
                language: asr.language,
                created_at: chrono::Utc::now(),
                updated_at: chrono::Utc::now(),
            },
            paragraphs,
            translations: Default::default(),
        }
    }
}

use std::path::PathBuf;

/// Run the sidecar against a 16 kHz mono WAV file, return the parsed result.
pub async fn transcribe_file(
    wav: &Path,
    model: &str,
    language: Option<&str>,
) -> AppResult<AsrOutV1> {
    let aligner = crate::data::modelconfig::load().asr_aligner;
    transcribe_file_with_aligner(wav, model, language, Some(&aligner)).await
}

/// Run ASR with an explicit forced-aligner model.
pub async fn transcribe_file_with_aligner(
    wav: &Path,
    model: &str,
    language: Option<&str>,
    aligner: Option<&str>,
) -> AppResult<AsrOutV1> {
    let sidecar = locate_sidecar("sidecars/asr/main.py").ok_or_else(|| AppError::Sidecar {
        sidecar: "lumen_cut_asr",
        message: "sidecar script not found — set LUMEN_CUT_ASR_SCRIPT, place it at \
             sidecars/asr/main.py, or install a bundle containing Resources/sidecars"
            .into(),
    })?;

    let py = resolve_python();
    let args = build_sidecar_args(wav, model, language, aligner, &sidecar);

    info!(bin = %py.display(), args = ?args, "spawning ASR sidecar");

    let arg_refs: Vec<&str> = args.iter().map(String::as_str).collect();
    let raw = proc::run(&py.to_string_lossy(), &arg_refs).await?;
    let parsed: AsrOutV1 = serde_json::from_str(&raw).map_err(|e| AppError::Sidecar {
        sidecar: "lumen_cut_asr",
        message: format!("json parse: {e}"),
    })?;
    Ok(parsed)
}

fn build_sidecar_args(
    wav: &Path,
    model: &str,
    language: Option<&str>,
    aligner: Option<&str>,
    sidecar: &Path,
) -> Vec<String> {
    let mut args = vec![
        sidecar.display().to_string(),
        "--audio".into(),
        wav.display().to_string(),
        "--model".into(),
        model.to_string(),
        "--out".into(),
        "-".into(),
    ];
    if let Some(lang) = language {
        args.push("--language".into());
        args.push(lang.to_string());
    }
    if let Some(aligner) = aligner.filter(|value| !value.trim().is_empty()) {
        args.push("--align".into());
        args.push(aligner.to_string());
    }
    args
}

fn locate_sidecar(rel: &str) -> Option<PathBuf> {
    if let Ok(p) = std::env::var("LUMEN_CUT_ASR_SCRIPT") {
        let path = PathBuf::from(p);
        if path.exists() {
            return Some(path);
        }
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

    #[test]
    fn parses_asr_out_v1() {
        let raw = r#"{
            "schema_version": 1,
            "language": "zh",
            "duration_seconds": 1.5,
            "paragraphs": [{
                "speaker": null,
                "sentences": [{
                    "text": "你好",
                    "words": [{"text": "你", "start": 0.0, "end": 0.3},
                              {"text": "好", "start": 0.3, "end": 0.6}]
                }]
            }]
        }"#;
        let parsed: AsrOutV1 = serde_json::from_str(raw).unwrap();
        assert_eq!(parsed.language.as_deref(), Some("zh"));
        let doc: Doc = parsed.into();
        assert_eq!(doc.paragraphs.len(), 1);
        assert_eq!(doc.paragraphs[0].sentences[0].text, "你好");
    }

    #[test]
    fn sidecar_args_include_configured_forced_aligner() {
        let args = build_sidecar_args(
            Path::new("/tmp/audio.wav"),
            "Qwen/Qwen3-ASR-0.6B",
            Some("Chinese"),
            Some("Qwen/Qwen3-ForcedAligner-0.6B"),
            Path::new("/tmp/main.py"),
        );
        assert!(args
            .windows(2)
            .any(|pair| pair == ["--align", "Qwen/Qwen3-ForcedAligner-0.6B"]));
        assert!(args
            .windows(2)
            .any(|pair| pair == ["--language", "Chinese"]));
    }
}
