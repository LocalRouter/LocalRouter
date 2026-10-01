//! OAuth credentials storage
//!
//! Stores OAuth credentials securely in a JSON file with proper permissions.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use tokio::fs;
use tokio::sync::{OwnedRwLockWriteGuard, RwLock};

use super::OAuthCredentials;
use lr_types::{AppError, AppResult};

/// OAuth credentials storage
pub struct OAuthStorage {
    /// Path to the credentials file
    storage_path: PathBuf,
    /// In-memory cache of credentials
    cache: Arc<RwLock<HashMap<String, OAuthCredentials>>>,
}

#[derive(Debug, Serialize, Deserialize)]
struct StorageFormat {
    credentials: HashMap<String, OAuthCredentials>,
}

impl OAuthStorage {
    /// Create a new OAuth storage
    ///
    /// # Arguments
    /// * `storage_path` - Path to store the credentials file
    pub async fn new(storage_path: PathBuf) -> AppResult<Self> {
        let storage = Self {
            storage_path,
            cache: Arc::new(RwLock::new(HashMap::new())),
        };

        // Load existing credentials
        storage.load().await?;

        Ok(storage)
    }

    /// Load credentials from disk
    async fn load(&self) -> AppResult<()> {
        if !self.storage_path.exists() {
            return Ok(());
        }

        let content = fs::read_to_string(&self.storage_path)
            .await
            .map_err(|e| AppError::Storage(format!("Failed to read OAuth storage: {}", e)))?;

        let storage: StorageFormat = serde_json::from_str(&content)
            .map_err(|e| AppError::Storage(format!("Failed to parse OAuth storage: {}", e)))?;

        *self.cache.write().await = storage.credentials;

        Ok(())
    }

    /// Save credentials to disk
    async fn save(
        &self,
        mut cache: OwnedRwLockWriteGuard<HashMap<String, OAuthCredentials>>,
        updated: HashMap<String, OAuthCredentials>,
    ) -> AppResult<()> {
        let storage = StorageFormat {
            credentials: updated,
        };
        let content = serde_json::to_string_pretty(&storage)
            .map_err(|e| AppError::Storage(format!("Failed to serialize OAuth storage: {}", e)))?;

        let path = self.storage_path.clone();
        tokio::task::spawn_blocking(move || -> AppResult<()> {
            use std::io::Write;

            let parent = path
                .parent()
                .filter(|parent| !parent.as_os_str().is_empty())
                .unwrap_or_else(|| std::path::Path::new("."));
            std::fs::create_dir_all(parent).map_err(|e| {
                AppError::Storage(format!("Failed to create storage directory: {}", e))
            })?;

            // NamedTempFile creates Unix files as 0600 from the outset. Write
            // beside the destination so replacement is atomic: neither readers
            // nor a failed write can observe a truncated credential document.
            let mut file = tempfile::NamedTempFile::new_in(parent).map_err(|e| {
                AppError::Storage(format!("Failed to create OAuth storage file: {}", e))
            })?;
            file.write_all(content.as_bytes())
                .and_then(|()| file.as_file().sync_all())
                .map_err(|e| AppError::Storage(format!("Failed to write OAuth storage: {}", e)))?;
            file.persist(path).map_err(|e| {
                AppError::Storage(format!("Failed to replace OAuth storage: {}", e.error))
            })?;
            // The blocking task owns the lock until both disk and cache are
            // committed, even if the caller is cancelled while saving.
            *cache = storage.credentials;
            Ok(())
        })
        .await
        .map_err(|e| AppError::Storage(format!("OAuth storage task failed: {}", e)))?
    }

    /// Store credentials for a provider
    pub async fn store_credentials(&self, credentials: &OAuthCredentials) -> AppResult<()> {
        // Serialize updates through the cache lock and publish only after a
        // successful save. A failed write must not report unpersisted tokens
        // as the current credentials.
        let cache = self.cache.clone().write_owned().await;
        let mut updated = cache.clone();
        updated.insert(credentials.provider_id.clone(), credentials.clone());
        self.save(cache, updated).await
    }

    /// Get credentials for a provider
    pub async fn get_credentials(&self, provider_id: &str) -> AppResult<Option<OAuthCredentials>> {
        Ok(self.cache.read().await.get(provider_id).cloned())
    }

    /// Delete credentials for a provider
    pub async fn delete_credentials(&self, provider_id: &str) -> AppResult<()> {
        let cache = self.cache.clone().write_owned().await;
        let mut updated = cache.clone();
        updated.remove(provider_id);
        self.save(cache, updated).await
    }

