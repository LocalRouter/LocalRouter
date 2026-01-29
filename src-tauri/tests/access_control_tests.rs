//! Integration tests for access control enforcement
//!
//! Tests that clients can only access authorized LLM providers and MCP servers.

use localrouter::clients::ClientManager;
use localrouter::config::{McpServerConfig, McpTransportConfig, McpTransportType};
use localrouter::mcp::McpServerManager;
use localrouter::utils::errors::AppResult;
use std::collections::HashMap;

#[test]
fn test_client_llm_provider_access_control() -> AppResult<()> {
    let manager = ClientManager::new(vec![]);

    // Create a client
    let (_client_id, _secret, client) = manager.create_client("Test Client".to_string())?;

    // Initially, client has no provider access
    assert!(client.allowed_llm_providers.is_empty());

    // Add openai provider access
    manager.add_llm_provider(&client.id, "openai")?;

    // Verify client now has openai access
    let updated_client = manager.get_client(&client.id).unwrap();
    assert_eq!(updated_client.allowed_llm_providers.len(), 1);
    assert!(updated_client
        .allowed_llm_providers
        .contains(&"openai".to_string()));

    // Verify client does NOT have anthropic access
    assert!(!updated_client
        .allowed_llm_providers
        .contains(&"anthropic".to_string()));

    // Add anthropic provider access
    manager.add_llm_provider(&client.id, "anthropic")?;

    // Verify client now has both
    let updated_client = manager.get_client(&client.id).unwrap();
    assert_eq!(updated_client.allowed_llm_providers.len(), 2);
    assert!(updated_client
        .allowed_llm_providers
        .contains(&"openai".to_string()));
    assert!(updated_client
        .allowed_llm_providers
        .contains(&"anthropic".to_string()));

    // Remove openai access
    manager.remove_llm_provider(&client.id, "openai")?;

    // Verify client only has anthropic now
    let updated_client = manager.get_client(&client.id).unwrap();
    assert_eq!(updated_client.allowed_llm_providers.len(), 1);
    assert!(!updated_client
        .allowed_llm_providers
        .contains(&"openai".to_string()));
    assert!(updated_client
        .allowed_llm_providers
        .contains(&"anthropic".to_string()));

    Ok(())
}

#[test]
fn test_client_mcp_server_access_control() -> AppResult<()> {
    let client_manager = ClientManager::new(vec![]);
    let mcp_manager = McpServerManager::new();

    // Create some MCP servers
    let server1 = McpServerConfig::new(
        "Server 1".to_string(),
        McpTransportType::Stdio,
        McpTransportConfig::Stdio {
            command: "echo".to_string(),
            args: vec![],
            env: HashMap::new(),
        },
    );
    let server1_id = server1.id.clone();

    let server2 = McpServerConfig::new(
        "Server 2".to_string(),
        McpTransportType::Stdio,
        McpTransportConfig::Stdio {
            command: "echo".to_string(),
            args: vec![],
            env: HashMap::new(),
        },
    );
    let server2_id = server2.id.clone();

    mcp_manager.add_config(server1);
    mcp_manager.add_config(server2);

    // Create a client
    let (_client_id, _secret, client) = client_manager.create_client("Test Client".to_string())?;

    // Initially, client has no MCP server access
    assert!(!client.mcp_server_access.has_any_access());

    // Grant access to server1
    client_manager.add_mcp_server(&client.id, &server1_id)?;

    // Verify client has access to server1
    let updated_client = client_manager.get_client(&client.id).unwrap();
    let servers = updated_client.mcp_server_access.specific_servers().unwrap();
    assert_eq!(servers.len(), 1);
    assert!(servers.contains(&server1_id));

    // Verify client does NOT have access to server2
    assert!(!servers.contains(&server2_id));

    // Grant access to server2
    client_manager.add_mcp_server(&client.id, &server2_id)?;

    // Verify client has access to both servers
    let updated_client = client_manager.get_client(&client.id).unwrap();
    let servers = updated_client.mcp_server_access.specific_servers().unwrap();
    assert_eq!(servers.len(), 2);
    assert!(servers.contains(&server1_id));
    assert!(servers.contains(&server2_id));

    // Revoke access to server1
    client_manager.remove_mcp_server(&client.id, &server1_id)?;

    // Verify client only has server2 access now
    let updated_client = client_manager.get_client(&client.id).unwrap();
    let servers = updated_client.mcp_server_access.specific_servers().unwrap();
    assert_eq!(servers.len(), 1);
    assert!(!servers.contains(&server1_id));
    assert!(servers.contains(&server2_id));

    Ok(())
}

