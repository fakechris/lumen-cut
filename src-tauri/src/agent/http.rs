//! Local axum server exposing the claim/submit/submit-next protocol.
//!
//! Routes (Stage 4 baseline):
//!
//! | Method | Path                     | Body                | Result
//! |--------|--------------------------|---------------------|------------------------
//! | GET    | `/agent/next`            | —                   | next call + lease
//! | POST   | `/agent/submit`          | `BridgeAnswer` JSON | submit answer
//! | POST   | `/agent/submit-next`     | `BridgeAnswer` JSON | submit + new lease
//! | GET    | `/agent/answer/:call_id` | —                   | stored submission
//! | POST   | `/agent/heartbeat`       | `{worker}`          | bump last_heartbeat
//! | GET    | `/agent/workers`         | —                   | known workers + stale flag
//! | GET    | `/healthz`               | —                   | 200 OK
//!
//! The server is bound to **127.0.0.1 only**, never 0.0.0.0, by
//! construction (`SocketAddr::from(([127,0,0,1], port))`).  Tests verify
//! this.

use std::net::SocketAddr;
use std::sync::{Arc, Mutex};

use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::{IntoResponse, Json},
    routing::{get, post},
    Router,
};
use serde::{Deserialize, Serialize};

use crate::agent::allocate::{Allocator, PendingCall, SubmitError};
use crate::agent::bridge::BridgeAnswer;

#[derive(Clone)]
pub struct ServerState {
    /// The one shared allocator: the IPC layer enqueues into the same
    /// instance the HTTP routes claim from.
    pub allocator: Arc<Allocator>,
    /// Persistent worker supervisor — heartbeats and reaping route here so
    /// the HTTP layer and the orchestrator agree on worker state.
    pub pool: Arc<Mutex<crate::agent::pool::WorkerPool>>,
}

impl ServerState {
    pub fn new(
        allocator: Arc<Allocator>,
        pool: Arc<Mutex<crate::agent::pool::WorkerPool>>,
    ) -> Self {
        Self { allocator, pool }
    }
}

pub fn router(state: ServerState) -> Router {
    Router::new()
        .route("/agent/next", get(agent_next))
        .route("/agent/submit", post(agent_submit))
        .route("/agent/submit-next", post(agent_submit_next))
        .route("/agent/answer/:call_id", get(agent_answer))
        .route("/agent/heartbeat", post(agent_heartbeat))
        .route("/agent/workers", get(agent_workers))
        .route("/healthz", get(healthz))
        .with_state(state)
}

#[derive(Debug, Serialize)]
struct ClaimResponse {
    call: PendingCall,
    lease_id: String,
    duration_seconds: u32,
}

pub(crate) async fn agent_next(State(s): State<ServerState>) -> impl IntoResponse {
    s.allocator.reap_expired();
    match s.allocator.allocate() {
        Some((call, lease)) => {
            let body = ClaimResponse {
                call,
                lease_id: lease.lease_id,
                duration_seconds: lease.duration.as_secs() as u32,
            };
            (StatusCode::OK, Json(body)).into_response()
        }
        None => (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(serde_json::json!({"error": "no pending call"})),
        )
            .into_response(),
    }
}

#[derive(Debug, Deserialize)]
pub(crate) struct SubmitBody {
    lease_id: String,
    #[serde(default)]
    answer: Option<BridgeAnswer>,
    #[serde(default)]
    error: Option<String>,
}

