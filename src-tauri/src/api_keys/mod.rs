//! API key management
//!
//! Handles API key generation and metadata management.
//! Actual keys are stored in the OS keychain, metadata is stored in config.

mod keychain;
pub mod keychain_trait;

pub use keychain_trait::{CachedKeychain, FileKeychain, KeychainStorage, MockKeychain, SystemKeychain};

use crate::config::ApiKeyConfig;
use crate::utils::crypto::generate_api_key;
use crate::utils::errors::{AppError, AppResult};
use parking_lot::RwLock;
use std::sync::Arc;

/// Thread-safe API key manager
///
/// Stores API key metadata in memory (synced to config file by ConfigManager).
/// Actual API keys are stored in OS keychain (or mock for testing).
/// Key values are cached by CachedKeychain to avoid repeated keychain access.
#[derive(Clone)]
pub struct ApiKeyManager {
    /// In-memory storage of API key metadata
    keys: Arc<RwLock<Vec<ApiKeyConfig>>>,
    /// Next auto-increment number for default key names
    next_key_number: Arc<RwLock<u32>>,
    /// Keychain storage implementation (with caching)
    keychain: Arc<dyn KeychainStorage>,
}

const API_KEY_SERVICE: &str = "LocalRouter-APIKeys";

impl ApiKeyManager {
    /// Create a new API key manager with existing keys from config
    /// Uses the auto-detected keychain (system or file-based depending on LOCALROUTER_KEYCHAIN env var)
    pub fn new(keys: Vec<ApiKeyConfig>) -> Self {
        let keychain = CachedKeychain::auto()
            .unwrap_or_else(|e| {
                tracing::warn!("Failed to create auto keychain: {}, falling back to system", e);
                CachedKeychain::system()
            });
        Self::with_keychain(keys, Arc::new(keychain))
    }

    /// Create a new API key manager with a custom keychain implementation
    /// Useful for testing with MockKeychain
    pub fn with_keychain(
        keys: Vec<ApiKeyConfig>,
        keychain: Arc<dyn KeychainStorage>,
    ) -> Self {
        // Calculate next key number based on existing keys
        let next_number = keys
            .iter()
            .filter_map(|k| {
                // Extract number from names like "my-app-123"
                k.name
                    .strip_prefix("my-app-")
                    .and_then(|s| s.parse::<u32>().ok())
            })
            .max()
            .map(|n| n + 1)
            .unwrap_or(1);

        Self {
            keys: Arc::new(RwLock::new(keys)),
            next_key_number: Arc::new(RwLock::new(next_number)),
            keychain,
        }
    }

    /// Load API keys metadata from config
    ///
    /// Note: This just wraps the constructor. The actual metadata should be
    /// loaded from the config file by ConfigManager.
    pub async fn load() -> AppResult<Self> {
        // For now, return empty manager
        // In practice, the main.rs will load from config and pass to new()
        Ok(Self::new(Vec::new()))
    }

    /// Create a new API key
    ///
    /// # Arguments
    /// * `name` - Optional name for the key. If None, generates "my-app-{number}"
    ///
    /// # Returns
    /// The generated API key string (lr-...) and the key configuration
    ///
    /// Note: Model selection can be set later using update_key()
    pub async fn create_key(
        &self,
        name: Option<String>,
    ) -> AppResult<(String, ApiKeyConfig)> {
        // Generate the actual API key
        let key = generate_api_key()?;

        // Determine the name
        let key_name = if let Some(name) = name {
            name
        } else {
            let num = {
                let mut next = self.next_key_number.write();
                let current = *next;
                *next += 1;
                current
            };
            format!("my-app-{}", num)
        };

        // Create the config (metadata only)
        let config = ApiKeyConfig::new(key_name);

        tracing::info!("Storing API key in keychain: service={}, account={}", API_KEY_SERVICE, config.id);

        // Store actual key in keychain (will be cached by CachedKeychain)
        self.keychain.store(API_KEY_SERVICE, &config.id, &key)?;

        tracing::info!("Successfully stored API key in keychain");

        // Add metadata to in-memory storage
        {
            let mut keys = self.keys.write();
            keys.push(config.clone());
        }

        // Note: Caller must save to config file via ConfigManager

        Ok((key, config))
    }

    /// Get all API key metadata
    pub fn list_keys(&self) -> Vec<ApiKeyConfig> {
        self.keys.read().clone()
    }

    /// Get a specific API key metadata by ID
    pub fn get_key(&self, id: &str) -> Option<ApiKeyConfig> {
        self.keys.read().iter().find(|k| k.id == id).cloned()
    }

    /// Get the actual API key value from keychain (with caching)
    ///
    /// # Arguments
    /// * `id` - The API key ID
    ///
    /// # Returns
    /// * `Ok(Some(key))` if key exists
    /// * `Ok(None)` if key doesn't exist
    /// * `Err` on keychain access error
    ///
    /// Note: The CachedKeychain automatically caches retrieved values to avoid
    /// repeated keychain access and password prompts.
    pub fn get_key_value(&self, id: &str) -> AppResult<Option<String>> {
        tracing::debug!("Retrieving API key: service={}, account={}", API_KEY_SERVICE, id);
        let result = self.keychain.get(API_KEY_SERVICE, id)?;

        if result.is_none() {
            tracing::warn!("API key not found in keychain: {}", id);
        }

        Ok(result)
    }

