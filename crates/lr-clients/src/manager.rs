//! Unified client management for both LLM and MCP access
//!
//! This module provides a unified client system that replaces the separate
//! API key and OAuth client systems. Each client has:
//! - A unique client_id (visible, stored in config)
//! - A secret (stored securely in keychain)
//! - Access control for LLM providers and MCP servers
//! - Support for two authentication methods:
//!   1. Direct bearer token (using the secret directly)
//!   2. OAuth client credentials flow (generates short-lived tokens)

#![allow(dead_code)]

use lr_api_keys::keychain_trait::KeychainStorage;
use lr_api_keys::CachedKeychain;
use lr_config::{Client, PermissionState};
use lr_types::{AppError, AppResult};
use lr_utils::crypto;
use parking_lot::RwLock;
use std::sync::Arc;

const CLIENT_SERVICE: &str = "LocalRouter-Clients";

/// Manages unified client configurations and authentication
pub struct ClientManager {
    /// In-memory storage of client metadata
    clients: Arc<RwLock<Vec<Client>>>,
    /// Keychain storage for client secrets
    keychain: Arc<dyn KeychainStorage>,
}

impl ClientManager {
    /// Create a new client manager with existing clients from config
    /// Uses the auto-detected keychain (system or file-based depending on LOCALROUTER_KEYCHAIN env var)
    pub fn new(clients: Vec<Client>) -> Self {
        let keychain = CachedKeychain::auto().unwrap_or_else(|e| {
            tracing::warn!(
                "Failed to create auto keychain: {}, falling back to system",
                e
            );
            CachedKeychain::system()
        });
        Self::with_keychain(clients, Arc::new(keychain))
    }

    /// Create a new client manager with a specific keychain implementation
    /// Used for testing with mock keychain
    pub fn with_keychain(clients: Vec<Client>, keychain: Arc<dyn KeychainStorage>) -> Self {
        Self {
            clients: Arc::new(RwLock::new(clients)),
            keychain,
        }
    }

    /// Create a new client with auto-generated client_id and secret
    ///
    /// # Arguments
    /// * `name` - Human-readable name for the client
    /// * `strategy_id` - The ID of the strategy this client should use
    ///
    /// # Returns
    /// Returns (client_id, secret, client_config) tuple
    /// The secret is also stored in the keychain automatically
    pub fn create_client(
        &self,
        name: String,
        strategy_id: String,
    ) -> AppResult<(String, String, Client)> {
        // Generate secret (same format as API keys)
        let secret = crypto::generate_api_key()
            .map_err(|e| AppError::Config(format!("Failed to generate client secret: {}", e)))?;

        // Create client config
        let client = Client::new_with_strategy(name, strategy_id);

        // Store secret in keychain
        self.keychain
            .store(CLIENT_SERVICE, &client.id, &secret)
            .map_err(|e| {
                AppError::Config(format!("Failed to store client secret in keychain: {}", e))
            })?;

        // Add to in-memory storage
        self.clients.write().push(client.clone());

        Ok((client.id.clone(), secret, client))
    }

    /// Add an existing client and generate a secret for it
    ///
    /// # Arguments
    /// * `client` - The client configuration to add
    ///
    /// # Returns
    /// Returns the generated secret
    /// The secret is also stored in the keychain automatically
    pub fn add_client_with_secret(&self, client: Client) -> AppResult<String> {
        // Generate secret (same format as API keys)
        let secret = crypto::generate_api_key()
            .map_err(|e| AppError::Config(format!("Failed to generate client secret: {}", e)))?;

        // Store secret in keychain
        self.keychain
            .store(CLIENT_SERVICE, &client.id, &secret)
            .map_err(|e| {
                AppError::Config(format!("Failed to store client secret in keychain: {}", e))
            })?;

        // Add to in-memory storage only if not already present
        // (sync_clients may have already added it via ConfigManager callback)
        let mut clients = self.clients.write();
        if !clients.iter().any(|c| c.id == client.id) {
            clients.push(client);
        }

        Ok(secret)
    }

