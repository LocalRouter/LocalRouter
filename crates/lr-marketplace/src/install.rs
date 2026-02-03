//! Install logic for MCP servers and skills
//!
//! This module provides helper functions for creating configs
//! from install requests. The actual installation (updating ConfigManager,
//! McpServerManager, SkillManager) is done by Tauri commands which have
//! access to these managers.

use crate::types::{
    InstalledServer, InstalledSkill, MarketplaceError, McpInstallConfig, McpServerListing,
};
use chrono::Utc;
use lr_config::{McpAuthConfig, McpServerConfig, McpTransportConfig, McpTransportType};
use std::collections::HashMap;
use uuid::Uuid;

/// Create an MCP server config from an install config
pub fn create_mcp_server_config(
    _listing: &McpServerListing,
    config: &McpInstallConfig,
) -> Result<McpServerConfig, MarketplaceError> {
    let id = Uuid::new_v4().to_string();

    let transport = match config.transport.as_str() {
        "stdio" => McpTransportType::Stdio,
        "http_sse" | "sse" => McpTransportType::HttpSse,
        "websocket" => McpTransportType::WebSocket,
        other => {
            return Err(MarketplaceError::InvalidArguments(format!(
                "Unknown transport type: {}",
                other
            )))
        }
    };

    let transport_config = match transport {
        McpTransportType::Stdio => {
            let command = config.command.clone().ok_or_else(|| {
                MarketplaceError::InvalidArguments(
                    "command is required for stdio transport".to_string(),
                )
            })?;

            // Build command string with args
            let full_command = if config.args.is_empty() {
                command
            } else {
                format!("{} {}", command, config.args.join(" "))
            };

            McpTransportConfig::Stdio {
                command: full_command,
                args: vec![], // deprecated, use command string
                env: config.env.clone(),
            }
        }
        McpTransportType::HttpSse => {
            let url = config.url.clone().ok_or_else(|| {
                MarketplaceError::InvalidArguments(
                    "url is required for http_sse transport".to_string(),
                )
            })?;

            McpTransportConfig::HttpSse {
                url,
                headers: HashMap::new(),
            }
        }
        McpTransportType::WebSocket => {
            let url = config.url.clone().ok_or_else(|| {
                MarketplaceError::InvalidArguments(
                    "url is required for websocket transport".to_string(),
                )
            })?;

            McpTransportConfig::WebSocket {
                url,
                headers: HashMap::new(),
            }
        }
        #[allow(deprecated)]
        McpTransportType::Sse => {
            return Err(MarketplaceError::InvalidArguments(
                "Sse transport is deprecated, use http_sse".to_string(),
            ))
        }
    };

    // Auth is handled separately after server creation
    // Bearer tokens need to be stored in keychain, which is done by the Tauri command
    // For now, we create the config without auth - auth is configured separately
    let auth_config = match config.auth_type.as_str() {
        "none" | "" => None,
        "bearer" => {
            // The actual token will be stored in keychain by the Tauri command
            // Here we just mark that bearer auth is intended
            // token_ref will be set to the server ID by the installer
            Some(McpAuthConfig::BearerToken {
                token_ref: "pending".to_string(), // Placeholder - set by Tauri command
            })
        }
        other => {
            return Err(MarketplaceError::InvalidArguments(format!(
                "Unknown auth type: {}. OAuth must be configured after installation.",
                other
            )))
        }
    };

    Ok(McpServerConfig {
        id,
        name: config.name.clone(),
        transport,
        transport_config,
        auth_config,
        discovered_oauth: None,
        oauth_config: None,
        enabled: true,
        created_at: Utc::now(),
    })
}

/// Generate a default MCP install config from a listing
pub fn generate_default_mcp_config(listing: &McpServerListing) -> McpInstallConfig {
    // Prefer stdio transport with first available package
    if let Some(pkg) = listing.packages.first() {
        let (command, args) = if let Some(ref cmd) = pkg.command {
            (Some(cmd.clone()), pkg.args.clone())
        } else {
            // Generate default based on registry
            match pkg.registry.as_str() {
                "npm" => (
                    Some("npx".to_string()),
                    vec!["-y".to_string(), pkg.name.clone()],
                ),
                "pypi" => (Some("uvx".to_string()), vec![pkg.name.clone()]),
                _ => (None, vec![]),
            }
        };

        return McpInstallConfig {
            name: listing.name.clone(),
            transport: "stdio".to_string(),
            command,
            args,
            url: None,
            env: pkg.env.clone(),
            auth_type: "none".to_string(),
            bearer_token: None,
        };
    }

    // Fall back to first remote
    if let Some(remote) = listing.remotes.first() {
        let transport = match remote.transport.as_str() {
            "sse" | "http" => "http_sse",
            "websocket" | "ws" => "websocket",
            other => other,
        };

        return McpInstallConfig {
            name: listing.name.clone(),
            transport: transport.to_string(),
            command: None,
            args: vec![],
            url: Some(remote.url.clone()),
            env: HashMap::new(),
            auth_type: remote.auth.clone().unwrap_or_else(|| "none".to_string()),
            bearer_token: None,
        };
    }

    // Empty config (user must fill in)
    McpInstallConfig {
        name: listing.name.clone(),
        transport: "stdio".to_string(),
        command: None,
        args: vec![],
        url: None,
        env: HashMap::new(),
        auth_type: "none".to_string(),
        bearer_token: None,
    }
}

