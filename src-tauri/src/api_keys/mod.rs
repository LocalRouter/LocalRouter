//! API key management
//!
//! Handles API key generation, storage, and CRUD operations.
//! Keys are stored in encrypted form in api_keys.json with thread-safe access.

mod storage;

use crate::config::{ApiKeyConfig, ModelSelection};
use crate::utils::crypto::{generate_api_key, hash_api_key, verify_api_key};
use crate::utils::errors::{AppError, AppResult};
use parking_lot::RwLock;
use std::sync::Arc;

/// Thread-safe API key manager
#[derive(Debug, Clone)]
pub struct ApiKeyManager {
    /// In-memory storage of API keys
    keys: Arc<RwLock<Vec<ApiKeyConfig>>>,
    /// Next auto-increment number for default key names
    next_key_number: Arc<RwLock<u32>>,
}

impl ApiKeyManager {
    /// Create a new API key manager with existing keys
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

    /// Load API keys from disk
    pub async fn load() -> AppResult<Self> {
        let keys = storage::load_api_keys().await?;
        Ok(Self::new(keys))
    }

    /// Save API keys to disk
    pub async fn save(&self) -> AppResult<()> {
        let keys = self.keys.read().clone();
        storage::save_api_keys(&keys).await
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

        // Hash it for storage
        let key_hash = hash_api_key(&key)?;

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

        // Create the config
        let config = ApiKeyConfig::new(key_name, key_hash, model_selection);

        // Add to storage
        {
            let mut keys = self.keys.write();
            keys.push(config.clone());
        }

        // Save to disk
        self.save().await?;

        Ok((key, config))
    }

    /// Get all API keys
    pub fn list_keys(&self) -> Vec<ApiKeyConfig> {
        self.keys.read().clone()
    }

    /// Get a specific API key by ID
    pub fn get_key(&self, id: &str) -> Option<ApiKeyConfig> {
        self.keys.read().iter().find(|k| k.id == id).cloned()
    }

    /// Update an API key's configuration
    pub async fn update_key<F>(&self, id: &str, update_fn: F) -> AppResult<ApiKeyConfig>
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