    /// Delete a client and remove its secret from keychain
    pub fn delete_client(&self, client_id: &str) -> AppResult<()> {
        let mut clients = self.clients.write();

        // Find the client
        let client = clients
            .iter()
            .find(|c| c.id == client_id)
            .ok_or_else(|| AppError::Config(format!("Client not found: {}", client_id)))?;

        let id = client.id.clone();

        // Remove from in-memory storage
        clients.retain(|c| c.id != client_id);

        // Delete from keychain
        self.keychain.delete(CLIENT_SERVICE, &id).map_err(|e| {
            AppError::Config(format!(
                "Failed to delete client secret from keychain: {}",
                e
            ))
        })?;

        Ok(())
    }

    /// Verify client credentials using OAuth client credentials flow
    /// Returns the client config if credentials are valid and client is enabled
    ///
    /// # Arguments
    /// * `client_id` - The client_id to verify
    /// * `client_secret` - The client secret to verify
    pub fn verify_credentials(
        &self,
        client_id: &str,
        client_secret: &str,
    ) -> AppResult<Option<Client>> {
        let clients = self.clients.read();

        // Find client by client_id
        let client = match clients.iter().find(|c| c.id == client_id) {
            Some(c) => c,
            None => return Ok(None),
        };

        // Check if client is enabled
        if !client.enabled {
            return Ok(None);
        }

        // Verify secret from keychain
        let stored_secret = self.keychain.get(CLIENT_SERVICE, &client.id).map_err(|e| {
            AppError::Config(format!(
                "Failed to retrieve client secret from keychain: {}",
                e
            ))
        })?;

        match stored_secret {
            Some(secret) if secret == client_secret => {
                // Mark client as used
                drop(clients);
                let mut clients = self.clients.write();
                if let Some(client) = clients.iter_mut().find(|c| c.id == client_id) {
                    client.mark_used();
                    Ok(Some(client.clone()))
                } else {
                    Ok(None)
                }
            }
            _ => Ok(None),
        }
    }

    /// Get the secret for a client by internal ID
    ///
    /// # Arguments
    /// * `id` - The internal client ID (not client_id)
    ///
    /// # Returns
    /// * `Ok(Some(secret))` if secret exists
    /// * `Ok(None)` if secret doesn't exist
    /// * `Err` on keychain access error
    pub fn get_secret(&self, id: &str) -> AppResult<Option<String>> {
        tracing::debug!(
            "Retrieving client secret: service={}, account={}",
            CLIENT_SERVICE,
            id
        );
        let result = self.keychain.get(CLIENT_SERVICE, id)?;

        if result.is_none() {
            tracing::warn!("Client secret not found in keychain: {}", id);
        }

        Ok(result)
    }

    /// Verify client secret for direct bearer token authentication
    /// Returns the client config if secret is valid and client is enabled
    ///
    /// # Arguments
    /// * `secret` - The client secret to verify
    pub fn verify_secret(&self, secret: &str) -> AppResult<Option<Client>> {
        let clients = self.clients.read();

        // Try to find a client with matching secret
        // We need to check all clients since we only have the secret, not the client_id
        let mut found_client_id: Option<String> = None;
        for client in clients.iter() {
            if !client.enabled {
                continue;
            }

            let stored_secret = self.keychain.get(CLIENT_SERVICE, &client.id).map_err(|e| {
                AppError::Config(format!(
                    "Failed to retrieve client secret from keychain: {}",
                    e
                ))
            })?;

            if let Some(s) = stored_secret {
                if s == secret {
                    found_client_id = Some(client.id.clone());
                    break;
                }
            }
        }

        // Drop read lock before acquiring write lock
        drop(clients);

        // If we found a matching client, mark it as used
        if let Some(client_id) = found_client_id {
            let mut clients = self.clients.write();
            if let Some(client) = clients.iter_mut().find(|c| c.id == client_id) {
                client.mark_used();
                return Ok(Some(client.clone()));
            }
        }

        Ok(None)
    }

