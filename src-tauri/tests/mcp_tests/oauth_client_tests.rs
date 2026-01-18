//! OAuth client tests
//!
//! Integration tests for OAuth client creation, credential verification, and server linking.
//! These tests verify the OAuth client system works correctly for MCP authentication.

use super::common::*;
use localrouter_ai::api_keys::KeychainStorage;
use localrouter_ai::oauth_clients::OAuthClientManager;
use std::sync::Arc;
use base64::{engine::general_purpose::STANDARD, Engine};

#[tokio::test]
async fn test_oauth_client_creation() {
    let mock_keychain = Arc::new(MockKeychain::new());
    let manager = OAuthClientManager::with_keychain(vec![], mock_keychain.clone());

    let (client_id, client_secret, config) = manager
        .create_client(Some("test-client".to_string()))
        .await
        .expect("Failed to create OAuth client");

    // Verify client_id format
    assert!(client_id.starts_with("lr-"), "Client ID should start with lr-");

    // Verify client_secret format
    assert!(client_secret.starts_with("lr-"), "Client secret should start with lr-");

    // Verify config
    assert_eq!(config.name, "test-client");
    assert_eq!(config.client_id, client_id);
    assert!(config.enabled);
    assert!(config.linked_server_ids.is_empty());

    // Verify secret stored in keychain
    let stored_secret = mock_keychain
        .get("LocalRouter-OAuthClients", &config.id)
        .expect("Failed to get from keychain")
        .expect("Secret not found in keychain");
    assert_eq!(stored_secret, client_secret);
}

#[tokio::test]
async fn test_oauth_client_auto_naming() {
    let mock_keychain = Arc::new(MockKeychain::new());
    let manager = OAuthClientManager::with_keychain(vec![], mock_keychain);

    // Create without name
    let (_, _, config1) = manager.create_client(None).await.unwrap();
    assert_eq!(config1.name, "mcp-client-1");

    // Create another without name
    let (_, _, config2) = manager.create_client(None).await.unwrap();
    assert_eq!(config2.name, "mcp-client-2");
}

#[tokio::test]
async fn test_oauth_verify_credentials_basic_auth() {
    let mock_keychain = Arc::new(MockKeychain::new());
    let manager = OAuthClientManager::with_keychain(vec![], mock_keychain);

    let (client_id, client_secret, config) = manager.create_client(None).await.unwrap();

    // Create Basic Auth header
    let credentials = format!("{}:{}", client_id, client_secret);
    let encoded = STANDARD.encode(credentials.as_bytes());
    let basic_auth = format!("Basic {}", encoded);

    // Verify credentials
    let verified = manager.verify_credentials(&basic_auth);
    assert!(verified.is_some(), "Credentials should be valid");

    let verified_config = verified.unwrap();
    assert_eq!(verified_config.id, config.id);
    assert_eq!(verified_config.client_id, client_id);
}

#[tokio::test]
async fn test_oauth_verify_credentials_raw() {
    let mock_keychain = Arc::new(MockKeychain::new());
    let manager = OAuthClientManager::with_keychain(vec![], mock_keychain);

    let (client_id, client_secret, config) = manager.create_client(None).await.unwrap();

    // Create raw credentials
    let credentials = format!("{}:{}", client_id, client_secret);

    // Verify credentials
    let verified = manager.verify_credentials(&credentials);
    assert!(verified.is_some(), "Credentials should be valid");
    assert_eq!(verified.unwrap().id, config.id);
}

#[tokio::test]
async fn test_oauth_verify_credentials_wrong_secret() {
    let mock_keychain = Arc::new(MockKeychain::new());
    let manager = OAuthClientManager::with_keychain(vec![], mock_keychain);

    let (client_id, _, _) = manager.create_client(None).await.unwrap();

    // Try with wrong secret
    let credentials = format!("{}:wrong-secret", client_id);
    let verified = manager.verify_credentials(&credentials);
    assert!(verified.is_none(), "Wrong credentials should not verify");
}

#[tokio::test]
async fn test_oauth_verify_credentials_wrong_client_id() {
    let mock_keychain = Arc::new(MockKeychain::new());
    let manager = OAuthClientManager::with_keychain(vec![], mock_keychain);

    manager.create_client(None).await.unwrap();

    // Try with non-existent client_id
    let credentials = "lr-nonexistent:some-secret";
    let verified = manager.verify_credentials(credentials);
    assert!(verified.is_none(), "Non-existent client should not verify");
}

#[tokio::test]
async fn test_oauth_client_disabled() {
    let mock_keychain = Arc::new(MockKeychain::new());
    let manager = OAuthClientManager::with_keychain(vec![], mock_keychain);

    let (client_id, client_secret, config) = manager.create_client(None).await.unwrap();

    // Disable the client
    manager
        .update_client(&config.id, |c| c.enabled = false)
        .expect("Failed to disable client");

    // Try to verify credentials
    let credentials = format!("{}:{}", client_id, client_secret);
    let verified = manager.verify_credentials(&credentials);
    assert!(verified.is_none(), "Disabled client should not verify");
}

