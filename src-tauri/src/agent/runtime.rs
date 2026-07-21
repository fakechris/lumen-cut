//! Built-in worker runtime — persistent workers that claim pending calls
//! and drive them through the LLM bridge, so lumen-cut can run a pipeline
//! without an external Claude Code worker process.
//!
//! lumen-cut supports external claim/submit workers through the HTTP task
//! surface and also offers built-in workers. `agent_serve` reads the LLM endpoint from
//! `~/.lumen-cut/settings.json` and, if present, spawns `workerCount` tasks
//! that loop `allocate → AgentBridge.call → submit`. The prompt for each
//! call is the materialised contract (answering guide) + the batch
//! payload, which is the same input exposed to an external worker.

use std::sync::Arc;
use std::time::Duration;

use crate::agent::allocate::Allocator;
use crate::agent::bridge::{AgentBridge, BridgeCall, BridgeConfig, Provider};
use crate::agent::PendingCall;

/// Load the LLM bridge config from `~/.lumen-cut/settings.json` (written by
/// `settings_export`). Returns `None` when the endpoint is missing/empty,
/// in which case the caller skips spawning built-in workers.
pub fn load_bridge_config() -> Option<BridgeConfig> {
    let cfg = crate::data::modelconfig::load();
    if !crate::data::modelconfig::llm_configured(&cfg) {
        return None;
    }
    let provider = if cfg.llm_endpoint.contains("anthropic") {
        Provider::Anthropic
    } else {
        Provider::OpenAi
    };
    let api_key = if cfg.llm_api_key.is_empty() {
        None
    } else {
        Some(cfg.llm_api_key)
    };
    Some(BridgeConfig {
        endpoint: cfg.llm_endpoint,
        api_key,
        model: cfg.llm_model,
        provider,
        max_attempts: 3,
    })
}

/// Build the prompt for a pending call: the materialised contract
/// (answering guide) followed by the batch payload read from
/// `payload_ref`. Pure — extracted from the worker loop for testing.
pub fn build_prompt(call: &PendingCall) -> String {
    let contract = call.contract.clone().unwrap_or_default();
    let payload = std::fs::read_to_string(&call.payload_ref).unwrap_or_default();
    if contract.is_empty() {
        payload
    } else {
        format!("{contract}\n\n---\n\n# Payload\n\n{payload}\n\nRespond per the contract above.")
    }
}

/// Spawn `n` built-in workers. Each loops: claim a pending call, drive it
/// through the LLM bridge, submit the answer or error. The tasks are
/// detached and live for the process lifetime. Returns immediately.
pub async fn spawn_workers(allocator: Arc<Allocator>, cfg: BridgeConfig, n: usize) {
    let bridge = Arc::new(AgentBridge::new(cfg));
    for i in 0..n.max(1) {
        let alloc = allocator.clone();
        let br = bridge.clone();
        let name = format!("worker-{i}");
        tokio::spawn(async move {
            worker_loop(alloc, br, name).await;
        });
    }
}

async fn worker_loop(allocator: Arc<Allocator>, bridge: Arc<AgentBridge>, name: String) {
    loop {
        allocator.reap_expired();
        if let Some((call, lease)) = allocator.allocate() {
            tracing::info!(worker = %name, kind = %call.kind, call = %call.id, "claimed call");
            let prompt = build_prompt(&call);
            let bc = BridgeCall {
                prompt,
                system: None,
                max_tokens: 4096,
                validator_feedback: vec![],
            };
            let result = bridge
                .call(bc, |ans| {
                    crate::agent::task::validate_call_answer(&call, ans)
                })
                .await;
            match result {
                Ok(ans) => {
                    allocator.submit(&lease.lease_id, Some(ans), None);
                }
                Err(e) => {
                    tracing::warn!(worker = %name, error = %e, "call failed");
                    allocator.submit(&lease.lease_id, None, Some(e.to_string()));
                }
            }
        } else {
            tokio::time::sleep(Duration::from_millis(500)).await;
        }
    }
}

