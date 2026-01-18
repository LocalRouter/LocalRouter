//! Integration tests for MCP authentication configuration
//!
//! Tests MCP server authentication methods and configuration.

use localrouter_ai::config::{McpAuthConfig, McpServerConfig, McpTransportConfig, McpTransportType};
use localrouter_ai::mcp::McpServerManager;
use localrouter_ai::utils::errors::AppResult;
use std::collections::HashMap;

#[test]
fn test_mcp_server_with_no_auth() -> AppResult<()> {
    let manager = McpServerManager::new();

    // Create server with no auth config
    let config = McpServerConfig::new(
        "Test Server".to_string(),
        McpTransportType::Stdio,
        McpTransportConfig::Stdio {
            command: "echo".to_string(),
            args: vec!["test".to_string()],
            env: HashMap::new(),
        },
    );

    // Verify no auth config
    assert!(config.auth_config.is_none());

    // Add to manager
    manager.add_config(config.clone());

    // Verify can retrieve
    let retrieved = manager.get_config(&config.id).unwrap();
    assert!(retrieved.auth_config.is_none());

    Ok(())
}

#[test]
fn test_mcp_server_with_env_vars_auth() -> AppResult<()> {
    let manager = McpServerManager::new();

    // Create auth config with environment variables
    let mut env_vars = HashMap::new();
    env_vars.insert("API_KEY".to_string(), "test-key-123".to_string());
    env_vars.insert("SECRET".to_string(), "test-secret-456".to_string());

    let auth_config = McpAuthConfig::EnvVars {
        env: env_vars.clone(),
    };

    // Create server
    let mut config = McpServerConfig::new(
        "Test Server".to_string(),
        McpTransportType::Stdio,
        McpTransportConfig::Stdio {
            command: "echo".to_string(),
            args: vec!["test".to_string()],
            env: HashMap::new(),
        },
    );
    config.auth_config = Some(auth_config);

    // Add to manager
    manager.add_config(config.clone());

    // Verify auth config stored correctly
    let retrieved = manager.get_config(&config.id).unwrap();
    assert!(retrieved.auth_config.is_some());

    match retrieved.auth_config.unwrap() {
        McpAuthConfig::EnvVars { env } => {
            assert_eq!(env.len(), 2);
            assert_eq!(env.get("API_KEY").unwrap(), "test-key-123");
            assert_eq!(env.get("SECRET").unwrap(), "test-secret-456");
        }
        _ => panic!("Expected EnvVars auth config"),
    }

    Ok(())
}

#[test]
fn test_mcp_server_with_bearer_token_auth() -> AppResult<()> {
    let manager = McpServerManager::new();

    // Create auth config with bearer token
    let auth_config = McpAuthConfig::BearerToken {
        token_ref: "my-token-ref".to_string(),
    };

    // Create SSE server
    let mut config = McpServerConfig::new(
        "Test SSE Server".to_string(),
        McpTransportType::HttpSse,
        McpTransportConfig::HttpSse {
            url: "http://localhost:8080/sse".to_string(),
            headers: HashMap::new(),
        },
    );
    config.auth_config = Some(auth_config);

    // Add to manager
    manager.add_config(config.clone());

    // Verify auth config stored correctly
    let retrieved = manager.get_config(&config.id).unwrap();
    assert!(retrieved.auth_config.is_some());

    match retrieved.auth_config.unwrap() {
        McpAuthConfig::BearerToken { token_ref } => {
            assert_eq!(token_ref, "my-token-ref");
        }
        _ => panic!("Expected BearerToken auth config"),
    }

    Ok(())
}