/// Fenced submit: only the current, unexpired lease for a call is
/// accepted — stale or duplicate workers get 409, a missing lease id 400.
fn submit_fenced_blocking(
    s: &ServerState,
    body: SubmitBody,
) -> Result<String, (StatusCode, String)> {
    if body.lease_id.trim().is_empty() {
        return Err((StatusCode::BAD_REQUEST, "missing lease_id".into()));
    }
    if body.answer.is_some() == body.error.is_some() {
        return Err((
            StatusCode::BAD_REQUEST,
            "submit requires exactly one of answer or error".into(),
        ));
    }
    let call = s
        .allocator
        .leased_call(&body.lease_id)
        .ok_or((StatusCode::CONFLICT, "unknown or stale lease".into()))?;
    if let Some(answer) = &body.answer {
        if let Err(errors) = crate::agent::task::validate_call_answer(&call, &answer.text) {
            return Err((
                StatusCode::UNPROCESSABLE_ENTITY,
                format!("submit lint: {}", errors.join("; ")),
            ));
        }
    }
    s.allocator
        .submit(&body.lease_id, body.answer, body.error)
        .map_err(|error| match error {
            SubmitError::StaleLease => (StatusCode::CONFLICT, "unknown or stale lease".into()),
            SubmitError::Persistence(message) => (StatusCode::INTERNAL_SERVER_ERROR, message),
        })
}

async fn submit_fenced(s: &ServerState, body: SubmitBody) -> Result<String, (StatusCode, String)> {
    let state = s.clone();
    tokio::task::spawn_blocking(move || submit_fenced_blocking(&state, body))
        .await
        .map_err(|error| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("submission task failed: {error}"),
            )
        })?
}

pub(crate) async fn agent_submit(
    State(s): State<ServerState>,
    Json(body): Json<SubmitBody>,
) -> impl IntoResponse {
    match submit_fenced(&s, body).await {
        Ok(call_id) => (
            StatusCode::OK,
            Json(serde_json::json!({"ok": true, "call_id": call_id})),
        )
            .into_response(),
        Err((code, msg)) => (code, Json(serde_json::json!({"error": msg}))).into_response(),
    }
}

pub(crate) async fn agent_submit_next(
    State(s): State<ServerState>,
    Json(body): Json<SubmitBody>,
) -> impl IntoResponse {
    match submit_fenced(&s, body).await {
        Err((code, msg)) => (code, Json(serde_json::json!({"error": msg}))).into_response(),
        Ok(_) => {
            s.allocator.reap_expired();
            match s.allocator.allocate() {
                Some((call, lease)) => {
                    let resp = ClaimResponse {
                        call,
                        lease_id: lease.lease_id,
                        duration_seconds: lease.duration.as_secs() as u32,
                    };
                    (StatusCode::OK, Json(resp)).into_response()
                }
                None => (
                    StatusCode::OK,
                    Json(serde_json::json!({
                        "status": "done",
                        "call": null,
                    })),
                )
                    .into_response(),
            }
        }
    }
}

/// The orchestrator picks up submitted outcomes here.
pub(crate) async fn agent_answer(
    State(s): State<ServerState>,
    Path(call_id): Path<String>,
) -> impl IntoResponse {
    match s.allocator.completed(&call_id) {
        Some(sub) => (StatusCode::OK, Json(sub)).into_response(),
        None => (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({"error": "no submission for call"})),
        )
            .into_response(),
    }
}

#[derive(Debug, Deserialize)]
pub(crate) struct HeartbeatBody {
    worker: String,
}

pub(crate) async fn agent_heartbeat(
    State(s): State<ServerState>,
    Json(body): Json<HeartbeatBody>,
) -> impl IntoResponse {
    let name = body.worker.trim();
    if name.is_empty() {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({"error": "missing worker"})),
        )
            .into_response();
    }
    s.pool.lock().expect("pool poisoned").heartbeat(name);
    (StatusCode::OK, Json(serde_json::json!({"ok": true}))).into_response()
}

#[derive(Debug, Serialize)]
struct WorkerReport {
    worker: String,
    last_seen_ms_ago: u64,
    stale: bool,
}

