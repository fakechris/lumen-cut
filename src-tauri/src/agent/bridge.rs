//! Provider-neutral streaming bridge for background AI tasks.
//!
//! Public surface:
//!   * `AgentBridge`  — owns a `reqwest::Client` and config (endpoint,
//!     auth, model).
//!   * `BridgeCall`   — single-call envelope with prompt, validation, and
//!     retry policy.
//!   * `BridgeAnswer` — validated answer struct + reasoning string.
//!
//! The bridge is **provider-agnostic**: any endpoint that exposes a
//! Server-Sent Events stream of the form `data: {"delta": "..."}` is
//! supported. We accept both `OpenAI`-style and `Anthropic`-style
//! SSE events transparently because we just collect `delta`-bearing
//! payloads.
//!
//! **Retry policy**:
//!   1. transport error           → retry up to 3× with exponential backoff
//!   2. validation rejection      → retry on the *same* lease ≤3× without
//!      consuming the lease, and inject the validator's `errors[]` into the
//!      prompt
//!   3. any other failure         → mark `failed`, do NOT retry
//!
//! **Validation feedback injection** is the small but important trick
//! that lets the model self-correct without us writing a custom
//! validator. We append the validator's `errors[]` to the prompt with
//! the marker `<<validator-feedback>>` so a tail of the prompt is always
//! re-readable.

use std::time::Duration;

use futures::StreamExt;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone)]
pub struct BridgeConfig {
    pub endpoint: String,        // e.g. https://api.openai.com/v1/chat/completions
    pub api_key: Option<String>, // Bearer token
    pub model: String,           // e.g. gpt-4o-mini or claude-3-5-sonnet
    pub provider: Provider,
    pub max_attempts: u32, // default 3
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Provider {
    OpenAi,
    Anthropic,
    Custom,
}

#[derive(Debug, Clone)]
pub struct BridgeCall {
    pub prompt: String,
    /// Optional system message; for Anthropic this is the dedicated field.
    pub system: Option<String>,
    /// Max output tokens. Default 1024.
    pub max_tokens: u32,
    /// Optional last-mile `errors[]` from the previous validator.
    pub validator_feedback: Vec<String>,
}

impl BridgeCall {
    fn render_prompt(&self) -> String {
        if self.validator_feedback.is_empty() {
            return self.prompt.clone();
        }
        let mut out = self.prompt.clone();
        out.push_str("\n\n<<validator-feedback>>\n");
        for (i, e) in self.validator_feedback.iter().enumerate() {
            out.push_str(&format!("  {}. {}\n", i + 1, e));
        }
        out
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct BridgeAnswer {
    pub text: String,
    pub reasoning: String,
    pub prompt_tokens: u32,
    pub completion_tokens: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BridgeAttemptEvent {
    /// One-based attempt number shown to the user.
    pub attempt: u32,
    pub max_attempts: u32,
    /// True while waiting to start this attempt after a failed request.
    pub retrying: bool,
}

#[derive(Debug, Clone)]
pub enum BridgeError {
    Transport(String),
    BadStatus { status: u16, body: String },
    ValidationFailed(Vec<String>),
    Cancelled,
}

impl std::fmt::Display for BridgeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            BridgeError::Transport(s) => write!(f, "transport: {s}"),
            BridgeError::BadStatus { status, body } => {
                write!(f, "bad status {status}: {body}")
            }
            BridgeError::ValidationFailed(es) => write!(f, "validation: {es:?}"),
            BridgeError::Cancelled => write!(f, "cancelled"),
        }
    }
}

impl std::error::Error for BridgeError {}

#[derive(Clone)]
pub struct AgentBridge {
    cfg: BridgeConfig,
    client: reqwest::Client,
}

impl AgentBridge {
    pub fn new(cfg: BridgeConfig) -> Self {
        let client = reqwest::Client::builder()
            .connect_timeout(Duration::from_secs(15))
            .read_timeout(Duration::from_secs(60))
            .build()
            .expect("reqwest client");
        Self { cfg, client }
    }

    pub fn config(&self) -> &BridgeConfig {
        &self.cfg
    }

    /// Execute one call, with retry + validation-feedback injection.
    /// The `validate` closure decides whether the assembled answer is
    /// acceptable; rejections trigger the in-place retry budget.
    pub async fn call<F>(&self, call: BridgeCall, validate: F) -> Result<BridgeAnswer, BridgeError>
    where
        F: Fn(&str) -> Result<(), Vec<String>>,
    {
        self.call_observed(call, validate, |_| {}).await
    }