    /// Check if a client can access a specific LLM provider
    pub fn can_access_llm(&self, client_id: &str, provider_name: &str) -> bool {
        let clients = self.clients.read();

        clients
            .iter()
            .find(|c| c.id == client_id && c.enabled)
            .map(|c| {
                c.allowed_llm_providers.is_empty()
                    || c.allowed_llm_providers.contains(&provider_name.to_string())
            })
            .unwrap_or(false)
    }

    /// Check if a client can access a specific MCP server
    pub fn can_access_mcp_server(&self, client_id: &str, server_id: &str) -> bool {
        let clients = self.clients.read();

        clients
            .iter()
            .find(|c| c.id == client_id && c.enabled)
            .map(|c| c.mcp_permissions.resolve_server(server_id).is_enabled())
            .unwrap_or(false)
    }

    /// Add an LLM provider to a client's allowed list
    pub fn add_llm_provider(&self, client_id: &str, provider_name: &str) -> AppResult<()> {
        let mut clients = self.clients.write();

        let client = clients
            .iter_mut()
            .find(|c| c.id == client_id)
            .ok_or_else(|| AppError::Config(format!("Client not found: {}", client_id)))?;

        if !client
            .allowed_llm_providers
            .contains(&provider_name.to_string())
        {
            client.allowed_llm_providers.push(provider_name.to_string());
        }

        Ok(())
    }

    /// Remove an LLM provider from a client's allowed list
    pub fn remove_llm_provider(&self, client_id: &str, provider_name: &str) -> AppResult<()> {
        let mut clients = self.clients.write();

        let client = clients
            .iter_mut()
            .find(|c| c.id == client_id)
            .ok_or_else(|| AppError::Config(format!("Client not found: {}", client_id)))?;

        client.allowed_llm_providers.retain(|p| p != provider_name);

        Ok(())
    }

    /// Add an MCP server permission (sets to Allow)
    /// Uses the new mcp_permissions system
    pub fn add_mcp_server(&self, client_id: &str, server_id: &str) -> AppResult<()> {
        let mut clients = self.clients.write();

        let client = clients
            .iter_mut()
            .find(|c| c.id == client_id)
            .ok_or_else(|| AppError::Config(format!("Client not found: {}", client_id)))?;

        // Set server permission to Allow using the new permission system
        client
            .mcp_permissions
            .servers
            .insert(server_id.to_string(), PermissionState::Allow);

        Ok(())
    }

    /// Remove an MCP server permission (sets to Off)
    /// Uses the new mcp_permissions system
    pub fn remove_mcp_server(&self, client_id: &str, server_id: &str) -> AppResult<()> {
        let mut clients = self.clients.write();

        let client = clients
            .iter_mut()
            .find(|c| c.id == client_id)
            .ok_or_else(|| AppError::Config(format!("Client not found: {}", client_id)))?;

        // Set server permission to Off using the new permission system
        client
            .mcp_permissions
            .servers
            .insert(server_id.to_string(), PermissionState::Off);

        Ok(())
    }

    /// Check if a client has MCP server access
    /// Uses the new mcp_permissions system
    pub fn has_mcp_server_access(&self, client_id: &str, server_id: &str) -> bool {
        let clients = self.clients.read();
        clients
            .iter()
            .find(|c| c.id == client_id)
            .map(|c| c.mcp_permissions.resolve_server(server_id).is_enabled())
            .unwrap_or(false)
    }

    /// Set MCP deferred loading for a client
    pub fn set_mcp_deferred_loading(&self, client_id: &str, enabled: bool) -> AppResult<()> {
        let mut clients = self.clients.write();

        let client = clients
            .iter_mut()
            .find(|c| c.id == client_id)
            .ok_or_else(|| AppError::Config(format!("Client not found: {}", client_id)))?;

        client.mcp_deferred_loading = enabled;

        Ok(())
    }

    /// Enable a client
    pub fn enable_client(&self, client_id: &str) -> AppResult<()> {
        let mut clients = self.clients.write();

        let client = clients
            .iter_mut()
            .find(|c| c.id == client_id)
            .ok_or_else(|| AppError::Config(format!("Client not found: {}", client_id)))?;

        client.enabled = true;

        Ok(())
    }

