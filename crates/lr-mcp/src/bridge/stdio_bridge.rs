//! STDIO ↔ HTTP Bridge for MCP
//!
//! This module implements a lightweight proxy that reads JSON-RPC requests from stdin,
//! forwards them to the LocalRouter HTTP server, and writes responses back to stdout.
//!
//! This allows external MCP clients (Claude Desktop, Cursor, VS Code) to connect to
//! LocalRouter's unified MCP gateway via standard input/output.

use crate::protocol::{JsonRpcError, JsonRpcRequest, JsonRpcResponse};
use lr_api_keys::keychain_trait::{CachedKeychain, KeychainStorage};
use lr_config::{AppConfig, Client, ConfigManager};
use lr_types::{AppError, AppResult};
use serde_json::Value;
use std::path::PathBuf;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tracing::{debug, error, info, trace, warn};

/// STDIO ↔ HTTP Bridge
///
/// Lightweight proxy that forwards JSON-RPC requests from stdin to the LocalRouter HTTP server.
pub struct StdioBridge {
    /// Client ID for authentication
    client_id: String,

    /// Client secret for Authorization header
    client_secret: String,

    /// LocalRouter HTTP endpoint
    server_url: String,

    /// HTTP client
    http_client: reqwest::Client,

    /// STDIO handles
    stdin: BufReader<tokio::io::Stdin>,
    stdout: tokio::io::Stdout,
}

impl StdioBridge {
    /// Create a new STDIO bridge
    ///
    /// # Arguments
    /// * `client_id` - Optional client ID (auto-detects if None)
    /// * `config_path` - Optional config path (uses default if None)
    ///
    /// # Returns
    /// A new StdioBridge instance ready to process requests
    ///
    /// # Errors
    /// Returns error if:
    /// - Config file not found
    /// - Client not found or disabled
    /// - Client secret not found
    /// - Client has no MCP servers allowed
    pub async fn new(client_id: Option<String>, config_path: Option<PathBuf>) -> AppResult<Self> {
        // Load config
        let config_manager = if let Some(path) = config_path {
            ConfigManager::load_from_path(path).await?
        } else {
            ConfigManager::load().await?
        };

        let config = config_manager.get();

        // Resolve client and secret
        let (client_id, client_secret) = resolve_client_secret(client_id, &config).await?;

        // Validate client has MCP servers configured using mcp_permissions
        let client = find_client_by_id(&client_id, &config)?;
        if !client.mcp_permissions.global.is_enabled() && client.mcp_permissions.servers.is_empty()
        {
            return Err(AppError::Config(format!(
                "Client '{}' has no MCP servers configured. Set 'mcp_permissions' in config.yaml",
                client_id
            )));
        }

        // Get server count for logging
        let server_count = if client.mcp_permissions.global.is_enabled() {
            config.mcp_servers.len()
        } else {
            client.mcp_permissions.servers.len()
        };

        info!(
            "Bridge initialized for client '{}' with {} MCP servers (global: {:?})",
            client_id, server_count, client.mcp_permissions.global
        );

        Ok(Self {
            client_id,
            client_secret,
            server_url: "http://localhost:3625/mcp".to_string(),
            http_client: reqwest::Client::new(),
            stdin: BufReader::new(tokio::io::stdin()),
            stdout: tokio::io::stdout(),
        })
    }

