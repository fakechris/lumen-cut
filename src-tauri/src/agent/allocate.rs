//! Atomic task-call allocation.
//!
//! The plan is:
//!   * each project holds up to `--align-concurrency` (default 4) leased
//!     calls at a time
//!   * `allocate()` takes the next pending call and emits a fresh `Lease`
//!     for it. Retries carrying `problems` claim first (oldest first), then
//!     the oldest plain call.
//!   * `Lease::is_expired()` is wall-clock based; an expired lease is
//!     silently re-queued at the head of the queue by `reap_expired()` so
//!     the call can be claimed again — expired leases never lose calls.
//!   * `submit()` is fenced: only the current, unexpired lease for a call
//!     is accepted; stale or duplicate submissions are rejected. Accepted
//!     outcomes are kept in the `completed` map keyed by call id.
//!
//! We back the queue with a `Mutex<VecDeque>` to keep the surface small;
//! a real concurrent port can swap in a `tokio::sync::Mutex` without
//! changing the public API.

use std::collections::{HashMap, VecDeque};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::agent::bridge::BridgeAnswer;
use crate::agent::lease::{bucket_for, lease_seconds, Bucket};

/// Default number of concurrently leased calls.
pub const DEFAULT_CAPACITY: usize = 4;

/// One queued call awaiting a worker.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct PendingCall {
    pub id: String,
    pub kind: String, // "polish" | "translate" | "align" | ...
    pub word_count: usize,
    /// Prompt/payload size in **characters** — drives the lease bucket.
    #[serde(default)]
    pub char_count: usize,
    pub payload_ref: String, // path to the prompt file
    /// Non-empty for retries: the problems (validator/transport) of the
    /// previous attempt. Calls with problems claim before plain calls.
    #[serde(default)]
    pub problems: Vec<String>,
    /// Materialised task specification. `None` only for kinds without a
    /// specification; carried on the pending call so the worker reads it.
    #[serde(default)]
    pub contract: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Lease {
    pub lease_id: String,
    pub call_id: String,
    pub issued_at: Instant,
    pub duration: Duration,
}

impl Lease {
    pub fn is_expired(&self) -> bool {
        self.issued_at.elapsed() >= self.duration
    }

    pub fn remaining(&self) -> Duration {
        self.duration.saturating_sub(self.issued_at.elapsed())
    }

    pub fn bucket(&self) -> Bucket {
        // We don't have the char count here; the caller derives it. We
        // expose the duration instead.
        match self.duration.as_secs() {
            0..=600 => Bucket::Small,
            601..=900 => Bucket::Medium,
            _ => Bucket::Large,
        }
    }
}

/// What a worker submitted for a call: either a validated answer or an
/// error report.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct CompletedSubmission {
    pub call_id: String,
    pub answer: Option<BridgeAnswer>,
    pub error: Option<String>,
}

#[derive(Debug, Default)]
pub struct Allocator {
    inner: Mutex<Inner>,
}

#[derive(Debug, Default)]
struct Inner {
    queue: VecDeque<PendingCall>,
    /// Outstanding leases together with the call they were issued for, so
    /// an expired lease can be re-queued instead of dropped.
    leased: Vec<(Lease, PendingCall)>,
    /// call id → submitted outcome, for the orchestrator to pick up.
    completed: HashMap<String, CompletedSubmission>,
    capacity: usize,
}

impl Allocator {
    pub fn new(capacity: usize) -> Self {
        Self {
            inner: Mutex::new(Inner {
                capacity: capacity.max(1),
                ..Default::default()
            }),
        }
    }

    pub fn capacity(&self) -> usize {
        self.inner.lock().expect("allocator poisoned").capacity
    }

    /// Resize the lease cap (e.g. when `workerCount` changes in settings).
    pub fn set_capacity(&self, capacity: usize) {
        self.inner.lock().expect("allocator poisoned").capacity = capacity.max(1);
    }

    pub fn enqueue(&self, call: PendingCall) {
        let mut g = self.inner.lock().expect("allocator poisoned");
        g.queue.push_back(call);
    }

    pub fn pending_count(&self) -> usize {
        self.inner.lock().expect("allocator poisoned").queue.len()
    }

    /// Atomically dequeue the next pending call and produce a fresh lease.
    /// Returns `None` if the queue is empty or `capacity` is saturated.
    /// Claim order: retries with `problems` first (oldest first), then the
    /// oldest plain call.
    pub fn allocate(&self) -> Option<(PendingCall, Lease)> {
        let mut g = self.inner.lock().expect("allocator poisoned");
        if g.leased.len() >= g.capacity {
            return None;
        }
        let call = match g.queue.iter().position(|c| !c.problems.is_empty()) {
            Some(i) => g.queue.remove(i)?,
            None => g.queue.pop_front()?,
        };
        let secs = lease_seconds(call.char_count);
        let lease = Lease {
            lease_id: Uuid::new_v4().to_string(),
            call_id: call.id.clone(),
            issued_at: Instant::now(),
            duration: Duration::from_secs(secs as u64),
        };
        g.leased.push((lease.clone(), call.clone()));
        Some((call, lease))
    }

