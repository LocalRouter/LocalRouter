//! Keychain trait abstraction for testability
//!
//! Provides a trait-based interface for keychain operations,
//! allowing for real (system keyring) and mock (in-memory) implementations.
//!
//! The CachedKeychain wrapper provides in-memory caching to prevent
//! repeated password prompts for the same service:account combination.

use crate::chunked::{self, RawEntryStore};
pub use crate::resilient::{PersistenceNotice, PersistenceObserver, ResilientSecretStorage};
use lr_types::errors::AppResult;
use parking_lot::RwLock;
use std::collections::HashMap;
use std::fs;
use std::sync::{Arc, Mutex};
use tracing::{debug, trace, warn};
use zeroize::Zeroizing;

/// Trait for keychain operations
pub trait KeychainStorage: Send + Sync {
    /// True when this process can only retain mappings in memory.
    fn persistence_warning(&self) -> bool {
        false
    }

    /// Metadata-only outage state and transitions. Defaults preserve ordinary stores.
    fn persistence_details(&self) -> Option<PersistenceNotice> {
        None
    }
    fn take_persistence_events(&self) -> Vec<PersistenceNotice> {
        Vec::new()
    }
    fn retry_persistence(&self) {}
    fn observe_persistence(&self, _observer: Arc<PersistenceObserver>) {}
    fn quarantine_unreadable_with_reason(
        &self,
        service: &str,
        account: &str,
        _reason: &str,
    ) -> bool {
        self.quarantine_unreadable(service, account)
    }

    /// Isolate unreadable redaction data without modifying durable records.
    fn quarantine_unreadable(&self, _service: &str, _account: &str) -> bool {
        false
    }

    /// Store a key-value pair
    fn store(&self, service: &str, account: &str, secret: &str) -> AppResult<()>;

    /// Retrieve a value by service and account
    fn get(&self, service: &str, account: &str) -> AppResult<Option<String>>;

    /// Delete a key-value pair
    fn delete(&self, service: &str, account: &str) -> AppResult<()>;
}

/// Real keychain implementation using system keyring
///
/// Secrets longer than the platform's per-entry limit (Windows Credential
/// Manager: 2560 bytes of UTF-16) are split across several entries; see
/// [`crate::chunked`].
pub struct SystemKeychain;

/// Reversible originals use native credentials in release builds. Development
/// honors the same file/system choice as other debug credentials, using a
/// separate file so independently cached client credentials cannot overwrite it.
pub fn redacted_secret_storage() -> Arc<dyn KeychainStorage> {
    type Stores = parking_lot::Mutex<HashMap<String, Arc<dyn KeychainStorage>>>;
    static STORES: std::sync::OnceLock<Stores> = std::sync::OnceLock::new();
    let path = lr_utils::paths::config_dir();
    let namespace = path
        .as_ref()
        .ok()
        .and_then(|p| fs::read_to_string(p.join("redacted-secrets.namespace")).ok());
    let key = format!(
        "{:?}|{:?}|{:?}",
        path,
        std::env::var("LOCALROUTER_KEYCHAIN"),
        namespace
    );
    STORES
        .get_or_init(Stores::default)
        .lock()
        .entry(key)
        .or_insert_with(|| {
            ResilientSecretStorage::with_background_recovery(Arc::new(LazySecretStorage {
                backend: parking_lot::Mutex::new(None),
                factory: Box::new(durable_redacted_secret_storage),
            }))
        })
        .clone()
}

/// Initialization errors are retried rather than cached as a permanently unavailable backend.
struct LazySecretStorage {
    backend: parking_lot::Mutex<Option<Arc<dyn KeychainStorage>>>,
    factory: Box<dyn Fn() -> AppResult<Arc<dyn KeychainStorage>> + Send + Sync>,
}
impl LazySecretStorage {
    fn backend(&self) -> AppResult<Arc<dyn KeychainStorage>> {
        let mut backend = self.backend.lock();
        if let Some(backend) = backend.as_ref() {
            return Ok(backend.clone());
        }
        let initialized = (self.factory)()?;
        *backend = Some(initialized.clone());
        Ok(initialized)
    }
}
impl KeychainStorage for LazySecretStorage {
    fn store(&self, s: &str, a: &str, v: &str) -> AppResult<()> {
        self.backend()?.store(s, a, v)
    }
    fn get(&self, s: &str, a: &str) -> AppResult<Option<String>> {
        self.backend()?.get(s, a)
    }
    fn delete(&self, s: &str, a: &str) -> AppResult<()> {
        self.backend()?.delete(s, a)
    }
}
fn durable_redacted_secret_storage() -> AppResult<Arc<dyn KeychainStorage>> {
    #[cfg(debug_assertions)]
    if std::env::var("LOCALROUTER_KEYCHAIN").as_deref() != Ok("system") {
        type DebugStores = Mutex<HashMap<std::path::PathBuf, Arc<dyn KeychainStorage>>>;
        static STORES: std::sync::OnceLock<DebugStores> = std::sync::OnceLock::new();
        let path = lr_utils::paths::config_dir()?.join("redacted-secrets.debug.json");
        let mut stores = STORES
            .get_or_init(Mutex::default)
            .lock()
            .expect("debug credential stores");
        if let Some(storage) = stores.get(&path) {
            return Ok(storage.clone());
        }
        let storage: Arc<dyn KeychainStorage> = Arc::new(FileKeychain::new(path.clone())?);
        stores.insert(path, storage.clone());
        return Ok(storage);
    }
    let reset_namespace = lr_utils::paths::config_dir()
        .map_err(|error| error.to_string())
        .and_then(|path| read_redaction_namespace(&path.join("redacted-secrets.namespace")))
        .map_err(lr_types::AppError::Config)?;
    #[cfg(debug_assertions)]
    let namespace = Some(format!(
        "debug-{}{}",
        std::env::var("LOCALROUTER_ENV").unwrap_or_else(|_| "dev".into()),
        reset_namespace
            .map(|value| format!("-{value}"))
            .unwrap_or_default(),
    ));
    #[cfg(not(debug_assertions))]
    let namespace = reset_namespace;
    Ok(match namespace {
        Some(namespace) => Arc::new(NamespacedSecretStorage {
            backend: Arc::new(NonInteractiveSystemKeychain),
            namespace,
        }),
        None => Arc::new(NonInteractiveSystemKeychain),
    })
}

