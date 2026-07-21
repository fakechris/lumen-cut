//! Agent claim/submit protocol. Drives LLM calls through a local HTTP
//! server that persistent worker processes can interact with.

pub mod allocate;
pub mod bridge;
pub mod contract;
pub mod http;
pub mod lease;
pub mod mcp;
pub mod pool;
pub mod runtime;
pub mod task;

pub use allocate::{Allocator, CompletedSubmission, Lease, PendingCall, DEFAULT_CAPACITY};
pub use bridge::{AgentBridge, BridgeAnswer, BridgeCall, BridgeConfig, BridgeError, Provider};
pub use contract::contract_for_kind;
pub use http::{bind, router, ServerState};
pub use lease::{bucket_for, lease_seconds, Bucket};
pub use pool::{WorkerPool, WorkerSpec, WorkerState, WorkerStatus};
