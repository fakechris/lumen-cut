//! Global admission control for memory/compute-heavy pipelines.
//!
//! Apple unified memory is shared by CPU and GPU. Running ASR, diarization,
//! and video rendering together can therefore make every job slower and can
//! destabilize the desktop. Keep one heavy job active while lightweight UI,
//! file, and network work remains concurrent.

use std::sync::{Arc, Mutex, OnceLock};

use tokio::sync::{OwnedSemaphorePermit, Semaphore};

use crate::error::{AppError, AppResult};

#[derive(Clone)]
pub struct HeavyWorkGate {
    permits: Arc<Semaphore>,
    active: Arc<Mutex<Option<String>>>,
    waiting: Arc<std::sync::atomic::AtomicUsize>,
}

pub struct HeavyWorkPermit {
    _permit: OwnedSemaphorePermit,
    active: Arc<Mutex<Option<String>>>,
}

struct WaitingGuard(Arc<std::sync::atomic::AtomicUsize>);

impl Drop for WaitingGuard {
    fn drop(&mut self) {
        self.0.fetch_sub(1, std::sync::atomic::Ordering::Relaxed);
    }
}

impl Drop for HeavyWorkPermit {
    fn drop(&mut self) {
        *self.active.lock().expect("heavy work gate poisoned") = None;
    }
}

impl HeavyWorkGate {
    pub fn new(max_concurrent: usize) -> Self {
        Self {
            permits: Arc::new(Semaphore::new(max_concurrent.max(1))),
            active: Arc::new(Mutex::new(None)),
            waiting: Arc::new(std::sync::atomic::AtomicUsize::new(0)),
        }
    }

    pub fn active_label(&self) -> Option<String> {
        self.active
            .lock()
            .expect("heavy work gate poisoned")
            .clone()
    }

    pub fn waiting_count(&self) -> usize {
        self.waiting.load(std::sync::atomic::Ordering::Relaxed)
    }

    pub async fn acquire(&self, label: &str) -> AppResult<HeavyWorkPermit> {
        self.waiting
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let waiting = WaitingGuard(self.waiting.clone());
        let permit = loop {
            tokio::select! {
                permit = self.permits.clone().acquire_owned() => {
                    break permit.map_err(|_| AppError::Schema("heavy work gate closed".into()))?;
                }
                _ = tokio::time::sleep(std::time::Duration::from_millis(100)) => {
                    if crate::proc::cancellation_requested() {
                        return Err(AppError::Cancelled);
                    }
                }
            }
        };
        drop(waiting);
        *self.active.lock().expect("heavy work gate poisoned") = Some(label.to_string());
        Ok(HeavyWorkPermit {
            _permit: permit,
            active: self.active.clone(),
        })
    }
}

fn global_gate() -> &'static HeavyWorkGate {
    static GATE: OnceLock<HeavyWorkGate> = OnceLock::new();
    GATE.get_or_init(|| HeavyWorkGate::new(1))
}

pub async fn acquire_heavy(label: &str) -> AppResult<HeavyWorkPermit> {
    global_gate().acquire(label).await
}

pub fn active_heavy_label() -> Option<String> {
    global_gate().active_label()
}

pub fn waiting_heavy_count() -> usize {
    global_gate().waiting_count()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn heavy_work_is_serialized_across_pipeline_kinds() {
        let gate = HeavyWorkGate::new(1);
        let first = gate.acquire("transcription").await.unwrap();
        let waiting = gate.acquire("video-export");
        tokio::pin!(waiting);

        assert!(
            tokio::time::timeout(std::time::Duration::from_millis(30), &mut waiting)
                .await
                .is_err()
        );
        assert_eq!(gate.active_label().as_deref(), Some("transcription"));
        assert_eq!(gate.waiting_count(), 1);
        drop(first);
        assert!(
            tokio::time::timeout(std::time::Duration::from_secs(1), &mut waiting)
                .await
                .is_ok()
        );
        assert_eq!(gate.waiting_count(), 0);
    }
}