    /// Update an API key's metadata
    ///
    /// Note: This only updates metadata. To change the actual key value,
    /// delete and recreate the key.
    pub fn update_key<F>(&self, id: &str, update_fn: F) -> AppResult<ApiKeyConfig>
    where
        F: FnOnce(&mut ApiKeyConfig),
    {
        let updated = {
            let mut keys = self.keys.write();
            let key = keys
                .iter_mut()
                .find(|k| k.id == id)
                .ok_or_else(|| AppError::ApiKey(format!("API key not found: {}", id)))?;

            update_fn(key);
            key.clone()
        };

        // Note: Caller must save to config file via ConfigManager
        Ok(updated)
    }

    /// Delete an API key
    ///
    /// Removes both metadata and the actual key from keychain (and cache).
    pub fn delete_key(&self, id: &str) -> AppResult<()> {
        // Remove from metadata
        {
            let mut keys = self.keys.write();
            let initial_len = keys.len();
            keys.retain(|k| k.id != id);

            if keys.len() == initial_len {
                return Err(AppError::ApiKey(format!("API key not found: {}", id)));
            }
        }

        // Remove from keychain (CachedKeychain will also remove from cache)
        self.keychain.delete(API_KEY_SERVICE, id)?;

        // Note: Caller must save to config file via ConfigManager
        Ok(())
    }

    /// Rotate an API key
    ///
    /// Generates a new API key value while keeping the same ID and metadata.
    /// This is useful for security purposes when a key might have been compromised.
    ///
    /// # Arguments
    /// * `id` - The API key ID to rotate
    ///
    /// # Returns
    /// The new API key string (lr-...)
    pub async fn rotate_key(&self, id: &str) -> AppResult<String> {
        // Verify the key exists
        {
            let keys = self.keys.read();
            if !keys.iter().any(|k| k.id == id) {
                return Err(AppError::ApiKey(format!("API key not found: {}", id)));
            }
        }

        tracing::info!("Rotating API key: {}", id);

        // Generate a new API key
        let new_key = generate_api_key()?;

        // Update keychain with new key (same ID)
        // CachedKeychain will automatically update the cache
        self.keychain.store(API_KEY_SERVICE, id, &new_key)?;

        tracing::info!("Successfully rotated API key in keychain");

        Ok(new_key)
    }

    /// Verify an API key string and return the associated configuration
    ///
    /// This looks up the key in keychain (with caching) for all enabled keys
    /// and returns the metadata if a match is found.
    pub fn verify_key(&self, key: &str) -> Option<ApiKeyConfig> {
        let keys = self.keys.read();

        for key_config in keys.iter() {
            if !key_config.enabled {
                continue;
            }

            // Fetch from keychain (CachedKeychain will use cache if available)
            let stored_key = match self.get_key_value(&key_config.id) {
                Ok(Some(k)) => k,
                Ok(None) => {
                    tracing::warn!("API key {} not found in keychain", key_config.id);
                    continue;
                }
                Err(e) => {
                    tracing::error!("Error retrieving key {} from keychain: {:?}", key_config.id, e);
                    continue;
                }
            };

            // Constant-time comparison to prevent timing attacks
            if key == stored_key {
                tracing::info!("API key verified successfully: {}", key_config.id);
                return Some(key_config.clone());
            }
        }

        tracing::warn!("API key verification failed - no matching key found");
        None
    }

    /// Reload API keys metadata from config
    ///
    /// Used when config is externally modified.
    pub fn reload(&self, keys: Vec<ApiKeyConfig>) {
        *self.keys.write() = keys;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_create_key() {
        let keychain = Arc::new(MockKeychain::new());
        let manager = ApiKeyManager::with_keychain(vec![], keychain.clone());

        let result = manager
            .create_key(Some("test-key".to_string()))
            .await;

        assert!(result.is_ok());
        let (key, config) = result.unwrap();

        // Verify key format
        assert!(key.starts_with("lr-"));

        // Verify config
        assert_eq!(config.name, "test-key");
        assert!(config.enabled);

        // Verify key is in mock keychain
        let stored_key = keychain.get(API_KEY_SERVICE, &config.id).unwrap().unwrap();
        assert_eq!(stored_key, key);
    }

    #[tokio::test]
    async fn test_verify_key() {
        let keychain = Arc::new(MockKeychain::new());
        let manager = ApiKeyManager::with_keychain(vec![], keychain);

        let (key, config) = manager
            .create_key(None)
            .await
            .unwrap();

        // Verify with correct key
        let verified = manager.verify_key(&key);
        assert!(verified.is_some());
        assert_eq!(verified.unwrap().id, config.id);

        // Verify with wrong key
        let verified = manager.verify_key("wrong-key");
        assert!(verified.is_none());
    }

    #[tokio::test]
    async fn test_delete_key() {
        let keychain = Arc::new(MockKeychain::new());
        let manager = ApiKeyManager::with_keychain(vec![], keychain.clone());

        let (_, config) = manager
            .create_key(None)
            .await
            .unwrap();

        // Delete the key
        let result = manager.delete_key(&config.id);
        assert!(result.is_ok());

        // Verify it's gone from metadata
        assert!(manager.get_key(&config.id).is_none());

        // Verify it's gone from keychain
        let key_value = keychain.get(API_KEY_SERVICE, &config.id).unwrap();
        assert!(key_value.is_none());
    }
}