/// A local reset selects a fresh native service without accessing stale entries
/// whose ACL may require an interactive password. Absence preserves existing maps.
fn read_redaction_namespace(path: &std::path::Path) -> Result<Option<String>, String> {
    let raw = match fs::read_to_string(path) {
        Ok(raw) => raw,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(format!("Cannot read redaction store namespace: {error}")),
    };
    let namespace = raw.trim();
    if namespace.len() != 32 || !namespace.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err("Invalid redaction store namespace".into());
    }
    Ok(Some(format!("reset-{}", namespace.to_ascii_lowercase())))
}

struct NamespacedSecretStorage {
    backend: Arc<dyn KeychainStorage>,
    namespace: String,
}
impl NamespacedSecretStorage {
    fn service(&self, service: &str) -> String {
        format!("{service}-{}", self.namespace)
    }
}
impl KeychainStorage for NamespacedSecretStorage {
    fn store(&self, service: &str, account: &str, secret: &str) -> AppResult<()> {
        self.backend.store(&self.service(service), account, secret)
    }
    fn get(&self, service: &str, account: &str) -> AppResult<Option<String>> {
        self.backend.get(&self.service(service), account)
    }
    fn delete(&self, service: &str, account: &str) -> AppResult<()> {
        self.backend.delete(&self.service(service), account)
    }
}

/// Serializes system keyring access across every `SystemKeychain` user in the
/// process: several `CachedKeychain` instances share the store, and a secret
/// written in parts must not interleave with another write of the same secret.
static SYSTEM_KEYRING_LOCK: RwLock<()> = RwLock::new(());

/// Single `keyring::Entry` operations underneath the chunking layer.
struct KeyringEntries;

impl KeyringEntries {
    fn entry(service: &str, account: &str) -> AppResult<keyring::Entry> {
        keyring::Entry::new(service, account)
            .map_err(|e| lr_types::AppError::Internal(format!("Failed to access keyring: {}", e)))
    }
}

impl RawEntryStore for KeyringEntries {
    fn set(&self, service: &str, account: &str, value: &str) -> AppResult<()> {
        Self::entry(service, account)?
            .set_password(value)
            .map_err(|e| lr_types::AppError::Internal(format!("Failed to store key: {}", e)))
    }

    fn get(&self, service: &str, account: &str) -> AppResult<Option<String>> {
        match Self::entry(service, account)?.get_password() {
            Ok(secret) => Ok(Some(secret)),
            Err(keyring::Error::NoEntry) => Ok(None),
            Err(e) => Err(lr_types::AppError::Internal(format!(
                "Failed to retrieve key: {}",
                e
            ))),
        }
    }

    fn remove(&self, service: &str, account: &str) -> AppResult<bool> {
        match Self::entry(service, account)?.delete_credential() {
            Ok(()) => Ok(true),
            Err(keyring::Error::NoEntry) => Ok(false),
            Err(e) => Err(lr_types::AppError::Internal(format!(
                "Failed to delete key: {}",
                e
            ))),
        }
    }
}

impl KeychainStorage for SystemKeychain {
    fn store(&self, service: &str, account: &str, secret: &str) -> AppResult<()> {
        trace!("SystemKeychain: storing {}:{}", service, account);
        let _guard = SYSTEM_KEYRING_LOCK.write();
        chunked::store(
            &KeyringEntries,
            chunked::PLATFORM_LIMIT,
            service,
            account,
            secret,
        )?;
        debug!("SystemKeychain: stored {}:{}", service, account);
        Ok(())
    }

    fn get(&self, service: &str, account: &str) -> AppResult<Option<String>> {
        trace!(
            "SystemKeychain: retrieving {}:{} from system keyring",
            service,
            account
        );
        let _guard = SYSTEM_KEYRING_LOCK.read();
        let secret = chunked::load(&KeyringEntries, service, account)?;
        if secret.is_some() {
            debug!(
                "SystemKeychain: retrieved {}:{} from system keyring",
                service, account
            );
        } else {
            trace!("SystemKeychain: no entry found for {}:{}", service, account);
        }
        Ok(secret)
    }

    fn delete(&self, service: &str, account: &str) -> AppResult<()> {
        trace!("SystemKeychain: deleting {}:{}", service, account);
        let _guard = SYSTEM_KEYRING_LOCK.write();
        chunked::delete(&KeyringEntries, service, account)?;
        debug!("SystemKeychain: deleted {}:{}", service, account);
        Ok(())
    }
}