#[tokio::test]
async fn test_oauth_server_linking() {
    let mock_keychain = Arc::new(MockKeychain::new());
    let manager = OAuthClientManager::with_keychain(vec![], mock_keychain);

    let (_, _, config) = manager.create_client(None).await.unwrap();

    // Initially no servers linked
    assert!(!manager.can_access_server(&config.id, "server-1"));

    // Link to server-1
    manager
        .link_server(&config.id, "server-1".to_string())
        .expect("Failed to link server");

    // Verify can access
    assert!(manager.can_access_server(&config.id, "server-1"));
    assert!(!manager.can_access_server(&config.id, "server-2"));

    // Link to server-2
    manager
        .link_server(&config.id, "server-2".to_string())
        .expect("Failed to link server");

    // Verify can access both
    assert!(manager.can_access_server(&config.id, "server-1"));
    assert!(manager.can_access_server(&config.id, "server-2"));
}

#[tokio::test]
async fn test_oauth_server_unlinking() {
    let mock_keychain = Arc::new(MockKeychain::new());
    let manager = OAuthClientManager::with_keychain(vec![], mock_keychain);

    let (_, _, config) = manager.create_client(None).await.unwrap();

    // Link servers
    manager.link_server(&config.id, "server-1".to_string()).unwrap();
    manager.link_server(&config.id, "server-2".to_string()).unwrap();

    // Verify both linked
    assert!(manager.can_access_server(&config.id, "server-1"));
    assert!(manager.can_access_server(&config.id, "server-2"));

    // Unlink server-1
    manager
        .unlink_server(&config.id, "server-1")
        .expect("Failed to unlink server");

    // Verify server-1 unlinked but server-2 still linked
    assert!(!manager.can_access_server(&config.id, "server-1"));
    assert!(manager.can_access_server(&config.id, "server-2"));
}

#[tokio::test]
async fn test_oauth_duplicate_link() {
    let mock_keychain = Arc::new(MockKeychain::new());
    let manager = OAuthClientManager::with_keychain(vec![], mock_keychain);

    let (_, _, config) = manager.create_client(None).await.unwrap();

    // Link server twice
    manager.link_server(&config.id, "server-1".to_string()).unwrap();
    manager.link_server(&config.id, "server-1".to_string()).unwrap();

    // Should only be linked once
    let client = manager.get_client(&config.id).unwrap();
    let count = client.linked_server_ids.iter().filter(|id| *id == "server-1").count();
    assert_eq!(count, 1, "Server should only be linked once");
}

#[tokio::test]
async fn test_oauth_delete_client() {
    let mock_keychain = Arc::new(MockKeychain::new());
    let manager = OAuthClientManager::with_keychain(vec![], mock_keychain.clone());

    let (_, _, config) = manager.create_client(None).await.unwrap();

    // Verify client exists
    assert!(manager.get_client(&config.id).is_some());

    // Delete client
    manager
        .delete_client(&config.id)
        .expect("Failed to delete client");

    // Verify client removed from metadata
    assert!(manager.get_client(&config.id).is_none());

    // Verify secret removed from keychain
    let secret = mock_keychain
        .get("LocalRouter-OAuthClients", &config.id)
        .unwrap();
    assert!(secret.is_none(), "Secret should be removed from keychain");
}

#[tokio::test]
async fn test_oauth_rotate_secret() {
    let mock_keychain = Arc::new(MockKeychain::new());
    let manager = OAuthClientManager::with_keychain(vec![], mock_keychain.clone());

    let (client_id, old_secret, config) = manager.create_client(None).await.unwrap();

    // Rotate secret
    let new_secret = manager
        .rotate_secret(&config.id)
        .await
        .expect("Failed to rotate secret");

    // Verify new secret is different
    assert_ne!(old_secret, new_secret);

    // Verify old secret no longer works
    let old_creds = format!("{}:{}", client_id, old_secret);
    let verified = manager.verify_credentials(&old_creds);
    assert!(verified.is_none(), "Old secret should not work");

    // Verify new secret works
    let new_creds = format!("{}:{}", client_id, new_secret);
    let verified = manager.verify_credentials(&new_creds);
    assert!(verified.is_some(), "New secret should work");

    // Verify new secret stored in keychain
    let stored_secret = mock_keychain
        .get("LocalRouter-OAuthClients", &config.id)
        .unwrap()
        .unwrap();
    assert_eq!(stored_secret, new_secret);
}

#[tokio::test]
async fn test_oauth_list_clients() {
    let mock_keychain = Arc::new(MockKeychain::new());
    let manager = OAuthClientManager::with_keychain(vec![], mock_keychain);

    // Initially empty
    assert_eq!(manager.list_clients().len(), 0);

    // Create some clients
    let (_, _, config1) = manager.create_client(Some("client-1".to_string())).await.unwrap();
    let (_, _, config2) = manager.create_client(Some("client-2".to_string())).await.unwrap();

    // Verify list
    let clients = manager.list_clients();
    assert_eq!(clients.len(), 2);
    assert!(clients.iter().any(|c| c.id == config1.id));
    assert!(clients.iter().any(|c| c.id == config2.id));
}

#[tokio::test]
async fn test_oauth_get_client_by_client_id() {
    let mock_keychain = Arc::new(MockKeychain::new());
    let manager = OAuthClientManager::with_keychain(vec![], mock_keychain);

    let (client_id, _, config) = manager.create_client(None).await.unwrap();

    // Get by client_id
    let found = manager.get_client_by_client_id(&client_id);
    assert!(found.is_some());
    assert_eq!(found.unwrap().id, config.id);

    // Try non-existent
    let not_found = manager.get_client_by_client_id("lr-nonexistent");
    assert!(not_found.is_none());
}
