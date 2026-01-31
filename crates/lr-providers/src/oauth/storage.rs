//! OAuth credentials storage
//!
//! Stores OAuth credentials securely in a JSON file with proper permissions.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;
use tokio::fs;
use tokio::sync::RwLock;

use super::OAuthCredentials;
use lr_types::{AppError, AppResult};

/// OAuth credentials storage
pub struct OAuthStorage {
    /// Path to the credentials file
    storage_path: PathBuf,
    /// In-memory cache of credentials
    cache: RwLock<HashMap<String, OAuthCredentials>>,
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
            cache: RwLock::new(HashMap::new()),
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
    async fn save(&self) -> AppResult<()> {
        // Ensure parent directory exists
        if let Some(parent) = self.storage_path.parent() {
            fs::create_dir_all(parent).await.map_err(|e| {
                AppError::Storage(format!("Failed to create storage directory: {}", e))
            })?;
        }

        let cache = self.cache.read().await;
        let storage = StorageFormat {
            credentials: cache.clone(),
        };

        let content = serde_json::to_string_pretty(&storage)
            .map_err(|e| AppError::Storage(format!("Failed to serialize OAuth storage: {}", e)))?;

        fs::write(&self.storage_path, content)
            .await
            .map_err(|e| AppError::Storage(format!("Failed to write OAuth storage: {}", e)))?;

        // Set file permissions to 0600 (owner read/write only) on Unix
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mut perms = fs::metadata(&self.storage_path)
                .await
                .map_err(|e| AppError::Storage(format!("Failed to get file metadata: {}", e)))?
                .permissions();
            perms.set_mode(0o600);
            fs::set_permissions(&self.storage_path, perms)
                .await
                .map_err(|e| AppError::Storage(format!("Failed to set file permissions: {}", e)))?;
        }

        // Note: On Windows, file permissions work differently (ACLs).
        // Windows files are already restricted to the current user by default
        // when created in the user's APPDATA directory, so no additional
        // permission setting is needed. The keychain is the primary secure
        // storage method; this file-based storage is a fallback.

        Ok(())
    }

    /// Store credentials for a provider
    pub async fn store_credentials(&self, credentials: &OAuthCredentials) -> AppResult<()> {
        self.cache
            .write()
            .await
            .insert(credentials.provider_id.clone(), credentials.clone());

        self.save().await
    }

    /// Get credentials for a provider
    pub async fn get_credentials(&self, provider_id: &str) -> AppResult<Option<OAuthCredentials>> {
        Ok(self.cache.read().await.get(provider_id).cloned())
    }

    /// Delete credentials for a provider
    pub async fn delete_credentials(&self, provider_id: &str) -> AppResult<()> {
        self.cache.write().await.remove(provider_id);
        self.save().await
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