/// Secret scanning and automatic recovery must never request interactive access.
/// The shared native lock covers the temporary process-wide interaction setting.
struct NonInteractiveSystemKeychain;
#[cfg(target_os = "macos")]
mod keychain_interaction {
    #[link(name = "Security", kind = "framework")]
    unsafe extern "C" {
        fn SecKeychainGetUserInteractionAllowed(state: *mut u8) -> i32;
        fn SecKeychainSetUserInteractionAllowed(state: u8) -> i32;
    }
    pub(super) struct Guard(u8);
    impl Guard {
        pub(super) fn new() -> lr_types::AppResult<Self> {
            let mut previous = 0;
            // SAFETY: the SDK's Boolean is an unsigned byte. The caller owns the
            // process-wide native credential lock until this guard is dropped.
            let status = unsafe { SecKeychainGetUserInteractionAllowed(&mut previous) };
            if status != 0 {
                return Err(lr_types::AppError::Storage(format!(
                    "Cannot inspect noninteractive Keychain access: OSStatus {status}"
                )));
            }
            let status = unsafe { SecKeychainSetUserInteractionAllowed(0) };
            if status != 0 {
                return Err(lr_types::AppError::Storage(format!(
                    "Cannot disable Keychain dialogs: OSStatus {status}"
                )));
            }
            Ok(Self(previous))
        }
    }
    impl Drop for Guard {
        fn drop(&mut self) {
            // SAFETY: same shared native lock remains held during restoration.
            let status = unsafe { SecKeychainSetUserInteractionAllowed(self.0) };
            if status != 0 {
                tracing::error!(status, "Could not restore Keychain interaction setting");
            }
        }
    }
}
impl KeychainStorage for NonInteractiveSystemKeychain {
    fn store(&self, service: &str, account: &str, value: &str) -> AppResult<()> {
        let _lock = SYSTEM_KEYRING_LOCK.write();
        #[cfg(target_os = "macos")]
        let _interaction = keychain_interaction::Guard::new()?;
        chunked::store(
            &KeyringEntries,
            chunked::PLATFORM_LIMIT,
            service,
            account,
            value,
        )
    }
    fn get(&self, service: &str, account: &str) -> AppResult<Option<String>> {
        let _lock = SYSTEM_KEYRING_LOCK.write();
        #[cfg(target_os = "macos")]
        let _interaction = keychain_interaction::Guard::new()?;
        chunked::load(&KeyringEntries, service, account)
    }
    fn delete(&self, service: &str, account: &str) -> AppResult<()> {
        let _lock = SYSTEM_KEYRING_LOCK.write();
        #[cfg(target_os = "macos")]
        let _interaction = keychain_interaction::Guard::new()?;
        chunked::delete(&KeyringEntries, service, account)
    }
}

/// File-based keychain implementation for development
///
/// Stores secrets in a JSON file in the config directory.
/// WARNING: This is NOT secure and should ONLY be used for development to avoid
/// constant keychain permission prompts. Do NOT use in production.
///
/// Key format: "service:account"
#[derive(Clone)]
pub struct FileKeychain {
    file_path: Arc<std::path::PathBuf>,
    storage: Arc<Mutex<HashMap<String, String>>>,
}

impl FileKeychain {
    /// Create a new file-based keychain
    ///
    /// # Arguments
    /// * `file_path` - Path to the JSON file for storing secrets
    pub fn new(file_path: std::path::PathBuf) -> AppResult<Self> {
        let keychain = Self {
            file_path: Arc::new(file_path.clone()),
            storage: Arc::new(Mutex::new(HashMap::new())),
        };

        // Load existing secrets from file if it exists
        if file_path.exists() {
            keychain.load_from_file()?;
        } else {
            warn!(
                "FileKeychain: secrets file does not exist, will create on first write: {}",
                file_path.display()
            );
        }

        Ok(keychain)
    }

    /// Create key for storage lookup
    fn make_key(service: &str, account: &str) -> String {
        format!("{}:{}", service, account)
    }

    /// Load secrets from file
    fn load_from_file(&self) -> AppResult<()> {
        let contents = fs::read_to_string(self.file_path.as_ref()).map_err(|e| {
            lr_types::AppError::Internal(format!("Failed to read secrets file: {}", e))
        })?;

        // Handle empty file (treat as empty HashMap)
        let data: HashMap<String, String> = if contents.trim().is_empty() {
            HashMap::new()
        } else {
            serde_json::from_str(&contents).map_err(|error| {
                // Data errors can quote an entire credential as the unexpected
                // JSON value. Retain the shape/position without echoing that value.
                let detail = if error.classify() == serde_json::error::Category::Data {
                    format!(
                        "expected a JSON object containing string values at line {} column {}",
                        error.line(),
                        error.column()
                    )
                } else {
                    error.to_string()
                };
                lr_types::AppError::Internal(format!("Failed to parse secrets file: {detail}"))
            })?
        };

        let mut storage = self.storage.lock().unwrap();
        *storage = data;
        debug!(
            "FileKeychain: loaded {} secrets from {}",
            storage.len(),
            self.file_path.display()
        );

        Ok(())
    }