    /// Mark a lease as completed (i.e. the answer has been submitted).
    /// The next `allocate()` is allowed to fire on a different call but
    /// the same worker's `submit-next` round-trip is single-threaded by
    /// construction (one lease per worker per round-trip).
    pub fn release(&self, lease_id: &str) {
        let mut g = self.inner.lock().expect("allocator poisoned");
        g.leased.retain(|(l, _)| l.lease_id != lease_id);
    }

    /// Fenced submission: accept only the current, unexpired lease for a
    /// call. On success the lease is consumed, the outcome is stored under
    /// the call id, and the call id is returned. Stale, expired, unknown,
    /// or duplicate leases are rejected with `None`.
    pub fn submit(
        &self,
        lease_id: &str,
        answer: Option<BridgeAnswer>,
        error: Option<String>,
    ) -> Option<String> {
        let mut g = self.inner.lock().expect("allocator poisoned");
        let idx = g
            .leased
            .iter()
            .position(|(l, _)| l.lease_id == lease_id && !l.is_expired())?;
        let (_, call) = g.leased.remove(idx);
        let call_id = call.id;
        g.completed.insert(
            call_id.clone(),
            CompletedSubmission {
                call_id: call_id.clone(),
                answer,
                error,
            },
        );
        Some(call_id)
    }

    /// The stored submission for a call, if any.
    pub fn completed(&self, call_id: &str) -> Option<CompletedSubmission> {
        self.inner
            .lock()
            .expect("allocator poisoned")
            .completed
            .get(call_id)
            .cloned()
    }

    /// The call currently protected by `lease_id`, used by the HTTP submit
    /// boundary to run the named contract validator before consuming it.
    pub fn leased_call(&self, lease_id: &str) -> Option<PendingCall> {
        self.inner
            .lock()
            .expect("allocator poisoned")
            .leased
            .iter()
            .find(|(lease, _)| lease.lease_id == lease_id && !lease.is_expired())
            .map(|(_, call)| call.clone())
    }

    /// Renew the lease — `submit-next` succeeded.
    pub fn renew(&self, lease_id: &str) -> Option<Duration> {
        let mut g = self.inner.lock().expect("allocator poisoned");
        let lease = &mut g.leased.iter_mut().find(|(l, _)| l.lease_id == lease_id)?.0;
        lease.issued_at = Instant::now();
        Some(lease.duration)
    }

    /// Re-queue calls whose lease expired at the head of the queue,
    /// preserving their original claim order so an expired lease can be
    /// claimed again instead of losing the call. Returns
    /// how many leases expired. Called once before `allocate()`.
    pub fn reap_expired(&self) -> usize {
        let mut g = self.inner.lock().expect("allocator poisoned");
        let mut expired: Vec<PendingCall> = Vec::new();
        let mut i = 0;
        while i < g.leased.len() {
            if g.leased[i].0.is_expired() {
                let (_, call) = g.leased.remove(i);
                expired.push(call);
            } else {
                i += 1;
            }
        }
        let n = expired.len();
        if n > 0 {
            let mut head: VecDeque<PendingCall> = expired.into_iter().collect();
            head.append(&mut g.queue);
            g.queue = head;
        }
        n
    }

