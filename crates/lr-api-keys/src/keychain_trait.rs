//! Keychain trait abstraction for testability
//!
//! Provides a trait-based interface for keychain operations,
//! allowing for real (system keyring) and mock (in-memory) implementations.
//!
//! The CachedKeychain wrapper provides in-memory caching to prevent
//! repeated password prompts for the same service:account combination.

use lr_types::errors::AppResult;
use parking_lot::RwLock;
use std::collections::HashMap;
use std::fs;
use std::sync::{Arc, Mutex};
use tracing::{debug, trace, warn};

#[cfg(not(debug_assertions))]
use tracing::info;
use zeroize::Zeroizing;

/// Trait for keychain operations
pub trait KeychainStorage: Send + Sync {
    /// Store a key-value pair
    fn store(&self, service: &str, account: &str, secret: &str) -> AppResult<()>;

    /// Retrieve a value by service and account
    fn get(&self, service: &str, account: &str) -> AppResult<Option<String>>;

    /// Delete a key-value pair
    fn delete(&self, service: &str, account: &str) -> AppResult<()>;
}

/// Real keychain implementation using system keyring
pub struct SystemKeychain;

impl KeychainStorage for SystemKeychain {
    fn store(&self, service: &str, account: &str, secret: &str) -> AppResult<()> {
        trace!("SystemKeychain: storing {}:{}", service, account);
        let entry = keyring::Entry::new(service, account).map_err(|e| {
            lr_types::AppError::Internal(format!("Failed to access keyring: {}", e))
        })?;

        entry
            .set_password(secret)
            .map_err(|e| lr_types::AppError::Internal(format!("Failed to store key: {}", e)))?;

        debug!("SystemKeychain: stored {}:{}", service, account);
        Ok(())
    }

    fn get(&self, service: &str, account: &str) -> AppResult<Option<String>> {
        trace!(
            "SystemKeychain: retrieving {}:{} from system keyring",
            service,
            account
        );
        let entry = keyring::Entry::new(service, account).map_err(|e| {
            lr_types::AppError::Internal(format!("Failed to access keyring: {}", e))
        })?;

        match entry.get_password() {
            Ok(secret) => {
                debug!(
                    "SystemKeychain: retrieved {}:{} from system keyring",
                    service, account
                );
                Ok(Some(secret))
            }
            Err(keyring::Error::NoEntry) => {
                trace!("SystemKeychain: no entry found for {}:{}", service, account);
                Ok(None)
            }
            Err(e) => Err(lr_types::AppError::Internal(format!(
                "Failed to retrieve key: {}",
                e
            ))),
        }
    }