#[test]
fn test_multiple_clients_independent_access() -> AppResult<()> {
    let manager = ClientManager::new(vec![]);

    // Create two clients
    let (_id1, _secret1, client1) = manager.create_client("Client 1".to_string())?;
    let (_id2, _secret2, client2) = manager.create_client("Client 2".to_string())?;

    // Grant different provider access to each client
    manager.add_llm_provider(&client1.id, "openai")?;
    manager.add_llm_provider(&client1.id, "anthropic")?;
    manager.add_llm_provider(&client2.id, "anthropic")?;
    manager.add_llm_provider(&client2.id, "google")?;

    // Verify client1 permissions
    let updated_client1 = manager.get_client(&client1.id).unwrap();
    assert_eq!(updated_client1.allowed_llm_providers.len(), 2);
    assert!(updated_client1
        .allowed_llm_providers
        .contains(&"openai".to_string()));
    assert!(updated_client1
        .allowed_llm_providers
        .contains(&"anthropic".to_string()));
    assert!(!updated_client1
        .allowed_llm_providers
        .contains(&"google".to_string()));

    // Verify client2 permissions
    let updated_client2 = manager.get_client(&client2.id).unwrap();
    assert_eq!(updated_client2.allowed_llm_providers.len(), 2);
    assert!(updated_client2
        .allowed_llm_providers
        .contains(&"anthropic".to_string()));
    assert!(updated_client2
        .allowed_llm_providers
        .contains(&"google".to_string()));
    assert!(!updated_client2
        .allowed_llm_providers
        .contains(&"openai".to_string()));

    Ok(())
}

#[test]
fn test_disabled_client_loses_access() -> AppResult<()> {
    let manager = ClientManager::new(vec![]);

    // Create a client with provider access
    let (_client_id, secret, client) = manager.create_client("Test Client".to_string())?;
    manager.add_llm_provider(&client.id, "openai")?;

    // Verify client is enabled and can authenticate
    let verified = manager.verify_secret(&secret)?;
    assert!(verified.is_some());
    let verified_client = verified.unwrap();
    assert!(verified_client.enabled);
    assert!(verified_client
        .allowed_llm_providers
        .contains(&"openai".to_string()));

    // Disable the client
    manager.update_client(&client.id, None, Some(false))?;

    // Verify client is disabled and authentication fails
    let verified_disabled = manager.verify_secret(&secret)?;
    assert!(verified_disabled.is_none()); // verify_secret filters out disabled clients

    // But the client still exists with its permissions
    let client_from_manager = manager.get_client(&client.id).unwrap();
    assert!(!client_from_manager.enabled);
    assert!(client_from_manager
        .allowed_llm_providers
        .contains(&"openai".to_string()));

    Ok(())
}

#[test]
fn test_access_control_persists_across_updates() -> AppResult<()> {
    let manager = ClientManager::new(vec![]);

    // Create a client
    let (_client_id, _secret, client) = manager.create_client("Test Client".to_string())?;

    // Grant access to providers and servers
    manager.add_llm_provider(&client.id, "openai")?;
    manager.add_llm_provider(&client.id, "anthropic")?;
    manager.add_mcp_server(&client.id, "server-1")?;
    manager.add_mcp_server(&client.id, "server-2")?;

    // Update client name
    manager.update_client(&client.id, Some("New Name".to_string()), None)?;

    // Verify permissions are still intact
    let updated_client = manager.get_client(&client.id).unwrap();
    assert_eq!(updated_client.name, "New Name");
    assert_eq!(updated_client.allowed_llm_providers.len(), 2);
    let servers = updated_client.mcp_server_access.specific_servers().unwrap();
    assert_eq!(servers.len(), 2);
    assert!(updated_client
        .allowed_llm_providers
        .contains(&"openai".to_string()));
    assert!(updated_client
        .allowed_llm_providers
        .contains(&"anthropic".to_string()));
    assert!(servers.contains(&"server-1".to_string()));
    assert!(servers.contains(&"server-2".to_string()));

    // Disable and re-enable client
    manager.update_client(&client.id, None, Some(false))?;
    manager.update_client(&client.id, None, Some(true))?;

    // Verify permissions are still intact
    let updated_client = manager.get_client(&client.id).unwrap();
    assert!(updated_client.enabled);
    assert_eq!(updated_client.allowed_llm_providers.len(), 2);
    let servers = updated_client.mcp_server_access.specific_servers().unwrap();
    assert_eq!(servers.len(), 2);

    Ok(())
}

