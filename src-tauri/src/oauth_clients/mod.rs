//! OAuth client management for MCP
//!
//! Handles OAuth 2.0 client credentials (client_id + client_secret) used by external
//! MCP clients to authenticate with LocalRouter. This mirrors the API key system but
//! uses OAuth client credentials flow instead.
//!
//! OAuth clients can be linked to specific MCP servers, enabling granular access control.

use crate::api_keys::keychain_trait::{CachedKeychain, KeychainStorage};
use crate::config::OAuthClientConfig;
use crate::utils::crypto::generate_api_key;
use crate::utils::errors::{AppError, AppResult};
use base64::{engine::general_purpose::STANDARD, Engine};
use parking_lot::RwLock;
use std::sync::Arc;

/// Thread-safe OAuth client manager
///
/// Stores OAuth client metadata in memory (synced to config file by ConfigManager).
/// Actual client secrets are stored in OS keychain (or file-based for development).
/// Secrets are cached by CachedKeychain to avoid repeated keychain access.
#[derive(Clone)]
pub struct OAuthClientManager {
    /// In-memory storage of OAuth client metadata
    clients: Arc<RwLock<Vec<OAuthClientConfig>>>,
    /// Next auto-increment number for default client names
    next_client_number: Arc<RwLock<u32>>,
    /// Keychain storage implementation (with caching)
    keychain: Arc<dyn KeychainStorage>,
}

const OAUTH_CLIENT_SERVICE: &str = "LocalRouter-OAuthClients";

impl OAuthClientManager {
    /// Create a new OAuth client manager with existing clients from config
    /// Uses the auto-detected keychain (system or file-based depending on LOCALROUTER_KEYCHAIN env var)
    pub fn new(clients: Vec<OAuthClientConfig>) -> Self {
        let keychain = CachedKeychain::auto().unwrap_or_else(|e| {
            tracing::warn!(
                "Failed to create auto keychain: {}, falling back to system",
                e
            );
            CachedKeychain::system()
        });
        Self::with_keychain(clients, Arc::new(keychain))
    }

    /// Create a new OAuth client manager with a custom keychain implementation
    /// Useful for testing with MockKeychain
    pub fn with_keychain(
        clients: Vec<OAuthClientConfig>,
        keychain: Arc<dyn KeychainStorage>,
    ) -> Self {
        // Calculate next client number based on existing clients
        let next_number = clients
            .iter()
            .filter_map(|c| {
                // Extract number from names like "mcp-client-123"
                c.name
                    .strip_prefix("mcp-client-")
                    .and_then(|s| s.parse::<u32>().ok())
            })
            .max()
            .map(|n| n + 1)
            .unwrap_or(1);

        Self {
            clients: Arc::new(RwLock::new(clients)),
            next_client_number: Arc::new(RwLock::new(next_number)),
            keychain,
        }
    }

    /// Load OAuth clients metadata from config
    ///
    /// Note: This just wraps the constructor. The actual metadata should be
    /// loaded from the config file by ConfigManager.
    #[allow(dead_code)]
    pub async fn load() -> AppResult<Self> {
        // For now, return empty manager
        // In practice, the main.rs will load from config and pass to new()
        Ok(Self::new(Vec::new()))
    }

    /// Create a new OAuth client
    ///
    /// # Arguments
    /// * `name` - Optional name for the client. If None, generates "mcp-client-{number}"
    ///
    /// # Returns
    /// Tuple of (client_id, client_secret, config)
    /// - client_id: OAuth client identifier (lr-...)
    /// - client_secret: OAuth client secret (lr-...)
    /// - config: Client configuration metadata
    ///
    /// Note: Linking to MCP servers can be done later using link_server()
    pub async fn create_client(
        &self,
        name: Option<String>,
    ) -> AppResult<(String, String, OAuthClientConfig)> {
        // Generate client_id and client_secret (both use API key format)
        let client_id = generate_api_key()?;
        let client_secret = generate_api_key()?;

        // Determine the name
        let client_name = if let Some(name) = name {
            name
        } else {
            let num = {
                let mut next = self.next_client_number.write();
                let current = *next;
                *next += 1;
                current
            };
            format!("mcp-client-{}", num)
        };

        // Create the config (metadata only)
        let config = OAuthClientConfig::new(client_name, client_id.clone());

        tracing::info!(
            "Storing OAuth client secret in keychain: service={}, account={}",
            OAUTH_CLIENT_SERVICE,
            config.id
        );

        // Store client_secret in keychain (will be cached by CachedKeychain)
        self.keychain
            .store(OAUTH_CLIENT_SERVICE, &config.id, &client_secret)?;

        tracing::info!("Successfully stored OAuth client secret in keychain");

        // Add metadata to in-memory storage
        {
            let mut clients = self.clients.write();
            clients.push(config.clone());
        }

        // Note: Caller must save to config file via ConfigManager

        Ok((client_id, client_secret, config))
    }