    /// Persist a complete snapshot before publishing it to readers.
    fn save_to_file(&self, storage: &HashMap<String, String>) -> AppResult<()> {
        use std::io::Write;

        let parent = self
            .file_path
            .parent()
            .filter(|path| !path.as_os_str().is_empty())
            .unwrap_or_else(|| std::path::Path::new("."));
        fs::create_dir_all(parent).map_err(|e| {
            lr_types::AppError::Internal(format!("Failed to create secrets directory: {}", e))
        })?;

        let contents = serde_json::to_string_pretty(storage).map_err(|e| {
            lr_types::AppError::Internal(format!("Failed to serialize secrets: {}", e))
        })?;

        // NamedTempFile creates the file exclusively with owner-only permissions
        // on Unix. Persist replaces the destination atomically, including on
        // Windows, without exposing a partially written or world-readable file.
        let mut temporary = tempfile::NamedTempFile::new_in(parent).map_err(|e| {
            lr_types::AppError::Internal(format!("Failed to create secrets file: {}", e))
        })?;
        temporary.write_all(contents.as_bytes()).map_err(|e| {
            lr_types::AppError::Internal(format!("Failed to write secrets file: {}", e))
        })?;
        temporary.as_file().sync_all().map_err(|e| {
            lr_types::AppError::Internal(format!("Failed to sync secrets file: {}", e))
        })?;
        temporary.persist(self.file_path.as_ref()).map_err(|e| {
            lr_types::AppError::Internal(format!("Failed to replace secrets file: {}", e))
        })?;

        debug!(
            "FileKeychain: saved {} secrets to {}",
            storage.len(),
            self.file_path.display()
        );
        Ok(())
    }
}

impl KeychainStorage for FileKeychain {
    fn store(&self, service: &str, account: &str, secret: &str) -> AppResult<()> {
        let key = Self::make_key(service, account);
        let mut storage = self.storage.lock().unwrap();
        let mut updated = storage.clone();
        updated.insert(key, secret.to_string());
        self.save_to_file(&updated)?;
        *storage = updated;
        trace!("FileKeychain: stored {}:{}", service, account);
        Ok(())
    }

    fn get(&self, service: &str, account: &str) -> AppResult<Option<String>> {
        let key = Self::make_key(service, account);
        let storage = self.storage.lock().unwrap();
        Ok(storage.get(&key).cloned())
    }

    fn delete(&self, service: &str, account: &str) -> AppResult<()> {
        let key = Self::make_key(service, account);
        let mut storage = self.storage.lock().unwrap();
        let mut updated = storage.clone();
        updated.remove(&key);
        self.save_to_file(&updated)?;
        *storage = updated;
        trace!("FileKeychain: deleted {}:{}", service, account);
        Ok(())
    }
}

/// Cached keychain wrapper that adds in-memory caching to any KeychainStorage implementation
///
/// This wrapper sits right on top of the keyring library calls and caches all retrieved values
/// in memory to prevent repeated password prompts. The cache is maintained for the lifetime
/// of the application process.
///
/// Key format for cache: "service:account"
#[derive(Clone)]
pub struct CachedKeychain {
    /// The underlying keychain implementation
    inner: Arc<dyn KeychainStorage>,
    /// Serialize storage operations and cache publication. Otherwise a slow
    /// cache miss can repopulate a deleted or replaced secret.
    operation_lock: Arc<parking_lot::Mutex<()>>,
    /// In-memory cache of retrieved values
    /// Key: "service:account", Value: secret (zeroized on drop)
    cache: Arc<RwLock<HashMap<String, Zeroizing<String>>>>,
}