    /// List all providers with stored credentials
    pub async fn list_providers(&self) -> AppResult<Vec<String>> {
        Ok(self.cache.read().await.keys().cloned().collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;
    use tempfile::tempdir;

    fn credentials(provider_id: &str) -> OAuthCredentials {
        OAuthCredentials {
            provider_id: provider_id.to_owned(),
            access_token: format!("token-{provider_id}"),
            refresh_token: None,
            expires_at: None,
            account_id: None,
            created_at: Utc::now(),
        }
    }

    #[tokio::test]
    async fn concurrent_updates_persist_every_provider() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("oauth.json");
        let storage = Arc::new(OAuthStorage::new(path.clone()).await.unwrap());
        let mut tasks = Vec::new();
        for index in 0..24 {
            let storage = storage.clone();
            tasks.push(tokio::spawn(async move {
                storage
                    .store_credentials(&credentials(&format!("provider-{index}")))
                    .await
                    .unwrap();
            }));
        }
        for task in tasks {
            task.await.unwrap();
        }
        let loaded = OAuthStorage::new(path).await.unwrap();
        assert_eq!(loaded.list_providers().await.unwrap().len(), 24);
        for index in 0..24 {
            let id = format!("provider-{index}");
            assert_eq!(
                loaded
                    .get_credentials(&id)
                    .await
                    .unwrap()
                    .unwrap()
                    .access_token,
                format!("token-{id}")
            );
        }
    }

    #[tokio::test]
    async fn failed_updates_leave_cached_credentials_unchanged() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("oauth.json");
        let storage = OAuthStorage::new(path.clone()).await.unwrap();
        storage
            .store_credentials(&credentials("existing"))
            .await
            .unwrap();
        fs::remove_file(&path).await.unwrap();
        fs::create_dir(&path).await.unwrap();
        assert!(storage
            .store_credentials(&credentials("new"))
            .await
            .is_err());
        assert!(storage.get_credentials("new").await.unwrap().is_none());
        assert!(storage.delete_credentials("existing").await.is_err());
        assert!(storage.get_credentials("existing").await.unwrap().is_some());
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn saves_private_file_and_replaces_symlink_without_touching_target() {
        use std::os::unix::fs::{symlink, PermissionsExt};

        let dir = tempdir().unwrap();
        let path = dir.path().join("oauth.json");
        let target = dir.path().join("other-file");
        let storage = OAuthStorage::new(path.clone()).await.unwrap();
        fs::write(&target, "must stay unchanged").await.unwrap();
        symlink(&target, &path).unwrap();
        storage
            .store_credentials(&credentials("provider"))
            .await
            .unwrap();
        assert_eq!(
            fs::read_to_string(&target).await.unwrap(),
            "must stay unchanged"
        );
        assert!(!fs::symlink_metadata(&path)
            .await
            .unwrap()
            .file_type()
            .is_symlink());
        assert_eq!(
            fs::metadata(&path).await.unwrap().permissions().mode() & 0o777,
            0o600
        );
    }

    #[tokio::test]
    async fn test_storage_create_and_load() {
        let dir = tempdir().unwrap();
        let storage_path = dir.path().join("oauth.json");

        let storage = OAuthStorage::new(storage_path.clone()).await.unwrap();

        let creds = OAuthCredentials {
            provider_id: "test-provider".to_string(),
            access_token: "test-token".to_string(),
            refresh_token: Some("refresh-token".to_string()),
            expires_at: Some(Utc::now().timestamp() + 3600),
            account_id: Some("account-123".to_string()),
            created_at: Utc::now(),
        };

        storage.store_credentials(&creds).await.unwrap();

        // Create new storage instance to test loading
        let storage2 = OAuthStorage::new(storage_path).await.unwrap();
        let loaded = storage2.get_credentials("test-provider").await.unwrap();

        assert!(loaded.is_some());
        let loaded = loaded.unwrap();
        assert_eq!(loaded.provider_id, "test-provider");
        assert_eq!(loaded.access_token, "test-token");
    }

    #[tokio::test]
    async fn test_delete_credentials() {
        let dir = tempdir().unwrap();
        let storage_path = dir.path().join("oauth.json");

        let storage = OAuthStorage::new(storage_path).await.unwrap();

        let creds = OAuthCredentials {
            provider_id: "test-provider".to_string(),
            access_token: "test-token".to_string(),
            refresh_token: None,
            expires_at: None,
            account_id: None,
            created_at: Utc::now(),
        };

        storage.store_credentials(&creds).await.unwrap();
        assert!(storage
            .get_credentials("test-provider")
            .await
            .unwrap()
            .is_some());

        storage.delete_credentials("test-provider").await.unwrap();
        assert!(storage
            .get_credentials("test-provider")
            .await
            .unwrap()
            .is_none());
    }
}