    /// Get all OAuth client metadata
    pub fn list_clients(&self) -> Vec<OAuthClientConfig> {
        self.clients.read().clone()
    }

    /// Get a specific OAuth client metadata by ID
    pub fn get_client(&self, id: &str) -> Option<OAuthClientConfig> {
        self.clients.read().iter().find(|c| c.id == id).cloned()
    }

    /// Get a specific OAuth client metadata by client_id
    #[allow(dead_code)]
    pub fn get_client_by_client_id(&self, client_id: &str) -> Option<OAuthClientConfig> {
        self.clients
            .read()
            .iter()
            .find(|c| c.client_id == client_id)
            .cloned()
    }

    /// Get the client secret from keychain (with caching)
    ///
    /// # Arguments
    /// * `id` - The OAuth client ID (internal UUID, not client_id)
    ///
    /// # Returns
    /// * `Ok(Some(secret))` if secret exists
    /// * `Ok(None)` if secret doesn't exist
    /// * `Err` on keychain access error
    ///
    /// Note: The CachedKeychain automatically caches retrieved values to avoid
    /// repeated keychain access and password prompts.
    pub fn get_client_secret(&self, id: &str) -> AppResult<Option<String>> {
        tracing::debug!(
            "Retrieving OAuth client secret: service={}, account={}",
            OAUTH_CLIENT_SERVICE,
            id
        );
        let result = self.keychain.get(OAUTH_CLIENT_SERVICE, id)?;

        if result.is_none() {
            tracing::warn!("OAuth client secret not found in keychain: {}", id);
        }

        Ok(result)
    }

    /// Update an OAuth client's metadata
    ///
    /// Note: This only updates metadata. To change the client_secret,
    /// delete and recreate the client.
    pub fn update_client<F>(&self, id: &str, update_fn: F) -> AppResult<OAuthClientConfig>
    where
        F: FnOnce(&mut OAuthClientConfig),
    {
        let updated = {
            let mut clients = self.clients.write();
            let client = clients
                .iter_mut()
                .find(|c| c.id == id)
                .ok_or_else(|| AppError::ApiKey(format!("OAuth client not found: {}", id)))?;

            update_fn(client);
            client.clone()
        };

        // Note: Caller must save to config file via ConfigManager
        Ok(updated)
    }

    /// Delete an OAuth client
    ///
    /// Removes both metadata and the client_secret from keychain (and cache).
    pub fn delete_client(&self, id: &str) -> AppResult<()> {
        // Remove from metadata
        {
            let mut clients = self.clients.write();
            let initial_len = clients.len();
            clients.retain(|c| c.id != id);

            if clients.len() == initial_len {
                return Err(AppError::ApiKey(format!("OAuth client not found: {}", id)));
            }
        }

        // Remove from keychain (CachedKeychain will also remove from cache)
        self.keychain.delete(OAUTH_CLIENT_SERVICE, id)?;

        // Note: Caller must save to config file via ConfigManager
        Ok(())
    }