        self.save().await?;
        Ok(updated)
    }

    /// Delete an API key
    pub async fn delete_key(&self, id: &str) -> AppResult<()> {
        {
            let mut keys = self.keys.write();
            let initial_len = keys.len();
            keys.retain(|k| k.id != id);

            if keys.len() == initial_len {
                return Err(AppError::ApiKey(format!("API key not found: {}", id)));
            }
        }

        self.save().await?;
        Ok(())
    }

    /// Verify an API key string and return the associated configuration
    ///
    /// This searches through all keys, verifying the hash against each until a match is found.
    pub fn verify_key(&self, key: &str) -> Option<ApiKeyConfig> {
        let keys = self.keys.read();

        for key_config in keys.iter() {
            if !key_config.enabled {
                continue;
            }

            if let Ok(valid) = verify_api_key(key, &key_config.key_hash) {
                if valid {
                    return Some(key_config.clone());
                }
            }
        }

        None
    }

    /// Regenerate an API key (creates new key value, keeps same config)
    pub async fn regenerate_key(&self, id: &str) -> AppResult<(String, ApiKeyConfig)> {
        // Generate new key
        let new_key = generate_api_key()?;
        let new_hash = hash_api_key(&new_key)?;

        // Update the hash
        let updated = self
            .update_key(id, |key| {
                key.key_hash = new_hash;
            })
            .await?;

        Ok((new_key, updated))
    }

    /// Enable or disable an API key
    pub async fn set_enabled(&self, id: &str, enabled: bool) -> AppResult<()> {
        self.update_key(id, |key| {
            key.enabled = enabled;
        })
        .await?;
        Ok(())
    }

    /// Update the last used timestamp for a key
    pub async fn update_last_used(&self, id: &str) -> AppResult<()> {
        self.update_key(id, |key| {
            key.last_used = Some(chrono::Utc::now());
        })
        .await?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serial_test::serial;

    #[tokio::test]
    #[serial]
    async fn test_create_key_with_default_name() {
        let manager = ApiKeyManager::new(vec![]);
        let (key, config) = manager
            .create_key(
                None,
                ModelSelection::Router {
                    router_name: "Minimum Cost".to_string(),
                },
            )
            .await
            .unwrap();

        assert!(key.starts_with("lr-"));
        assert_eq!(config.name, "my-app-1");
        assert!(config.enabled);
    }

    #[tokio::test]
    #[serial]
    async fn test_create_key_with_custom_name() {
        let manager = ApiKeyManager::new(vec![]);
        let (key, config) = manager
            .create_key(
                Some("custom-name".to_string()),
                ModelSelection::Router {
                    router_name: "Minimum Cost".to_string(),
                },
            )
            .await
            .unwrap();

        assert!(key.starts_with("lr-"));
        assert_eq!(config.name, "custom-name");
    }

    #[tokio::test]
    #[serial]
    async fn test_key_numbering() {
        let manager = ApiKeyManager::new(vec![]);

        let (_, config1) = manager
            .create_key(
                None,
                ModelSelection::Router {
                    router_name: "test".to_string(),
                },
            )
            .await
            .unwrap();

        let (_, config2) = manager
            .create_key(
                None,
                ModelSelection::Router {
                    router_name: "test".to_string(),
                },
            )
            .await
            .unwrap();

        assert_eq!(config1.name, "my-app-1");
        assert_eq!(config2.name, "my-app-2");
    }

    #[tokio::test]
    #[serial]
    async fn test_list_keys() {
        let manager = ApiKeyManager::new(vec![]);

        manager
            .create_key(
                None,
                ModelSelection::Router {
                    router_name: "test".to_string(),
                },
            )
            .await
            .unwrap();

        manager
            .create_key(
                None,
                ModelSelection::Router {
                    router_name: "test".to_string(),
                },
            )
            .await
            .unwrap();

        let keys = manager.list_keys();
        assert_eq!(keys.len(), 2);
    }

    #[tokio::test]
    #[serial]
    async fn test_get_key() {
        let manager = ApiKeyManager::new(vec![]);
        let (_, config) = manager
            .create_key(
                Some("test".to_string()),
                ModelSelection::Router {
                    router_name: "test".to_string(),
                },
            )
            .await
            .unwrap();

        let retrieved = manager.get_key(&config.id);
        assert!(retrieved.is_some());
        assert_eq!(retrieved.unwrap().name, "test");
    }

    #[tokio::test]
    #[serial]
    async fn test_update_key() {
        let manager = ApiKeyManager::new(vec![]);
        let (_, config) = manager
            .create_key(
                Some("old-name".to_string()),
                ModelSelection::Router {
                    router_name: "test".to_string(),
                },
            )
            .await
            .unwrap();

        let updated = manager
            .update_key(&config.id, |key| {
                key.name = "new-name".to_string();
            })
            .await
            .unwrap();

        assert_eq!(updated.name, "new-name");

        let retrieved = manager.get_key(&config.id).unwrap();
        assert_eq!(retrieved.name, "new-name");
    }

    #[tokio::test]
    #[serial]
    async fn test_delete_key() {
        let manager = ApiKeyManager::new(vec![]);
        let (_, config) = manager
            .create_key(
                None,
                ModelSelection::Router {
                    router_name: "test".to_string(),
                },
            )
            .await
            .unwrap();

        assert_eq!(manager.list_keys().len(), 1);

        manager.delete_key(&config.id).await.unwrap();

        assert_eq!(manager.list_keys().len(), 0);
        assert!(manager.get_key(&config.id).is_none());
    }

    #[tokio::test]
    #[serial]
    async fn test_verify_key() {
        let manager = ApiKeyManager::new(vec![]);
        let (key, config) = manager
            .create_key(
                Some("test".to_string()),
                ModelSelection::Router {
                    router_name: "test".to_string(),
                },
            )
            .await
            .unwrap();

        // Should verify successfully
        let verified = manager.verify_key(&key);
        assert!(verified.is_some());
        assert_eq!(verified.unwrap().id, config.id);

        // Should fail with wrong key
        assert!(manager.verify_key("lr-wrongkey").is_none());
    }

    #[tokio::test]
    #[serial]
    async fn test_verify_disabled_key() {
        let manager = ApiKeyManager::new(vec![]);
        let (key, config) = manager
            .create_key(
                None,
                ModelSelection::Router {
                    router_name: "test".to_string(),
                },
            )
            .await
            .unwrap();

        // Disable the key
        manager.set_enabled(&config.id, false).await.unwrap();

        // Should not verify disabled key
        assert!(manager.verify_key(&key).is_none());
    }

    #[tokio::test]
    #[serial]
    async fn test_regenerate_key() {
        let manager = ApiKeyManager::new(vec![]);
        let (old_key, config) = manager
            .create_key(
                Some("test".to_string()),
                ModelSelection::Router {
                    router_name: "test".to_string(),
                },
            )
            .await
            .unwrap();

        let (new_key, new_config) = manager.regenerate_key(&config.id).await.unwrap();

        // Keys should be different
        assert_ne!(old_key, new_key);

        // Config ID and name should be the same
        assert_eq!(config.id, new_config.id);
        assert_eq!(config.name, new_config.name);

        // Old key should not verify
        assert!(manager.verify_key(&old_key).is_none());

        // New key should verify
        assert!(manager.verify_key(&new_key).is_some());
    }

    #[tokio::test]
    #[serial]
    async fn test_set_enabled() {
        let manager = ApiKeyManager::new(vec![]);
        let (_, config) = manager
            .create_key(
                None,
                ModelSelection::Router {
                    router_name: "test".to_string(),
                },
            )
            .await
            .unwrap();

        assert!(manager.get_key(&config.id).unwrap().enabled);

        manager.set_enabled(&config.id, false).await.unwrap();
        assert!(!manager.get_key(&config.id).unwrap().enabled);

        manager.set_enabled(&config.id, true).await.unwrap();
        assert!(manager.get_key(&config.id).unwrap().enabled);
    }
}