#[test]
fn test_duplicate_access_grants_are_idempotent() -> AppResult<()> {
    let manager = ClientManager::new(vec![]);

    // Create a client
    let (_client_id, _secret, client) = manager.create_client("Test Client".to_string())?;

    // Grant access to openai multiple times
    manager.add_llm_provider(&client.id, "openai")?;
    manager.add_llm_provider(&client.id, "openai")?;
    manager.add_llm_provider(&client.id, "openai")?;

    // Verify only one entry exists
    let updated_client = manager.get_client(&client.id).unwrap();
    assert_eq!(updated_client.allowed_llm_providers.len(), 1);
    assert!(updated_client
        .allowed_llm_providers
        .contains(&"openai".to_string()));

    // Grant access to same MCP server multiple times
    manager.add_mcp_server(&client.id, "server-1")?;
    manager.add_mcp_server(&client.id, "server-1")?;

    // Verify only one entry exists
    let updated_client = manager.get_client(&client.id).unwrap();
    let servers = updated_client.mcp_server_access.specific_servers().unwrap();
    assert_eq!(servers.len(), 1);
    assert!(servers.contains(&"server-1".to_string()));

    Ok(())
}

#[test]
fn test_removing_nonexistent_access_is_safe() -> AppResult<()> {
    let manager = ClientManager::new(vec![]);

    // Create a client
    let (_client_id, _secret, client) = manager.create_client("Test Client".to_string())?;

    // Try to remove access that was never granted
    let result = manager.remove_llm_provider(&client.id, "openai");
    assert!(result.is_ok()); // Should not error

    let result = manager.remove_mcp_server(&client.id, "server-1");
    assert!(result.is_ok()); // Should not error

    // Verify lists are still empty
    let updated_client = manager.get_client(&client.id).unwrap();
    assert!(updated_client.allowed_llm_providers.is_empty());
    assert!(!updated_client.mcp_server_access.has_any_access());

    Ok(())
}

#[test]
fn test_client_deletion_removes_all_access() -> AppResult<()> {
    let manager = ClientManager::new(vec![]);

    // Create a client with access to providers and servers
    let (_client_id, secret, client) = manager.create_client("Test Client".to_string())?;
    manager.add_llm_provider(&client.id, "openai")?;
    manager.add_llm_provider(&client.id, "anthropic")?;
    manager.add_mcp_server(&client.id, "server-1")?;
    manager.add_mcp_server(&client.id, "server-2")?;

    // Verify access was granted
    let client_before = manager.get_client(&client.id).unwrap();
    assert_eq!(client_before.allowed_llm_providers.len(), 2);
    let servers = client_before.mcp_server_access.specific_servers().unwrap();
    assert_eq!(servers.len(), 2);

    // Delete the client
    manager.delete_client(&client.id)?;

    // Verify client is gone
    assert!(manager.get_client(&client.id).is_none());
    assert!(manager.verify_secret(&secret)?.is_none());

    // Attempting to manage access should fail
    let result = manager.add_llm_provider(&client.id, "google");
    assert!(result.is_err());

    Ok(())
}

#[test]
fn test_case_sensitivity_in_provider_names() -> AppResult<()> {
    let manager = ClientManager::new(vec![]);

    // Create a client
    let (_client_id, _secret, client) = manager.create_client("Test Client".to_string())?;

    // Add providers with different cases
    manager.add_llm_provider(&client.id, "openai")?;
    manager.add_llm_provider(&client.id, "OpenAI")?; // Different case
    manager.add_llm_provider(&client.id, "OPENAI")?; // Different case

    // Verify all three are stored as separate entries (case-sensitive)
    let updated_client = manager.get_client(&client.id).unwrap();
    assert_eq!(updated_client.allowed_llm_providers.len(), 3);
    assert!(updated_client
        .allowed_llm_providers
        .contains(&"openai".to_string()));
    assert!(updated_client
        .allowed_llm_providers
        .contains(&"OpenAI".to_string()));
    assert!(updated_client
        .allowed_llm_providers
        .contains(&"OPENAI".to_string()));

    Ok(())
}