    /// Run the bridge main loop
    ///
    /// Reads JSON-RPC requests from stdin line-by-line, forwards to HTTP server,
    /// and writes responses back to stdout.
    ///
    /// Logs to stderr only. All stdout output is JSON-RPC responses.
    ///
    /// # Errors
    /// Returns error if:
    /// - HTTP server is not available
    /// - Authentication fails
    /// - Other connection errors
    pub async fn run(mut self) -> AppResult<()> {
        info!("Bridge started, reading from stdin...");

        let mut line = String::new();

        loop {
            line.clear();

            // Read a line from stdin
            match self.stdin.read_line(&mut line).await {
                Ok(0) => {
                    // EOF reached
                    debug!("EOF reached on stdin, exiting");
                    break;
                }
                Ok(n) => {
                    trace!("Read {} bytes from stdin", n);

                    // Parse JSON-RPC request
                    match serde_json::from_str::<JsonRpcRequest>(&line) {
                        Ok(request) => {
                            debug!("Received request: method={}", request.method);

                            // Handle the request
                            let response = match self.handle_request(request).await {
                                Ok(resp) => resp,
                                Err(e) => {
                                    error!("Request handling failed: {}", e);
                                    // Return JSON-RPC error response
                                    JsonRpcResponse {
                                        jsonrpc: "2.0".to_string(),
                                        id: Value::Null,
                                        result: None,
                                        error: Some(JsonRpcError {
                                            code: -32603,
                                            message: format!("Internal error: {}", e),
                                            data: None,
                                        }),
                                    }
                                }
                            };

                            // Write response to stdout
                            let response_json = serde_json::to_string(&response)?;
                            self.stdout.write_all(response_json.as_bytes()).await?;
                            self.stdout.write_all(b"\n").await?;
                            self.stdout.flush().await?;

                            trace!("Response written to stdout");
                        }
                        Err(e) => {
                            warn!("Failed to parse JSON-RPC request: {}", e);
                            // Return JSON-RPC parse error
                            let error_response = JsonRpcResponse {
                                jsonrpc: "2.0".to_string(),
                                id: Value::Null,
                                result: None,
                                error: Some(JsonRpcError {
                                    code: -32700,
                                    message: format!("Parse error: {}", e),
                                    data: None,
                                }),
                            };

                            let response_json = serde_json::to_string(&error_response)?;
                            self.stdout.write_all(response_json.as_bytes()).await?;
                            self.stdout.write_all(b"\n").await?;
                            self.stdout.flush().await?;
                        }
                    }
                }
                Err(e) => {
                    error!("Failed to read from stdin: {}", e);
                    return Err(e.into());
                }
            }
        }

        Ok(())
    }

    /// Handle a single JSON-RPC request
    ///
    /// Forwards the request to the LocalRouter HTTP server and returns the response.
    ///
    /// # Arguments
    /// * `request` - JSON-RPC request to forward
    ///
    /// # Returns
    /// JSON-RPC response from the server
    ///
    /// # Errors
    /// Returns error if:
    /// - HTTP request fails (connection refused, timeout, etc.)
    /// - Server returns non-2xx status code
    /// - Response parsing fails
    async fn handle_request(&mut self, request: JsonRpcRequest) -> AppResult<JsonRpcResponse> {
        trace!("Forwarding request to {}", self.server_url);

        // Serialize JSON-RPC request
        let body = serde_json::to_string(&request)?;

        // POST to LocalRouter HTTP server
        let response = self
            .http_client
            .post(&self.server_url)
            .header("Authorization", format!("Bearer {}", self.client_secret))
            .header("Content-Type", "application/json")
            .body(body)
            .send()
            .await
            .map_err(|e| {
                if e.is_connect() {
                    AppError::Mcp(
                        "Could not connect to LocalRouter at localhost:3625. Is the app running?"
                            .to_string(),
                    )
                } else if e.is_timeout() {
                    AppError::Mcp("Request timed out".to_string())
                } else {
                    AppError::Mcp(format!("HTTP request failed: {}", e))
                }
            })?;

        // Check status code
        let status = response.status();
        if !status.is_success() {
            let text = response.text().await.unwrap_or_default();

            return Err(match status.as_u16() {
                401 => AppError::Mcp(
                    "Invalid client credentials. Check LOCALROUTER_CLIENT_SECRET or run GUI once to store credentials.".to_string()
                ),
                403 => AppError::Mcp(format!(
                    "Client '{}' is not allowed to access MCP servers. Check 'mcp_permissions' in config.yaml",
                    self.client_id
                )),
                404 => AppError::Mcp(
                    "MCP endpoint not found. Is LocalRouter running the latest version?".to_string()
                ),
                _ => AppError::Mcp(format!("HTTP {} error: {}", status, text)),
            });
        }

        // Parse JSON-RPC response
        let json_rpc_response: JsonRpcResponse = response
            .json()
            .await
            .map_err(|e| AppError::Mcp(format!("Failed to parse server response: {}", e)))?;

        trace!("Received response from server");
        Ok(json_rpc_response)
    }
}