/// Per-kind answer validation (the contract submit-lint). The worker
/// retries (up to `max_attempts`) with the validator's `errors[]` fed back
/// into the prompt; a persistently invalid answer is still recorded so the
/// orchestrator can inspect it.
pub fn validate_answer(kind: &str, answer: &str) -> Result<(), Vec<String>> {
    let trimmed = answer.trim();
    if trimmed.is_empty() {
        return Err(vec!["empty answer".into()]);
    }
    match kind {
        "translate" => {
            let v: serde_json::Value =
                serde_json::from_str(trimmed).map_err(|e| vec![format!("not JSON: {e}")])?;
            if v.get("translations").is_none() {
                return Err(vec!["missing `translations` object".into()]);
            }
            Ok(())
        }
        "polish" => {
            // The contract's atom-LCS coverage gate needs the source words,
            // which the worker carries in the payload; here we only check
            // the answer parses as the page/retry shape.
            let v: serde_json::Value =
                serde_json::from_str(trimmed).map_err(|e| vec![format!("not JSON: {e}")])?;
            if v.get("paragraphs").is_none() && v.get("sentences").is_none() {
                return Err(vec!["missing `paragraphs`/`sentences`".into()]);
            }
            Ok(())
        }
        "align" => {
            let v: serde_json::Value =
                serde_json::from_str(trimmed).map_err(|e| vec![format!("not JSON: {e}")])?;
            let pairs = v
                .get("pairs")
                .and_then(serde_json::Value::as_array)
                .ok_or_else(|| vec!["missing `pairs` array".into()])?;
            if pairs.iter().any(|pair| {
                pair.get("action")
                    .and_then(|action| action.as_str())
                    .is_none()
            }) {
                return Err(vec!["align pair missing `action` (recut|rewrite)".into()]);
            }
            Ok(())
        }
        _ => Ok(()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // HOME mutations must not race with other tests.
    static ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    #[test]
    fn build_prompt_includes_contract_and_payload() {
        let tmp = tempfile::tempdir().unwrap();
        let p = tmp.path().join("batch.json");
        std::fs::write(&p, r#"{"lines":[{"id":"k1","source":"hello"}]}"#).unwrap();
        let call = PendingCall {
            id: "c1".into(),
            kind: "translate".into(),
            word_count: 1,
            char_count: 5,
            payload_ref: p.to_string_lossy().to_string(),
            problems: vec![],
            contract: Some("# Translate contract\n\nanswer shape".into()),
        };
        let prompt = build_prompt(&call);
        assert!(prompt.contains("Translate contract"));
        assert!(prompt.contains("hello"));
        assert!(prompt.contains("Payload"));
    }

    #[test]
    fn build_prompt_without_contract_is_payload_only() {
        let tmp = tempfile::tempdir().unwrap();
        let p = tmp.path().join("b.json");
        std::fs::write(&p, "raw").unwrap();
        let call = PendingCall {
            id: "c1".into(),
            kind: "x".into(),
            word_count: 1,
            char_count: 1,
            payload_ref: p.to_string_lossy().to_string(),
            problems: vec![],
            contract: None,
        };
        assert_eq!(build_prompt(&call), "raw");
    }

    #[test]
    fn load_bridge_config_none_when_endpoint_missing() {
        let _g = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let dir = tempfile::tempdir().unwrap();
        std::env::set_var("HOME", dir.path());
        assert!(load_bridge_config().is_none());
        std::env::remove_var("HOME");
    }

    #[test]
    fn load_bridge_config_parses_settings() {
        let _g = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join(".lumen-cut")).unwrap();
        std::fs::write(
            dir.path().join(".lumen-cut/settings.json"),
            r#"{"llmEndpoint":"https://api.x.com","llmApiKey":"k","llmModel":"m","workerCount":2}"#,
        )
        .unwrap();
        std::env::set_var("HOME", dir.path());
        let cfg = load_bridge_config().unwrap();
        assert_eq!(cfg.endpoint, "https://api.x.com");
        assert_eq!(cfg.model, "m");
        assert_eq!(cfg.api_key.as_deref(), Some("k"));
        std::env::remove_var("HOME");
    }

    #[test]
    fn validate_translate_requires_translations() {
        assert!(validate_answer("translate", r#"{"translations":{"s1":"x"}}"#).is_ok());
        assert!(validate_answer("translate", r#"{"x":1}"#).is_err());
    }

    #[test]
    fn validate_align_requires_pairs_envelope_and_action() {
        assert!(validate_answer(
            "align",
            r#"{"pairs":[{"id":"s","action":"recut","cuts":[]}]}"#
        )
        .is_ok());
        assert!(validate_answer("align", r#"{"pairs":[{"id":"s"}]}"#).is_err());
        assert!(validate_answer("align", r#"{"id":"s","action":"recut"}"#).is_err());
    }

    #[test]
    fn validate_polish_requires_paragraphs_or_sentences() {
        assert!(validate_answer("polish", r#"{"paragraphs":[]}"#).is_ok());
        assert!(validate_answer("polish", r#"{"sentences":[]}"#).is_ok());
        assert!(validate_answer("polish", "").is_err());
        assert!(validate_answer("polish", r#"{"x":1}"#).is_err());
    }
}