impl CachedKeychain {
    /// Create a new cached keychain wrapping the given implementation
    pub fn new(inner: Arc<dyn KeychainStorage>) -> Self {
        Self {
            inner,
            operation_lock: Arc::new(parking_lot::Mutex::new(())),
            cache: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    /// Create a new cached keychain wrapping the system keychain
    pub fn system() -> Self {
        Self::new(Arc::new(SystemKeychain))
    }

    /// Create a new cached keychain wrapping the file-based keychain
    ///
    /// # Arguments
    /// * `file_path` - Path to the secrets file
    pub fn file(file_path: std::path::PathBuf) -> AppResult<Self> {
        let file_keychain = FileKeychain::new(file_path)?;
        Ok(Self::new(Arc::new(file_keychain)))
    }

    /// Create the appropriate keychain based on build type and environment configuration
    ///
    /// Automatically detects development vs production builds:
    /// - Debug builds (cargo tauri dev) -> File-based storage in ~/.localrouter-dev/secrets.json
    /// - Release builds -> System keyring (Keychain, Credential Manager, Secret Service)
    ///
    /// Can be overridden with `LOCALROUTER_KEYCHAIN` environment variable:
    /// - "file" -> Force file-based storage
    /// - "system" -> Force system keyring
    pub fn auto() -> AppResult<Self> {
        // Check environment variable first (allows override)
        match std::env::var("LOCALROUTER_KEYCHAIN").as_deref() {
            Ok("file") => {
                warn!("Using file-based keychain storage (env var override)");
                let secrets_path = lr_utils::paths::secrets_file()?;
                return Self::file(secrets_path);
            }
            Ok("system") => {
                debug!("Using system keyring (env var override)");
                return Ok(Self::system());
            }
            _ => {}
        }

        // Auto-detect based on build type (development vs production)
        #[cfg(debug_assertions)]
        {
            warn!("Using file-based keychain storage (DEVELOPMENT MODE)");
            let secrets_path = lr_utils::paths::secrets_file()?;
            Self::file(secrets_path)
        }

        #[cfg(not(debug_assertions))]
        {
            debug!("Using system keyring for secure storage");
            Ok(Self::system())
        }
    }

    /// Create key for cache lookup
    fn make_cache_key(service: &str, account: &str) -> String {
        format!("{}:{}", service, account)
    }

    /// Clear the entire cache
    /// Useful for testing or when you know the keychain has been modified externally
    #[allow(dead_code)]
    pub fn clear_cache(&self) {
        let _operation = self.operation_lock.lock();
        let mut cache = self.cache.write();
        cache.clear();
        debug!("CachedKeychain: cleared entire cache");
    }

    /// Remove a specific entry from the cache
    #[allow(dead_code)]
    pub fn invalidate(&self, service: &str, account: &str) {
        let _operation = self.operation_lock.lock();
        let cache_key = Self::make_cache_key(service, account);
        let mut cache = self.cache.write();
        cache.remove(&cache_key);
        trace!(
            "CachedKeychain: invalidated cache for {}:{}",
            service,
            account
        );
    }
}

impl KeychainStorage for CachedKeychain {
    fn store(&self, service: &str, account: &str, secret: &str) -> AppResult<()> {
        let _operation = self.operation_lock.lock();
        // Store in the underlying keychain
        self.inner.store(service, account, secret)?;

        // Update cache
        let cache_key = Self::make_cache_key(service, account);
        let mut cache = self.cache.write();
        cache.insert(cache_key, Zeroizing::new(secret.to_string()));
        trace!("CachedKeychain: cached {}:{} after store", service, account);

        Ok(())
    }

    fn get(&self, service: &str, account: &str) -> AppResult<Option<String>> {
        let cache_key = Self::make_cache_key(service, account);

        // Check cache first
        {
            let cache = self.cache.read();
            if let Some(cached_value) = cache.get(&cache_key) {
                debug!("CachedKeychain: cache hit for {}:{}", service, account);
                return Ok(Some(String::clone(cached_value)));
            }
        }

        trace!(
            "CachedKeychain: cache miss for {}:{}, fetching from keyring",
            service,
            account
        );

        let _operation = self.operation_lock.lock();
        // A writer or another cache miss may have populated it while we waited.
        if let Some(value) = self.cache.read().get(&cache_key) {
            return Ok(Some(String::clone(value)));
        }

        // Not in cache, fetch from underlying keychain
        let result = self.inner.get(service, account)?;

        // Cache the result if found
        if let Some(ref value) = result {
            let mut cache = self.cache.write();
            cache.insert(cache_key, Zeroizing::new(value.clone()));
            debug!("CachedKeychain: cached {}:{} after fetch", service, account);
        }

        Ok(result)
    }

    fn delete(&self, service: &str, account: &str) -> AppResult<()> {
        let _operation = self.operation_lock.lock();
        // Delete from underlying keychain
        self.inner.delete(service, account)?;

        // Remove from cache
        let cache_key = Self::make_cache_key(service, account);
        let mut cache = self.cache.write();
        cache.remove(&cache_key);
        trace!(
            "CachedKeychain: removed {}:{} from cache after delete",
            service,
            account
        );

        Ok(())
    }
}

/// Mock keychain implementation using in-memory storage
///
/// Key format: "service:account"
#[derive(Clone)]
#[allow(dead_code)]
pub struct MockKeychain {
    storage: Arc<Mutex<HashMap<String, String>>>,
}

impl MockKeychain {
    /// Create a new mock keychain
    pub fn new() -> Self {
        Self {
            storage: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    /// Create key for storage lookup
    fn make_key(service: &str, account: &str) -> String {
        format!("{}:{}", service, account)
    }
}

impl Default for MockKeychain {
    fn default() -> Self {
        Self::new()
    }
}

impl KeychainStorage for MockKeychain {
    fn store(&self, service: &str, account: &str, secret: &str) -> AppResult<()> {
        let key = Self::make_key(service, account);
        let mut storage = self.storage.lock().unwrap();
        storage.insert(key, secret.to_string());
        Ok(())
    }

    fn get(&self, service: &str, account: &str) -> AppResult<Option<String>> {
        let key = Self::make_key(service, account);
        let storage = self.storage.lock().unwrap();
        Ok(storage.get(&key).cloned())
    }

    fn delete(&self, service: &str, account: &str) -> AppResult<()> {
        let key = Self::make_key(service, account);
        let mut storage = self.storage.lock().unwrap();
        storage.remove(&key);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_mock_keychain() {
        let keychain = MockKeychain::new();

        // Store a value
        keychain
            .store("service", "account", "secret")
            .expect("Failed to store");

        // Retrieve it
        let retrieved = keychain
            .get("service", "account")
            .expect("Failed to get")
            .expect("Value not found");
        assert_eq!(retrieved, "secret");

        // Delete it
        keychain
            .delete("service", "account")
            .expect("Failed to delete");

        // Verify it's gone
        let deleted = keychain.get("service", "account").expect("Failed to get");
        assert!(deleted.is_none());
    }

    #[test]
    fn test_mock_keychain_overwrite() {
        let keychain = MockKeychain::new();

        keychain.store("service", "account", "old").unwrap();
        keychain.store("service", "account", "new").unwrap();

        let value = keychain.get("service", "account").unwrap().unwrap();
        assert_eq!(value, "new");
    }

    #[test]
    fn test_mock_keychain_isolation() {
        let keychain = MockKeychain::new();

        keychain.store("service1", "account", "value1").unwrap();
        keychain.store("service2", "account", "value2").unwrap();

        assert_eq!(
            keychain.get("service1", "account").unwrap().unwrap(),
            "value1"
        );
        assert_eq!(
            keychain.get("service2", "account").unwrap().unwrap(),
            "value2"
        );
    }

    #[test]
    fn concurrent_cache_misses_share_one_storage_read() {
        use std::sync::atomic::{AtomicUsize, Ordering};
        struct SlowKeychain(AtomicUsize);
        impl KeychainStorage for SlowKeychain {
            fn store(&self, _: &str, _: &str, _: &str) -> AppResult<()> {
                Ok(())
            }
            fn delete(&self, _: &str, _: &str) -> AppResult<()> {
                Ok(())
            }
            fn get(&self, _: &str, _: &str) -> AppResult<Option<String>> {
                self.0.fetch_add(1, Ordering::SeqCst);
                std::thread::sleep(std::time::Duration::from_millis(25));
                Ok(Some("secret".into()))
            }
        }
        let inner = Arc::new(SlowKeychain(AtomicUsize::new(0)));
        let cached = Arc::new(CachedKeychain::new(inner.clone()));
        let barrier = Arc::new(std::sync::Barrier::new(8));
        let threads: Vec<_> = (0..8)
            .map(|_| {
                let cached = cached.clone();
                let barrier = barrier.clone();
                std::thread::spawn(move || {
                    barrier.wait();
                    cached.get("service", "account").unwrap()
                })
            })
            .collect();
        for thread in threads {
            assert_eq!(thread.join().unwrap().as_deref(), Some("secret"));
        }
        assert_eq!(inner.0.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn test_cached_keychain_cache_hit() {
        let mock = Arc::new(MockKeychain::new());
        let cached = CachedKeychain::new(mock.clone());

        // Store a value
        cached.store("service", "account", "secret").unwrap();

        // First get should fetch from mock keychain
        let value1 = cached.get("service", "account").unwrap().unwrap();
        assert_eq!(value1, "secret");

        // Second get should hit cache (no additional keychain access)
        let value2 = cached.get("service", "account").unwrap().unwrap();
        assert_eq!(value2, "secret");

        // Both should return the same value
        assert_eq!(value1, value2);
    }

    #[test]
    fn test_cached_keychain_delete_invalidates_cache() {
        let mock = Arc::new(MockKeychain::new());
        let cached = CachedKeychain::new(mock.clone());

        // Store and retrieve to populate cache
        cached.store("service", "account", "secret").unwrap();
        let _ = cached.get("service", "account").unwrap();

        // Delete should remove from cache
        cached.delete("service", "account").unwrap();

        // Verify it's gone
        let result = cached.get("service", "account").unwrap();
        assert!(result.is_none());
    }

    #[test]
    fn test_cached_keychain_store_updates_cache() {
        let mock = Arc::new(MockKeychain::new());
        let cached = CachedKeychain::new(mock.clone());

        // Store a value
        cached.store("service", "account", "secret1").unwrap();

        // Update with new value
        cached.store("service", "account", "secret2").unwrap();

        // Get should return the new value from cache
        let value = cached.get("service", "account").unwrap().unwrap();
        assert_eq!(value, "secret2");
    }

    #[test]
    fn test_cached_keychain_clear_cache() {
        let mock = Arc::new(MockKeychain::new());
        let cached = CachedKeychain::new(mock.clone());

        // Store and get to populate cache
        cached.store("service", "account", "secret").unwrap();
        let _ = cached.get("service", "account").unwrap();

        // Clear cache
        cached.clear_cache();

        // Next get should still work (fetches from underlying mock)
        let value = cached.get("service", "account").unwrap().unwrap();
        assert_eq!(value, "secret");
    }

    #[test]
    fn test_cached_keychain_invalidate() {
        let mock = Arc::new(MockKeychain::new());
        let cached = CachedKeychain::new(mock.clone());

        // Store two values
        cached.store("service", "account1", "secret1").unwrap();
        cached.store("service", "account2", "secret2").unwrap();

        // Get both to populate cache
        let _ = cached.get("service", "account1").unwrap();
        let _ = cached.get("service", "account2").unwrap();

        // Invalidate only one
        cached.invalidate("service", "account1");

        // Both should still be retrievable (one from cache, one from underlying)
        let value1 = cached.get("service", "account1").unwrap().unwrap();
        let value2 = cached.get("service", "account2").unwrap().unwrap();
        assert_eq!(value1, "secret1");
        assert_eq!(value2, "secret2");
    }

    #[test]
    fn test_cached_keychain_multiple_services() {
        let mock = Arc::new(MockKeychain::new());
        let cached = CachedKeychain::new(mock);

        // Store in different services
        cached.store("service1", "account", "secret1").unwrap();
        cached.store("service2", "account", "secret2").unwrap();

        // Should be isolated
        assert_eq!(
            cached.get("service1", "account").unwrap().unwrap(),
            "secret1"
        );
        assert_eq!(
            cached.get("service2", "account").unwrap().unwrap(),
            "secret2"
        );
    }

    #[test]
    fn test_file_keychain() {
        use tempfile::NamedTempFile;

        let temp_file = NamedTempFile::new().unwrap();
        let file_path = temp_file.path().to_path_buf();

        let keychain = FileKeychain::new(file_path.clone()).unwrap();

        // Store a value
        keychain.store("service", "account", "secret").unwrap();

        // Retrieve it
        let retrieved = keychain.get("service", "account").unwrap().unwrap();
        assert_eq!(retrieved, "secret");

        // Create a new instance to verify persistence
        let keychain2 = FileKeychain::new(file_path.clone()).unwrap();
        let retrieved2 = keychain2.get("service", "account").unwrap().unwrap();
        assert_eq!(retrieved2, "secret");

        // Delete it
        keychain2.delete("service", "account").unwrap();

        // Verify it's gone
        let deleted = keychain2.get("service", "account").unwrap();
        assert!(deleted.is_none());
    }

    #[test]
    fn failed_file_write_preserves_in_memory_secrets() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("secrets.json");
        let keychain = FileKeychain::new(path.clone()).unwrap();
        keychain.store("service", "account", "original").unwrap();

        // A directory at the destination makes atomic replacement fail on all
        // platforms without depending on user permissions or a read-only disk.
        fs::remove_file(&path).unwrap();
        fs::create_dir(&path).unwrap();
        assert!(keychain.store("service", "account", "replacement").is_err());
        assert!(keychain.store("service", "new", "new secret").is_err());
        assert!(keychain.delete("service", "account").is_err());
        assert_eq!(
            keychain.get("service", "account").unwrap().as_deref(),
            Some("original")
        );
        assert_eq!(keychain.get("service", "new").unwrap(), None);
        assert_eq!(
            fs::read_dir(dir.path()).unwrap().count(),
            1,
            "failed writes leave no temporary secrets"
        );
    }

    #[cfg(unix)]
    #[test]
    fn file_keychain_persists_with_owner_only_permissions() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("secrets.json");
        let keychain = FileKeychain::new(path.clone()).unwrap();
        keychain.store("service", "account", "secret").unwrap();
        assert_eq!(
            fs::metadata(&path).unwrap().permissions().mode() & 0o777,
            0o600
        );

        // Replacing a legacy, permissive file also tightens its permissions.
        fs::set_permissions(&path, fs::Permissions::from_mode(0o644)).unwrap();
        keychain.store("service", "account", "updated").unwrap();
        assert_eq!(
            fs::metadata(&path).unwrap().permissions().mode() & 0o777,
            0o600
        );
        assert_eq!(
            FileKeychain::new(path)
                .unwrap()
                .get("service", "account")
                .unwrap()
                .as_deref(),
            Some("updated")
        );
    }

    #[test]
    fn test_file_keychain_multiple_keys() {
        use tempfile::NamedTempFile;

        let temp_file = NamedTempFile::new().unwrap();
        let file_path = temp_file.path().to_path_buf();

        let keychain = FileKeychain::new(file_path).unwrap();

        // Store multiple values
        keychain.store("service1", "account1", "secret1").unwrap();
        keychain.store("service2", "account2", "secret2").unwrap();

        // Retrieve them
        assert_eq!(
            keychain.get("service1", "account1").unwrap().unwrap(),
            "secret1"
        );
        assert_eq!(
            keychain.get("service2", "account2").unwrap().unwrap(),
            "secret2"
        );
    }

    #[test]
    fn test_file_keychain_overwrite() {
        use tempfile::NamedTempFile;

        let temp_file = NamedTempFile::new().unwrap();
        let file_path = temp_file.path().to_path_buf();

        let keychain = FileKeychain::new(file_path).unwrap();

        keychain.store("service", "account", "old").unwrap();
        keychain.store("service", "account", "new").unwrap();

        let value = keychain.get("service", "account").unwrap().unwrap();
        assert_eq!(value, "new");
    }

    #[test]
    fn test_cached_file_keychain() {
        use tempfile::NamedTempFile;

        let temp_file = NamedTempFile::new().unwrap();
        let file_path = temp_file.path().to_path_buf();

        let cached = CachedKeychain::file(file_path).unwrap();

        // Store a value
        cached.store("service", "account", "secret").unwrap();

        // First get should fetch from file
        let value1 = cached.get("service", "account").unwrap().unwrap();
        assert_eq!(value1, "secret");

        // Second get should hit cache
        let value2 = cached.get("service", "account").unwrap().unwrap();
        assert_eq!(value2, "secret");
    }

    #[test]
    fn test_file_keychain_concurrent_writers_never_corrupt() {
        // Regression test: parallel writers (multiple instances sharing one
        // secrets file, as in test runs) must never leave the file
        // unparseable. Writes are atomic (temp file + rename), so the file
        // is always valid JSON even if some updates lose the race.
        let dir = std::env::temp_dir().join(format!(
            "lr-keychain-test-{}-{:?}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("secrets.json");

        let mut handles = Vec::new();
        for writer in 0..8 {
            let path = path.clone();
            handles.push(std::thread::spawn(move || {
                let keychain = FileKeychain::new(path).unwrap();
                for i in 0..25 {
                    keychain
                        .store("svc", &format!("acct-{}-{}", writer, i), "secret-value")
                        .unwrap();
                }
            }));
        }
        for handle in handles {
            handle.join().unwrap();
        }

        // The file must be readable and parseable by a fresh instance
        let reloaded = FileKeychain::new(path).unwrap();
        // At least the last writer's final entry survives (last-writer-wins
        // between instances is acceptable; corruption is not)
        assert!(reloaded.get("svc", "acct-7-24").is_ok());

        std::fs::remove_dir_all(&dir).ok();
    }
}

#[cfg(all(test, debug_assertions))]
mod debug_secret_namespace_tests {
    use super::*;
    #[test]
    fn local_native_reset_is_validated_and_cannot_reopen_the_old_service() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("namespace");
        assert_eq!(read_redaction_namespace(&path).unwrap(), None);
        fs::write(&path, "bad/namespace").unwrap();
        assert!(read_redaction_namespace(&path).is_err());
        fs::write(&path, "ABCDEF0123456789abcdef0123456789\n").unwrap();
        let namespace = read_redaction_namespace(&path).unwrap().unwrap();
        assert_eq!(namespace, "reset-abcdef0123456789abcdef0123456789");
        let native = Arc::new(MockKeychain::new());
        native
            .store("LocalRouter-Redacted-Secrets-v1", "owner-index-v1", "stale")
            .unwrap();
        let reset = NamespacedSecretStorage {
            backend: native.clone(),
            namespace,
        };
        assert_eq!(
            reset
                .get("LocalRouter-Redacted-Secrets-v1", "owner-index-v1")
                .unwrap(),
            None
        );
        reset
            .store("LocalRouter-Redacted-Secrets-v1", "owner-index-v1", "fresh")
            .unwrap();
        reset
            .delete("LocalRouter-Redacted-Secrets-v1", "owner-index-v1")
            .unwrap();
        assert_eq!(
            native
                .get("LocalRouter-Redacted-Secrets-v1", "owner-index-v1")
                .unwrap()
                .as_deref(),
            Some("stale")
        );
    }

    #[test]
    fn native_debug_storage_never_reads_or_modifies_production_entries() {
        let native = Arc::new(MockKeychain::new());
        let debug = NamespacedSecretStorage {
            backend: native.clone(),
            namespace: "isolated-test".into(),
        };
        native
            .store(
                "LocalRouter-Redacted-Secrets-v1",
                "owner-index-v1",
                "production",
            )
            .unwrap();
        debug
            .store("LocalRouter-Redacted-Secrets-v1", "owner-index-v1", "debug")
            .unwrap();
        assert_eq!(
            native
                .get("LocalRouter-Redacted-Secrets-v1", "owner-index-v1")
                .unwrap()
                .as_deref(),
            Some("production")
        );
        assert_eq!(
            debug
                .get("LocalRouter-Redacted-Secrets-v1", "owner-index-v1")
                .unwrap()
                .as_deref(),
            Some("debug")
        );
        debug
            .delete("LocalRouter-Redacted-Secrets-v1", "owner-index-v1")
            .unwrap();
        assert_eq!(
            native
                .get("LocalRouter-Redacted-Secrets-v1", "owner-index-v1")
                .unwrap()
                .as_deref(),
            Some("production")
        );
    }
}

#[cfg(test)]
mod lazy_recovery_tests {
    use super::*;
    #[test]
    fn failed_initialization_is_retried_and_durable_records_are_reconciled() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("secrets.json");
        std::fs::write(&path, "not valid JSON").unwrap();
        let location = path.clone();
        let backend = Arc::new(LazySecretStorage {
            backend: parking_lot::Mutex::new(None),
            factory: Box::new(move || {
                Ok(Arc::new(FileKeychain::new(location.clone())?) as Arc<dyn KeychainStorage>)
            }),
        });
        let storage = ResilientSecretStorage::new(backend);
        storage
            .store(
                "s",
                "a",
                r#"{"owner":"client","entries":{"new":"pending-secret"}}"#,
            )
            .unwrap();
        assert!(storage.persistence_warning());
        assert!(storage
            .persistence_details()
            .unwrap()
            .cause
            .contains("parse secrets file"));
        let old = serde_json::json!({"s:a":serde_json::json!({"owner":"client","entries":{"old":"saved-secret"}}).to_string()});
        std::fs::write(&path, old.to_string()).unwrap();
        storage.retry_persistence();
        assert!(!storage.persistence_warning());
        let fresh = FileKeychain::new(path).unwrap();
        let value: serde_json::Value =
            serde_json::from_str(&fresh.get("s", "a").unwrap().unwrap()).unwrap();
        assert_eq!(
            value["entries"],
            serde_json::json!({"old":"saved-secret","new":"pending-secret"})
        );
    }
}

#[cfg(test)]
mod credential_file_error_privacy_tests {
    use super::*;
    #[test]
    fn initial_file_shape_errors_retain_the_cause_without_echoing_credentials() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("secrets.json");
        let secret = "synthetic-private-credential-in-invalid-file";
        std::fs::write(&path, serde_json::to_string(secret).unwrap()).unwrap();
        let location = path.clone();
        let storage = ResilientSecretStorage::new(Arc::new(LazySecretStorage {
            backend: parking_lot::Mutex::new(None),
            factory: Box::new(move || {
                Ok(Arc::new(FileKeychain::new(location.clone())?) as Arc<dyn KeychainStorage>)
            }),
        }));
        assert_eq!(storage.get("s", "a").unwrap(), None);
        let notice = storage.take_persistence_events().pop().unwrap();
        assert!(!notice.cause.contains(secret));
        assert!(notice
            .cause
            .contains("JSON object containing string values"));
        assert!(notice.cause.contains("line 1 column"));
    }
}