    fn delete(&self, service: &str, account: &str) -> AppResult<()> {
        trace!("SystemKeychain: deleting {}:{}", service, account);
        let entry = keyring::Entry::new(service, account).map_err(|e| {
            lr_types::AppError::Internal(format!("Failed to access keyring: {}", e))
        })?;

        match entry.delete_credential() {
            Ok(()) => {
                debug!("SystemKeychain: deleted {}:{}", service, account);
                Ok(())
            }
            Err(keyring::Error::NoEntry) => {
                trace!(
                    "SystemKeychain: no entry to delete for {}:{}",
                    service,
                    account
                );
                Ok(())
            }
            Err(e) => Err(lr_types::AppError::Internal(format!(
                "Failed to delete key: {}",
                e
            ))),
        }
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
            serde_json::from_str(&contents).map_err(|e| {
                lr_types::AppError::Internal(format!("Failed to parse secrets file: {}", e))
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

    /// Save secrets to file
    fn save_to_file(&self) -> AppResult<()> {
        let storage = self.storage.lock().unwrap();

        // Ensure parent directory exists
        if let Some(parent) = self.file_path.parent() {
            fs::create_dir_all(parent).map_err(|e| {
                lr_types::AppError::Internal(format!("Failed to create secrets directory: {}", e))
            })?;
        }

        let contents = serde_json::to_string_pretty(&*storage).map_err(|e| {
            lr_types::AppError::Internal(format!("Failed to serialize secrets: {}", e))
        })?;

        fs::write(self.file_path.as_ref(), contents).map_err(|e| {
            lr_types::AppError::Internal(format!("Failed to write secrets file: {}", e))
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
        {
            let mut storage = self.storage.lock().unwrap();
            storage.insert(key.clone(), secret.to_string());
        }
        self.save_to_file()?;
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
        {
            let mut storage = self.storage.lock().unwrap();
            storage.remove(&key);
        }
        self.save_to_file()?;
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
    /// In-memory cache of retrieved values
    /// Key: "service:account", Value: secret (zeroized on drop)
    cache: Arc<RwLock<HashMap<String, Zeroizing<String>>>>,
}

impl CachedKeychain {
    /// Create a new cached keychain wrapping the given implementation
    pub fn new(inner: Arc<dyn KeychainStorage>) -> Self {
        Self {
            inner,
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
        let mut cache = self.cache.write();
        cache.clear();
        debug!("CachedKeychain: cleared entire cache");
    }

    /// Remove a specific entry from the cache
    #[allow(dead_code)]
    pub fn invalidate(&self, service: &str, account: &str) {
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

/// Migrate secrets from a plaintext file to the system keychain.
///
/// When a user copies their dev config directory (`~/.localrouter-dev/`) to the production
/// directory (`~/.localrouter/`), the `secrets.json` file (used for file-based storage in
/// dev mode) comes along. In production, secrets are stored in the OS keychain, so this
/// file is normally ignored.
///
/// This function reads the secrets file, stores each secret in the system keychain
/// (skipping entries that already exist), and deletes the file afterwards.
///
/// This is a no-op if:
/// - The secrets file does not exist
/// - `LOCALROUTER_KEYCHAIN` is set to `"file"` (user explicitly wants file-based storage)
///
/// Only compiled into release builds.
#[cfg(not(debug_assertions))]
pub fn migrate_secrets_file_to_keychain() -> AppResult<()> {
    // If the user explicitly wants file-based storage, don't migrate
    if matches!(
        std::env::var("LOCALROUTER_KEYCHAIN").as_deref(),
        Ok("file")
    ) {
        return Ok(());
    }

    let secrets_path = lr_utils::paths::secrets_file()?;

    if !secrets_path.exists() {
        return Ok(());
    }

    info!(
        "Found plaintext secrets file at {}, migrating to system keychain...",
        secrets_path.display()
    );

    let contents = fs::read_to_string(&secrets_path).map_err(|e| {
        lr_types::AppError::Internal(format!("Failed to read secrets file: {}", e))
    })?;

    if contents.trim().is_empty() {
        fs::remove_file(&secrets_path).ok();
        info!("Removed empty secrets file");
        return Ok(());
    }

    let secrets: HashMap<String, String> = serde_json::from_str(&contents).map_err(|e| {
        lr_types::AppError::Internal(format!("Failed to parse secrets file: {}", e))
    })?;

    if secrets.is_empty() {
        fs::remove_file(&secrets_path).ok();
        info!("Removed empty secrets file");
        return Ok(());
    }

    let system_keychain = SystemKeychain;
    let mut migrated = 0u32;
    let mut skipped = 0u32;
    let mut errors = 0u32;

    for (key, secret) in &secrets {
        // Key format is "service:account"
        let Some((service, account)) = key.split_once(':') else {
            warn!("Skipping secret with invalid key format (expected 'service:account'): {}", key);
            continue;
        };

        // Check if already exists in system keychain
        match system_keychain.get(service, account) {
            Ok(Some(_)) => {
                debug!(
                    "Secret already exists in system keychain for {}:{}, skipping",
                    service, account
                );
                skipped += 1;
            }
            Ok(None) => match system_keychain.store(service, account, secret) {
                Ok(()) => {
                    info!(
                        "Migrated secret for {}:{} to system keychain",
                        service, account
                    );
                    migrated += 1;
                }
                Err(e) => {
                    warn!(
                        "Failed to migrate secret for {}:{} to system keychain: {}",
                        service, account, e
                    );
                    errors += 1;
                }
            },
            Err(e) => {
                warn!(
                    "Failed to check system keychain for {}:{}: {}",
                    service, account, e
                );
                errors += 1;
            }
        }
    }

    info!(
        "Secrets file migration complete: {} migrated, {} already existed, {} errors",
        migrated, skipped, errors
    );

    if errors > 0 {
        warn!(
            "Some secrets failed to migrate ({} errors). Keeping secrets file for retry on next launch.",
            errors
        );
        return Ok(());
    }

    // All secrets migrated successfully, delete the file
    fs::remove_file(&secrets_path).map_err(|e| {
        lr_types::AppError::Internal(format!("Failed to delete secrets file: {}", e))
    })?;

    info!("Deleted plaintext secrets file: {}", secrets_path.display());

    Ok(())
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
}