#[test]
fn test_mcp_server_with_custom_headers_auth() -> AppResult<()> {
    let manager = McpServerManager::new();

    // Create auth config with custom headers
    let mut headers = HashMap::new();
    headers.insert("X-API-Key".to_string(), "secret-key-123".to_string());
    headers.insert("X-Custom-Header".to_string(), "custom-value".to_string());

    let auth_config = McpAuthConfig::CustomHeaders {
        headers: headers.clone(),
    };

    // Create SSE server
    let mut config = McpServerConfig::new(
        "Test SSE Server".to_string(),
        McpTransportType::HttpSse,
        McpTransportConfig::HttpSse {
            url: "http://localhost:8080/sse".to_string(),
            headers: HashMap::new(),
        },
    );
    config.auth_config = Some(auth_config);

    // Add to manager
    manager.add_config(config.clone());

    // Verify auth config stored correctly
    let retrieved = manager.get_config(&config.id).unwrap();
    assert!(retrieved.auth_config.is_some());

    match retrieved.auth_config.unwrap() {
        McpAuthConfig::CustomHeaders { headers } => {
            assert_eq!(headers.len(), 2);
            assert_eq!(headers.get("X-API-Key").unwrap(), "secret-key-123");
            assert_eq!(headers.get("X-Custom-Header").unwrap(), "custom-value");
        }
        _ => panic!("Expected CustomHeaders auth config"),
    }

    Ok(())
}

#[test]
fn test_mcp_server_with_oauth_auth() -> AppResult<()> {
    let manager = McpServerManager::new();

    // Create auth config with OAuth
    let auth_config = McpAuthConfig::OAuth {
        client_id: "oauth-client-123".to_string(),
        client_secret_ref: "oauth-secret-ref".to_string(),
        auth_url: "https://auth.example.com/authorize".to_string(),
        token_url: "https://auth.example.com/token".to_string(),
        scopes: vec!["read".to_string(), "write".to_string()],
    };

    // Create SSE server
    let mut config = McpServerConfig::new(
        "Test OAuth Server".to_string(),
        McpTransportType::HttpSse,
        McpTransportConfig::HttpSse {
            url: "http://localhost:8080/sse".to_string(),
            headers: HashMap::new(),
        },
    );
    config.auth_config = Some(auth_config);

    // Add to manager
    manager.add_config(config.clone());

    // Verify auth config stored correctly
    let retrieved = manager.get_config(&config.id).unwrap();
    assert!(retrieved.auth_config.is_some());

    match retrieved.auth_config.unwrap() {
        McpAuthConfig::OAuth {
            client_id,
            client_secret_ref,
            auth_url,
            token_url,
            scopes,
        } => {
            assert_eq!(client_id, "oauth-client-123");
            assert_eq!(client_secret_ref, "oauth-secret-ref");
            assert_eq!(auth_url, "https://auth.example.com/authorize");
            assert_eq!(token_url, "https://auth.example.com/token");
            assert_eq!(scopes.len(), 2);
            assert!(scopes.contains(&"read".to_string()));
            assert!(scopes.contains(&"write".to_string()));
        }
        _ => panic!("Expected OAuth auth config"),
    }

    Ok(())
}

#[test]
fn test_mcp_server_auth_config_update() -> AppResult<()> {
    let manager = McpServerManager::new();

    // Create server with no auth
    let mut config = McpServerConfig::new(
        "Test Server".to_string(),
        McpTransportType::Stdio,
        McpTransportConfig::Stdio {
            command: "echo".to_string(),
            args: vec![],
            env: HashMap::new(),
        },
    );

    manager.add_config(config.clone());

    // Verify no auth initially
    let retrieved = manager.get_config(&config.id).unwrap();
    assert!(retrieved.auth_config.is_none());

    // Update with auth config
    let mut env_vars = HashMap::new();
    env_vars.insert("TOKEN".to_string(), "secret-token".to_string());
    config.auth_config = Some(McpAuthConfig::EnvVars { env: env_vars });

    manager.add_config(config.clone()); // Re-add with updated config

    // Verify auth config was updated
    let retrieved = manager.get_config(&config.id).unwrap();
    assert!(retrieved.auth_config.is_some());

    match retrieved.auth_config.unwrap() {
        McpAuthConfig::EnvVars { env } => {
            assert_eq!(env.len(), 1);
            assert_eq!(env.get("TOKEN").unwrap(), "secret-token");
        }
        _ => panic!("Expected EnvVars auth config"),
    }

    Ok(())
}