/// Resolve client ID and secret
///
/// Resolution order:
/// 1. LOCALROUTER_CLIENT_SECRET env var (if set, use provided client_id or "env")
/// 2. Load from config + keychain (find client, load secret)
///
/// # Arguments
/// * `client_id` - Optional explicit client ID
/// * `config` - Application configuration
///
/// # Returns
/// Tuple of (client_id, client_secret)
///
/// # Errors
/// Returns error if:
/// - Client not found in config
/// - Client is disabled
/// - Client secret not found in keychain
async fn resolve_client_secret(
    client_id: Option<String>,
    config: &AppConfig,
) -> AppResult<(String, String)> {
    // Try environment variable first
    if let Ok(secret) = std::env::var("LOCALROUTER_CLIENT_SECRET") {
        let id = client_id.unwrap_or_else(|| {
            warn!("Using LOCALROUTER_CLIENT_SECRET without explicit client ID");
            "env".to_string()
        });

        info!("Using client secret from environment variable");
        return Ok((id, secret));
    }

    // Load from config + keychain
    let client = if let Some(id) = client_id.as_ref() {
        // Explicit client ID provided
        find_client_by_id(id, config)?
    } else {
        // Auto-detect first enabled client with MCP servers
        find_first_enabled_client(config)?
    };

    // Load secret from keychain
    let keychain = CachedKeychain::auto()?;
    let secret = keychain
        .get("LocalRouter-Clients", &client.id)?
        .ok_or_else(|| {
            AppError::Config(format!(
                "Client secret not found for '{}'. Run LocalRouter GUI once to create credentials, or set LOCALROUTER_CLIENT_SECRET env var",
                client.id
            ))
        })?;

    info!("Using client '{}' from config", client.id);
    Ok((client.id.clone(), secret))
}

/// Find client by ID in config
fn find_client_by_id<'a>(client_id: &str, config: &'a AppConfig) -> AppResult<&'a Client> {
    let client = config
        .clients
        .iter()
        .find(|c| c.id == client_id)
        .ok_or_else(|| {
            AppError::Config(format!(
                "Client '{}' not found in config.yaml. Available clients: {}",
                client_id,
                config
                    .clients
                    .iter()
                    .map(|c| c.id.as_str())
                    .collect::<Vec<_>>()
                    .join(", ")
            ))
        })?;

    if !client.enabled {
        return Err(AppError::Config(format!(
            "Client '{}' is disabled. Enable it in config.yaml",
            client_id
        )));
    }

    Ok(client)
}

