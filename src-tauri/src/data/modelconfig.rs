//! Model orchestration config — one place for every model the pipeline
//! touches (ASR / diarize / forced-aligner / LLM), so the GUI's "models"
//! view and the sidecars agree.
//!
//! Persists to `~/.lumen-cut/settings.json` alongside the LLM fields. Sidecars
//! read their model ids from this shared configuration.

use serde::{Deserialize, Serialize};

/// Memory-bounded defaults for Apple-silicon ASR and portable diarization.
fn defaults() -> ModelConfig {
    ModelConfig {
        // The pinned mlx-qwen3-asr runtime reads these native MLX quantized
        // checkpoints directly. Keeping both models quantized matters on unified
        // memory machines even though the two stages also run in isolation.
        asr_model: "mlx-community/Qwen3-ASR-0.6B-8bit".into(),
        asr_aligner: "mlx-community/Qwen3-ForcedAligner-0.6B-4bit".into(),
        diarize_model: "pyannote/speaker-diarization-3.1".into(),
        hf_token: String::new(),
        llm_endpoint: String::new(),
        llm_api_key: String::new(),
        llm_model: "gpt-4o-mini".into(),
        worker_count: 4,
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default = "defaults")]
pub struct ModelConfig {
    pub asr_model: String,
    pub asr_aligner: String,
    pub diarize_model: String,
    pub hf_token: String,
    pub llm_endpoint: String,
    pub llm_api_key: String,
    pub llm_model: String,
    pub worker_count: u32,
}

impl Default for ModelConfig {
    fn default() -> Self {
        defaults()
    }
}

/// Load from `~/.lumen-cut/settings.json`, merged over the defaults (so a
/// partial file still resolves every field).
pub fn load() -> ModelConfig {
    let mut cfg = ModelConfig::default();
    if let Some(home) = std::env::var_os("HOME") {
        if let Ok(raw) =
            std::fs::read_to_string(std::path::Path::new(&home).join(".lumen-cut/settings.json"))
        {
            if let Ok(v) = serde_json::from_str::<serde_json::Value>(&raw) {
                if let Some(s) = v.get("asrModel").and_then(|x| x.as_str()) {
                    cfg.asr_model = s.into();
                }
                if let Some(s) = v.get("diarizeModel").and_then(|x| x.as_str()) {
                    cfg.diarize_model = match s {
                        // Community-1 requires the incompatible pyannote 4.x
                        // runtime. Keep persisted prerelease settings on the
                        // only pipeline this build installs and validates.
                        "pyannote/speaker-diarization-community-1" => {
                            "pyannote/speaker-diarization-3.1".into()
                        }
                        _ => s.into(),
                    };
                }
                if let Some(s) = v.get("hfToken").and_then(|x| x.as_str()) {
                    cfg.hf_token = s.into();
                }
                if let Some(s) = v.get("asrAligner").and_then(|x| x.as_str()) {
                    cfg.asr_aligner = s.into();
                }
                if let Some(s) = v.get("llmEndpoint").and_then(|x| x.as_str()) {
                    cfg.llm_endpoint = s.into();
                }
                if let Some(s) = v.get("llmApiKey").and_then(|x| x.as_str()) {
                    cfg.llm_api_key = s.into();
                }
                if let Some(s) = v.get("llmModel").and_then(|x| x.as_str()) {
                    cfg.llm_model = s.into();
                }
                if let Some(n) = v.get("workerCount").and_then(|x| x.as_u64()) {
                    cfg.worker_count = n as u32;
                }
            }
        }
    }
    cfg
}

/// Whether the built-in worker has enough provider information to run.
/// API keys remain optional so local OpenAI-compatible services work.
pub fn llm_configured(cfg: &ModelConfig) -> bool {
    !cfg.llm_endpoint.trim().is_empty() && !cfg.llm_model.trim().is_empty()
}

/// Hugging Face uses `models--org--name` directories for cached repos.
pub fn cache_path(home: &std::path::Path, model_id: &str) -> std::path::PathBuf {
    hugging_face_cache_root(home).join(format!("models--{}", model_id.replace('/', "--")))
}