#[test]
fn test_mcp_server_config_serialization() -> AppResult<()> {
    // Create server with OAuth auth config
    let auth_config = McpAuthConfig::OAuth {
        client_id: "test-client".to_string(),
        client_secret_ref: "test-secret-ref".to_string(),
        auth_url: "https://auth.example.com/authorize".to_string(),
        token_url: "https://auth.example.com/token".to_string(),
        scopes: vec!["read".to_string()],
    };

    let mut config = McpServerConfig::new(
        "Test Server".to_string(),
        McpTransportType::HttpSse,
        McpTransportConfig::HttpSse {
            url: "http://localhost:8080/sse".to_string(),
            headers: HashMap::new(),
        },
    );
    config.auth_config = Some(auth_config);

    // Serialize to JSON
    let json = serde_json::to_string(&config)?;

    // Debug: Print the JSON to see the actual format
    eprintln!("Serialized JSON: {}", json);

    // The auth_config is serialized with tag format, check for the actual structure
    assert!(json.contains("auth_config"));
    assert!(json.contains("test-client"));
    assert!(json.contains("https://auth.example.com/authorize"));

    // Deserialize back
    let deserialized: McpServerConfig = serde_json::from_str(&json)?;
    assert_eq!(deserialized.id, config.id);
    assert_eq!(deserialized.name, config.name);
    assert!(deserialized.auth_config.is_some());

    match deserialized.auth_config.unwrap() {
        McpAuthConfig::OAuth {
            client_id,
            token_url,
            ..
        } => {
            assert_eq!(client_id, "test-client");
            assert_eq!(token_url, "https://auth.example.com/token");
        }
        _ => panic!("Expected OAuth auth config after deserialization"),
    }

    Ok(())
}

#[test]
fn test_multiple_servers_with_different_auth() -> AppResult<()> {
    let manager = McpServerManager::new();

    // Server 1: No auth
    let config1 = McpServerConfig::new(
        "Server 1".to_string(),
        McpTransportType::Stdio,
        McpTransportConfig::Stdio {
            command: "echo".to_string(),
            args: vec![],
            env: HashMap::new(),
        },
    );

    // Server 2: EnvVars auth
    let mut config2 = McpServerConfig::new(
        "Server 2".to_string(),
        McpTransportType::Stdio,
        McpTransportConfig::Stdio {
            command: "echo".to_string(),
            args: vec![],
            env: HashMap::new(),
        },
    );
    let mut env = HashMap::new();
    env.insert("KEY".to_string(), "value".to_string());
    config2.auth_config = Some(McpAuthConfig::EnvVars { env });

    // Server 3: BearerToken auth
    let mut config3 = McpServerConfig::new(
        "Server 3".to_string(),
        McpTransportType::HttpSse,
        McpTransportConfig::HttpSse {
            url: "http://localhost:8080".to_string(),
            headers: HashMap::new(),
        },
    );
    config3.auth_config = Some(McpAuthConfig::BearerToken {
        token_ref: "token-ref".to_string(),
    });

    // Add all servers
    manager.add_config(config1.clone());
    manager.add_config(config2.clone());
    manager.add_config(config3.clone());

    // Verify all configs
    let configs = manager.list_configs();
    assert_eq!(configs.len(), 3);

    // Verify each has correct auth
    let retrieved1 = manager.get_config(&config1.id).unwrap();
    assert!(retrieved1.auth_config.is_none());

    let retrieved2 = manager.get_config(&config2.id).unwrap();
    assert!(matches!(
        retrieved2.auth_config,
        Some(McpAuthConfig::EnvVars { .. })
    ));

    let retrieved3 = manager.get_config(&config3.id).unwrap();
    assert!(matches!(
        retrieved3.auth_config,
        Some(McpAuthConfig::BearerToken { .. })
    ));

    Ok(())
}