    pub async fn call_observed<F, O>(
        &self,
        mut call: BridgeCall,
        validate: F,
        observe: O,
    ) -> Result<BridgeAnswer, BridgeError>
    where
        F: Fn(&str) -> Result<(), Vec<String>>,
        O: Fn(BridgeAttemptEvent),
    {
        let max = self.cfg.max_attempts.max(1);
        let mut last_err = Vec::<String>::new();
        for attempt in 0..max {
            observe(BridgeAttemptEvent {
                attempt: attempt + 1,
                max_attempts: max,
                retrying: false,
            });
            call.validator_feedback = last_err.clone();
            let mut attempt_call = call.clone();
            // The very first attempt has no feedback; the prompt is plain.
            if attempt == 0 {
                attempt_call.validator_feedback.clear();
            }
            let res = self.stream_call(&attempt_call).await;
            match res {
                Ok(ans) => {
                    if let Err(errs) = validate(&ans.text) {
                        last_err = errs;
                        tracing::warn!(
                            attempt,
                            "validation failed: {} errors; retrying with feedback",
                            last_err.len()
                        );
                        if attempt + 1 < max {
                            observe(BridgeAttemptEvent {
                                attempt: attempt + 2,
                                max_attempts: max,
                                retrying: true,
                            });
                        }
                        continue;
                    }
                    return Ok(ans);
                }
                Err(BridgeError::Transport(msg)) => {
                    let backoff = Duration::from_millis(250 * (1u64 << attempt));
                    tracing::warn!(attempt, "transport error: {msg}; backing off {backoff:?}");
                    if attempt + 1 < max {
                        observe(BridgeAttemptEvent {
                            attempt: attempt + 2,
                            max_attempts: max,
                            retrying: true,
                        });
                    }
                    tokio::time::sleep(backoff).await;
                    continue;
                }
                Err(e) => return Err(e),
            }
        }
        Err(BridgeError::ValidationFailed(last_err))
    }

    async fn stream_call(&self, call: &BridgeCall) -> Result<BridgeAnswer, BridgeError> {
        let prompt = call.render_prompt();
        match self.cfg.provider {
            Provider::OpenAi | Provider::Custom => self.stream_openai(&prompt, call).await,
            Provider::Anthropic => self.stream_anthropic(&prompt, call).await,
        }
    }

    async fn stream_openai(
        &self,
        prompt: &str,
        call: &BridgeCall,
    ) -> Result<BridgeAnswer, BridgeError> {
        let body = self.openai_request_body(prompt, call);
        let mut req = self
            .client
            .post(&self.cfg.endpoint)
            .header("content-type", "application/json")
            .json(&body);
        if let Some(k) = &self.cfg.api_key {
            req = req.bearer_auth(k);
        }
        let resp = req
            .send()
            .await
            .map_err(|e| BridgeError::Transport(e.to_string()))?;
        if !resp.status().is_success() {
            let status = resp.status().as_u16();
            let body = resp.text().await.unwrap_or_default();
            return Err(BridgeError::BadStatus { status, body });
        }
        let mut stream = resp.bytes_stream();
        let mut buf: Vec<u8> = Vec::new();
        let mut text = String::new();
        while let Some(chunk) = stream.next().await {
            let chunk = chunk.map_err(|e| BridgeError::Transport(e.to_string()))?;
            buf.extend_from_slice(&chunk);
            while let Some(idx) = find_sse_event(&buf) {
                let line = drain_sse_line(&mut buf, idx);
                if let Some(data) = line.strip_prefix("data: ") {
                    if data == "[DONE]" {
                        return Ok(BridgeAnswer {
                            text: std::mem::take(&mut text),
                            reasoning: String::new(),
                            prompt_tokens: 0,
                            completion_tokens: 0,
                        });
                    }
                    if let Ok(v) = serde_json::from_str::<serde_json::Value>(data) {
                        if let Some(delta) = v
                            .pointer("/choices/0/delta/content")
                            .and_then(|x| x.as_str())
                        {
                            text.push_str(delta);
                        }
                    }
                }
            }
        }
        Ok(BridgeAnswer {
            text,
            reasoning: String::new(),
            prompt_tokens: 0,
            completion_tokens: 0,
        })
    }

    fn openai_request_body(&self, prompt: &str, call: &BridgeCall) -> serde_json::Value {
        let mut body = serde_json::json!({
            "model": self.cfg.model,
            "messages": [
                { "role": "user", "content": prompt },
            ],
            "max_tokens": call.max_tokens,
            "stream": true,
        });
        let endpoint = self.cfg.endpoint.to_ascii_lowercase();
        let is_minimax = endpoint.contains("minimax.io") || endpoint.contains("minimaxi.com");
        if is_minimax {
            let object = body
                .as_object_mut()
                .expect("OpenAI request body is always an object");
            object.remove("max_tokens");
            object.insert(
                "max_completion_tokens".into(),
                serde_json::json!(call.max_tokens),
            );
            object.insert("reasoning_split".into(), serde_json::json!(true));
            if self
                .cfg
                .model
                .to_ascii_lowercase()
                .starts_with("minimax-m3")
            {
                object.insert("thinking".into(), serde_json::json!({ "type": "disabled" }));
            }
        }
        body
    }