    /// Disable a client
    pub fn disable_client(&self, client_id: &str) -> AppResult<()> {
        let mut clients = self.clients.write();

        let client = clients
            .iter_mut()
            .find(|c| c.id == client_id)
            .ok_or_else(|| AppError::Config(format!("Client not found: {}", client_id)))?;

        client.enabled = false;

        Ok(())
    }

    /// Set the strategy for a client
    pub fn set_client_strategy(&self, client_id: &str, strategy_id: &str) -> AppResult<()> {
        let mut clients = self.clients.write();

        let client = clients
            .iter_mut()
            .find(|c| c.id == client_id)
            .ok_or_else(|| AppError::Config(format!("Client not found: {}", client_id)))?;

        client.strategy_id = strategy_id.to_string();

        Ok(())
    }

    /// Get a client by client_id
    pub fn get_client(&self, client_id: &str) -> Option<Client> {
        let clients = self.clients.read();
        clients.iter().find(|c| c.id == client_id).cloned()
    }

    /// Get a client by internal id
    pub fn get_client_by_id(&self, id: &str) -> Option<Client> {
        let clients = self.clients.read();
        clients.iter().find(|c| c.id == id).cloned()
    }

    /// List all clients
    pub fn list_clients(&self) -> Vec<Client> {
        let clients = self.clients.read();
        clients.clone()
    }

    /// Update a client's name and enabled status
    pub fn update_client(
        &self,
        client_id: &str,
        name: Option<String>,
        enabled: Option<bool>,
    ) -> AppResult<()> {
        let mut clients = self.clients.write();

        let client = clients
            .iter_mut()
            .find(|c| c.id == client_id)
            .ok_or_else(|| AppError::Config(format!("Client not found: {}", client_id)))?;

        if let Some(new_name) = name {
            client.name = new_name;
        }

        if let Some(new_enabled) = enabled {
            client.enabled = new_enabled;
        }

        Ok(())
    }

    /// Get all clients from the internal storage (returns a copy)
    pub fn get_all_clients(&self) -> Vec<Client> {
        self.list_clients()
    }

    /// Get the current client configs for saving to disk
    pub fn get_configs(&self) -> Vec<Client> {
        self.clients.read().clone()
    }

    /// Sync in-memory clients from an external source (e.g., ConfigManager)
    ///
    /// This updates the client metadata while preserving keychain secrets.
    /// Use this after config changes to keep ClientManager in sync.
    pub fn sync_clients(&self, clients: Vec<Client>) {
        *self.clients.write() = clients;
    }

