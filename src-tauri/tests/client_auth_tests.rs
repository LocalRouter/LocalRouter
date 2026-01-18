//! Integration tests for client authentication system
//!
//! Tests client creation, authentication, and access control.

use localrouter_ai::clients::{ClientManager, TokenStore};
use localrouter_ai::utils::errors::AppResult;

#[test]
fn test_client_creation() -> AppResult<()> {
    let manager = ClientManager::new(vec![]);

    // Create a new client
    let (client_id, secret, client) = manager.create_client("Test Client".to_string())?;

    // Verify client was created
    assert_eq!(client.name, "Test Client");
    assert_eq!(client.id, client_id);
    assert!(client.enabled);
    assert!(client.allowed_llm_providers.is_empty());
    assert!(client.allowed_mcp_servers.is_empty());
    assert!(!client.id.is_empty());
    assert!(!secret.is_empty());

    // Verify secret format (should be lr-... format)
    assert!(secret.starts_with("lr-"));

    Ok(())
}

#[test]
fn test_client_authentication_with_secret() -> AppResult<()> {
    let manager = ClientManager::new(vec![]);

    // Create a client
    let (_client_id, secret, client) = manager.create_client("Auth Test Client".to_string())?;

    // Verify authentication with correct secret
    let verified = manager.verify_secret(&secret)?;
    assert!(verified.is_some());
    let verified_client = verified.unwrap();
    assert_eq!(verified_client.id, client.id);

    // Verify authentication fails with wrong secret
    let wrong_secret = "lr-wrongsecret123456789012345678901234567890";
    let verified_wrong = manager.verify_secret(wrong_secret)?;
    assert!(verified_wrong.is_none());

    Ok(())
}

#[test]
fn test_client_credentials_verification() -> AppResult<()> {
    let manager = ClientManager::new(vec![]);

    // Create a client
    let (client_id, secret, _client) = manager.create_client("Creds Test Client".to_string())?;

    // Verify with correct credentials
    let verified = manager.verify_credentials(&client_id, &secret)?;
    assert!(verified.is_some());

    // Verify fails with wrong client_id
    let verified_wrong_id = manager.verify_credentials("wrong-client-id", &secret)?;
    assert!(verified_wrong_id.is_none());

    // Verify fails with wrong secret
    let wrong_secret = "lr-wrongsecret123456789012345678901234567890";
    let verified_wrong_secret = manager.verify_credentials(&client_id, wrong_secret)?;
    assert!(verified_wrong_secret.is_none());

    Ok(())
}

#[test]
fn test_client_disabled_authentication() -> AppResult<()> {
    let manager = ClientManager::new(vec![]);

    // Create a client
    let (_client_id, secret, client) = manager.create_client("Disabled Test".to_string())?;

    // Verify authentication works initially
    let verified = manager.verify_secret(&secret)?;
    assert!(verified.is_some());

    // Disable the client
    manager.update_client(&client.id, None, Some(false))?;

    // Verify authentication fails for disabled clients (filtered at verify_secret level)
    let verified_after_disable = manager.verify_secret(&secret)?;
    assert!(verified_after_disable.is_none());

    Ok(())
}

#[test]
fn test_token_store_generation() -> AppResult<()> {
    let token_store = TokenStore::new();

    // Generate a token for a client
    let (token, expires_in) = token_store.generate_token("test-client-id".to_string())?;

    // Verify token format
    assert!(token.starts_with("lr-"));
    assert!(expires_in > 0);
    assert!(expires_in >= 3599 && expires_in <= 3600); // Default 1 hour (allow 1 sec tolerance)

    // Verify token can be verified
    let client_id = token_store.verify_token(&token);
    assert!(client_id.is_some());
    assert_eq!(client_id.unwrap(), "test-client-id");

    Ok(())
}

#[test]
fn test_token_store_verification() -> AppResult<()> {
    let token_store = TokenStore::new();

    // Generate a token
    let (token, _) = token_store.generate_token("test-client-id".to_string())?;

    // Verify token
    let client_id = token_store.verify_token(&token);
    assert_eq!(client_id, Some("test-client-id".to_string()));

    // Verify invalid token
    let invalid = token_store.verify_token("invalid-token");
    assert_eq!(invalid, None);

    Ok(())
}

#[test]
fn test_token_revocation() -> AppResult<()> {
    let token_store = TokenStore::new();

    // Generate tokens for two clients
    let (token1, _) = token_store.generate_token("client-1".to_string())?;
    let (token2, _) = token_store.generate_token("client-1".to_string())?;
    let (token3, _) = token_store.generate_token("client-2".to_string())?;

    // Verify all tokens work
    assert!(token_store.verify_token(&token1).is_some());
    assert!(token_store.verify_token(&token2).is_some());
    assert!(token_store.verify_token(&token3).is_some());

    // Revoke one token
    let revoked = token_store.revoke_token(&token1);
    assert!(revoked);

    // Verify token1 no longer works
    assert!(token_store.verify_token(&token1).is_none());
    assert!(token_store.verify_token(&token2).is_some());
    assert!(token_store.verify_token(&token3).is_some());

    // Revoke all tokens for client-1
    let count = token_store.revoke_client_tokens("client-1");
    assert_eq!(count, 1); // token2 was remaining

    // Verify only client-2 token works
    assert!(token_store.verify_token(&token1).is_none());
    assert!(token_store.verify_token(&token2).is_none());
    assert!(token_store.verify_token(&token3).is_some());

    Ok(())
}