    async fn stream_anthropic(
        &self,
        prompt: &str,
        call: &BridgeCall,
    ) -> Result<BridgeAnswer, BridgeError> {
        let body = serde_json::json!({
            "model": self.cfg.model,
            "max_tokens": call.max_tokens,
            "messages": [{ "role": "user", "content": prompt }],
            "stream": true,
            "system": call.system.clone().unwrap_or_default(),
        });
        let mut req = self
            .client
            .post(&self.cfg.endpoint)
            .header("content-type", "application/json");
        if let Some(k) = &self.cfg.api_key {
            req = req.header("x-api-key", k);
            req = req.header("anthropic-version", "2023-06-01");
        }
        let body_bytes =
            serde_json::to_vec(&body).map_err(|e| BridgeError::Transport(e.to_string()))?;
        let resp = req
            .body(body_bytes)
            .send()
            .await
            .map_err(|e: reqwest::Error| BridgeError::Transport(e.to_string()))?;
        if !resp.status().is_success() {
            let status = resp.status().as_u16();
            let body = resp.text().await.unwrap_or_default();
            return Err(BridgeError::BadStatus { status, body });
        }
        let mut stream = resp.bytes_stream();
        let mut buf: Vec<u8> = Vec::new();
        let mut text = String::new();
        while let Some(chunk) = stream.next().await {
            let chunk = chunk.map_err(|e: reqwest::Error| BridgeError::Transport(e.to_string()))?;
            buf.extend_from_slice(&chunk);
            while let Some(idx) = find_sse_event(&buf) {
                let line = drain_sse_line(&mut buf, idx);
                if let Some(data) = line.strip_prefix("data: ") {
                    if let Ok(v) = serde_json::from_str::<serde_json::Value>(data) {
                        if v.get("type").and_then(|x| x.as_str()) == Some("content_block_delta") {
                            if let Some(delta) = v.pointer("/delta/text").and_then(|x| x.as_str()) {
                                text.push_str(delta);
                            }
                        }
                    }
                }
            }
        }
        Ok(BridgeAnswer {
            text,
            reasoning: String::new(),
            prompt_tokens: 0,
            completion_tokens: 0,
        })
    }
}

/// Find the index of the next `\n\n` SSE event boundary in `buf`.
fn find_sse_event(buf: &[u8]) -> Option<usize> {
    buf.windows(2).position(|w| w == b"\n\n").map(|i| i + 2)
}

/// Drain everything up to and including `idx` from `buf`, returning the
/// drained slice as a UTF-8 string. Trims trailing whitespace per SSE.
fn drain_sse_line(buf: &mut Vec<u8>, idx: usize) -> String {
    let drained: Vec<u8> = buf.drain(..idx).collect();
    String::from_utf8_lossy(&drained).trim().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validator_feedback_is_appended() {
        let c = BridgeCall {
            prompt: "Hello".into(),
            system: None,
            max_tokens: 256,
            validator_feedback: vec!["err 1".into(), "err 2".into()],
        };
        let rendered = c.render_prompt();
        assert!(rendered.contains("Hello"));
        assert!(rendered.contains("<<validator-feedback>>"));
        assert!(rendered.contains("err 1"));
        assert!(rendered.contains("err 2"));
    }

    #[test]
    fn sse_drain_extracts_lines() {
        let mut buf: Vec<u8> = b"data: hello\n\n".to_vec();
        let idx = find_sse_event(&buf).unwrap();
        let line = drain_sse_line(&mut buf, idx);
        assert_eq!(line, "data: hello");
        assert!(buf.is_empty());
    }

    #[test]
    fn minimax_m3_structured_request_disables_thinking() {
        let bridge = AgentBridge::new(BridgeConfig {
            endpoint: "https://api.minimaxi.com/v1/chat/completions".into(),
            api_key: None,
            model: "MiniMax-M3".into(),
            provider: Provider::OpenAi,
            max_attempts: 3,
        });
        let body = bridge.openai_request_body(
            "Return JSON",
            &BridgeCall {
                prompt: "Return JSON".into(),
                system: None,
                max_tokens: 4096,
                validator_feedback: vec![],
            },
        );

        assert_eq!(body["thinking"]["type"], "disabled");
        assert_eq!(body["reasoning_split"], true);
        assert_eq!(body["max_completion_tokens"], 4096);
        assert!(body.get("max_tokens").is_none());
    }
}