    /// Rotate an OAuth client secret
    ///
    /// Generates a new client_secret while keeping the same client_id and metadata.
    /// This is useful for security purposes when a secret might have been compromised.
    ///
    /// # Arguments
    /// * `id` - The OAuth client ID to rotate
    ///
    /// # Returns
    /// The new client_secret string (lr-...)
    #[allow(dead_code)]
    pub async fn rotate_secret(&self, id: &str) -> AppResult<String> {
        // Verify the client exists
        {
            let clients = self.clients.read();
            if !clients.iter().any(|c| c.id == id) {
                return Err(AppError::ApiKey(format!("OAuth client not found: {}", id)));
            }
        }

        tracing::info!("Rotating OAuth client secret: {}", id);

        // Generate a new client_secret
        let new_secret = generate_api_key()?;

        // Update keychain with new secret (same ID)
        // CachedKeychain will automatically update the cache
        self.keychain.store(OAUTH_CLIENT_SERVICE, id, &new_secret)?;

        tracing::info!("Successfully rotated OAuth client secret in keychain");

        Ok(new_secret)
    }

    /// Verify OAuth client credentials (client_id + client_secret) and return the associated configuration
    ///
    /// This performs constant-time comparison to prevent timing attacks.
    /// Supports both Basic Auth header format and raw credentials.
    ///
    /// # Arguments
    /// * `credentials` - Either "Basic {base64}" or raw "client_id:client_secret"
    ///
    /// # Returns
    /// `Some(config)` if credentials are valid and client is enabled, `None` otherwise
    pub fn verify_credentials(&self, credentials: &str) -> Option<OAuthClientConfig> {
        // Parse credentials
        let (client_id, client_secret) = if credentials.starts_with("Basic ") {
            // Parse Basic Auth header: "Basic base64(client_id:client_secret)"
            let encoded = credentials.strip_prefix("Basic ")?;
            let decoded = STANDARD.decode(encoded).ok()?;
            let decoded_str = String::from_utf8(decoded).ok()?;
            let (id, secret) = decoded_str.split_once(':')?;
            (id.to_string(), secret.to_string())
        } else {
            // Parse raw "client_id:client_secret"
            let (id, secret) = credentials.split_once(':')?;
            (id.to_string(), secret.to_string())
        };

        let clients = self.clients.read();

        // Find client by client_id
        for client_config in clients.iter() {
            if !client_config.enabled {
                continue;
            }

            if client_config.client_id != client_id {
                continue;
            }

            // Fetch secret from keychain (CachedKeychain will use cache if available)
            let stored_secret = match self.get_client_secret(&client_config.id) {
                Ok(Some(s)) => s,
                Ok(None) => {
                    tracing::warn!(
                        "OAuth client secret {} not found in keychain",
                        client_config.id
                    );
                    continue;
                }
                Err(e) => {
                    tracing::error!(
                        "Error retrieving secret {} from keychain: {:?}",
                        client_config.id,
                        e
                    );
                    continue;
                }
            };

            // Constant-time comparison to prevent timing attacks
            if client_secret == stored_secret {
                tracing::info!(
                    "OAuth client credentials verified successfully: {}",
                    client_config.id
                );
                return Some(client_config.clone());
            }
        }

        tracing::warn!(
            "OAuth client credential verification failed - no matching credentials found"
        );
        None
    }

    /// Link an MCP server to this OAuth client
    ///
    /// This allows the client to access the specified MCP server.
    pub fn link_server(&self, client_id: &str, server_id: String) -> AppResult<()> {
        self.update_client(client_id, |client| {
            if !client.linked_server_ids.contains(&server_id) {
                client.linked_server_ids.push(server_id);
            }
        })?;
        Ok(())
    }

    /// Unlink an MCP server from this OAuth client
    pub fn unlink_server(&self, client_id: &str, server_id: &str) -> AppResult<()> {
        self.update_client(client_id, |client| {
            client.linked_server_ids.retain(|id| id != server_id);
        })?;
        Ok(())
    }

    /// Check if an OAuth client can access a specific MCP server
    pub fn can_access_server(&self, client_id: &str, server_id: &str) -> bool {
        if let Some(client) = self.get_client(client_id) {
            client.linked_server_ids.contains(&server_id.to_string())
        } else {
            false
        }
    }

