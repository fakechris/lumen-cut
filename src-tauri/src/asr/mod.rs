//! Stage-3 ASR sidecar adapter.
//!
//! The actual model lives in a small Python package (see `sidecars/asr/`). This
//! module spawns it via `crate::proc::run` and parses its JSON output
//! (`asr_out.v1`) into our `Doc` shape.

use std::path::Path;

use serde::{Deserialize, Serialize};
use tracing::info;

use crate::data::{Doc, MediaRef, Meta, Paragraph, Sentence, Word};
use crate::error::{AppError, AppResult};
use crate::proc;

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

    let py = std::env::var("LUMEN_CUT_PYTHON").unwrap_or_else(|_| "python3".to_string());
    let args = build_sidecar_args(wav, model, language, aligner, &sidecar);

    info!(bin = %py, args = ?args, "spawning ASR sidecar");

    let arg_refs: Vec<&str> = args.iter().map(String::as_str).collect();
    let raw = proc::run(&py, &arg_refs).await?;
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
            "mlx-community/Qwen3-ASR-0.6B-8bit",
            Some("Chinese"),
            Some("mlx-community/Qwen3-ForcedAligner-0.6B-8bit"),
            Path::new("/tmp/main.py"),
        );
        assert!(args
            .windows(2)
            .any(|pair| pair == ["--align", "mlx-community/Qwen3-ForcedAligner-0.6B-8bit"]));
        assert!(args
            .windows(2)
            .any(|pair| pair == ["--language", "Chinese"]));
    }
}
