//! Utility functions and helpers for LocalRouter

pub mod crypto;
pub mod paths;
pub mod test_mode;

// Re-export errors from lr-types for backward compatibility (utils::errors::AppError)
pub use lr_types::errors;

#[cfg(test)]
pub(crate) mod test_support {
    use std::sync::{Mutex, MutexGuard};

    /// Mutex serializing every test in this crate that reads or writes the
    /// LOCALROUTER_ENV env var. Env vars are process-global, so parallel
    /// tests that mutate them race — the lock must be shared across all
    /// test modules (paths, test_mode), not per-module.
    static ENV_LOCK: Mutex<()> = Mutex::new(());

    pub(crate) fn env_lock() -> MutexGuard<'static, ()> {
        // A test that panicked while holding the lock poisons it; the env
        // var state is still fine for the next test, so recover the guard.
        ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner())
    }
}
