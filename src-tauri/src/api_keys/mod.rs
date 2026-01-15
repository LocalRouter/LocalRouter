//! API key management
//!
//! Handles API key generation and metadata management.
//! Actual keys are stored in the OS keychain, metadata is stored in config.

mod keychain;

use crate::config::{ApiKeyConfig, ModelSelection};
use crate::utils::crypto::generate_api_key;
use crate::utils::errors::{AppError, AppResult};
use parking_lot::RwLock;
use std::sync::Arc;

/// Thread-safe API key manager
///
/// Stores API key metadata in memory (synced to config file by ConfigManager).
/// Actual API keys are stored in OS keychain.
#[derive(Debug, Clone)]
pub struct ApiKeyManager {
    /// In-memory storage of API key metadata
    keys: Arc<RwLock<Vec<ApiKeyConfig>>>,
    /// Next auto-increment number for default key names
    next_key_number: Arc<RwLock<u32>>,
}

impl ApiKeyManager {
    /// Create a new API key manager with existing keys from config
    pub fn new(keys: Vec<ApiKeyConfig>) -> Self {
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
    /// * `model_selection` - Model or router to use for this key
    ///
    /// # Returns
    /// The generated API key string (lr-...) and the key configuration
    pub async fn create_key(
        &self,
        name: Option<String>,
        model_selection: ModelSelection,
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
        let config = ApiKeyConfig::new(key_name, model_selection);

        // Store actual key in keychain
        keychain::store_api_key(&config.id, &key)?;

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

    /// Get the actual API key value from keychain
    ///
    /// # Arguments
    /// * `id` - The API key ID
    ///
    /// # Returns
    /// * `Ok(Some(key))` if key exists in keychain
    /// * `Ok(None)` if key doesn't exist in keychain
    /// * `Err` on keychain access error
    pub fn get_key_value(&self, id: &str) -> AppResult<Option<String>> {
        keychain::get_api_key(id)
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
    /// Removes both metadata and the actual key from keychain.
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

        // Remove from keychain
        keychain::delete_api_key(id)?;

        // Note: Caller must save to config file via ConfigManager
        Ok(())
    }

    /// Verify an API key string and return the associated configuration
    ///
    /// This looks up the key in the keychain for all enabled keys
    /// and returns the metadata if a match is found.
    pub fn verify_key(&self, key: &str) -> Option<ApiKeyConfig> {
        let keys = self.keys.read();

        for key_config in keys.iter() {
            if !key_config.enabled {
                continue;
            }

            // Try to get the actual key from keychain
            if let Ok(Some(stored_key)) = keychain::get_api_key(&key_config.id) {
                // Constant-time comparison to prevent timing attacks
                if key == stored_key {
                    return Some(key_config.clone());
                }
            }
        }

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
    use serial_test::serial;

    #[tokio::test]
    #[serial]
    async fn test_create_key() {
        let manager = ApiKeyManager::new(vec![]);

        let result = manager
            .create_key(
                Some("test-key".to_string()),
                ModelSelection::DirectModel {
                    provider: "ollama".to_string(),
                    model: "llama2".to_string(),
                },
            )
            .await;

        assert!(result.is_ok());
        let (key, config) = result.unwrap();

        // Verify key format
        assert!(key.starts_with("lr-"));

        // Verify config
        assert_eq!(config.name, "test-key");
        assert!(config.enabled);

        // Cleanup
        let _ = keychain::delete_api_key(&config.id);
    }

    #[tokio::test]
    #[serial]
    async fn test_verify_key() {
        let manager = ApiKeyManager::new(vec![]);

        let (key, config) = manager
            .create_key(
                None,
                ModelSelection::DirectModel {
                    provider: "ollama".to_string(),
                    model: "llama2".to_string(),
                },
            )
            .await
            .unwrap();

        // Verify with correct key
        let verified = manager.verify_key(&key);
        assert!(verified.is_some());
        assert_eq!(verified.unwrap().id, config.id);

        // Verify with wrong key
        let verified = manager.verify_key("wrong-key");
        assert!(verified.is_none());

        // Cleanup
        let _ = keychain::delete_api_key(&config.id);
    }

    #[tokio::test]
    #[serial]
    async fn test_delete_key() {
        let manager = ApiKeyManager::new(vec![]);

        let (_, config) = manager
            .create_key(
                None,
                ModelSelection::DirectModel {
                    provider: "ollama".to_string(),
                    model: "llama2".to_string(),
                },
            )
            .await
            .unwrap();

        // Delete the key
        let result = manager.delete_key(&config.id);
        assert!(result.is_ok());

        // Verify it's gone from metadata
        assert!(manager.get_key(&config.id).is_none());

        // Verify it's gone from keychain
        let key_value = keychain::get_api_key(&config.id).unwrap();
        assert!(key_value.is_none());
    }
}
