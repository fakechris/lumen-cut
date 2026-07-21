//! Model orchestration config — one place for every model the pipeline
//! touches (ASR / diarize / forced-aligner / LLM), so the GUI's "models"
//! view and the sidecars agree.
//!
//! Persists to `~/.lumen-cut/settings.json` alongside the LLM fields. Sidecars
//! read their model ids from this shared configuration.

use serde::{Deserialize, Serialize};

/// Conservative defaults for Apple-silicon ASR and portable diarization.
fn defaults() -> ModelConfig {
    ModelConfig {
        asr_model: "mlx-community/Qwen3-ASR-0.6B-8bit".into(),
        asr_aligner: "mlx-community/Qwen3-ForcedAligner-0.6B-8bit".into(),
        diarize_model: "pyannote/speaker-diarization-3.1".into(),
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
                    cfg.diarize_model = s.into();
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
    home.join(".cache")
        .join("huggingface")
        .join("hub")
        .join(format!("models--{}", model_id.replace('/', "--")))
}

pub fn model_cached(home: &std::path::Path, model_id: &str) -> bool {
    cache_path(home, model_id).is_dir()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_name_supported_model_families() {
        let c = ModelConfig::default();
        assert!(c.asr_model.contains("Qwen3-ASR"));
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
}
