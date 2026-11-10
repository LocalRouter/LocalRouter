//! Nonblocking credential outage handling with durable, lossless recovery.
use crate::KeychainStorage;
use lr_types::{AppError, AppResult};
use parking_lot::Mutex;
use std::{
    collections::{BTreeMap, VecDeque},
    sync::Arc,
    time::{Duration, Instant},
};
use zeroize::Zeroizing;

type Key = (String, String);
type Secret = Option<Zeroizing<String>>;
#[derive(Clone, Default)]
struct Entry {
    value: Secret,
    // None: never successfully read. Some(None): confirmed absent.
    baseline: Option<Secret>,
    dirty: bool,
    quarantined: bool,
    probe: bool,
}
#[derive(Clone, Debug)]
pub struct PersistenceNotice {
    pub recovered: bool,
    pub operation: String,
    pub cause: String,
    pub pending_entries: usize,
}
impl PersistenceNotice {
    pub fn message(&self) -> String {
        if self.recovered {
            "Secret storage recovered; pending mappings and deletions are durable".into()
        } else {
            format!("Secret storage {} failed: {}. Mappings remain usable in memory; recovery retries automatically. Pending changes are not durable until recovery completes.",self.operation,self.cause)
        }
    }
}
#[derive(Default)]
struct State {
    volatile: bool,
    recovering: bool,
    retry_at: Option<Instant>,
    failures: u32,
    entries: BTreeMap<Key, Entry>,
    failure: Option<PersistenceNotice>,
    events: VecDeque<PersistenceNotice>,
    observer: Option<Arc<PersistenceObserver>>,
}
impl State {
    fn pending(&self) -> usize {
        self.entries
            .values()
            .filter(|e| e.dirty || e.quarantined || e.probe || e.baseline.is_none())
            .count()
    }
    fn enqueue(&mut self, event: PersistenceNotice) {
        if self.events.len() == 64 {
            self.events.pop_front();
        }
        self.events.push_back(event);
    }
    fn failed(&mut self, operation: &str, error: &AppError) {
        let mut cause = error.to_string();
        // Backend diagnostics must not reproduce a credential or serialized map.
        let mut originals = Vec::new();
        fn strings(value: &serde_json::Value, out: &mut Vec<String>) {
            match value {
                serde_json::Value::String(s) if !s.is_empty() => out.push(s.clone()),
                serde_json::Value::Object(v) => {
                    for value in v.values() {
                        strings(value, out)
                    }
                }
                serde_json::Value::Array(v) => {
                    for value in v {
                        strings(value, out)
                    }
                }
                _ => {}
            }
        }
        for entry in self.entries.values() {
            for value in entry.value.iter().chain(entry.baseline.iter().flatten()) {
                originals.push(value.to_string());
                if let Ok(value) = serde_json::from_str::<serde_json::Value>(value) {
                    strings(&value, &mut originals);
                }
            }
        }
        originals.sort_by_key(|s| std::cmp::Reverse(s.len()));
        for value in originals {
            cause = cause.replace(&value, "[redacted]");
        }
        cause = cause.chars().take(2048).collect();
        self.failures = self.failures.saturating_add(1);
        let delay = Duration::from_secs(1u64 << self.failures.saturating_sub(1).min(5));
        self.retry_at = Some(Instant::now() + delay);
        let event = PersistenceNotice {
            recovered: false,
            operation: operation.into(),
            cause,
            pending_entries: self.pending(),
        };
        if !self.volatile {
            self.enqueue(event.clone());
        }
        self.failure = Some(event);
        self.volatile = true;
    }
}
pub type PersistenceObserver = dyn Fn(PersistenceNotice) + Send + Sync;
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
    pub fn with_background_recovery(backend: Arc<dyn KeychainStorage>) -> Arc<Self> {
        let storage = Arc::new(Self::new(backend));
        let weak = Arc::downgrade(&storage);
        std::thread::spawn(move || loop {
            std::thread::sleep(Duration::from_secs(1));
            let Some(storage) = weak.upgrade() else { break };
            storage.recover(false);
        });
        storage
    }
    fn dispatch_background_events(&self) {
        let delivery = {
            let mut state = self.state.lock();
            state
                .observer
                .clone()
                .map(|observer| (observer, state.events.drain(..).collect::<Vec<_>>()))
        };
        if let Some((observer, events)) = delivery {
            for event in events {
                observer(event);
            }
        }
    }
    fn recover(&self, force: bool) {
        let snapshot = {
            let mut state = self.state.lock();
            if !state.volatile
                || state.recovering
                || (!force && state.retry_at.is_some_and(|at| Instant::now() < at))
            {
                return;
            }
            state.recovering = true;
            state.entries.clone()
        };
        let mut operation = "read";
        let result = (|| -> AppResult<()> {
            let mut plan = BTreeMap::<Key, (Entry, Secret)>::new();
            for (key, entry) in &snapshot {
                operation = "read";
                let durable = self.backend.get(&key.0, &key.1)?.map(Zeroizing::new);
                if entry.quarantined
                    && !(entry.dirty && entry.value.is_none())
                    && entry.baseline.as_ref() == Some(&durable)
                {
                    return Err(AppError::Storage("Unreadable durable record is unchanged; it was preserved for repair or explicit purge".into()));
                }
                operation = "reconcile";
                let merged = if entry.dirty {
                    crate::recovery_merge::reconcile(
                        entry
                            .baseline
                            .as_ref()
                            .map(|v| v.as_deref().map(String::as_str)),
                        durable.as_deref().map(String::as_str),
                        entry.value.as_deref().map(String::as_str),
                    )?
                    .map(Zeroizing::new)
                } else {
                    durable.clone()
                };
                {
                    let mut state = self.state.lock();
                    let current = state.entries.get_mut(key).expect("cached recovery entry");
                    let value = crate::recovery_merge::reconcile(
                        Some(entry.value.as_deref().map(String::as_str)),
                        merged.as_deref().map(String::as_str),
                        current.value.as_deref().map(String::as_str),
                    )?
                    .map(Zeroizing::new);
                    current.value = value;
                    current.baseline = Some(durable.clone());
                    current.dirty = current.value != durable;
                    current.quarantined = false;
                    current.probe = false;
                }
                plan.insert(key.clone(), (entry.clone(), merged));
            }
            // Discovery metadata precedes new records, so interrupted recovery
            // cannot leave a removed client's originals undiscoverable for cleanup.
            operation = "write owner registry";
            for (key, (before, value)) in plan.iter().filter(|(key, _)| key.1 == "owner-index-v1") {
                if !before.dirty {
                    continue;
                }
                if let Some(value) = value {
                    self.backend.store(&key.0, &key.1, value)?;
                } else {
                    self.backend.delete(&key.0, &key.1)?;
                }
                let mut state = self.state.lock();
                let current = state.entries.get_mut(key).expect("registered owners");
                current.baseline = Some(value.clone());
                current.dirty = current.value != *value;
            }
            // Persist a revocation fence before modifying records. If a retry or
            // process exits mid-purge, an old durable index cannot reveal them.
            operation = "write recovery fence";
            let fence_needed = self.state.lock().entries.values().any(|entry| entry.dirty);
            let mut fenced = std::collections::BTreeSet::new();
            for (key, (_, value)) in &plan {
                if !fence_needed || !key.1.starts_with("index-") {
                    continue;
                }
                let Some(value) = value else { continue };
                let Ok(mut index) = serde_json::from_str::<serde_json::Value>(value) else {
                    continue;
                };
                if !index["scopes"].is_object() {
                    continue;
                }
                index["revoked"] = true.into();
                let fence = Zeroizing::new(index.to_string());
                self.backend.store(&key.0, &key.1, &fence)?;
                let mut state = self.state.lock();
                let entry = state.entries.get_mut(key).expect("fenced entry");
                entry.baseline = Some(Some(fence));
                entry.dirty = true;
                fenced.insert(key.clone());
            }
            // Values and deletion tombstones precede final discovery metadata.
            let metadata = |key: &Key| key.1.starts_with("index-") || key.1 == "owner-index-v1";
            for (key, (before, value)) in plan.iter().filter(|(key, _)| !metadata(key)) {
                if !before.dirty {
                    continue;
                }
                if let Some(value) = value {
                    operation = "write";
                    self.backend.store(&key.0, &key.1, value)?;
                } else {
                    operation = "delete";
                    self.backend.delete(&key.0, &key.1)?;
                }
                let mut state = self.state.lock();
                let current = state.entries.get_mut(key).expect("recovered entry");
                current.baseline = Some(value.clone());
                current.dirty = current.value != *value;
            }
            // A concurrent purge or new mapping keeps the durable fence closed
            // until another pass has persisted the complete current snapshot.
            let state = self.state.lock();
            if state
                .entries
                .iter()
                .any(|(key, e)| !metadata(key) && e.dirty)
                || plan
                    .iter()
                    .filter(|(key, _)| metadata(key))
                    .any(|(key, (_, value))| state.entries[key].value != *value)
            {
                return Ok(());
            }
            drop(state);
            for (key, (before, value)) in plan.iter().filter(|(key, _)| metadata(key)) {
                if !before.dirty && !fenced.contains(key) {
                    continue;
                }
                if let Some(value) = value {
                    operation = "write metadata";
                    self.backend.store(&key.0, &key.1, value)?;
                } else {
                    operation = "delete metadata";
                    self.backend.delete(&key.0, &key.1)?;
                }
                let mut state = self.state.lock();
                let current = state.entries.get_mut(key).expect("recovered metadata");
                current.baseline = Some(value.clone());
                current.dirty = current.value != *value;
                // A purge arriving during the final write must keep the durable
                // index closed until the newer snapshot is persisted too.
                let refence = if current.dirty && key.1.starts_with("index-") {
                    current
                        .value
                        .as_ref()
                        .and_then(|value| serde_json::from_str::<serde_json::Value>(value).ok())
                        .filter(|value| value["scopes"].is_object())
                        .map(|mut value| {
                            value["revoked"] = true.into();
                            Zeroizing::new(value.to_string())
                        })
                } else {
                    None
                };
                drop(state);
                if let Some(fence) = refence {
                    operation = "write recovery fence";
                    self.backend.store(&key.0, &key.1, &fence)?;
                    let mut state = self.state.lock();
                    let current = state.entries.get_mut(key).expect("refenced metadata");
                    current.baseline = Some(Some(fence));
                    current.dirty = true;
                    return Ok(());
                }
            }
            Ok(())
        })();
        let mut state = self.state.lock();
        state.recovering = false;
        match result {
            Ok(()) if state.pending() == 0 => {
                state.volatile = false;
                state.failure = None;
                state.failures = 0;
                state.retry_at = None;
                state.enqueue(PersistenceNotice {
                    recovered: true,
                    operation: "recovery".into(),
                    cause: String::new(),
                    pending_entries: 0,
                });
            }
            Ok(()) => state.retry_at = Some(Instant::now()),
            Err(error) => state.failed(operation, &error),
        }
        drop(state);
        self.dispatch_background_events();
    }
}
impl KeychainStorage for ResilientSecretStorage {
    fn persistence_warning(&self) -> bool {
        self.state.lock().volatile
    }
    fn persistence_details(&self) -> Option<PersistenceNotice> {
        let state = self.state.lock();
        state.failure.clone().map(|mut notice| {
            notice.pending_entries = state.pending();
            notice
        })
    }
    fn take_persistence_events(&self) -> Vec<PersistenceNotice> {
        self.state.lock().events.drain(..).collect()
    }
    fn retry_persistence(&self) {
        self.recover(true)
    }
    fn observe_persistence(&self, observer: Arc<PersistenceObserver>) {
        self.state.lock().observer = Some(observer);
    }
    fn quarantine_unreadable(&self, service: &str, account: &str) -> bool {
        self.quarantine_unreadable_with_reason(
            service,
            account,
            "Stored credential record could not be decoded or did not match its owner",
        )
    }
    fn quarantine_unreadable_with_reason(
        &self,
        service: &str,
        account: &str,
        reason: &str,
    ) -> bool {
        let mut state = self.state.lock();
        let entry = state
            .entries
            .entry((service.into(), account.into()))
            .or_default();
        entry.value = None;
        entry.quarantined = true;
        state.failed("decode", &AppError::Storage(reason.into()));
        true
    }
    fn get(&self, service: &str, account: &str) -> AppResult<Option<String>> {
        let mut state = self.state.lock();
        let key = (service.into(), account.into());
        if state.volatile {
            return Ok(state
                .entries
                .entry(key)
                .or_default()
                .value
                .as_ref()
                .map(|v| v.to_string()));
        }
        match self.backend.get(service, account) {
            Ok(value) => {
                state.entries.insert(
                    key,
                    Entry {
                        value: value.clone().map(Zeroizing::new),
                        baseline: Some(value.clone().map(Zeroizing::new)),
                        ..Default::default()
                    },
                );
                Ok(value)
            }
            Err(error) => {
                let entry = state.entries.entry(key).or_default();
                entry.probe = true;
                let value = entry.value.as_ref().map(|v| v.to_string());
                state.failed("read", &error);
                Ok(value)
            }
        }
    }
    fn store(&self, service: &str, account: &str, value: &str) -> AppResult<()> {
        let mut state = self.state.lock();
        let key = (service.into(), account.into());
        let entry = state.entries.entry(key.clone()).or_default();
        entry.value = Some(Zeroizing::new(value.into()));
        entry.dirty = true;
        if !state.volatile {
            match self.backend.store(service, account, value) {
                Ok(()) => {
                    let entry = state.entries.get_mut(&key).expect("written entry");
                    entry.baseline = Some(entry.value.clone());
                    entry.dirty = false;
                }
                Err(error) => state.failed("write", &error),
            }
        }
        Ok(())
    }
    fn delete(&self, service: &str, account: &str) -> AppResult<()> {
        let mut state = self.state.lock();
        let key = (service.into(), account.into());
        let entry = state.entries.entry(key.clone()).or_default();
        entry.value = None;
        entry.dirty = true;
        if !state.volatile {
            match self.backend.delete(service, account) {
                Ok(()) => {
                    let entry = state.entries.get_mut(&key).expect("deleted entry");
                    entry.baseline = Some(None);
                    entry.dirty = false;
                }
                Err(error) => state.failed("delete", &error),
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::MockKeychain;
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
    struct Backend {
        data: MockKeychain,
        fail: AtomicBool,
        calls: AtomicUsize,
        fail_at: AtomicUsize,
    }
    impl Backend {
        fn new() -> Arc<Self> {
            Arc::new(Self {
                data: MockKeychain::new(),
                fail: AtomicBool::new(false),
                calls: AtomicUsize::new(0),
                fail_at: AtomicUsize::new(usize::MAX),
            })
        }
        fn check(&self) -> AppResult<()> {
            let call = self.calls.fetch_add(1, Ordering::SeqCst) + 1;
            if self.fail.load(Ordering::SeqCst) || call == self.fail_at.load(Ordering::SeqCst) {
                Err(AppError::Storage(
                    "No space left on device (os error 28)".into(),
                ))
            } else {
                Ok(())
            }
        }
    }
    impl KeychainStorage for Backend {
        fn get(&self, s: &str, a: &str) -> AppResult<Option<String>> {
            self.check()?;
            self.data.get(s, a)
        }
        fn store(&self, s: &str, a: &str, v: &str) -> AppResult<()> {
            self.check()?;
            self.data.store(s, a, v)
        }
        fn delete(&self, s: &str, a: &str) -> AppResult<()> {
            self.check()?;
            self.data.delete(s, a)
        }
    }
    fn json(text: &str) -> serde_json::Value {
        serde_json::from_str(text).unwrap()
    }
    #[test]
    fn disk_full_keeps_values_and_reports_the_cause_once_then_recovers() {
        let backend = Backend::new();
        backend.data.store("s", "a", "durable-original").unwrap();
        let storage = ResilientSecretStorage::new(backend.clone());
        assert_eq!(
            storage.get("s", "a").unwrap().as_deref(),
            Some("durable-original")
        );
        backend.fail.store(true, Ordering::SeqCst);
        storage.store("s", "a", "new-original").unwrap();
        for _ in 0..20 {
            storage.store("s", "a", "new-original").unwrap();
            assert_eq!(
                storage.get("s", "a").unwrap().as_deref(),
                Some("new-original")
            );
        }
        let events = storage.take_persistence_events();
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].operation, "write");
        assert!(events[0].cause.contains("os error 28"));
        assert!(storage.take_persistence_events().is_empty());
        assert_eq!(
            backend.data.get("s", "a").unwrap().as_deref(),
            Some("durable-original")
        );
        backend.fail.store(false, Ordering::SeqCst);
        storage.retry_persistence();
        assert!(!storage.persistence_warning());
        assert_eq!(
            backend.data.get("s", "a").unwrap().as_deref(),
            Some("new-original")
        );
        let events = storage.take_persistence_events();
        assert_eq!(events.len(), 1);
        assert!(events[0].recovered);
        storage.retry_persistence();
        assert!(storage.take_persistence_events().is_empty());
    }
    #[test]
    fn unread_client_maps_and_owner_registry_are_merged_on_recovery() {
        let backend = Backend::new();
        backend
            .data
            .store(
                "s",
                "a",
                r#"{"owner":"client","entries":{"old":"old-secret"}}"#,
            )
            .unwrap();
        backend
            .data
            .store("s", "owner-index-v1", r#"["old-client"]"#)
            .unwrap();
        backend.fail.store(true, Ordering::SeqCst);
        let storage = ResilientSecretStorage::new(backend.clone());
        assert_eq!(storage.get("s", "a").unwrap(), None);
        storage
            .store(
                "s",
                "a",
                r#"{"owner":"client","entries":{"new":"new-secret"}}"#,
            )
            .unwrap();
        storage
            .store("s", "owner-index-v1", r#"["new-client"]"#)
            .unwrap();
        backend.fail.store(false, Ordering::SeqCst);
        storage.retry_persistence();
        assert!(!storage.persistence_warning());
        assert_eq!(
            json(&backend.data.get("s", "a").unwrap().unwrap())["entries"],
            serde_json::json!({"old":"old-secret","new":"new-secret"})
        );
        assert_eq!(
            json(&backend.data.get("s", "owner-index-v1").unwrap().unwrap()),
            serde_json::json!(["new-client", "old-client"])
        );
    }
    #[test]
    fn failed_reads_retain_cached_originals_and_deletions_recover() {
        let backend = Backend::new();
        backend.data.store("s", "a", "secret").unwrap();
        let storage = ResilientSecretStorage::new(backend.clone());
        storage.get("s", "a").unwrap();
        backend.fail.store(true, Ordering::SeqCst);
        assert_eq!(storage.get("s", "a").unwrap().as_deref(), Some("secret"));
        storage.delete("s", "a").unwrap();
        assert_eq!(storage.get("s", "a").unwrap(), None);
        backend.fail.store(false, Ordering::SeqCst);
        storage.retry_persistence();
        assert_eq!(backend.data.get("s", "a").unwrap(), None);
        assert!(!storage.persistence_warning());
        backend.fail.store(true, Ordering::SeqCst);
        storage.delete("s", "b").unwrap();
        assert_eq!(storage.take_persistence_events().len(), 3);
        assert_eq!(storage.persistence_details().unwrap().operation, "delete");
    }
    #[test]
    fn partial_recovery_leaves_a_revocation_fence_until_all_values_are_durable() {
        let backend = Backend::new();
        let index = r#"{"scopes":{"record":1},"revoked":false}"#;
        backend.data.store("s", "index-client", index).unwrap();
        backend.data.store("s", "record", "old-secret").unwrap();
        let storage = ResilientSecretStorage::new(backend.clone());
        storage.get("s", "index-client").unwrap();
        storage.get("s", "record").unwrap();
        backend.fail.store(true, Ordering::SeqCst);
        storage
            .store("s", "index-client", r#"{"scopes":{},"revoked":false}"#)
            .unwrap();
        storage.delete("s", "record").unwrap();
        backend.fail.store(false, Ordering::SeqCst);
        // Two reads, then the fence write, then the deletion fails.
        backend
            .fail_at
            .store(backend.calls.load(Ordering::SeqCst) + 4, Ordering::SeqCst);
        storage.retry_persistence();
        assert!(storage.persistence_warning());
        assert_eq!(
            json(&backend.data.get("s", "index-client").unwrap().unwrap())["revoked"],
            true
        );
        assert_eq!(storage.get("s", "record").unwrap(), None);
        storage.retry_persistence();
        assert!(!storage.persistence_warning());
        assert_eq!(backend.data.get("s", "record").unwrap(), None);
        assert_eq!(
            json(&backend.data.get("s", "index-client").unwrap().unwrap())["revoked"],
            false
        );
    }
    #[test]
    fn unread_opaque_conflict_and_corrupt_record_are_not_overwritten() {
        let backend = Backend::new();
        backend.data.store("s", "a", "old opaque").unwrap();
        backend.fail.store(true, Ordering::SeqCst);
        let storage = ResilientSecretStorage::new(backend.clone());
        storage.get("s", "a").unwrap();
        storage.store("s", "a", "new opaque").unwrap();
        backend.fail.store(false, Ordering::SeqCst);
        storage.retry_persistence();
        assert!(storage.persistence_warning());
        assert_eq!(
            backend.data.get("s", "a").unwrap().as_deref(),
            Some("old opaque")
        );
        backend.data.delete("s", "a").unwrap();
        storage.retry_persistence();
        assert!(!storage.persistence_warning());
        assert_eq!(
            backend.data.get("s", "a").unwrap().as_deref(),
            Some("new opaque")
        );
        backend.data.store("s", "bad", "{broken").unwrap();
        storage.get("s", "bad").unwrap();
        storage.quarantine_unreadable_with_reason(
            "s",
            "bad",
            "Invalid JSON in stored secret record",
        );
        storage
            .store("s", "bad", r#"{"owner":"client","entries":{}}"#)
            .unwrap();
        storage.retry_persistence();
        assert!(storage.persistence_warning());
        assert_eq!(
            backend.data.get("s", "bad").unwrap().as_deref(),
            Some("{broken")
        );
        backend.data.delete("s", "bad").unwrap();
        storage.retry_persistence();
        assert!(!storage.persistence_warning());
    }
    #[test]
    fn retry_backoff_is_bounded_and_new_requests_do_not_retry_the_backend() {
        let backend = Backend::new();
        backend.fail.store(true, Ordering::SeqCst);
        let storage = ResilientSecretStorage::new(backend.clone());
        storage.get("s", "a").unwrap();
        let calls = backend.calls.load(Ordering::SeqCst);
        for _ in 0..100 {
            storage.get("s", "a").unwrap();
            storage.recover(false);
        }
        assert_eq!(backend.calls.load(Ordering::SeqCst), calls);
        for _ in 0..12 {
            storage.retry_persistence();
        }
        let state = storage.state.lock();
        assert!(
            state
                .retry_at
                .unwrap()
                .saturating_duration_since(Instant::now())
                <= Duration::from_secs(32)
        );
        assert_eq!(state.events.len(), 1);
    }
    #[test]
    fn cached_read_failures_require_a_successful_probe_and_not_a_write() {
        struct ReadOnly {
            data: crate::MockKeychain,
            fail: AtomicBool,
            writes: AtomicUsize,
        }
        impl KeychainStorage for ReadOnly {
            fn get(&self, s: &str, a: &str) -> AppResult<Option<String>> {
                if self.fail.load(Ordering::SeqCst) {
                    Err(AppError::Storage("read unavailable".into()))
                } else {
                    self.data.get(s, a)
                }
            }
            fn store(&self, _: &str, _: &str, _: &str) -> AppResult<()> {
                self.writes.fetch_add(1, Ordering::SeqCst);
                Err(AppError::Storage("read-only backend".into()))
            }
            fn delete(&self, _: &str, _: &str) -> AppResult<()> {
                panic!("read recovery must not delete")
            }
        }
        let backend = Arc::new(ReadOnly {
            data: crate::MockKeychain::new(),
            fail: AtomicBool::new(false),
            writes: AtomicUsize::new(0),
        });
        backend.data.store("s", "a", "cached original").unwrap();
        let storage = ResilientSecretStorage::new(backend.clone());
        storage.get("s", "a").unwrap();
        backend.fail.store(true, Ordering::SeqCst);
        assert_eq!(
            storage.get("s", "a").unwrap().as_deref(),
            Some("cached original")
        );
        storage.retry_persistence();
        assert!(storage.persistence_warning());
        backend.fail.store(false, Ordering::SeqCst);
        storage.retry_persistence();
        assert!(!storage.persistence_warning());
        assert_eq!(backend.writes.load(Ordering::SeqCst), 0);
    }
    #[test]
    fn background_worker_recovers_without_waiting_for_another_request() {
        let backend = Backend::new();
        backend.fail.store(true, Ordering::SeqCst);
        let storage = ResilientSecretStorage::with_background_recovery(backend.clone());
        let (sent, received) = std::sync::mpsc::channel();
        storage.observe_persistence(Arc::new(move |notice| {
            sent.send(notice.recovered).unwrap();
        }));
        storage.store("s", "a", "original").unwrap();
        backend.fail.store(false, Ordering::SeqCst);
        while !received.recv_timeout(Duration::from_secs(5)).unwrap() {}
        assert!(!storage.persistence_warning());
        assert_eq!(
            backend.data.get("s", "a").unwrap().as_deref(),
            Some("original")
        );
    }
    #[test]
    fn failed_file_write_recovers_and_survives_a_fresh_store_instance() {
        let directory = tempfile::tempdir().unwrap();
        let parent = directory.path().join("store");
        std::fs::create_dir(&parent).unwrap();
        let file = parent.join("secrets.json");
        let backend = Arc::new(crate::keychain_trait::FileKeychain::new(file.clone()).unwrap());
        let storage = ResilientSecretStorage::new(backend);
        storage.store("s", "a", "old").unwrap();
        let saved = directory.path().join("saved");
        std::fs::rename(&parent, &saved).unwrap();
        std::fs::write(&parent, "blocks directory").unwrap();
        storage.store("s", "a", "new").unwrap();
        assert!(storage.persistence_warning());
        assert!(storage
            .persistence_details()
            .unwrap()
            .cause
            .contains("directory"));
        std::fs::remove_file(&parent).unwrap();
        std::fs::rename(&saved, &parent).unwrap();
        storage.retry_persistence();
        assert!(!storage.persistence_warning());
        let fresh = crate::keychain_trait::FileKeychain::new(file).unwrap();
        assert_eq!(fresh.get("s", "a").unwrap().as_deref(), Some("new"));
    }
    #[test]
    fn concurrent_purge_does_not_wait_for_recovery_io_or_reopen_old_records() {
        use std::sync::mpsc;
        for target in ["record", "index-client"] {
            struct Blocking {
                inner: Arc<Backend>,
                block: AtomicBool,
                target: &'static str,
                entered: mpsc::Sender<()>,
                resume: Mutex<mpsc::Receiver<()>>,
            }
            impl KeychainStorage for Blocking {
                fn get(&self, s: &str, a: &str) -> AppResult<Option<String>> {
                    self.inner.get(s, a)
                }
                fn store(&self, s: &str, a: &str, v: &str) -> AppResult<()> {
                    if a == self.target
                        && (a != "index-client" || json(v)["revoked"] == false)
                        && !self.inner.fail.load(Ordering::SeqCst)
                        && self.block.swap(false, Ordering::SeqCst)
                    {
                        self.entered.send(()).unwrap();
                        self.resume
                            .lock()
                            .recv_timeout(Duration::from_secs(5))
                            .unwrap();
                    }
                    self.inner.store(s, a, v)
                }
                fn delete(&self, s: &str, a: &str) -> AppResult<()> {
                    self.inner.delete(s, a)
                }
            }
            let backend = Backend::new();
            backend
                .data
                .store(
                    "s",
                    "index-client",
                    r#"{"scopes":{"record":1},"revoked":false}"#,
                )
                .unwrap();
            backend.data.store("s", "record", "old").unwrap();
            let (entered_tx, entered_rx) = mpsc::channel();
            let (resume_tx, resume_rx) = mpsc::channel();
            let blocking = Arc::new(Blocking {
                inner: backend.clone(),
                block: AtomicBool::new(true),
                target,
                entered: entered_tx,
                resume: Mutex::new(resume_rx),
            });
            let storage = Arc::new(ResilientSecretStorage::new(blocking));
            storage.get("s", "index-client").unwrap();
            storage.get("s", "record").unwrap();
            backend.fail.store(true, Ordering::SeqCst);
            storage.store("s", "record", "intermediate").unwrap();
            storage
                .store(
                    "s",
                    "index-client",
                    r#"{"scopes":{"record":2},"revoked":false}"#,
                )
                .unwrap();
            backend.fail.store(false, Ordering::SeqCst);
            let retry = storage.clone();
            let worker = std::thread::spawn(move || retry.retry_persistence());
            entered_rx.recv_timeout(Duration::from_secs(5)).unwrap();
            let purge = storage.clone();
            let (done_tx, done_rx) = mpsc::channel();
            let mutation = std::thread::spawn(move || {
                purge.delete("s", "record").unwrap();
                purge
                    .store("s", "index-client", r#"{"scopes":{},"revoked":false}"#)
                    .unwrap();
                done_tx.send(()).unwrap();
            });
            let completed = done_rx.recv_timeout(Duration::from_secs(1));
            resume_tx.send(()).unwrap();
            worker.join().unwrap();
            mutation.join().unwrap();
            assert!(
                completed.is_ok(),
                "requests waited for blocked recovery I/O"
            );
            assert!(storage.persistence_warning());
            assert_eq!(storage.get("s", "record").unwrap(), None);
            assert_eq!(
                json(&backend.data.get("s", "index-client").unwrap().unwrap())["revoked"],
                true
            );
            storage.retry_persistence();
            assert!(!storage.persistence_warning());
            assert_eq!(backend.data.get("s", "record").unwrap(), None);
        }
    }
    #[test]
    fn backend_error_does_not_expose_credential_values() {
        struct Echo;
        impl KeychainStorage for Echo {
            fn store(&self, _: &str, _: &str, v: &str) -> AppResult<()> {
                Err(AppError::Storage(format!("disk full while writing {v}")))
            }
            fn get(&self, _: &str, _: &str) -> AppResult<Option<String>> {
                Ok(None)
            }
            fn delete(&self, _: &str, _: &str) -> AppResult<()> {
                Ok(())
            }
        }
        let storage = ResilientSecretStorage::new(Arc::new(Echo));
        storage
            .store(
                "s",
                "a",
                r#"{"entries":{"id":"synthetic-private-original"}}"#,
            )
            .unwrap();
        let event = storage.take_persistence_events().pop().unwrap();
        assert!(!event.cause.contains("synthetic-private-original"));
        assert!(event.cause.contains("disk full"));
    }
}
