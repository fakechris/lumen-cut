//! Persistent worker pool — spawn/lifecycle.
//!
//! Tracks persistent worker processes that claim and submit tasks in a loop.
//! The actual worker code lives in
//! `agent::http` (HTTP server side) — this module is the lifecycle
//! supervisor on the orchestrator side.

use std::time::Duration;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkerSpec {
    pub name: String,
    pub lease_capacity: usize,
    pub command: Vec<String>,
    pub cwd: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum WorkerState {
    Spawning,
    Idle,
    Claiming,
    Submitting,
    Retrying,
    Exited,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkerStatus {
    pub name: String,
    pub state: WorkerState,
    pub last_heartbeat: chrono::DateTime<chrono::Utc>,
    pub errors: u32,
    pub completions: u32,
}

#[derive(Debug)]
pub struct WorkerPool {
    spec: Vec<WorkerSpec>,
    pub status: Vec<WorkerStatus>,
    pub heartbeat_grace: Duration,
}

impl WorkerPool {
    /// Build a pool of `n` named worker slots (`worker-0`..`worker-{n-1}`)
    /// matching the orchestrator's default concurrency — the slots external
    /// workers (or the in-process workers a future stage spawns) claim.
    pub fn new_workers(n: usize) -> Self {
        let spec: Vec<WorkerSpec> = (0..n.max(1))
            .map(|i| WorkerSpec {
                name: format!("worker-{i}"),
                lease_capacity: 1,
                command: vec![],
                cwd: None,
            })
            .collect();
        Self::new(spec)
    }

    pub fn new(spec: Vec<WorkerSpec>) -> Self {
        let status = spec
            .iter()
            .map(|s| WorkerStatus {
                name: s.name.clone(),
                state: WorkerState::Spawning,
                last_heartbeat: chrono::Utc::now(),
                errors: 0,
                completions: 0,
            })
            .collect();
        Self {
            spec,
            status,
            heartbeat_grace: Duration::from_secs(30),
        }
    }

    pub fn workers(&self) -> &[WorkerStatus] {
        &self.status
    }

    pub fn spec(&self, name: &str) -> Option<&WorkerSpec> {
        self.spec.iter().find(|s| s.name == name)
    }

    /// Mark a worker as having sent a heartbeat — fresh enough to keep
    /// running.
    pub fn heartbeat(&mut self, name: &str) {
        let now = chrono::Utc::now();
        if let Some(w) = self.status.iter_mut().find(|w| w.name == name) {
            w.last_heartbeat = now;
            if w.state == WorkerState::Spawning {
                w.state = WorkerState::Idle;
            }
        } else {
            // Register an external worker the first time it heartbeats —
            // External workers become visible through their heartbeats.
            self.status.push(WorkerStatus {
                name: name.into(),
                state: WorkerState::Idle,
                last_heartbeat: now,
                errors: 0,
                completions: 0,
            });
        }
    }

    /// Mark a worker as having completed one answer.
    pub fn record_completion(&mut self, name: &str) {
        if let Some(w) = self.status.iter_mut().find(|w| w.name == name) {
            w.completions = w.completions.saturating_add(1);
            w.state = WorkerState::Idle;
        }
    }

    /// Mark a worker as having errored.
    pub fn record_error(&mut self, name: &str) {
        if let Some(w) = self.status.iter_mut().find(|w| w.name == name) {
            w.errors = w.errors.saturating_add(1);
            w.state = WorkerState::Retrying;
        }
    }

    /// Stale workers (`now - heartbeat >= grace`) are marked Exited. The
    /// supervisor can then re-spawn them.
    pub fn reap_stale(&mut self) -> Vec<String> {
        let now = chrono::Utc::now();
        let mut dead = Vec::new();
        for w in &mut self.status {
            let age = now.signed_duration_since(w.last_heartbeat);
            if w.state != WorkerState::Exited
                && age.num_seconds() as u64 >= self.heartbeat_grace.as_secs()
            {
                w.state = WorkerState::Exited;
                dead.push(w.name.clone());
            }
        }
        dead
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn heartbeat_promotes_spawning_to_idle() {
        let mut p = WorkerPool::new(vec![WorkerSpec {
            name: "w1".into(),
            lease_capacity: 3,
            command: vec![],
            cwd: None,
        }]);
        assert_eq!(p.workers()[0].state, WorkerState::Spawning);
        p.heartbeat("w1");
        assert_eq!(p.workers()[0].state, WorkerState::Idle);
    }

    #[test]
    fn reap_marks_stale_as_exited() {
        let mut p = WorkerPool::new(vec![WorkerSpec {
            name: "w1".into(),
            lease_capacity: 1,
            command: vec![],
            cwd: None,
        }]);
        p.heartbeat_grace = Duration::from_millis(0);
        let dead = p.reap_stale();
        assert_eq!(dead, vec!["w1"]);
        assert_eq!(p.workers()[0].state, WorkerState::Exited);
    }
}