pub fn hugging_face_cache_root(home: &std::path::Path) -> std::path::PathBuf {
    if let Some(path) = std::env::var_os("HF_HUB_CACHE").filter(|value| !value.is_empty()) {
        return path.into();
    }
    if let Some(path) = std::env::var_os("HF_HOME").filter(|value| !value.is_empty()) {
        return std::path::PathBuf::from(path).join("hub");
    }
    let cache_home = std::env::var_os("XDG_CACHE_HOME")
        .filter(|value| !value.is_empty())
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| home.join(".cache"));
    cache_home.join("huggingface").join("hub")
}

fn snapshot_has_config(snapshot: &std::path::Path) -> bool {
    snapshot
        .join("config.json")
        .metadata()
        .map(|metadata| metadata.len() > 0)
        .unwrap_or(false)
        || snapshot
            .join("config.yaml")
            .metadata()
            .map(|metadata| metadata.len() > 0)
            .unwrap_or(false)
}

fn snapshot_has_weights(snapshot: &std::path::Path) -> bool {
    walkdir::WalkDir::new(snapshot)
        .max_depth(2)
        .follow_links(true)
        .into_iter()
        .filter_map(Result::ok)
        .filter(|entry| entry.file_type().is_file())
        .any(|entry| {
            let name = entry.file_name().to_string_lossy();
            let is_weight = name.ends_with(".safetensors")
                || name.ends_with(".npz")
                || name.ends_with(".bin")
                || name.ends_with(".ckpt")
                || name.ends_with(".pt")
                || name.ends_with(".pth")
                || name.ends_with(".onnx");
            is_weight
                && entry
                    .metadata()
                    .map(|metadata| metadata.len() > 1_000_000)
                    .unwrap_or(false)
        })
}

fn cached_snapshot(home: &std::path::Path, model_id: &str, require_weights: bool) -> bool {
    cached_snapshot_at(&cache_path(home, model_id), require_weights)
}

fn cached_snapshot_at(repo: &std::path::Path, require_weights: bool) -> bool {
    let snapshots = repo.join("snapshots");
    let Ok(revisions) = std::fs::read_dir(snapshots) else {
        return false;
    };
    revisions.filter_map(Result::ok).any(|revision| {
        let snapshot = revision.path();
        snapshot_has_config(&snapshot) && (!require_weights || snapshot_has_weights(&snapshot))
    })
}

fn pyannote_cache_path(home: &std::path::Path, model_id: &str) -> std::path::PathBuf {
    let root = std::env::var_os("PYANNOTE_CACHE")
        .filter(|value| !value.is_empty())
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| home.join(".cache/torch/pyannote"));
    root.join(format!("models--{}", model_id.replace('/', "--")))
}

fn diarize_cached_snapshot(home: &std::path::Path, model_id: &str, require_weights: bool) -> bool {
    cached_snapshot(home, model_id, require_weights)
        || cached_snapshot_at(&pyannote_cache_path(home, model_id), require_weights)
}

pub fn model_cached(home: &std::path::Path, model_id: &str) -> bool {
    // Repository metadata can appear before a multi-gigabyte weight finishes.
    // Core models and transitive diarization dependencies are ready only when
    // both their config and at least one materialized weight are present.
    cached_snapshot(home, model_id, true)
}