pub(crate) async fn agent_workers(State(s): State<ServerState>) -> impl IntoResponse {
    let workers: Vec<WorkerReport> = {
        let mut p = s.pool.lock().expect("pool poisoned");
        let _dead = p.reap_stale();
        let now = chrono::Utc::now();
        p.workers()
            .iter()
            .map(|w| WorkerReport {
                worker: w.name.clone(),
                last_seen_ms_ago: now
                    .signed_duration_since(w.last_heartbeat)
                    .num_milliseconds()
                    .max(0) as u64,
                stale: w.state == crate::agent::pool::WorkerState::Exited,
            })
            .collect()
    };
    (
        StatusCode::OK,
        Json(serde_json::json!({ "workers": workers })),
    )
        .into_response()
}

async fn healthz() -> impl IntoResponse {
    (StatusCode::OK, "ok")
}

/// Bind to 127.0.0.1 with the requested port (0 means "OS-assigned").
/// The router shares the caller-provided allocator, so calls enqueued
/// through the IPC layer are claimable over HTTP. Returns the bound
/// address so the orchestrator can publish it via env var to the worker
/// processes.
pub async fn bind(
    port: u16,
    allocator: Arc<Allocator>,
    pool: Arc<Mutex<crate::agent::pool::WorkerPool>>,
) -> std::io::Result<(SocketAddr, Router)> {
    let addr = SocketAddr::from(([127, 0, 0, 1], port));
    let router = router(ServerState::new(allocator, pool));
    Ok((addr, router))
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::to_bytes;

    fn state_with_capacity(cap: usize) -> (Arc<Allocator>, ServerState) {
        let alloc = Arc::new(Allocator::new(cap));
        let pool = Arc::new(Mutex::new(crate::agent::pool::WorkerPool::new_workers(
            cap.max(1),
        )));
        let state = ServerState::new(alloc.clone(), pool);
        (alloc, state)
    }

    fn call(id: &str) -> PendingCall {
        PendingCall {
            id: id.into(),
            kind: "test".into(),
            word_count: 5,
            char_count: 5,
            payload_ref: "/tmp/x".into(),
            submission_ref: None,
            problems: vec![],
            contract: None,
        }
    }

    fn submit(lease_id: &str) -> SubmitBody {
        SubmitBody {
            lease_id: lease_id.into(),
            answer: Some(BridgeAnswer {
                text: "done".into(),
                reasoning: String::new(),
                prompt_tokens: 0,
                completion_tokens: 0,
            }),
            error: None,
        }
    }

    async fn body_json(resp: axum::response::Response) -> serde_json::Value {
        let bytes = to_bytes(resp.into_body(), usize::MAX).await.unwrap();
        serde_json::from_slice(&bytes).unwrap()
    }

    #[tokio::test]
    async fn bind_only_localhost() {
        let alloc = Arc::new(Allocator::new(1));
        let pool = Arc::new(Mutex::new(crate::agent::pool::WorkerPool::new_workers(1)));
        let (addr, _router) = bind(0, alloc, pool).await.unwrap();
        assert_eq!(addr.ip().to_string(), "127.0.0.1");
    }

    /// Regression: the HTTP router and the IPC layer share ONE allocator —
    /// a call enqueued through the shared handle is claimable from the
    /// router's state. (The old code built a second allocator for IPC, so
    /// enqueued calls could never be claimed.)
    #[tokio::test]
    async fn claim_returns_call_enqueued_via_shared_allocator() {
        let (alloc, state) = state_with_capacity(1);
        alloc.enqueue(call("c1"));
        let resp = agent_next(State(state)).await.into_response();
        assert_eq!(resp.status(), StatusCode::OK);
        let v = body_json(resp).await;
        assert_eq!(v["call"]["id"], "c1");
        assert!(v["lease_id"].is_string());
    }

    #[tokio::test]
    async fn next_returns_503_when_queue_empty() {
        let (_alloc, state) = state_with_capacity(1);
        let resp = agent_next(State(state)).await.into_response();
        assert_eq!(resp.status(), StatusCode::SERVICE_UNAVAILABLE);
    }

    #[tokio::test]
    async fn submit_with_wrong_lease_is_rejected() {
        let (alloc, state) = state_with_capacity(1);
        alloc.enqueue(call("c1"));
        let claim = agent_next(State(state.clone())).await.into_response();
        assert_eq!(claim.status(), StatusCode::OK);

        let resp = agent_submit(State(state.clone()), Json(submit("bogus-lease")))
            .await
            .into_response();
        assert_eq!(resp.status(), StatusCode::CONFLICT);

        let resp = agent_submit(State(state), Json(submit("  ")))
            .await
            .into_response();
        assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn submit_requires_exactly_one_outcome() {
        let (alloc, state) = state_with_capacity(1);
        alloc.enqueue(call("c1"));
        let claim = agent_next(State(state.clone())).await.into_response();
        let value = body_json(claim).await;
        let lease_id = value["lease_id"].as_str().unwrap().to_string();

        let empty = SubmitBody {
            lease_id: lease_id.clone(),
            answer: None,
            error: None,
        };
        let response = agent_submit(State(state.clone()), Json(empty))
            .await
            .into_response();
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);

        let mut both = submit(&lease_id);
        both.error = Some("worker failed".into());
        let response = agent_submit(State(state), Json(both)).await.into_response();
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn submit_stores_answer_and_answer_is_retrievable() {
        let (alloc, state) = state_with_capacity(1);
        alloc.enqueue(call("c1"));
        let claim = agent_next(State(state.clone())).await.into_response();
        let v = body_json(claim).await;
        let lease_id = v["lease_id"].as_str().unwrap().to_string();

        let answer = BridgeAnswer {
            text: "done".into(),
            reasoning: "r".into(),
            prompt_tokens: 1,
            completion_tokens: 2,
        };
        let mut body = submit(&lease_id);
        body.answer = Some(answer);
        let resp = agent_submit(State(state.clone()), Json(body))
            .await
            .into_response();
        assert_eq!(resp.status(), StatusCode::OK);

        // Duplicate submission of the consumed lease is rejected.
        let dup = agent_submit(State(state.clone()), Json(submit(&lease_id)))
            .await
            .into_response();
        assert_eq!(dup.status(), StatusCode::CONFLICT);

        let resp = agent_answer(State(state.clone()), Path("c1".to_string()))
            .await
            .into_response();
        assert_eq!(resp.status(), StatusCode::OK);
        let v = body_json(resp).await;
        assert_eq!(v["call_id"], "c1");
        assert_eq!(v["answer"]["text"], "done");

        let missing = agent_answer(State(state), Path("nope".to_string()))
            .await
            .into_response();
        assert_eq!(missing.status(), StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn invalid_task_answer_is_rejected_without_consuming_lease() {
        let (alloc, state) = state_with_capacity(1);
        let tmp = tempfile::tempdir().unwrap();
        let payload = tmp.path().join("translate.json");
        std::fs::write(
            &payload,
            r#"{"lang":"zh","lines":[{"id":"s1","source":"hello","maxChars":22,"rt":[]}]}"#,
        )
        .unwrap();
        alloc.enqueue(PendingCall {
            id: "translate-1".into(),
            kind: "translate".into(),
            word_count: 1,
            char_count: 5,
            payload_ref: payload.to_string_lossy().into_owned(),
            submission_ref: None,
            problems: vec![],
            contract: None,
        });
        let claim = agent_next(State(state.clone())).await.into_response();
        let value = body_json(claim).await;
        let lease_id = value["lease_id"].as_str().unwrap().to_string();

        let mut invalid = submit(&lease_id);
        invalid.answer = Some(BridgeAnswer {
            text: r#"{"translations":{}}"#.into(),
            reasoning: String::new(),
            prompt_tokens: 0,
            completion_tokens: 0,
        });
        let rejected = agent_submit(State(state.clone()), Json(invalid))
            .await
            .into_response();
        assert_eq!(rejected.status(), StatusCode::UNPROCESSABLE_ENTITY);
        assert!(alloc.leased_call(&lease_id).is_some());

        let mut valid = submit(&lease_id);
        valid.answer = Some(BridgeAnswer {
            text: r#"{"summary":"x","terms":[],"namedEntities":[],"translations":{"s1":"你好"}}"#
                .into(),
            reasoning: String::new(),
            prompt_tokens: 0,
            completion_tokens: 0,
        });
        let accepted = agent_submit(State(state), Json(valid))
            .await
            .into_response();
        assert_eq!(accepted.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn submit_next_fences_then_claims_next() {
        let (alloc, state) = state_with_capacity(1);
        alloc.enqueue(call("c1"));
        alloc.enqueue(call("c2"));
        let claim = agent_next(State(state.clone())).await.into_response();
        let v = body_json(claim).await;
        let lease_id = v["lease_id"].as_str().unwrap().to_string();

        // Wrong lease → 409, and no next call is handed out.
        let bad = agent_submit_next(State(state.clone()), Json(submit("bogus-lease")))
            .await
            .into_response();
        assert_eq!(bad.status(), StatusCode::CONFLICT);

        let resp = agent_submit_next(State(state), Json(submit(&lease_id)))
            .await
            .into_response();
        assert_eq!(resp.status(), StatusCode::OK);
        let v = body_json(resp).await;
        assert_eq!(v["call"]["id"], "c2");
    }

    #[tokio::test]
    async fn submit_next_reports_done_after_accepting_the_final_call() {
        let (allocator, state) = state_with_capacity(1);
        allocator.enqueue(call("only"));
        let claim = agent_next(State(state.clone())).await.into_response();
        let claim_json = body_json(claim).await;
        let lease_id = claim_json["lease_id"].as_str().unwrap().to_string();
        let response = agent_submit_next(State(state), Json(submit(&lease_id)))
            .await
            .into_response();
        assert_eq!(response.status(), StatusCode::OK);
        let value = body_json(response).await;
        assert_eq!(value["status"], "done");
        assert!(value["call"].is_null());
    }

    #[tokio::test]
    async fn heartbeat_registers_worker_and_reports_stale() {
        let (_alloc, state) = state_with_capacity(1);
        let resp = agent_heartbeat(
            State(state.clone()),
            Json(HeartbeatBody {
                worker: "w1".into(),
            }),
        )
        .await
        .into_response();
        assert_eq!(resp.status(), StatusCode::OK);

        // w1 registers dynamically alongside the pool's worker-0 slot, so
        // look it up by name rather than by index.
        let find = |v: &serde_json::Value| -> serde_json::Value {
            v["workers"]
                .as_array()
                .unwrap()
                .iter()
                .find(|w| w["worker"] == "w1")
                .cloned()
                .expect("w1 registered by heartbeat")
        };

        let resp = agent_workers(State(state.clone())).await.into_response();
        let v = body_json(resp).await;
        assert_eq!(find(&v)["stale"], false);

        // Backdate the worker beyond the pool's heartbeat grace window.
        {
            let mut p = state.pool.lock().expect("pool poisoned");
            let w = p.status.iter_mut().find(|w| w.name == "w1").unwrap();
            w.last_heartbeat = chrono::Utc::now() - chrono::Duration::seconds(120);
        }
        let resp = agent_workers(State(state)).await.into_response();
        let v = body_json(resp).await;
        assert_eq!(find(&v)["stale"], true);
    }

    #[tokio::test]
    async fn heartbeat_rejects_empty_worker() {
        let (_alloc, state) = state_with_capacity(1);
        let resp = agent_heartbeat(
            State(state),
            Json(HeartbeatBody {
                worker: "  ".into(),
            }),
        )
        .await
        .into_response();
        assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    }
}
