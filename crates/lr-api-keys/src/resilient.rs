//! A storage outage must not turn secret scanning into a request gate.
//! After one backend failure, stay in memory for this process lifetime. This
//! avoids repeated permission prompts and never overwrites unread durable maps
//! with a partial fallback inventory when the backend becomes available again.
use crate::KeychainStorage;
use lr_types::AppResult;
use parking_lot::Mutex;
use std::{collections::BTreeMap, sync::Arc};
use zeroize::Zeroizing;

#[derive(Default)]
struct State {
    volatile: bool,
    values: BTreeMap<(String, String), Option<Zeroizing<String>>>,
}
pub struct ResilientSecretStorage {
    backend: Arc<dyn KeychainStorage>,
    state: Mutex<State>,
}
impl ResilientSecretStorage {
    pub fn new(backend: Arc<dyn KeychainStorage>) -> Self {
        Self {
            backend,
            state: Mutex::new(State::default()),
        }
    }
}
impl KeychainStorage for ResilientSecretStorage {
    fn persistence_warning(&self) -> bool {
        self.state.lock().volatile
    }
    fn quarantine_unreadable(&self, service: &str, account: &str) -> bool {
        let mut state = self.state.lock();
        state.volatile = true;
        state.values.remove(&(service.into(), account.into()));
        true
    }
    fn get(&self, service: &str, account: &str) -> AppResult<Option<String>> {
        let mut state = self.state.lock();
        let key = (service.into(), account.into());
        if !state.volatile {
            match self.backend.get(service, account) {
                Ok(value) => {
                    state
                        .values
                        .insert(key, value.as_ref().map(|v| Zeroizing::new(v.clone())));
                    return Ok(value);
                }
                Err(_) => state.volatile = true,
            }
        }
        Ok(state
            .values
            .get(&key)
            .and_then(|v| v.as_ref())
            .map(|v| v.to_string()))
    }
    fn store(&self, service: &str, account: &str, value: &str) -> AppResult<()> {
        let mut state = self.state.lock();
        if !state.volatile && self.backend.store(service, account, value).is_err() {
            state.volatile = true;
        }
        state.values.insert(
            (service.into(), account.into()),
            Some(Zeroizing::new(value.into())),
        );
        Ok(())
    }
    fn delete(&self, service: &str, account: &str) -> AppResult<()> {
        let mut state = self.state.lock();
        if !state.volatile && self.backend.delete(service, account).is_err() {
            state.volatile = true;
        }
        state.values.insert((service.into(), account.into()), None);
        Ok(())
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};
    struct Broken(AtomicUsize);
    impl KeychainStorage for Broken {
        fn get(&self, _: &str, _: &str) -> AppResult<Option<String>> {
            self.0.fetch_add(1, Ordering::Relaxed);
            Err(lr_types::AppError::Internal("fixture".into()))
        }
        fn store(&self, _: &str, _: &str, _: &str) -> AppResult<()> {
            self.0.fetch_add(1, Ordering::Relaxed);
            Err(lr_types::AppError::Internal("fixture".into()))
        }
        fn delete(&self, _: &str, _: &str) -> AppResult<()> {
            self.0.fetch_add(1, Ordering::Relaxed);
            Err(lr_types::AppError::Internal("fixture".into()))
        }
    }
    #[test]
    fn outages_keep_owned_values_and_tombstones_without_repeated_backend_calls() {
        let backend = Arc::new(Broken(AtomicUsize::new(0)));
        let storage = ResilientSecretStorage::new(backend.clone());
        assert_eq!(storage.get("s", "client-a").unwrap(), None);
        storage.store("s", "client-a", "synthetic").unwrap();
        assert_eq!(
            storage.get("s", "client-a").unwrap().as_deref(),
            Some("synthetic")
        );
        assert_eq!(storage.get("s", "client-b").unwrap(), None);
        storage.delete("s", "client-a").unwrap();
        assert_eq!(storage.get("s", "client-a").unwrap(), None);
        assert!(storage.persistence_warning());
        assert_eq!(backend.0.load(Ordering::Relaxed), 1);
    }
}

#[cfg(test)]
mod write_failure_tests {
    use super::*;
    use std::sync::atomic::{AtomicBool, Ordering};
    struct Backend {
        data: crate::MockKeychain,
        fail: AtomicBool,
    }
    impl KeychainStorage for Backend {
        fn get(&self, s: &str, a: &str) -> AppResult<Option<String>> {
            self.data.get(s, a)
        }
        fn store(&self, s: &str, a: &str, v: &str) -> AppResult<()> {
            if self.fail.load(Ordering::Relaxed) {
                Err(lr_types::AppError::Storage("write unavailable".into()))
            } else {
                self.data.store(s, a, v)
            }
        }
        fn delete(&self, s: &str, a: &str) -> AppResult<()> {
            self.data.delete(s, a)
        }
    }
    #[test]
    fn failed_writes_keep_usable_memory_values_without_overwriting_durable_records_on_recovery() {
        let backend = Arc::new(Backend {
            data: crate::MockKeychain::new(),
            fail: AtomicBool::new(false),
        });
        backend.store("s", "a", "durable-original").unwrap();
        let storage = ResilientSecretStorage::new(backend.clone());
        assert_eq!(
            storage.get("s", "a").unwrap().as_deref(),
            Some("durable-original")
        );
        backend.fail.store(true, Ordering::Relaxed);
        storage.store("s", "a", "memory-original").unwrap();
        assert_eq!(
            storage.get("s", "a").unwrap().as_deref(),
            Some("memory-original")
        );
        backend.fail.store(false, Ordering::Relaxed);
        storage.store("s", "a", "second-memory-original").unwrap();
        assert_eq!(
            storage.get("s", "a").unwrap().as_deref(),
            Some("second-memory-original")
        );
        assert_eq!(
            backend.get("s", "a").unwrap().as_deref(),
            Some("durable-original")
        );
        assert!(storage.persistence_warning());
    }
}