/// Create the install result for a successfully installed server
pub fn create_installed_server_result(config: &McpServerConfig) -> InstalledServer {
    InstalledServer {
        server_id: config.id.clone(),
        name: config.name.clone(),
    }
}

/// Create the install result for a successfully installed skill
pub fn create_installed_skill_result(name: &str, path: &str) -> InstalledSkill {
    InstalledSkill {
        name: name.to_string(),
        path: path.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{McpPackageInfo, McpRemoteInfo, MCP_REGISTRY_SOURCE_ID};

    #[test]
    fn test_create_mcp_server_config_stdio() {
        let listing = McpServerListing {
            name: "test-server".to_string(),
            description: "Test".to_string(),
            source_id: MCP_REGISTRY_SOURCE_ID.to_string(),
            homepage: None,
            vendor: None,
            packages: vec![],
            remotes: vec![],
            available_transports: vec!["stdio".to_string()],
            install_hint: None,
        };

        let config = McpInstallConfig {
            name: "My Server".to_string(),
            transport: "stdio".to_string(),
            command: Some("npx".to_string()),
            args: vec!["-y".to_string(), "@test/server".to_string()],
            url: None,
            env: HashMap::new(),
            auth_type: "none".to_string(),
            bearer_token: None,
        };

        let result = create_mcp_server_config(&listing, &config).unwrap();
        assert_eq!(result.name, "My Server");
        assert_eq!(result.transport, McpTransportType::Stdio);
        assert!(result.enabled);

        match result.transport_config {
            McpTransportConfig::Stdio { command, .. } => {
                // Command includes args as a single string now
                assert_eq!(command, "npx -y @test/server");
            }
            _ => panic!("Expected Stdio transport config"),
        }
    }

    #[test]
    fn test_create_mcp_server_config_http_sse() {
        let listing = McpServerListing {
            name: "test-server".to_string(),
            description: "Test".to_string(),
            source_id: MCP_REGISTRY_SOURCE_ID.to_string(),
            homepage: None,
            vendor: None,
            packages: vec![],
            remotes: vec![],
            available_transports: vec!["http_sse".to_string()],
            install_hint: None,
        };

        let config = McpInstallConfig {
            name: "My Server".to_string(),
            transport: "http_sse".to_string(),
            command: None,
            args: vec![],
            url: Some("https://example.com/mcp".to_string()),
            env: HashMap::new(),
            auth_type: "bearer".to_string(),
            bearer_token: Some("secret-token".to_string()),
        };

        let result = create_mcp_server_config(&listing, &config).unwrap();
        assert_eq!(result.transport, McpTransportType::HttpSse);

        match result.transport_config {
            McpTransportConfig::HttpSse { url, .. } => {
                assert_eq!(url, "https://example.com/mcp");
            }
            _ => panic!("Expected HttpSse transport config"),
        }

        match result.auth_config {
            Some(McpAuthConfig::BearerToken { token_ref }) => {
                // Token ref is "pending" as placeholder - real token stored in keychain
                assert_eq!(token_ref, "pending");
            }
            _ => panic!("Expected BearerToken auth config"),
        }
    }

    #[test]
    fn test_generate_default_mcp_config_npm() {
        let listing = McpServerListing {
            name: "filesystem".to_string(),
            description: "Filesystem server".to_string(),
            source_id: MCP_REGISTRY_SOURCE_ID.to_string(),
            homepage: None,
            vendor: None,
            packages: vec![McpPackageInfo {
                registry: "npm".to_string(),
                name: "@anthropic/mcp-server-filesystem".to_string(),
                version: Some("1.0.0".to_string()),
                runtime: Some("node".to_string()),
                command: None,
                args: vec![],
                env: HashMap::new(),
            }],
            remotes: vec![],
            available_transports: vec!["stdio".to_string()],
            install_hint: None,
        };

        let config = generate_default_mcp_config(&listing);
        assert_eq!(config.name, "filesystem");
        assert_eq!(config.transport, "stdio");
        assert_eq!(config.command, Some("npx".to_string()));
        assert_eq!(config.args, vec!["-y", "@anthropic/mcp-server-filesystem"]);
    }

    #[test]
    fn test_generate_default_mcp_config_remote() {
        let listing = McpServerListing {
            name: "cloud-server".to_string(),
            description: "Cloud server".to_string(),
            source_id: MCP_REGISTRY_SOURCE_ID.to_string(),
            homepage: None,
            vendor: None,
            packages: vec![],
            remotes: vec![McpRemoteInfo {
                transport: "sse".to_string(),
                url: "https://example.com/mcp".to_string(),
                auth: Some("bearer".to_string()),
                oauth: None,
            }],
            available_transports: vec!["http_sse".to_string()],
            install_hint: None,
        };

        let config = generate_default_mcp_config(&listing);
        assert_eq!(config.name, "cloud-server");
        assert_eq!(config.transport, "http_sse");
        assert_eq!(config.url, Some("https://example.com/mcp".to_string()));
        assert_eq!(config.auth_type, "bearer");
    }
}