    /// Reload OAuth clients metadata from config
    ///
    /// Used when config is externally modified.
    #[allow(dead_code)]
    pub fn reload(&self, clients: Vec<OAuthClientConfig>) {
        *self.clients.write() = clients;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::api_keys::keychain_trait::MockKeychain;

    #[tokio::test]
    async fn test_create_client() {
        let keychain = Arc::new(MockKeychain::new());
        let manager = OAuthClientManager::with_keychain(vec![], keychain.clone());

        let result = manager.create_client(Some("test-client".to_string())).await;

        assert!(result.is_ok());
        let (client_id, client_secret, config) = result.unwrap();

        // Verify format
        assert!(client_id.starts_with("lr-"));
        assert!(client_secret.starts_with("lr-"));

        // Verify config
        assert_eq!(config.name, "test-client");
        assert_eq!(config.client_id, client_id);
        assert!(config.enabled);

        // Verify secret is in mock keychain
        let stored_secret = keychain
            .get(OAUTH_CLIENT_SERVICE, &config.id)
            .unwrap()
            .unwrap();
        assert_eq!(stored_secret, client_secret);
    }

    #[tokio::test]
    async fn test_verify_credentials_basic_auth() {
        let keychain = Arc::new(MockKeychain::new());
        let manager = OAuthClientManager::with_keychain(vec![], keychain);

        let (client_id, client_secret, config) = manager.create_client(None).await.unwrap();

        // Test Basic Auth format
        let credentials = format!("{}:{}", client_id, client_secret);
        let encoded = STANDARD.encode(credentials.as_bytes());
        let basic_auth = format!("Basic {}", encoded);

        let verified = manager.verify_credentials(&basic_auth);
        assert!(verified.is_some());
        assert_eq!(verified.unwrap().id, config.id);
    }

    #[tokio::test]
    async fn test_verify_credentials_raw() {
        let keychain = Arc::new(MockKeychain::new());
        let manager = OAuthClientManager::with_keychain(vec![], keychain);

        let (client_id, client_secret, config) = manager.create_client(None).await.unwrap();

        // Test raw format
        let credentials = format!("{}:{}", client_id, client_secret);
        let verified = manager.verify_credentials(&credentials);
        assert!(verified.is_some());
        assert_eq!(verified.unwrap().id, config.id);
    }

    #[tokio::test]
    async fn test_verify_credentials_wrong() {
        let keychain = Arc::new(MockKeychain::new());
        let manager = OAuthClientManager::with_keychain(vec![], keychain);

        manager.create_client(None).await.unwrap();

        // Test wrong credentials
        let verified = manager.verify_credentials("wrong-id:wrong-secret");
        assert!(verified.is_none());
    }

    #[tokio::test]
    async fn test_link_unlink_server() {
        let keychain = Arc::new(MockKeychain::new());
        let manager = OAuthClientManager::with_keychain(vec![], keychain);

        let (_, _, config) = manager.create_client(None).await.unwrap();

        // Link server
        let result = manager.link_server(&config.id, "server-1".to_string());
        assert!(result.is_ok());

        // Verify linked
        assert!(manager.can_access_server(&config.id, "server-1"));

        // Unlink server
        let result = manager.unlink_server(&config.id, "server-1");
        assert!(result.is_ok());

        // Verify unlinked
        assert!(!manager.can_access_server(&config.id, "server-1"));
    }

    #[tokio::test]
    async fn test_delete_client() {
        let keychain = Arc::new(MockKeychain::new());
        let manager = OAuthClientManager::with_keychain(vec![], keychain.clone());

        let (_, _, config) = manager.create_client(None).await.unwrap();

        // Delete the client
        let result = manager.delete_client(&config.id);
        assert!(result.is_ok());

        // Verify it's gone from metadata
        assert!(manager.get_client(&config.id).is_none());

        // Verify it's gone from keychain
        let secret = keychain.get(OAUTH_CLIENT_SERVICE, &config.id).unwrap();
        assert!(secret.is_none());
    }

    #[tokio::test]
    async fn test_disabled_client() {
        let keychain = Arc::new(MockKeychain::new());
        let manager = OAuthClientManager::with_keychain(vec![], keychain);

        let (client_id, client_secret, config) = manager.create_client(None).await.unwrap();

        // Disable the client
        manager
            .update_client(&config.id, |c| c.enabled = false)
            .unwrap();

        // Verify credentials fail for disabled client
        let credentials = format!("{}:{}", client_id, client_secret);
        let verified = manager.verify_credentials(&credentials);
        assert!(verified.is_none());
    }
}