/// Find first enabled client with MCP servers
fn find_first_enabled_client(config: &AppConfig) -> AppResult<&Client> {
    config
        .clients
        .iter()
        .find(|c| {
            c.enabled
                && (c.mcp_permissions.global.is_enabled() || !c.mcp_permissions.servers.is_empty())
        })
        .ok_or_else(|| {
            AppError::Config(
                "No enabled clients with MCP servers found. Configure a client in config.yaml"
                    .to_string(),
            )
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;
    use lr_config::{
        FirewallRules, McpPermissions, McpServerAccess, ModelPermissions, PermissionState,
        SkillsPermissions,
    };

    fn test_config() -> AppConfig {
        let mut config = AppConfig::default();

        // Create mcp_permissions with filesystem server enabled
        let mut test_mcp_permissions = McpPermissions::default();
        test_mcp_permissions
            .servers
            .insert("filesystem".to_string(), PermissionState::Allow);

        config.clients = vec![
            Client {
                id: "test_client".to_string(),
                name: "Test Client".to_string(),
                enabled: true,
                allowed_llm_providers: vec![],
                mcp_server_access: McpServerAccess::Specific(vec!["filesystem".to_string()]),
                mcp_deferred_loading: false,
                created_at: Utc::now(),
                last_used: None,
                strategy_id: "default".to_string(),
                roots: None,
                mcp_sampling_enabled: false,
                mcp_sampling_requires_approval: true,
                mcp_sampling_max_tokens: None,
                mcp_sampling_rate_limit: None,
                firewall: FirewallRules::default(),
                skills_access: lr_config::SkillsAccess::None,
                marketplace_enabled: false,
                mcp_permissions: test_mcp_permissions,
                skills_permissions: SkillsPermissions::default(),
                model_permissions: ModelPermissions::default(),
                marketplace_permission: PermissionState::Off,
                client_mode: lr_config::ClientMode::default(),
                template_id: None,
                sync_config: false,
                guardrails_enabled: None,
            },
            Client {
                id: "disabled_client".to_string(),
                name: "Disabled Client".to_string(),
                enabled: false,
                allowed_llm_providers: vec![],
                mcp_server_access: McpServerAccess::Specific(vec!["web".to_string()]),
                mcp_deferred_loading: false,
                created_at: Utc::now(),
                last_used: None,
                strategy_id: "default".to_string(),
                roots: None,
                mcp_sampling_enabled: false,
                mcp_sampling_requires_approval: true,
                mcp_sampling_max_tokens: None,
                mcp_sampling_rate_limit: None,
                firewall: FirewallRules::default(),
                skills_access: lr_config::SkillsAccess::None,
                marketplace_enabled: false,
                mcp_permissions: McpPermissions::default(),
                skills_permissions: SkillsPermissions::default(),
                model_permissions: ModelPermissions::default(),
                marketplace_permission: PermissionState::Off,
                client_mode: lr_config::ClientMode::default(),
                template_id: None,
                sync_config: false,
                guardrails_enabled: None,
            },
            Client {
                id: "no_mcp_client".to_string(),
                name: "No MCP Client".to_string(),
                enabled: true,
                allowed_llm_providers: vec![],
                mcp_server_access: McpServerAccess::None,
                mcp_deferred_loading: false,
                created_at: Utc::now(),
                last_used: None,
                strategy_id: "default".to_string(),
                roots: None,
                mcp_sampling_enabled: false,
                mcp_sampling_requires_approval: true,
                mcp_sampling_max_tokens: None,
                mcp_sampling_rate_limit: None,
                firewall: FirewallRules::default(),
                skills_access: lr_config::SkillsAccess::None,
                marketplace_enabled: false,
                mcp_permissions: McpPermissions::default(),
                skills_permissions: SkillsPermissions::default(),
                model_permissions: ModelPermissions::default(),
                marketplace_permission: PermissionState::Off,
                client_mode: lr_config::ClientMode::default(),
                template_id: None,
                sync_config: false,
                guardrails_enabled: None,
            },
        ];
        config
    }

    #[test]
    fn test_find_client_by_id() {
        let config = test_config();

        // Find existing enabled client
        let client = find_client_by_id("test_client", &config).unwrap();
        assert_eq!(client.id, "test_client");

        // Disabled client should error
        let result = find_client_by_id("disabled_client", &config);
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("disabled"));

        // Non-existent client should error
        let result = find_client_by_id("nonexistent", &config);
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("not found"));
    }

    #[test]
    fn test_find_first_enabled_client() {
        let config = test_config();

        // Should find test_client (first enabled with MCP servers)
        let client = find_first_enabled_client(&config).unwrap();
        assert_eq!(client.id, "test_client");

        // Empty config should error
        let mut empty_config = AppConfig::default();
        empty_config.clients = vec![];
        let result = find_first_enabled_client(&empty_config);
        assert!(result.is_err());
    }
}