#[test]
fn test_client_llm_provider_access() -> AppResult<()> {
    let manager = ClientManager::new(vec![]);

    // Create a client
    let (_client_id, _secret, client) = manager.create_client("Provider Access Test".to_string())?;

    // Initially no providers allowed
    assert!(client.allowed_llm_providers.is_empty());

    // Add a provider
    manager.add_llm_provider(&client.id, "openai")?;

    // Verify provider was added
    let updated_client = manager.get_client(&client.id).unwrap();
    assert_eq!(updated_client.allowed_llm_providers.len(), 1);
    assert!(updated_client.allowed_llm_providers.contains(&"openai".to_string()));

    // Add another provider
    manager.add_llm_provider(&client.id, "anthropic")?;

    // Verify both providers
    let updated_client = manager.get_client(&client.id).unwrap();
    assert_eq!(updated_client.allowed_llm_providers.len(), 2);
    assert!(updated_client.allowed_llm_providers.contains(&"openai".to_string()));
    assert!(updated_client.allowed_llm_providers.contains(&"anthropic".to_string()));

    // Remove a provider
    manager.remove_llm_provider(&client.id, "openai")?;

    // Verify only anthropic remains
    let updated_client = manager.get_client(&client.id).unwrap();
    assert_eq!(updated_client.allowed_llm_providers.len(), 1);
    assert!(updated_client.allowed_llm_providers.contains(&"anthropic".to_string()));
    assert!(!updated_client.allowed_llm_providers.contains(&"openai".to_string()));

    Ok(())
}

#[test]
fn test_client_mcp_server_access() -> AppResult<()> {
    let manager = ClientManager::new(vec![]);

    // Create a client
    let (_client_id, _secret, client) = manager.create_client("MCP Access Test".to_string())?;

    // Initially no servers allowed
    assert!(client.allowed_mcp_servers.is_empty());

    // Add a server
    manager.add_mcp_server(&client.id, "server-1")?;

    // Verify server was added
    let updated_client = manager.get_client(&client.id).unwrap();
    assert_eq!(updated_client.allowed_mcp_servers.len(), 1);
    assert!(updated_client.allowed_mcp_servers.contains(&"server-1".to_string()));

    // Add another server
    manager.add_mcp_server(&client.id, "server-2")?;

    // Verify both servers
    let updated_client = manager.get_client(&client.id).unwrap();
    assert_eq!(updated_client.allowed_mcp_servers.len(), 2);
    assert!(updated_client.allowed_mcp_servers.contains(&"server-1".to_string()));
    assert!(updated_client.allowed_mcp_servers.contains(&"server-2".to_string()));

    // Remove a server
    manager.remove_mcp_server(&client.id, "server-1")?;

    // Verify only server-2 remains
    let updated_client = manager.get_client(&client.id).unwrap();
    assert_eq!(updated_client.allowed_mcp_servers.len(), 1);
    assert!(updated_client.allowed_mcp_servers.contains(&"server-2".to_string()));
    assert!(!updated_client.allowed_mcp_servers.contains(&"server-1".to_string()));

    Ok(())
}

#[test]
fn test_client_deletion() -> AppResult<()> {
    let manager = ClientManager::new(vec![]);

    // Create a client
    let (_client_id, secret, client) = manager.create_client("Delete Test".to_string())?;

    // Verify client exists
    assert!(manager.get_client(&client.id).is_some());
    assert!(manager.verify_secret(&secret)?.is_some());

    // Delete the client
    manager.delete_client(&client.id)?;

    // Verify client no longer exists
    assert!(manager.get_client(&client.id).is_none());
    assert!(manager.verify_secret(&secret)?.is_none());

    Ok(())
}

#[test]
fn test_client_update() -> AppResult<()> {
    let manager = ClientManager::new(vec![]);

    // Create a client
    let (_client_id, _secret, client) = manager.create_client("Update Test".to_string())?;

    // Update name
    manager.update_client(&client.id, Some("New Name".to_string()), None)?;
    let updated = manager.get_client(&client.id).unwrap();
    assert_eq!(updated.name, "New Name");
    assert!(updated.enabled);

    // Update enabled status
    manager.update_client(&client.id, None, Some(false))?;
    let updated = manager.get_client(&client.id).unwrap();
    assert_eq!(updated.name, "New Name");
    assert!(!updated.enabled);

    // Update both
    manager.update_client(&client.id, Some("Final Name".to_string()), Some(true))?;
    let updated = manager.get_client(&client.id).unwrap();
    assert_eq!(updated.name, "Final Name");
    assert!(updated.enabled);

    Ok(())
}

#[test]
fn test_multiple_clients() -> AppResult<()> {
    let manager = ClientManager::new(vec![]);

    // Create multiple clients
    let (client_id1, secret1, client1) = manager.create_client("Client 1".to_string())?;
    let (client_id2, secret2, client2) = manager.create_client("Client 2".to_string())?;
    let (client_id3, secret3, client3) = manager.create_client("Client 3".to_string())?;

    // Verify all clients exist
    let clients = manager.list_clients();
    assert_eq!(clients.len(), 3);

    // Verify each secret authenticates to correct client
    let verified1 = manager.verify_secret(&secret1)?.unwrap();
    assert_eq!(verified1.id, client1.id);

    let verified2 = manager.verify_secret(&secret2)?.unwrap();
    assert_eq!(verified2.id, client2.id);

    let verified3 = manager.verify_secret(&secret3)?.unwrap();
    assert_eq!(verified3.id, client3.id);

    // Verify cross-authentication fails
    let wrong = manager.verify_credentials(&client_id1, &secret2)?;
    assert!(wrong.is_none());

    Ok(())
}