/// Diarization pipelines can be tiny YAML repositories that reference model
/// weights stored in separate gated repositories. Do not report the pipeline
/// ready until those transitive snapshots are materialized too.
pub fn diarize_model_cached(home: &std::path::Path, model_id: &str) -> bool {
    // The 3.1 pipeline repository itself is intentionally only a YAML graph;
    // its referenced segmentation and embedding repositories own the weights.
    if !diarize_cached_snapshot(home, model_id, false) {
        return false;
    }
    match model_id {
        "pyannote/speaker-diarization-3.1" => [
            "pyannote/segmentation-3.0",
            "pyannote/wespeaker-voxceleb-resnet34-LM",
        ]
        .into_iter()
        .all(|dependency| diarize_cached_snapshot(home, dependency, true)),
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_name_supported_model_families() {
        let c = ModelConfig::default();
        assert_eq!(c.asr_model, "mlx-community/Qwen3-ASR-0.6B-8bit");
        assert_eq!(c.asr_aligner, "mlx-community/Qwen3-ForcedAligner-0.6B-4bit");
        assert_eq!(c.diarize_model, "pyannote/speaker-diarization-3.1");
        assert_eq!(c.worker_count, 4);
    }

    #[test]
    fn llm_configured_requires_endpoint_and_model() {
        let mut c = ModelConfig::default();
        c.llm_model.clear();
        assert!(!llm_configured(&c));
        c.llm_endpoint = "https://x".into();
        assert!(!llm_configured(&c));
        c.llm_model = "model".into();
        assert!(llm_configured(&c));
    }

    #[test]
    fn load_merges_over_defaults() {
        let _g = ();
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join(".lumen-cut")).unwrap();
        std::fs::write(
            dir.path().join(".lumen-cut/settings.json"),
            r#"{"asrModel":"custom-asr","llmEndpoint":"https://e"}"#,
        )
        .unwrap();
        std::env::set_var("HOME", dir.path());
        let c = load();
        assert_eq!(c.asr_model, "custom-asr");
        assert_eq!(c.llm_endpoint, "https://e");
        // untouched fields keep defaults
        assert_eq!(c.diarize_model, "pyannote/speaker-diarization-3.1");
        std::env::remove_var("HOME");
    }

    #[test]
    fn huggingface_cache_path_matches_hub_layout() {
        assert_eq!(
            cache_path(std::path::Path::new("/home/me"), "org/model"),
            std::path::Path::new("/home/me/.cache/huggingface/hub/models--org--model")
        );
    }

    #[test]
    fn incomplete_huggingface_directory_is_not_cached() {
        let home = tempfile::tempdir().unwrap();
        let repo = cache_path(home.path(), "org/model");
        std::fs::create_dir_all(repo.join("refs")).unwrap();
        std::fs::write(repo.join("refs/main"), "revision").unwrap();
        assert!(!model_cached(home.path(), "org/model"));
    }

    #[test]
    fn legacy_diarization_requires_transitive_model_snapshots() {
        let home = tempfile::tempdir().unwrap();
        let pipeline =
            cache_path(home.path(), "pyannote/speaker-diarization-3.1").join("snapshots/revision");
        std::fs::create_dir_all(&pipeline).unwrap();
        std::fs::write(pipeline.join("config.yaml"), "pipeline: ready").unwrap();
        let segmentation =
            cache_path(home.path(), "pyannote/segmentation-3.0").join("snapshots/revision");
        std::fs::create_dir_all(&segmentation).unwrap();
        std::fs::write(segmentation.join("config.yaml"), "model: ready").unwrap();
        std::fs::write(segmentation.join("model.bin"), vec![0; 1_000_001]).unwrap();
        assert!(!diarize_model_cached(
            home.path(),
            "pyannote/speaker-diarization-3.1"
        ));
        let weights = cache_path(home.path(), "pyannote/wespeaker-voxceleb-resnet34-LM")
            .join("snapshots/revision");
        std::fs::create_dir_all(&weights).unwrap();
        std::fs::write(weights.join("config.yaml"), "model: ready").unwrap();
        assert!(!diarize_model_cached(
            home.path(),
            "pyannote/speaker-diarization-3.1"
        ));
        std::fs::write(weights.join("model.bin"), vec![0; 1_000_001]).unwrap();
        assert!(diarize_model_cached(
            home.path(),
            "pyannote/speaker-diarization-3.1"
        ));
    }

    #[test]
    fn complete_snapshot_requires_config_and_materialized_weights() {
        let home = tempfile::tempdir().unwrap();
        let snapshot = cache_path(home.path(), "org/model")
            .join("snapshots")
            .join("revision");
        std::fs::create_dir_all(&snapshot).unwrap();
        std::fs::write(snapshot.join("config.json"), "{}").unwrap();
        std::fs::write(snapshot.join("model.safetensors"), vec![0; 1_000_001]).unwrap();
        assert!(model_cached(home.path(), "org/model"));
    }

    #[cfg(unix)]
    #[test]
    fn complete_snapshot_accepts_huggingface_weight_symlinks() {
        let home = tempfile::tempdir().unwrap();
        let repo = cache_path(home.path(), "org/model");
        let snapshot = repo.join("snapshots/revision");
        let blobs = repo.join("blobs");
        std::fs::create_dir_all(&snapshot).unwrap();
        std::fs::create_dir_all(&blobs).unwrap();
        std::fs::write(snapshot.join("config.json"), "{}").unwrap();
        std::fs::write(blobs.join("weights"), vec![0; 1_000_001]).unwrap();
        std::os::unix::fs::symlink("../../blobs/weights", snapshot.join("model.safetensors"))
            .unwrap();
        assert!(model_cached(home.path(), "org/model"));
    }
}