    /// Rotate the secret for a client
    ///
    /// # Arguments
    /// * `client_id` - The client ID whose secret should be rotated
    ///
    /// # Returns
    /// The new secret string
    pub fn rotate_secret(&self, client_id: &str) -> AppResult<String> {
        // Verify the client exists
        {
            let clients = self.clients.read();
            if !clients.iter().any(|c| c.id == client_id) {
                return Err(AppError::Config(format!("Client not found: {}", client_id)));
            }
        }

        tracing::info!("Rotating client secret: {}", client_id);

        // Generate a new secret
        let new_secret = crypto::generate_api_key()
            .map_err(|e| AppError::Config(format!("Failed to generate client secret: {}", e)))?;

        // Update keychain with new secret (same ID)
        self.keychain
            .store(CLIENT_SERVICE, client_id, &new_secret)
            .map_err(|e| {
                AppError::Config(format!("Failed to store rotated secret in keychain: {}", e))
            })?;

        tracing::info!("Successfully rotated client secret in keychain");

        Ok(new_secret)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use lr_api_keys::keychain_trait::KeychainStorage;
    use std::collections::HashMap;
    use std::sync::Mutex;

    /// Mock keychain for testing
    struct MockKeychain {
        storage: Mutex<HashMap<String, HashMap<String, String>>>,
    }

    impl MockKeychain {
        fn new() -> Self {
            Self {
                storage: Mutex::new(HashMap::new()),
            }
        }
    }

    impl KeychainStorage for MockKeychain {
        fn store(&self, service: &str, account: &str, secret: &str) -> AppResult<()> {
            let mut storage = self.storage.lock().unwrap();
            storage
                .entry(service.to_string())
                .or_default()
                .insert(account.to_string(), secret.to_string());
            Ok(())
        }

        fn get(&self, service: &str, account: &str) -> AppResult<Option<String>> {
            let storage = self.storage.lock().unwrap();
            Ok(storage
                .get(service)
                .and_then(|accounts| accounts.get(account))
                .cloned())
        }

        fn delete(&self, service: &str, account: &str) -> AppResult<()> {
            let mut storage = self.storage.lock().unwrap();
            if let Some(accounts) = storage.get_mut(service) {
                accounts.remove(account);
            }
            Ok(())
        }
    }

    #[test]
    fn test_create_client() {
        let mock_keychain = Arc::new(MockKeychain::new());
        let manager = ClientManager::with_keychain(vec![], mock_keychain.clone());

        let (client_id, secret, config) = manager
            .create_client("Test Client".to_string(), "default".to_string())
            .expect("Failed to create client");

        // Verify client_id is a valid UUID
        assert!(uuid::Uuid::parse_str(&client_id).is_ok());
        assert_eq!(config.id, client_id);
        assert_eq!(config.name, "Test Client");

        // Verify secret format (API key format)
        assert!(secret.starts_with("lr-"));

        // Verify secret is stored in keychain
        let stored_secret = mock_keychain
            .get(CLIENT_SERVICE, &config.id)
            .expect("Failed to get secret");
        assert_eq!(stored_secret, Some(secret));

        // Verify client is in manager
        assert_eq!(manager.list_clients().len(), 1);
    }

    #[test]
    fn test_verify_credentials_valid() {
        let mock_keychain = Arc::new(MockKeychain::new());
        let manager = ClientManager::with_keychain(vec![], mock_keychain.clone());

        let (client_id, secret, _) = manager
            .create_client("Test Client".to_string(), "default".to_string())
            .expect("Failed to create client");

        // Verify with correct credentials
        let result = manager
            .verify_credentials(&client_id, &secret)
            .expect("Failed to verify");
        assert!(result.is_some());
        let client = result.unwrap();
        assert_eq!(client.id, client_id);
    }

    #[test]
    fn test_verify_credentials_invalid() {
        let mock_keychain = Arc::new(MockKeychain::new());
        let manager = ClientManager::with_keychain(vec![], mock_keychain.clone());

        let (client_id, _, _) = manager
            .create_client("Test Client".to_string(), "default".to_string())
            .expect("Failed to create client");

        // Verify with wrong secret
        let result = manager
            .verify_credentials(&client_id, "wrong-secret")
            .expect("Failed to verify");
        assert!(result.is_none());

        // Verify with wrong client_id
        let result = manager
            .verify_credentials("lr-wrong-id", "secret")
            .expect("Failed to verify");
        assert!(result.is_none());
    }

    #[test]
    fn test_verify_secret() {
        let mock_keychain = Arc::new(MockKeychain::new());
        let manager = ClientManager::with_keychain(vec![], mock_keychain.clone());

        let (_, secret, config) = manager
            .create_client("Test Client".to_string(), "default".to_string())
            .expect("Failed to create client");

        // Verify with correct secret
        let result = manager.verify_secret(&secret).expect("Failed to verify");
        assert!(result.is_some());
        let client = result.unwrap();
        assert_eq!(client.id, config.id);

        // Verify with wrong secret
        let result = manager
            .verify_secret("wrong-secret")
            .expect("Failed to verify");
        assert!(result.is_none());
    }

    #[test]
    fn test_verify_disabled_client() {
        let mock_keychain = Arc::new(MockKeychain::new());
        let manager = ClientManager::with_keychain(vec![], mock_keychain.clone());

        let (client_id, secret, _) = manager
            .create_client("Test Client".to_string(), "default".to_string())
            .expect("Failed to create client");

        // Disable client
        manager
            .disable_client(&client_id)
            .expect("Failed to disable");

        // Verify credentials fail for disabled client
        let result = manager
            .verify_credentials(&client_id, &secret)
            .expect("Failed to verify");
        assert!(result.is_none());

        // Verify secret fails for disabled client
        let result = manager.verify_secret(&secret).expect("Failed to verify");
        assert!(result.is_none());
    }

    #[test]
    fn test_delete_client() {
        let mock_keychain = Arc::new(MockKeychain::new());
        let manager = ClientManager::with_keychain(vec![], mock_keychain.clone());

        let (client_id, _, config) = manager
            .create_client("Test Client".to_string(), "default".to_string())
            .expect("Failed to create client");

        // Delete client
        manager.delete_client(&client_id).expect("Failed to delete");

        // Verify client is removed
        assert_eq!(manager.list_clients().len(), 0);

        // Verify secret is removed from keychain
        let stored_secret = mock_keychain
            .get(CLIENT_SERVICE, &config.id)
            .expect("Failed to get secret");
        assert!(stored_secret.is_none());
    }

    #[test]
    fn test_access_control_llm() {
        let mock_keychain = Arc::new(MockKeychain::new());
        let manager = ClientManager::with_keychain(vec![], mock_keychain.clone());

        let (client_id, _, _) = manager
            .create_client("Test Client".to_string(), "default".to_string())
            .expect("Failed to create client");

        // Empty allowed list means access to all
        assert!(manager.can_access_llm(&client_id, "openai"));
        assert!(manager.can_access_llm(&client_id, "anthropic"));

        // Add specific provider
        manager
            .add_llm_provider(&client_id, "openai")
            .expect("Failed to add provider");

        // Now only openai is allowed
        assert!(manager.can_access_llm(&client_id, "openai"));
        assert!(!manager.can_access_llm(&client_id, "anthropic"));

        // Remove provider
        manager
            .remove_llm_provider(&client_id, "openai")
            .expect("Failed to remove provider");

        // Empty list again means all allowed
        assert!(manager.can_access_llm(&client_id, "openai"));
        assert!(manager.can_access_llm(&client_id, "anthropic"));
    }

    #[test]
    fn test_access_control_mcp() {
        let mock_keychain = Arc::new(MockKeychain::new());
        let manager = ClientManager::with_keychain(vec![], mock_keychain.clone());

        let (client_id, _, _) = manager
            .create_client("Test Client".to_string(), "default".to_string())
            .expect("Failed to create client");

        // Default is McpServerAccess::None - no access to any MCP servers
        assert!(!manager.can_access_mcp_server(&client_id, "server1"));
        assert!(!manager.can_access_mcp_server(&client_id, "server2"));

        // Add specific server
        manager
            .add_mcp_server(&client_id, "server1")
            .expect("Failed to add server");

        // Now only server1 is allowed
        assert!(manager.can_access_mcp_server(&client_id, "server1"));
        assert!(!manager.can_access_mcp_server(&client_id, "server2"));

        // Remove server
        manager
            .remove_mcp_server(&client_id, "server1")
            .expect("Failed to remove server");

        // Empty list returns to None - no access
        assert!(!manager.can_access_mcp_server(&client_id, "server1"));
        assert!(!manager.can_access_mcp_server(&client_id, "server2"));
    }

    #[test]
    fn test_enable_disable() {
        let mock_keychain = Arc::new(MockKeychain::new());
        let manager = ClientManager::with_keychain(vec![], mock_keychain.clone());

        let (client_id, _, _) = manager
            .create_client("Test Client".to_string(), "default".to_string())
            .expect("Failed to create client");

        // Client is enabled by default
        let client = manager.get_client(&client_id).unwrap();
        assert!(client.enabled);

        // Disable
        manager
            .disable_client(&client_id)
            .expect("Failed to disable");
        let client = manager.get_client(&client_id).unwrap();
        assert!(!client.enabled);

        // Enable
        manager.enable_client(&client_id).expect("Failed to enable");
        let client = manager.get_client(&client_id).unwrap();
        assert!(client.enabled);
    }
}
