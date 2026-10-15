//! Explicitly triggered native batch workers and bounded control operations.
//! Durable state belongs to lr-batches. Runtime state never resumes a create.
use dashmap::{DashMap, DashSet};
pub mod api;
pub(crate) mod context;
pub(crate) mod policy;
mod results;
pub(crate) mod rows;
pub mod types;
mod worker;
use std::sync::Arc;
use tokio::sync::{OwnedSemaphorePermit, Semaphore};
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

pub struct BatchRuntime {
    admission: parking_lot::Mutex<()>,
    workers: Arc<DashMap<String, WorkerState>>,
    controls: Arc<DashSet<String>>,
    pub(crate) validation_slots: Arc<Semaphore>,
    control_slots: Arc<Semaphore>,
}
struct WorkerState {
    run_id: String,
    token: CancellationToken,
}
pub struct BatchWorkerGuard {
    workers: Arc<DashMap<String, WorkerState>>,
    job_id: String,
    run_id: String,
    pub token: CancellationToken,
}
impl Drop for BatchWorkerGuard {
    fn drop(&mut self) {
        self.workers
            .remove_if(&self.job_id, |_, value| value.run_id == self.run_id);
    }
}
pub struct BatchControlGuard {
    controls: Arc<DashSet<String>>,
    job_id: String,
    _permit: OwnedSemaphorePermit,
}
impl Drop for BatchControlGuard {
    fn drop(&mut self) {
        self.controls.remove(&self.job_id);
    }
}
impl Default for BatchRuntime {
    fn default() -> Self {
        Self {
            admission: parking_lot::Mutex::new(()),
            workers: Arc::new(DashMap::new()),
            controls: Arc::new(DashSet::new()),
            validation_slots: Arc::new(Semaphore::new(2)),
            control_slots: Arc::new(Semaphore::new(2)),
        }
    }
}
impl BatchRuntime {
    pub fn try_preflight(&self) -> Option<OwnedSemaphorePermit> {
        self.validation_slots.clone().try_acquire_owned().ok()
    }
    pub fn claim_worker(&self, job_id: &str) -> Option<BatchWorkerGuard> {
        use dashmap::mapref::entry::Entry;
        let _admission = self.admission.lock();
        if self.controls.contains(job_id) {
            return None;
        }
        let token = CancellationToken::new();
        let run_id = Uuid::new_v4().simple().to_string();
        match self.workers.entry(job_id.into()) {
            Entry::Occupied(_) => None,
            Entry::Vacant(entry) => {
                entry.insert(WorkerState {
                    run_id: run_id.clone(),
                    token: token.clone(),
                });
                Some(BatchWorkerGuard {
                    workers: self.workers.clone(),
                    job_id: job_id.into(),
                    run_id,
                    token,
                })
            }
        }
    }
    pub fn worker_running(&self, job_id: &str) -> bool {
        self.workers.contains_key(job_id)
    }
    pub fn cancel_worker(&self, job_id: &str) -> bool {
        if let Some(worker) = self.workers.get(job_id) {
            worker.token.cancel();
            true
        } else {
            false
        }
    }
    pub fn cancel_all_workers(&self) {
        for worker in self.workers.iter() {
            worker.token.cancel();
        }
    }
    pub fn try_control(&self, job_id: &str) -> Option<BatchControlGuard> {
        let _admission = self.admission.lock();
        if self.workers.contains_key(job_id) {
            return None;
        }
        let permit = self.control_slots.clone().try_acquire_owned().ok()?;
        if !self.controls.insert(job_id.into()) {
            return None;
        }
        Some(BatchControlGuard {
            controls: self.controls.clone(),
            job_id: job_id.into(),
            _permit: permit,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn worker_claim_is_unique_and_shutdown_does_not_resubmit() {
        let runtime = BatchRuntime::default();
        let worker = runtime.claim_worker("job").unwrap();
        assert!(runtime.claim_worker("job").is_none());
        assert!(runtime.worker_running("job"));
        assert!(runtime.try_control("job").is_none());
        runtime.cancel_all_workers();
        assert!(worker.token.is_cancelled());
        drop(worker);
        assert!(!runtime.worker_running("job"));
        let resumed = runtime.claim_worker("job").unwrap();
        assert!(!resumed.token.is_cancelled());
    }
    #[test]
    fn controls_are_bounded_globally_and_per_job_and_drop_releases_them() {
        let runtime = BatchRuntime::default();
        let first = runtime.try_control("one").unwrap();
        assert!(runtime.claim_worker("one").is_none());
        assert!(runtime.try_control("one").is_none());
        let second = runtime.try_control("two").unwrap();
        assert!(runtime.try_control("three").is_none());
        drop(first);
        assert!(runtime.try_control("three").is_some());
        drop(second);
        assert!(runtime.try_control("two").is_some());
    }
}
