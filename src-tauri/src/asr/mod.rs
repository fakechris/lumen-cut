//! Stage-3 ASR sidecar adapter.
//!
//! The actual model lives in a small Python package (see `sidecars/asr/`). This
//! module spawns it via `crate::proc::run` and parses its JSON output
//! (`asr_out.v1`) into our `Doc` shape.

pub mod cloud;

use std::path::Path;
use std::process::{Command, Stdio};
use std::sync::Arc;

use serde::{Deserialize, Serialize};
use tracing::info;

use crate::data::{Doc, MediaRef, Meta, Paragraph, Sentence, Word};
use crate::error::{AppError, AppResult};
use crate::proc;

const ASR_PACKAGE: &str = "mlx-qwen3-asr[aligner]==0.3.5";
const DIARIZE_PACKAGE: &str = "pyannote.audio==3.4.0";
const TORCH_PACKAGE: &str = "torch==2.5.1";
const TORCHAUDIO_PACKAGE: &str = "torchaudio==2.5.1";
const HUGGING_FACE_HUB_PACKAGE: &str = "huggingface-hub==0.36.2";

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RuntimeStatus {
    pub engine: crate::data::modelconfig::AsrEngine,
    pub selected_ready: bool,
    pub cloud_configured: bool,
    pub python_path: Option<String>,
    pub runtime_ready: bool,
    pub runtime_detail: String,
    pub model_id: String,
    pub model_cached: bool,
    pub aligner_id: String,
    pub aligner_cached: bool,
    pub diarize_model_id: String,
    pub diarize_model_cached: bool,
    pub diarize_python_path: Option<String>,
    pub diarize_runtime_ready: bool,
    pub diarize_runtime_detail: String,
    pub hugging_face_token_set: bool,
    pub diarize_ready: bool,
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

fn diarize_package_version(python: &Path) -> Option<String> {
    let output = Command::new(python)
        .args([
            "-c",
            "import importlib.metadata as m; import pyannote.audio, torch, torchaudio, huggingface_hub; print('|'.join([m.version('pyannote.audio'), m.version('torch'), m.version('torchaudio'), m.version('huggingface-hub')]))",
        ])
        .stdin(Stdio::null())
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let raw = String::from_utf8_lossy(&output.stdout);
    let versions = raw.trim().split('|').collect::<Vec<_>>();
    let [pyannote, torch, torchaudio, hugging_face_hub] = versions.as_slice() else {
        return None;
    };
    if !compatible_diarize_versions(pyannote, torch, torchaudio, hugging_face_hub) {
        return None;
    }
    Some(format!(
        "pyannote.audio {pyannote} · torch {torch} · torchaudio {torchaudio} · hub {hugging_face_hub}"
    ))
}

fn compatible_diarize_versions(
    pyannote: &str,
    torch: &str,
    torchaudio: &str,
    hugging_face_hub: &str,
) -> bool {
    // pyannote.audio 3.x still depends on top-level AudioMetaData (removed in
    // torchaudio 2.9), its trusted legacy checkpoints predate PyTorch 2.6's
    // `weights_only` default, and it uses Hub's pre-1.0 authentication API.
    let mut audio_parts = torchaudio
        .split('.')
        .filter_map(|part| part.parse::<u32>().ok());
    let audio_major = audio_parts.next().unwrap_or(u32::MAX);
    let audio_minor = audio_parts.next().unwrap_or(u32::MAX);
    let mut torch_parts = torch.split('.').filter_map(|part| part.parse::<u32>().ok());
    let torch_major = torch_parts.next().unwrap_or(u32::MAX);
    let torch_minor = torch_parts.next().unwrap_or(u32::MAX);
    let torch_too_new = torch_major > 2 || (torch_major == 2 && torch_minor >= 6);
    if pyannote.starts_with("3.")
        && (audio_major > 2 || (audio_major == 2 && audio_minor >= 9) || torch_too_new)
    {
        return false;
    }
    if pyannote.starts_with("3.")
        && hugging_face_hub
            .split('.')
            .next()
            .and_then(|part| part.parse::<u32>().ok())
            .unwrap_or(u32::MAX)
            >= 1
    {
        return false;
    }
    true
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
    let candidates = python_candidates(&home);
    let runtime = candidates
        .iter()
        .cloned()
        .into_iter()
        .find_map(|candidate| package_version(&candidate).map(|version| (candidate, version)));
    let model_cached = crate::data::modelconfig::model_cached(&home, &config.asr_model);
    let aligner_cached = crate::data::modelconfig::model_cached(&home, &config.asr_aligner);
    let runtime_ready = runtime.is_some();
    let diarize_runtime = candidates.into_iter().find_map(|candidate| {
        diarize_package_version(&candidate).map(|version| (candidate, version))
    });
    let diarize_runtime_ready = diarize_runtime.is_some();
    let diarize_model_cached =
        crate::data::modelconfig::diarize_model_cached(&home, &config.diarize_model);
    let hugging_face_token_set = !config.hf_token.trim().is_empty()
        || ["HF_TOKEN", "HUGGING_FACE_HUB_TOKEN"]
            .into_iter()
            .any(|name| std::env::var_os(name).is_some_and(|value| !value.is_empty()));
    let cloud_configured = crate::data::modelconfig::cloud_asr_configured(&config);
    let selected_ready = match config.asr_engine {
        crate::data::modelconfig::AsrEngine::Local => {
            runtime_ready && model_cached && aligner_cached
        }
        crate::data::modelconfig::AsrEngine::OpenaiCompatible => cloud_configured,
    };
    RuntimeStatus {
        engine: config.asr_engine,
        selected_ready,
        cloud_configured,
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
        diarize_model_id: config.diarize_model,
        diarize_model_cached,
        diarize_python_path: diarize_runtime
            .as_ref()
            .map(|(path, _)| path.to_string_lossy().into_owned()),
        diarize_runtime_ready,
        diarize_runtime_detail: diarize_runtime
            .map(|(_, version)| version)
            .unwrap_or_else(|| "pyannote.audio is not installed in the selected runtime".into()),
        hugging_face_token_set,
        // A token is download authorization, not a runtime dependency. Once
        // every gated snapshot is cached the pipeline must remain usable
        // offline and after the user removes the token.
        diarize_ready: diarize_runtime_ready && diarize_model_cached,
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

async fn ensure_managed_runtime(home: &Path, uv: &Path) -> AppResult<PathBuf> {
    let runtime_dir = home.join(".lumen-cut/runtime");
    tokio::fs::create_dir_all(home.join(".lumen-cut")).await?;
    let python = managed_python(home);
    if !python.is_file() {
        let runtime = runtime_dir.display().to_string();
        proc::run(
            &uv.to_string_lossy(),
            &["venv", "--python", "3.12", &runtime],
        )
        .await?;
    }
    Ok(python)
}

async fn install_packages(
    uv: &Path,
    python: &Path,
    packages: &[&str],
    sidecar: &'static str,
) -> AppResult<()> {
    let python = python.display().to_string();
    let mut args = vec!["pip", "install", "--python", python.as_str()];
    args.extend_from_slice(packages);
    proc::run(&uv.to_string_lossy(), &args)
        .await
        .map_err(|error| AppError::Sidecar {
            sidecar,
            message: error.to_string(),
        })?;
    Ok(())
}

pub async fn install_asr_runtime() -> AppResult<RuntimeStatus> {
    let _heavy_work = crate::performance::acquire_heavy("install-asr-runtime").await?;
    let home = std::env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_default();
    let uv = find_uv(&home).ok_or_else(|| AppError::Sidecar {
        sidecar: "lumen_cut_asr",
        message: "the `uv` installer was not found; install uv from https://docs.astral.sh/uv/ and try again"
            .into(),
    })?;
    let python = ensure_managed_runtime(&home, &uv).await?;
    install_packages(&uv, &python, &[ASR_PACKAGE], "lumen_cut_asr").await?;
    proc::run(&python.to_string_lossy(), &["-c", "import mlx_qwen3_asr"])
        .await
        .map_err(|error| AppError::Sidecar {
            sidecar: "lumen_cut_asr",
            message: format!("installed packages could not be imported: {error}"),
        })?;
    Ok(runtime_status())
}

pub async fn install_diarize_runtime() -> AppResult<RuntimeStatus> {
    let _heavy_work = crate::performance::acquire_heavy("install-speaker-runtime").await?;
    let home = std::env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_default();
    let uv = find_uv(&home).ok_or_else(|| AppError::Sidecar {
        sidecar: "lumen_cut_diarize",
        message: "the `uv` installer was not found; install uv from https://docs.astral.sh/uv/ and try again"
            .into(),
    })?;
    let python = ensure_managed_runtime(&home, &uv).await?;
    install_packages(
        &uv,
        &python,
        &[
            DIARIZE_PACKAGE,
            TORCH_PACKAGE,
            TORCHAUDIO_PACKAGE,
            HUGGING_FACE_HUB_PACKAGE,
        ],
        "lumen_cut_diarize",
    )
    .await?;
    proc::run(
        &python.to_string_lossy(),
        &[
            "-c",
            "import pyannote.audio, torch, torchaudio, huggingface_hub",
        ],
    )
    .await
    .map_err(|error| AppError::Sidecar {
        sidecar: "lumen_cut_diarize",
        message: format!("installed packages could not be imported: {error}"),
    })?;
    Ok(runtime_status())
}

pub async fn download_asr_models() -> AppResult<RuntimeStatus> {
    let _heavy_work = crate::performance::acquire_heavy("download-asr-models").await?;
    let status = runtime_status();
    let python = status.python_path.ok_or_else(|| AppError::Sidecar {
        sidecar: "lumen_cut_asr",
        message: "install the local transcription runtime before downloading models".into(),
    })?;
    let script =
        "from huggingface_hub import snapshot_download; import sys; snapshot_download(sys.argv[1])";
    for model in [&status.model_id, &status.aligner_id] {
        proc::run(&python, &["-c", script, model]).await?;
    }
    Ok(runtime_status())
}

pub async fn download_diarize_model() -> AppResult<RuntimeStatus> {
    let _heavy_work = crate::performance::acquire_heavy("download-speaker-model").await?;
    let status = runtime_status();
    if status.diarize_model_cached {
        return Ok(status);
    }
    if !status.diarize_runtime_ready {
        return Err(AppError::Sidecar {
            sidecar: "lumen_cut_diarize",
            message: "install the speaker identification runtime before downloading its model"
                .into(),
        });
    }
    let python = status
        .diarize_python_path
        .ok_or_else(|| AppError::Sidecar {
            sidecar: "lumen_cut_diarize",
            message: "install the local runtime before downloading the speaker model".into(),
        })?;
    let config = crate::data::modelconfig::load();
    let token = (!config.hf_token.trim().is_empty())
        .then_some(config.hf_token)
        .or_else(|| std::env::var("HF_TOKEN").ok())
        .or_else(|| std::env::var("HUGGING_FACE_HUB_TOKEN").ok())
        .ok_or_else(|| AppError::Sidecar {
            sidecar: "lumen_cut_diarize",
            message: "set a Hugging Face token and accept the speaker-diarization-3.1 model terms before downloading"
                .into(),
        })?;
    let diarize_script =
        "from pyannote.audio import Pipeline; import sys; Pipeline.from_pretrained(sys.argv[1])";
    proc::run_with_env(
        &python,
        &["-c", diarize_script, &status.diarize_model_id],
        &[("HF_TOKEN", token.as_str())],
    )
    .await?;
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

const PROGRESS_PREFIX: &str = "LUMEN_CUT_PROGRESS ";

#[derive(Debug, Clone, Deserialize)]
pub struct AsrProgress {
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
    #[serde(default)]
    pub mlx_active_memory_mb: Option<u64>,
    #[serde(default)]
    pub mlx_cache_memory_mb: Option<u64>,
}

pub type AsrProgressCallback = Arc<dyn Fn(AsrProgress) + Send + Sync>;

fn parse_sidecar_progress(line: &str) -> Option<AsrProgress> {
    let payload = line.strip_prefix(PROGRESS_PREFIX)?;
    serde_json::from_str(payload).ok()
}

impl From<AsrOutV1> for Doc {
    fn from(asr: AsrOutV1) -> Self {
        let mut word_index = 0usize;
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
                            .map(|w| {
                                let id = format!("w{word_index}");
                                word_index += 1;
                                Word {
                                    id,
                                    text: w.text,
                                    start: w.start,
                                    end: w.end,
                                }
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
    transcribe_file_with_aligner_progress(wav, model, language, aligner, None).await
}

pub async fn transcribe_file_with_aligner_progress(
    wav: &Path,
    model: &str,
    language: Option<&str>,
    aligner: Option<&str>,
    on_progress: Option<AsrProgressCallback>,
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
    let raw = if let Some(callback) = on_progress {
        proc::run_with_progress(
            &py.to_string_lossy(),
            &arg_refs,
            Arc::new(move |line| {
                if let Some(progress) = parse_sidecar_progress(&line) {
                    callback(progress);
                }
            }),
        )
        .await?
    } else {
        proc::run(&py.to_string_lossy(), &arg_refs).await?
    };
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
    fn parses_structured_sidecar_progress_without_accepting_log_noise() {
        let progress = parse_sidecar_progress(
            r#"LUMEN_CUT_PROGRESS {"phase":"aligning","progress":81,"current":12,"total":20,"device":"mlx-metal","elapsed_seconds":18.2,"cpu_percent":74,"peak_memory_mb":2870,"memory_limit_mb":6144,"mlx_active_memory_mb":1900,"mlx_cache_memory_mb":240}"#,
        )
        .unwrap();
        assert_eq!(progress.phase, "aligning");
        assert_eq!(progress.progress, 81);
        assert_eq!(progress.current, Some(12));
        assert_eq!(progress.total, Some(20));
        assert_eq!(progress.device.as_deref(), Some("mlx-metal"));
        assert_eq!(progress.elapsed_seconds, Some(18.2));
        assert_eq!(progress.cpu_percent, Some(74));
        assert_eq!(progress.peak_memory_mb, Some(2870));
        assert_eq!(progress.memory_limit_mb, Some(6144));
        assert_eq!(progress.mlx_active_memory_mb, Some(1900));
        assert_eq!(progress.mlx_cache_memory_mb, Some(240));
        assert!(parse_sidecar_progress("Fetching model files").is_none());
    }

    #[test]
    fn legacy_diarization_versions_reject_known_broken_combinations() {
        assert!(compatible_diarize_versions(
            "3.4.0", "2.5.1", "2.5.1", "0.36.2"
        ));
        assert!(!compatible_diarize_versions(
            "3.4.0", "2.6.0", "2.6.0", "0.36.2"
        ));
        assert!(!compatible_diarize_versions(
            "3.4.0", "2.5.1", "2.9.0", "0.36.2"
        ));
        assert!(!compatible_diarize_versions(
            "3.4.0", "2.5.1", "2.5.1", "1.0.0"
        ));
    }

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
    fn generated_word_ids_are_unique_across_sentences() {
        let parsed = AsrOutV1 {
            schema_version: 1,
            language: Some("English".into()),
            duration_seconds: 1.0,
            paragraphs: vec![AsrParagraph {
                speaker: None,
                sentences: vec![
                    AsrSentence {
                        text: "one".into(),
                        words: vec![AsrWord {
                            text: "one".into(),
                            start: 0.0,
                            end: 0.4,
                        }],
                    },
                    AsrSentence {
                        text: "two".into(),
                        words: vec![AsrWord {
                            text: "two".into(),
                            start: 0.4,
                            end: 0.8,
                        }],
                    },
                ],
            }],
        };
        let doc: Doc = parsed.into();
        let ids: Vec<&str> = doc
            .all_words()
            .into_iter()
            .map(|word| word.id.as_str())
            .collect();
        assert_eq!(ids, ["w0", "w1"]);
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