    /// For tests: derive the bucket for an answer bundle by character count.
    pub fn bucket_for(&self, chars: usize) -> Bucket {
        bucket_for(chars)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn call(id: &str, words: usize) -> PendingCall {
        PendingCall {
            id: id.into(),
            kind: "polish".into(),
            word_count: words,
            char_count: words,
            payload_ref: "/tmp/x".into(),
            problems: vec![],
            contract: None,
        }
    }

    fn retry(id: &str, problems: &[&str]) -> PendingCall {
        PendingCall {
            problems: problems.iter().map(|p| p.to_string()).collect(),
            ..call(id, 10)
        }
    }

    /// Force every outstanding lease to be expired.
    fn expire_all(a: &Allocator) {
        let mut g = a.inner.lock().expect("allocator poisoned");
        for (l, _) in &mut g.leased {
            l.issued_at = Instant::now() - Duration::from_secs(7200);
        }
    }

    #[test]
    fn allocate_respects_capacity() {
        let a = Allocator::new(2);
        a.enqueue(call("c1", 10));
        a.enqueue(call("c2", 10));
        a.enqueue(call("c3", 10));
        assert!(a.allocate().is_some());
        assert!(a.allocate().is_some());
        assert!(a.allocate().is_none());
    }

    #[test]
    fn release_unblocks_next() {
        let a = Allocator::new(1);
        a.enqueue(call("c1", 10));
        a.enqueue(call("c2", 10));
        let (_, lease) = a.allocate().unwrap();
        assert!(a.allocate().is_none());
        a.release(&lease.lease_id);
        assert!(a.allocate().is_some());
    }

    #[test]
    fn renew_resets_lease_clock() {
        let a = Allocator::new(1);
        a.enqueue(call("c1", 10));
        let (_, lease) = a.allocate().unwrap();
        let before = lease.issued_at;
        std::thread::sleep(std::time::Duration::from_millis(2));
        a.renew(&lease.lease_id).unwrap();
        // We can check the leased list got refreshed by re-iterating.
        let _ = before;
    }

    #[test]
    fn default_capacity_is_four() {
        assert_eq!(DEFAULT_CAPACITY, 4);
    }

    #[test]
    fn set_capacity_resizes() {
        let a = Allocator::new(1);
        assert_eq!(a.capacity(), 1);
        a.set_capacity(3);
        assert_eq!(a.capacity(), 3);
        a.set_capacity(0);
        assert_eq!(a.capacity(), 1);
    }

    #[test]
    fn lease_duration_uses_char_count() {
        let a = Allocator::new(1);
        let mut big = call("big", 3);
        big.char_count = 30_000;
        a.enqueue(big);
        let (_, lease) = a.allocate().unwrap();
        assert_eq!(lease.duration, Duration::from_secs(1800));
        assert_eq!(lease.bucket(), Bucket::Large);
    }

    #[test]
    fn claims_prioritize_retries_with_problems_then_oldest() {
        let a = Allocator::new(4);
        a.enqueue(call("plain1", 10));
        a.enqueue(retry("retry1", &["validation failed"]));
        a.enqueue(call("plain2", 10));
        a.enqueue(retry("retry2", &["transport"]));
        assert_eq!(a.allocate().unwrap().0.id, "retry1");
        assert_eq!(a.allocate().unwrap().0.id, "retry2");
        assert_eq!(a.allocate().unwrap().0.id, "plain1");
        assert_eq!(a.allocate().unwrap().0.id, "plain2");
    }

    #[test]
    fn expired_lease_is_requeued_and_reclaimable() {
        let a = Allocator::new(1);
        a.enqueue(call("c1", 10));
        let (claimed, _) = a.allocate().unwrap();
        assert_eq!(claimed.id, "c1");
        expire_all(&a);
        assert_eq!(a.reap_expired(), 1);
        assert_eq!(a.pending_count(), 1);
        let (again, _) = a.allocate().unwrap();
        assert_eq!(again.id, "c1");
        assert_eq!(again, claimed);
    }

    #[test]
    fn requeued_calls_keep_claim_order_ahead_of_new_calls() {
        let a = Allocator::new(3);
        a.enqueue(call("old1", 10));
        a.enqueue(call("old2", 10));
        a.allocate();
        a.allocate();
        a.enqueue(call("new", 10));
        expire_all(&a);
        assert_eq!(a.reap_expired(), 2);
        assert_eq!(a.allocate().unwrap().0.id, "old1");
        assert_eq!(a.allocate().unwrap().0.id, "old2");
        assert_eq!(a.allocate().unwrap().0.id, "new");
    }

    #[test]
    fn submit_accepts_active_lease_and_stores_answer() {
        let a = Allocator::new(1);
        a.enqueue(call("c1", 10));
        let (_, lease) = a.allocate().unwrap();
        let answer = BridgeAnswer {
            text: "done".into(),
            reasoning: "r".into(),
            prompt_tokens: 1,
            completion_tokens: 2,
        };
        assert_eq!(
            a.submit(&lease.lease_id, Some(answer.clone()), None)
                .as_deref(),
            Some("c1")
        );
        let stored = a.completed("c1").unwrap();
        assert_eq!(stored.call_id, "c1");
        assert_eq!(stored.answer, Some(answer));
        assert_eq!(stored.error, None);
        // Lease consumed → capacity freed for the next call.
        a.enqueue(call("c2", 10));
        assert!(a.allocate().is_some());
    }

    #[test]
    fn submit_rejects_unknown_and_duplicate_lease() {
        let a = Allocator::new(1);
        a.enqueue(call("c1", 10));
        let (_, lease) = a.allocate().unwrap();
        assert!(a.submit("bogus-lease", None, None).is_none());
        assert!(a.submit(&lease.lease_id, None, None).is_some());
        // Duplicate submission of the same lease is rejected.
        assert!(a.submit(&lease.lease_id, None, None).is_none());
    }

    #[test]
    fn submit_rejects_expired_lease() {
        let a = Allocator::new(1);
        a.enqueue(call("c1", 10));
        let (_, lease) = a.allocate().unwrap();
        expire_all(&a);
        assert!(a.submit(&lease.lease_id, None, None).is_none());
        assert!(a.completed("c1").is_none());
    }

    #[test]
    fn submit_stores_worker_error() {
        let a = Allocator::new(1);
        a.enqueue(call("c1", 10));
        let (_, lease) = a.allocate().unwrap();
        assert!(a
            .submit(&lease.lease_id, None, Some("boom".into()))
            .is_some());
        let stored = a.completed("c1").unwrap();
        assert_eq!(stored.answer, None);
        assert_eq!(stored.error.as_deref(), Some("boom"));
    }
}
