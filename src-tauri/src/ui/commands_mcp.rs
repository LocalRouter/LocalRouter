//! MCP-related Tauri command handlers
//!
//! MCP server management, health checks, OAuth browser flows, and inline OAuth flows.

use std::sync::Arc;

use lr_api_keys::keychain_trait::KeychainStorage;
use lr_config::{
    ConfigManager, McpAuthConfig, McpServerConfig, McpTransportConfig, McpTransportType,
};
use lr_mcp::McpServerManager;
use lr_server::ServerManager;
use serde::{Deserialize, Serialize};
use tauri::{Emitter, State};

// ============================================================================
// MCP Server Commands
// ============================================================================

/// Probe an unsaved MCP connection; does not save a server or begin OAuth login.
#[tauri::command]
pub async fn discover_mcp_connection(
    target: String,
    transport_override: Option<String>,
    headers: std::collections::HashMap<String, String>,
    env: std::collections::HashMap<String, String>,
    cwd: Option<String>,
    oauth_manager: State<'_, Arc<lr_mcp::oauth::McpOAuthManager>>,
) -> Result<lr_mcp::discovery::McpConnectionDiscovery, String> {
    tokio::time::timeout(
        std::time::Duration::from_secs(45),
        lr_mcp::discovery::discover_connection(
            &target,
            transport_override.as_deref(),
            headers,
            env,
            cwd,
            &oauth_manager,
        ),
    )
    .await
    .map_err(|_| {
        "Discovery timed out. You can configure the server manually or retry with credentials."
            .to_string()
    })?
    .map_err(|e| e.to_string())
}

/// Frontend auth config format (with raw secrets)
#[derive(Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum FrontendAuthConfig {
    None,
    BearerToken {
        token: String,
    },
    CustomHeaders {
        headers: std::collections::HashMap<String, String>,
    },
    #[serde(alias = "oauth")]
    OAuth {
        client_id: String,
        client_secret: String,
        auth_url: String,
        token_url: String,
        scopes: Vec<String>,
    },
    EnvVars {
        env: std::collections::HashMap<String, String>,
    },
    /// OAuth with browser-based authorization code flow (PKCE)
    /// Initially a placeholder - full OAuth details configured during first auth
    #[serde(rename = "oauth_browser")]
    OAuthBrowser {
        /// OAuth client ID (optional - can be configured later)
        #[serde(default)]
        client_id: Option<String>,
        /// Client secret (optional - can be configured later)
        #[serde(default)]
        client_secret: Option<String>,
        /// Authorization endpoint URL (optional - can be auto-discovered)
        #[serde(default)]
        auth_url: Option<String>,
        /// Token endpoint URL (optional - can be auto-discovered)
        #[serde(default)]
        token_url: Option<String>,
        /// OAuth scopes to request
        #[serde(default)]
        scopes: Vec<String>,
        /// Authorization server issuer identifier from discovery (RFC 8414),
        /// used for RFC 9207 iss validation
        #[serde(default)]
        issuer: Option<String>,
        /// Redirect URI (defaults to http://localhost:8080/callback)
        #[serde(default)]
        redirect_uri: Option<String>,
    },
}

impl std::fmt::Debug for FrontendAuthConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            FrontendAuthConfig::None => write!(f, "None"),
            FrontendAuthConfig::BearerToken { .. } => {
                write!(f, "BearerToken {{ token: [REDACTED] }}")
            }
            FrontendAuthConfig::CustomHeaders { .. } => {
                write!(f, "CustomHeaders {{ headers: [REDACTED] }}")
            }
            FrontendAuthConfig::OAuth { .. } => write!(f, "OAuth {{ [REDACTED] }}"),
            FrontendAuthConfig::EnvVars { .. } => write!(f, "EnvVars {{ env: [REDACTED] }}"),
            FrontendAuthConfig::OAuthBrowser { .. } => write!(f, "OAuthBrowser {{ [REDACTED] }}"),
        }
    }
}

/// Process frontend auth config and store secrets in keychain
/// Returns the backend McpAuthConfig with keychain references
fn process_auth_config(
    server_id: &str,
    auth_cfg: Option<serde_json::Value>,
    existing: Option<&McpAuthConfig>,
) -> Result<Option<McpAuthConfig>, String> {
    let Some(auth_value) = auth_cfg.filter(|value| !value.is_null()) else {
        tracing::debug!("No auth config provided to process_auth_config");
        return Ok(None);
    };

    tracing::debug!("Processing auth config");

    // Parse frontend format
    let frontend_auth: FrontendAuthConfig =
        serde_json::from_value(auth_value.clone()).map_err(|e| {
            tracing::error!("Failed to deserialize frontend auth config: {}", e);
            tracing::error!("Failed to parse auth config");
            format!("Invalid auth config format: {}", e)
        })?;

    tracing::debug!("Parsed frontend auth: {:?}", frontend_auth);

    // Convert to backend format, storing secrets in keychain
    let backend_auth = match frontend_auth {
        FrontendAuthConfig::None => return Ok(None),
        FrontendAuthConfig::BearerToken { token } => {
            // Store token in keychain
            let keychain = lr_api_keys::CachedKeychain::auto()
                .map_err(|e| format!("Failed to access keychain: {}", e))?;

            let key = lr_config::mcp_bearer_token_account(server_id);
            keychain
                .store(lr_config::MCP_KEYRING_SERVICE, &key, &token)
                .map_err(|e| format!("Failed to store token in keychain: {}", e))?;

            tracing::debug!("Stored bearer token in keychain with key: {}", key);

            McpAuthConfig::BearerToken { token_ref: key }
        }
        FrontendAuthConfig::CustomHeaders { headers } => {
            let previous = match existing {
                Some(McpAuthConfig::CustomHeaders { header_refs }) => Some(header_refs),
                _ => None,
            };
            // Validate the entire update before writing any secret. Never echo values.
            let mut names = std::collections::HashSet::new();
            for (name, value) in &headers {
                if reqwest::header::HeaderName::from_bytes(name.as_bytes()).is_err()
                    || !names.insert(name.to_ascii_lowercase())
                {
                    return Err("Enter unique, valid custom header names".into());
                }
                if reqwest::header::HeaderValue::from_str(value).is_err() {
                    return Err("Invalid custom header value".into());
                }
            }
            let mut header_refs = std::collections::HashMap::new();
            for (name, value) in headers {
                // Public config exposes only names/references. Blank retained values
                // preserve saved secrets; omitted names remove their headers.
                if value.is_empty() {
                    if let Some(old_ref) = previous.and_then(|refs| {
                        refs.iter()
                            .find(|(key, _)| key.eq_ignore_ascii_case(&name))
                            .map(|(_, value)| value)
                    }) {
                        header_refs.insert(name, old_ref.clone());
                        continue;
                    }
                }
                let keychain = lr_api_keys::CachedKeychain::auto()
                    .map_err(|e| format!("Failed to access keychain: {}", e))?;
                let ref_key = uuid::Uuid::new_v4().to_string();
                keychain
                    .store(lr_config::MCP_KEYRING_SERVICE, &ref_key, &value)
                    .map_err(|_| "Failed to store custom header in keychain".to_string())?;
                header_refs.insert(name, ref_key);
            }
            McpAuthConfig::CustomHeaders { header_refs }
        }
        FrontendAuthConfig::OAuth {
            client_id,
            client_secret,
            auth_url,
            token_url,
            scopes,
        } => {
            let key = if client_secret.is_empty() {
                match existing {
                    Some(McpAuthConfig::OAuth {
                        client_secret_ref, ..
                    }) => client_secret_ref.clone(),
                    _ => return Err("Client Secret is required for OAuth".into()),
                }
            } else {
                let keychain = lr_api_keys::CachedKeychain::auto()
                    .map_err(|e| format!("Failed to access keychain: {}", e))?;
                let key = format!("{}_oauth_secret", server_id);
                keychain
                    .store(lr_config::MCP_KEYRING_SERVICE, &key, &client_secret)
                    .map_err(|e| format!("Failed to store OAuth secret in keychain: {}", e))?;
                key
            };

            McpAuthConfig::OAuth {
                client_id,
                client_secret_ref: key,
                auth_url,
                token_url,
                scopes,
            }
        }
        FrontendAuthConfig::EnvVars { env } => {
            // Store each env var value in keychain with a generated UUID reference
            let keychain = lr_api_keys::CachedKeychain::auto()
                .map_err(|e| format!("Failed to access keychain: {}", e))?;

            let mut env_refs = std::collections::HashMap::new();
            for (name, value) in env {
                let ref_key = uuid::Uuid::new_v4().to_string();
                keychain
                    .store(lr_config::MCP_KEYRING_SERVICE, &ref_key, &value)
                    .map_err(|e| {
                        format!("Failed to store env var '{}' in keychain: {}", name, e)
                    })?;
                env_refs.insert(name, ref_key);
            }

            tracing::debug!(
                "Stored {} env var(s) in keychain for server {}",
                env_refs.len(),
                server_id
            );

            McpAuthConfig::EnvVars { env_refs }
        }
        FrontendAuthConfig::OAuthBrowser {
            client_id,
            client_secret,
            auth_url,
            token_url,
            scopes,
            issuer,
            redirect_uri,
        } => {
            // Store client secret in keychain if provided
            let secret_ref = if let Some(secret) = client_secret {
                let keychain = lr_api_keys::CachedKeychain::auto()
                    .map_err(|e| format!("Failed to access keychain: {}", e))?;

                // A new reference prevents a running connection from reusing a cached
                // registered-app secret after the user replaces it.
                let key = format!(
                    "{}_oauth_browser_secret_{}",
                    server_id,
                    uuid::Uuid::new_v4()
                );
                keychain
                    .store(lr_config::MCP_KEYRING_SERVICE, &key, &secret)
                    .map_err(|e| format!("Failed to store OAuth secret in keychain: {}", e))?;

                tracing::debug!("Stored OAuth browser secret in keychain with key: {}", key);
                key
            } else {
                match existing {
                    Some(McpAuthConfig::OAuthBrowser {
                        client_secret_ref, ..
                    }) => client_secret_ref.clone(),
                    // A fresh browser flow without a secret must not reuse a
                    // stale registered-app secret after switching auth modes.
                    _ => format!(
                        "{}_oauth_browser_secret_{}",
                        server_id,
                        uuid::Uuid::new_v4()
                    ),
                }
            };

            McpAuthConfig::OAuthBrowser {
                client_id: client_id.unwrap_or_default(),
                client_secret_ref: secret_ref,
                auth_url: auth_url.unwrap_or_default(),
                token_url: token_url.unwrap_or_default(),
                scopes,
                redirect_uri: redirect_uri
                    .unwrap_or_else(|| "http://localhost:8080/callback".to_string()),
                issuer,
            }
        }
    };

    tracing::info!(
        "✅ Successfully processed auth config for server {}: {:?}",
        server_id,
        backend_auth
    );

    Ok(Some(backend_auth))
}

/// MCP server information for display
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct McpServerInfo {
    pub id: String,
    pub name: String,
    pub transport: String,
    pub transport_config: McpTransportConfig,
    pub auth_config: Option<McpAuthConfig>,
    pub enabled: bool,
    pub running: bool,
    pub created_at: String,
    /// The individual proxy endpoint URL for this server (e.g., http://localhost:3625/mcp/{server_id})
    pub proxy_url: String,
    /// The unified MCP gateway URL (always available at http://localhost:3625/)
    pub gateway_url: String,
    /// Legacy field for backward compatibility (deprecated, use proxy_url instead)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
}

/// List all MCP servers
#[tauri::command]
pub async fn list_mcp_servers(
    mcp_manager: State<'_, Arc<McpServerManager>>,
    server_manager: State<'_, Arc<ServerManager>>,
) -> Result<Vec<McpServerInfo>, String> {
    let configs = mcp_manager.list_configs();
    let mut servers = Vec::new();

    // Get the actual server port
    let port = server_manager.get_actual_port().unwrap_or(3625);
    let base_url = format!("http://localhost:{}", port);

    for config in configs {
        // All servers get a proxy URL at /mcp/{server_id}
        let proxy_url = format!("{}/mcp/{}", base_url, config.id);

        // Unified gateway URL is always at root
        let gateway_url = base_url.clone();

        // Legacy URL field for backward compatibility (deprecated)
        let url = Some(proxy_url.clone());

        servers.push(McpServerInfo {
            id: config.id.clone(),
            name: config.name.clone(),
            transport: format!("{:?}", config.transport),
            transport_config: config.transport_config.clone(),
            auth_config: config.auth_config.clone(),
            enabled: config.enabled,
            running: mcp_manager.is_running(&config.id),
            created_at: config.created_at.to_rfc3339(),
            proxy_url,
            gateway_url,
            url,
        });
    }

    Ok(servers)
}

/// Create a new MCP server
///
/// # Arguments
/// * `name` - Server name
/// * `transport` - Transport type ("stdio", "sse", "websocket")
/// * `transport_config` - Transport-specific configuration as JSON
///
/// # Returns
/// * The created server info
#[tauri::command]
pub async fn create_mcp_server(
    name: String,
    transport: String,
    transport_config: serde_json::Value,
    auth_config: Option<serde_json::Value>,
    mcp_manager: State<'_, Arc<McpServerManager>>,
    server_manager: State<'_, Arc<ServerManager>>,
    config_manager: State<'_, ConfigManager>,
    app: tauri::AppHandle,
) -> Result<McpServerInfo, String> {
    tracing::info!("Creating new MCP server: {} ({})", name, transport);

    // Parse transport type (case-insensitive)
    #[allow(deprecated)]
    let transport_type = match transport.to_lowercase().as_str() {
        "stdio" => McpTransportType::Stdio,
        "sse" | "httpsse" | "http_sse" => McpTransportType::HttpSse,
        _ => return Err(format!("Invalid transport type: {}", transport)),
    };

    // Parse transport config
    let parsed_config: McpTransportConfig = serde_json::from_value(transport_config)
        .map_err(|e| format!("Invalid transport config: {}", e))?;

    // Create server config (need ID for auth processing)
    let mut config = McpServerConfig::new(name, transport_type, parsed_config);

    // Parse auth config (if provided) and store secrets in keychain
    config.auth_config = process_auth_config(&config.id, auth_config, None)?;

    // Get the actual server port
    let port = server_manager.get_actual_port().unwrap_or(3625);
    let base_url = format!("http://localhost:{}", port);

    // All servers get a proxy URL at /mcp/{server_id}
    let proxy_url = format!("{}/mcp/{}", base_url, config.id);

    // Unified gateway URL is always at root
    let gateway_url = base_url.clone();

    // Legacy URL field for backward compatibility (deprecated)
    let url = Some(proxy_url.clone());

    let server_info = McpServerInfo {
        id: config.id.clone(),
        name: config.name.clone(),
        transport: format!("{:?}", config.transport),
        transport_config: config.transport_config.clone(),
        auth_config: config.auth_config.clone(),
        enabled: config.enabled,
        running: false,
        created_at: config.created_at.to_rfc3339(),
        proxy_url,
        gateway_url,
        url,
    };

    // Add to manager
    mcp_manager.add_config(config.clone());

    // Save to config file
    config_manager
        .update(|cfg| {
            cfg.mcp_servers.push(config);
        })
        .map_err(|e| e.to_string())?;

    // Persist to disk
    config_manager.save().await.map_err(|e| e.to_string())?;

    // Rebuild tray menu
    if let Err(e) = crate::ui::tray::rebuild_tray_menu(&app) {
        tracing::error!("Failed to rebuild tray menu: {}", e);
    }

    // Notify frontend
    let _ = app.emit("mcp-servers-changed", ());

    Ok(server_info)
}

/// Delete an MCP server
///
/// # Arguments
/// * `server_id` - The server ID to delete
///
/// # Returns
/// * Ok(()) if successful
#[tauri::command]
pub async fn delete_mcp_server(
    server_id: String,
    mcp_manager: State<'_, Arc<McpServerManager>>,
    config_manager: State<'_, ConfigManager>,
    app_state: State<'_, Arc<lr_server::state::AppState>>,
    app: tauri::AppHandle,
) -> Result<(), String> {
    tracing::info!("Deleting MCP server: {}", server_id);

    // Stop if running
    if mcp_manager.is_running(&server_id) {
        mcp_manager
            .stop_server(&server_id)
            .await
            .map_err(|e| e.to_string())?;
    }

    // Remove from manager
    mcp_manager.remove_config(&server_id);

    // Remove from config file
    config_manager
        .update(|cfg| {
            cfg.mcp_servers.retain(|s| s.id != server_id);
        })
        .map_err(|e| e.to_string())?;

    // Persist to disk
    config_manager.save().await.map_err(|e| e.to_string())?;

    // Remove from health cache (this emits health-status-changed event)
    app_state.health_cache.remove_mcp_server(&server_id);

    // Rebuild tray menu
    if let Err(e) = crate::ui::tray::rebuild_tray_menu(&app) {
        tracing::error!("Failed to rebuild tray menu: {}", e);
    }

    // Notify frontend
    let _ = app.emit("mcp-servers-changed", ());

    Ok(())
}

/// Clone an existing MCP server (deep copy with new identity)
///
/// Creates a new MCP server that is a copy of the source server, with a new UUID,
/// a "Clone of {name}" name, and independent keychain entries for all secrets.
///
/// # Arguments
/// * `server_id` - The source server ID to clone
///
/// # Returns
/// * The cloned server info
#[tauri::command]
pub async fn clone_mcp_server(
    server_id: String,
    config_manager: State<'_, ConfigManager>,
    mcp_manager: State<'_, Arc<McpServerManager>>,
    server_manager: State<'_, Arc<ServerManager>>,
    app: tauri::AppHandle,
) -> Result<McpServerInfo, String> {
    tracing::info!("Cloning MCP server: {}", server_id);

    let config = config_manager.get();

    // Find the source server
    let source = config
        .mcp_servers
        .iter()
        .find(|s| s.id == server_id)
        .ok_or_else(|| format!("MCP server not found: {}", server_id))?
        .clone();

    // Generate clone name
    let existing_names: Vec<&str> = config.mcp_servers.iter().map(|s| s.name.as_str()).collect();
    let clone_name = generate_clone_name(&source.name, &existing_names);

    let new_id = uuid::Uuid::new_v4().to_string();

    // Clone auth config with new keychain entries
    let new_auth_config = clone_mcp_auth_config(&source.auth_config, &server_id, &new_id)?;

    let new_server = McpServerConfig {
        id: new_id.clone(),
        name: clone_name,
        transport: source.transport,
        transport_config: source.transport_config.clone(),
        auth_config: new_auth_config,
        discovered_oauth: source.discovered_oauth.clone(),
        oauth_config: source.oauth_config.clone(),
        enabled: source.enabled,
        created_at: chrono::Utc::now(),
        capability_snapshot: None,
    };

    // Add to config
    config_manager
        .update(|cfg| {
            cfg.mcp_servers.push(new_server.clone());
        })
        .map_err(|e| e.to_string())?;

    // Add to manager
    mcp_manager.add_config(new_server.clone());

    // Persist to disk
    config_manager.save().await.map_err(|e| e.to_string())?;

    // Rebuild tray menu
    if let Err(e) = crate::ui::tray::rebuild_tray_menu(&app) {
        tracing::error!("Failed to rebuild tray menu: {}", e);
    }

    // Emit event
    if let Err(e) = app.emit("mcp-servers-changed", ()) {
        tracing::error!("Failed to emit mcp-servers-changed event: {}", e);
    }

    // Build proxy/gateway URLs
    let port = server_manager.get_actual_port().unwrap_or(3625);
    let base_url = format!("http://localhost:{}", port);
    let proxy_url = format!("{}/mcp/{}", base_url, new_id);
    let gateway_url = base_url;

    Ok(McpServerInfo {
        id: new_server.id,
        name: new_server.name,
        enabled: new_server.enabled,
        transport: format!("{:?}", new_server.transport),
        transport_config: new_server.transport_config,
        auth_config: new_server.auth_config,
        running: false,
        created_at: new_server.created_at.to_rfc3339(),
        url: Some(proxy_url.clone()),
        proxy_url,
        gateway_url,
    })
}

/// Clone MCP auth config, re-keying secrets to new keychain entries
fn clone_mcp_auth_config(
    auth: &Option<McpAuthConfig>,
    old_server_id: &str,
    new_server_id: &str,
) -> Result<Option<McpAuthConfig>, String> {
    let auth = match auth {
        Some(a) => a,
        None => return Ok(None),
    };

    let keychain = lr_api_keys::CachedKeychain::auto()
        .map_err(|e| format!("Failed to access keychain: {}", e))?;

    match auth {
        McpAuthConfig::None => Ok(Some(McpAuthConfig::None)),

        McpAuthConfig::BearerToken { token_ref } => {
            let new_ref = lr_config::mcp_bearer_token_account(new_server_id);
            // Pre-0.0.155 marketplace installs: the token is under the bare id.
            let old_ref = if token_ref == lr_config::LEGACY_PENDING_BEARER_TOKEN_REF {
                old_server_id
            } else {
                token_ref.as_str()
            };
            if let Ok(Some(token)) = keychain.get(lr_config::MCP_KEYRING_SERVICE, old_ref) {
                keychain
                    .store(lr_config::MCP_KEYRING_SERVICE, &new_ref, &token)
                    .map_err(|e| format!("Failed to store cloned bearer token: {}", e))?;
            }
            Ok(Some(McpAuthConfig::BearerToken { token_ref: new_ref }))
        }

        McpAuthConfig::CustomHeaders { header_refs } => {
            let mut new_refs = std::collections::HashMap::new();
            for (name, old_ref) in header_refs {
                let new_ref = uuid::Uuid::new_v4().to_string();
                if let Ok(Some(value)) = keychain.get(lr_config::MCP_KEYRING_SERVICE, old_ref) {
                    keychain
                        .store(lr_config::MCP_KEYRING_SERVICE, &new_ref, &value)
                        .map_err(|e| format!("Failed to store cloned header '{}': {}", name, e))?;
                }
                new_refs.insert(name.clone(), new_ref);
            }
            Ok(Some(McpAuthConfig::CustomHeaders {
                header_refs: new_refs,
            }))
        }

        McpAuthConfig::OAuth {
            client_id,
            client_secret_ref,
            auth_url,
            token_url,
            scopes,
        } => {
            let new_ref = format!("{}_oauth_secret", new_server_id);
            if let Ok(Some(secret)) =
                keychain.get(lr_config::MCP_KEYRING_SERVICE, client_secret_ref)
            {
                keychain
                    .store(lr_config::MCP_KEYRING_SERVICE, &new_ref, &secret)
                    .map_err(|e| format!("Failed to store cloned OAuth secret: {}", e))?;
            }
            Ok(Some(McpAuthConfig::OAuth {
                client_id: client_id.clone(),
                client_secret_ref: new_ref,
                auth_url: auth_url.clone(),
                token_url: token_url.clone(),
                scopes: scopes.clone(),
            }))
        }

        McpAuthConfig::OAuthBrowser {
            client_id,
            client_secret_ref,
            auth_url,
            token_url,
            scopes,
            redirect_uri,
            issuer,
        } => {
            let new_ref = format!("{}_oauth_browser_secret", new_server_id);
            if let Ok(Some(secret)) =
                keychain.get(lr_config::MCP_KEYRING_SERVICE, client_secret_ref)
            {
                keychain
                    .store(lr_config::MCP_KEYRING_SERVICE, &new_ref, &secret)
                    .map_err(|e| format!("Failed to store cloned OAuth browser secret: {}", e))?;
            }
            Ok(Some(McpAuthConfig::OAuthBrowser {
                client_id: client_id.clone(),
                client_secret_ref: new_ref,
                auth_url: auth_url.clone(),
                token_url: token_url.clone(),
                scopes: scopes.clone(),
                redirect_uri: redirect_uri.clone(),
                issuer: issuer.clone(),
            }))
        }

        McpAuthConfig::EnvVars { env_refs } => {
            let mut new_refs = std::collections::HashMap::new();
            for (name, old_ref) in env_refs {
                let new_ref = uuid::Uuid::new_v4().to_string();
                if let Ok(Some(value)) = keychain.get(lr_config::MCP_KEYRING_SERVICE, old_ref) {
                    keychain
                        .store(lr_config::MCP_KEYRING_SERVICE, &new_ref, &value)
                        .map_err(|e| format!("Failed to store cloned env var '{}': {}", name, e))?;
                }
                new_refs.insert(name.clone(), new_ref);
            }
            Ok(Some(McpAuthConfig::EnvVars { env_refs: new_refs }))
        }
    }
}

/// Generate a clone name with dedup suffix
fn generate_clone_name(original_name: &str, existing_names: &[&str]) -> String {
    let base = format!("Clone of {}", original_name);
    if !existing_names.contains(&base.as_str()) {
        return base;
    }
    let mut n = 2;
    loop {
        let candidate = format!("{} ({})", base, n);
        if !existing_names.contains(&candidate.as_str()) {
            return candidate;
        }
        n += 1;
    }
}

/// Health check result for streaming MCP health to frontend
#[derive(Clone, Serialize)]
pub struct McpHealthCheckResult {
    pub server_id: String,
    pub server_name: String,
    pub status: String,
    pub latency_ms: Option<u64>,
    pub error: Option<String>,
}

/// Start an MCP server
///
/// Used by the LocalRouter management MCP server (`admin_mcp::operations`).
pub async fn start_mcp_server(
    server_id: String,
    mcp_manager: State<'_, Arc<McpServerManager>>,
    app: tauri::AppHandle,
) -> Result<(), String> {
    tracing::info!("Starting MCP server: {}", server_id);

    mcp_manager
        .start_server(&server_id)
        .await
        .map_err(|e| e.to_string())?;

    // Notify frontend
    let _ = app.emit("mcp-servers-changed", ());

    Ok(())
}

/// Stop an MCP server
///
/// Used by the LocalRouter management MCP server (`admin_mcp::operations`).
pub async fn stop_mcp_server(
    server_id: String,
    mcp_manager: State<'_, Arc<McpServerManager>>,
    app: tauri::AppHandle,
) -> Result<(), String> {
    tracing::info!("Stopping MCP server: {}", server_id);

    mcp_manager
        .stop_server(&server_id)
        .await
        .map_err(|e| e.to_string())?;

    // Notify frontend
    let _ = app.emit("mcp-servers-changed", ());

    Ok(())
}

/// Start streaming health checks for all MCP servers
///
/// Emits "mcp-health-check" events as each server's health check completes.
/// Returns immediately with the list of server IDs being checked.
#[tauri::command]
pub async fn start_mcp_health_checks(
    app: tauri::AppHandle,
    mcp_manager: State<'_, Arc<McpServerManager>>,
    app_state: State<'_, Arc<lr_server::state::AppState>>,
) -> Result<Vec<String>, String> {
    let server_ids: Vec<String> = mcp_manager
        .list_configs()
        .iter()
        .map(|c| c.id.clone())
        .collect();

    let mcp_manager = mcp_manager.inner().clone();
    let app_handle = app.clone();
    let health_cache = app_state.health_cache.clone();

    // Spawn health checks for each server in parallel
    tokio::spawn(async move {
        let configs = mcp_manager.list_configs();
        let mut handles = Vec::new();

        for config in configs {
            let mcp_manager = mcp_manager.clone();
            let app_handle = app_handle.clone();
            let health_cache = health_cache.clone();
            let server_id = config.id.clone();
            let server_name = config.name.clone();
            let enabled = config.enabled;

            let handle = tokio::spawn(async move {
                // If server is disabled, emit disabled status without running health check
                if !enabled {
                    let result = McpHealthCheckResult {
                        server_id: server_id.clone(),
                        server_name: server_name.clone(),
                        status: "disabled".to_string(),
                        latency_ms: None,
                        error: None,
                    };
                    let _ = app_handle.emit("mcp-health-check", result);
                    health_cache.update_mcp_server(
                        &server_id,
                        lr_providers::health_cache::ItemHealth::disabled(server_name),
                    );
                    return;
                }

                let health = mcp_manager.get_server_health(&server_id).await;
                let status_str = match health.status {
                    lr_mcp::manager::HealthStatus::Healthy => "healthy",
                    lr_mcp::manager::HealthStatus::Ready => "ready",
                    lr_mcp::manager::HealthStatus::Unhealthy => "unhealthy",
                    lr_mcp::manager::HealthStatus::Unknown => "unknown",
                };
                let result = McpHealthCheckResult {
                    server_id: health.server_id.clone(),
                    server_name: health.server_name.clone(),
                    status: status_str.to_string(),
                    latency_ms: health.latency_ms,
                    error: health.error.clone(),
                };
                let _ = app_handle.emit("mcp-health-check", &result);

                // Update centralized health cache so aggregate status (tray + sidebar) recalculates
                use lr_providers::health_cache::ItemHealth;
                let item_health = match health.status {
                    lr_mcp::manager::HealthStatus::Ready => ItemHealth::ready(health.server_name),
                    lr_mcp::manager::HealthStatus::Healthy => {
                        ItemHealth::healthy(health.server_name, health.latency_ms)
                    }
                    lr_mcp::manager::HealthStatus::Unhealthy
                    | lr_mcp::manager::HealthStatus::Unknown => ItemHealth::unhealthy(
                        health.server_name,
                        health.error.unwrap_or_else(|| "Unhealthy".to_string()),
                    ),
                };
                health_cache.update_mcp_server(&result.server_id, item_health);
            });
            handles.push(handle);
        }

        // Wait for all health checks to complete
        for handle in handles {
            let _ = handle.await;
        }
    });

    Ok(server_ids)
}

/// Check health for a single MCP server
///
/// Emits "mcp-health-check" event when the health check completes.
#[tauri::command]
pub async fn check_single_mcp_health(
    app: tauri::AppHandle,
    mcp_manager: State<'_, Arc<McpServerManager>>,
    app_state: State<'_, Arc<lr_server::state::AppState>>,
    server_id: String,
) -> Result<(), String> {
    let mcp_manager = mcp_manager.inner().clone();
    let health_cache = app_state.health_cache.clone();
    let app_handle = app.clone();

    // Check if server is disabled
    let config = mcp_manager
        .get_config(&server_id)
        .ok_or_else(|| format!("Server not found: {}", server_id))?;

    if !config.enabled {
        let result = McpHealthCheckResult {
            server_id: config.id.clone(),
            server_name: config.name.clone(),
            status: "disabled".to_string(),
            latency_ms: None,
            error: None,
        };
        let _ = app_handle.emit("mcp-health-check", result);
        health_cache.update_mcp_server(
            &config.id,
            lr_providers::health_cache::ItemHealth::disabled(config.name),
        );
        return Ok(());
    }

    tokio::spawn(async move {
        let health = mcp_manager.get_server_health(&server_id).await;
        let result = McpHealthCheckResult {
            server_id: health.server_id.clone(),
            server_name: health.server_name.clone(),
            status: match health.status {
                lr_mcp::manager::HealthStatus::Healthy => "healthy".to_string(),
                lr_mcp::manager::HealthStatus::Ready => "ready".to_string(),
                lr_mcp::manager::HealthStatus::Unhealthy => "unhealthy".to_string(),
                lr_mcp::manager::HealthStatus::Unknown => "unknown".to_string(),
            },
            latency_ms: health.latency_ms,
            error: health.error.clone(),
        };
        let _ = app_handle.emit("mcp-health-check", result);

        // Update centralized health cache so aggregate status (tray + sidebar) recalculates
        use lr_providers::health_cache::ItemHealth;
        let item_health = match health.status {
            lr_mcp::manager::HealthStatus::Healthy => {
                ItemHealth::healthy(health.server_name, health.latency_ms)
            }
            lr_mcp::manager::HealthStatus::Ready => ItemHealth::ready(health.server_name),
            lr_mcp::manager::HealthStatus::Unhealthy => ItemHealth::unhealthy(
                health.server_name,
                health.error.unwrap_or_else(|| "Unhealthy".to_string()),
            ),
            lr_mcp::manager::HealthStatus::Unknown => ItemHealth::unhealthy(
                health.server_name,
                health.error.unwrap_or_else(|| "Unknown status".to_string()),
            ),
        };
        health_cache.update_mcp_server(&health.server_id, item_health);
    });

    Ok(())
}

/// Update an MCP server with partial updates
///
/// This command allows updating individual fields without requiring all fields.
/// Only the fields provided in the `updates` object will be modified.
///
/// # Arguments
/// * `server_id` - The server ID to update
/// * `updates` - JSON object with optional fields: name, transport_config, auth_config
///
/// # Returns
/// * Ok(()) if successful
#[tauri::command]
pub async fn update_mcp_server(
    server_id: String,
    updates: serde_json::Value,
    mcp_manager: State<'_, Arc<McpServerManager>>,
    config_manager: State<'_, ConfigManager>,
    app: tauri::AppHandle,
) -> Result<(), String> {
    tracing::info!("Updating MCP server with partial updates: {}", server_id);
    tracing::debug!("Processing MCP server update fields");

    let updates_obj = updates.as_object().ok_or("Updates must be a JSON object")?;

    // Extract optional update fields
    let name_update = updates_obj.get("name").and_then(|v| v.as_str());
    let transport_config_update = updates_obj.get("transport_config");
    let auth_config_update = updates_obj.get("auth_config");

    // Validate name if provided
    if let Some(name) = name_update {
        if name.trim().is_empty() {
            return Err("MCP server name cannot be empty".to_string());
        }
    }

    // Parse transport config if provided
    let parsed_transport_config = if let Some(tc) = transport_config_update {
        Some(
            serde_json::from_value::<McpTransportConfig>(tc.clone()).map_err(|e| {
                tracing::error!("Failed to parse transport config: {}", e);
                format!("Invalid transport config: {}", e)
            })?,
        )
    } else {
        None
    };

    // Parse auth config if provided and store secrets in keychain
    let parsed_auth_config = if auth_config_update.is_some() {
        process_auth_config(
            &server_id,
            auth_config_update.cloned(),
            config_manager
                .get()
                .mcp_servers
                .iter()
                .find(|s| s.id == server_id)
                .and_then(|s| s.auth_config.as_ref()),
        )?
    } else {
        None
    };

    // Update in config file
    config_manager
        .update(|cfg| {
            if let Some(server) = cfg.mcp_servers.iter_mut().find(|s| s.id == server_id) {
                // Update name if provided
                if let Some(name) = name_update {
                    server.name = name.to_string();
                }
                // Update transport config if provided
                if let Some(ref tc) = parsed_transport_config {
                    server.transport_config = tc.clone();
                }
                // Update auth config if provided (even if it's None to clear it)
                if auth_config_update.is_some() {
                    server.auth_config = parsed_auth_config.clone();
                }
            }
        })
        .map_err(|e| e.to_string())?;

    // Update in manager
    if let Some(config) = config_manager
        .get()
        .mcp_servers
        .iter()
        .find(|s| s.id == server_id)
        .cloned()
    {
        mcp_manager.add_config(config);
    }

    // Persist to disk
    config_manager.save().await.map_err(|e| e.to_string())?;

    // Rebuild tray menu
    if let Err(e) = crate::ui::tray::rebuild_tray_menu(&app) {
        tracing::error!("Failed to rebuild tray menu: {}", e);
    }

    // Notify frontend
    let _ = app.emit("mcp-servers-changed", ());

    Ok(())
}

/// Toggle an MCP server's enabled state
///
/// # Arguments
/// * `server_id` - The server ID to toggle
/// * `enabled` - Whether to enable (true) or disable (false)
///
/// # Returns
/// * Ok(()) if successful
#[tauri::command]
pub async fn toggle_mcp_server_enabled(
    server_id: String,
    enabled: bool,
    mcp_manager: State<'_, Arc<McpServerManager>>,
    config_manager: State<'_, ConfigManager>,
    app_state: State<'_, Arc<lr_server::state::AppState>>,
    app: tauri::AppHandle,
) -> Result<(), String> {
    // Get server name before updating (needed for health cache)
    let server_name = mcp_manager
        .list_configs()
        .iter()
        .find(|c| c.id == server_id)
        .map(|c| c.name.clone())
        .unwrap_or_else(|| server_id.clone());

    // Update in MCP manager (in-memory)
    mcp_manager.set_config_enabled(&server_id, enabled);

    // Update in config file
    config_manager
        .update(|cfg| {
            if let Some(server) = cfg.mcp_servers.iter_mut().find(|s| s.id == server_id) {
                server.enabled = enabled;
            }
        })
        .map_err(|e| e.to_string())?;

    // Persist to disk
    config_manager.save().await.map_err(|e| e.to_string())?;

    // Update health cache immediately - this recalculates aggregate status and emits event
    use lr_providers::health_cache::ItemHealth;
    if enabled {
        // Set to pending, then trigger a health check for this MCP server
        app_state
            .health_cache
            .update_mcp_server(&server_id, ItemHealth::pending(server_name.clone()));

        // Spawn background health check for this MCP server
        let health_cache = app_state.health_cache.clone();
        let mcp_manager = app_state.mcp_server_manager.clone();
        let id = server_id.clone();
        tokio::spawn(async move {
            let mcp_server_health = mcp_manager.get_server_health(&id).await;
            use lr_mcp::manager::HealthStatus as McpHealthStatus;
            let server_name = mcp_server_health.server_name.clone();
            let item_health = match mcp_server_health.status {
                McpHealthStatus::Ready => ItemHealth::ready(server_name),
                McpHealthStatus::Healthy => {
                    ItemHealth::healthy(server_name, mcp_server_health.latency_ms)
                }
                McpHealthStatus::Unhealthy | McpHealthStatus::Unknown => ItemHealth::unhealthy(
                    server_name,
                    mcp_server_health
                        .error
                        .unwrap_or_else(|| "Unhealthy".to_string()),
                ),
            };
            health_cache.update_mcp_server(&id, item_health);
        });
    } else {
        // Set to disabled immediately
        app_state
            .health_cache
            .update_mcp_server(&server_id, ItemHealth::disabled(server_name));
    }

    // Rebuild tray menu
    if let Err(e) = crate::ui::tray::rebuild_tray_menu(&app) {
        tracing::error!("Failed to rebuild tray menu: {}", e);
    }

    // Notify frontend
    let _ = app.emit("mcp-servers-changed", ());

    Ok(())
}

// ============================================================================
// MCP OAuth Browser Flow Commands
// ============================================================================

/// Start a browser-based OAuth flow for an MCP server
///
/// # Arguments
/// * `server_id` - MCP server ID
///
/// # Returns
/// * OAuth flow result with authorization URL to open in browser
#[tauri::command]
pub async fn start_mcp_oauth_browser_flow(
    server_id: String,
    oauth_browser_manager: State<'_, Arc<lr_mcp::oauth_browser::McpOAuthBrowserManager>>,
    mcp_manager: State<'_, Arc<McpServerManager>>,
    oauth_manager: State<'_, Arc<lr_mcp::oauth::McpOAuthManager>>,
    config_manager: State<'_, ConfigManager>,
) -> Result<lr_mcp::oauth_browser::OAuthBrowserFlowResult, String> {
    // Get server config
    let config = config_manager.get();
    let server = config
        .mcp_servers
        .iter()
        .find(|s| s.id == server_id)
        .ok_or_else(|| format!("MCP server not found: {}", server_id))?;

    // Get auth config
    let auth_config = server
        .auth_config
        .as_ref()
        .ok_or_else(|| format!("No auth config for server: {}", server_id))?;

    let resource_url = match &server.transport_config {
        McpTransportConfig::Sse { url, .. } | McpTransportConfig::HttpSse { url, .. } => {
            url.clone()
        }
        _ => return Err("Browser OAuth requires an HTTP MCP server".into()),
    };
    let prepared = oauth_manager
        .prepare_browser_config(&server_id, &resource_url, auth_config)
        .await
        .map_err(|e| e.to_string())?;
    config_manager
        .update(|cfg| {
            if let Some(server) = cfg.mcp_servers.iter_mut().find(|s| s.id == server_id) {
                server.auth_config = Some(prepared.clone());
            }
        })
        .map_err(|e| e.to_string())?;
    config_manager.save().await.map_err(|e| e.to_string())?;
    mcp_manager.add_config({
        let mut server = server.clone();
        server.auth_config = Some(prepared.clone());
        server
    });
    oauth_browser_manager
        .start_browser_flow(&server_id, &prepared, Some(&resource_url))
        .await
        .map_err(|e| e.to_string())
}

/// Poll the status of an OAuth browser flow
///
/// # Arguments
/// * `server_id` - MCP server ID
///
/// # Returns
/// * Current flow status (Pending, Success, Error, or Timeout)
#[tauri::command]
pub fn poll_mcp_oauth_browser_status(
    server_id: String,
    oauth_browser_manager: State<'_, Arc<lr_mcp::oauth_browser::McpOAuthBrowserManager>>,
) -> Result<lr_mcp::oauth_browser::OAuthBrowserFlowStatus, String> {
    oauth_browser_manager
        .poll_flow_status(&server_id)
        .map_err(|e| e.to_string())
}

/// Cancel an active OAuth browser flow
///
/// # Arguments
/// * `server_id` - MCP server ID
#[tauri::command]
pub fn cancel_mcp_oauth_browser_flow(
    server_id: String,
    oauth_browser_manager: State<'_, Arc<lr_mcp::oauth_browser::McpOAuthBrowserManager>>,
) -> Result<(), String> {
    oauth_browser_manager
        .cancel_flow(&server_id)
        .map_err(|e| e.to_string())
}

/// Discover OAuth endpoints for an MCP server
///
/// This uses the existing McpOAuthManager's discover_oauth function to find
/// OAuth configuration via .well-known/oauth-protected-resource endpoint.
///
/// # Arguments
/// * `base_url` - Base URL of the MCP server (e.g., "https://api.github.com")
///
/// # Returns
/// * OAuth discovery information (auth_url, token_url, scopes)
#[tauri::command]
pub async fn discover_mcp_oauth_endpoints(
    base_url: String,
    oauth_manager: State<'_, Arc<lr_mcp::oauth::McpOAuthManager>>,
) -> Result<Option<lr_config::McpOAuthDiscovery>, String> {
    match oauth_manager.discover_oauth(&base_url).await {
        Ok(Some(discovery)) => Ok(Some(lr_config::McpOAuthDiscovery {
            auth_url: discovery.auth_url,
            token_url: discovery.token_endpoint,
            scopes_supported: discovery.scopes_supported,
            grant_types_supported: discovery.grant_types_supported,
            discovered_at: chrono::Utc::now(),
        })),
        Ok(None) => Ok(None),
        Err(e) => Err(e.to_string()),
    }
}

/// Test OAuth connection for an MCP server
///
/// For browser OAuth, obtains a usable access token from the keychain,
/// refreshing it with the stored refresh token when expired — the in-memory
/// token cache is empty after a restart and stale after expiry, so it can't
/// answer this.
///
/// # Arguments
/// * `server_id` - MCP server ID
///
/// # Returns
/// * `true` if server has valid authentication, `false` otherwise
#[tauri::command]
pub async fn test_mcp_oauth_connection(
    server_id: String,
    mcp_manager: State<'_, Arc<McpServerManager>>,
    oauth_browser_manager: State<'_, Arc<lr_mcp::oauth_browser::McpOAuthBrowserManager>>,
) -> Result<bool, String> {
    let config = mcp_manager
        .get_config(&server_id)
        .ok_or_else(|| format!("MCP server not found: {}", server_id))?;
    let resource_url = match &config.transport_config {
        McpTransportConfig::Sse { url, .. } | McpTransportConfig::HttpSse { url, .. } => {
            Some(url.as_str())
        }
        _ => None,
    };
    match (&config.auth_config, resource_url) {
        (Some(auth @ lr_config::McpAuthConfig::OAuthBrowser { .. }), Some(url)) => {
            match mcp_manager
                .oauth_manager()
                .get_browser_token(&server_id, auth, url)
                .await
            {
                Ok(_) => Ok(true),
                Err(e) => {
                    tracing::info!("OAuth check for MCP server {} failed: {}", server_id, e);
                    Ok(false)
                }
            }
        }
        _ => Ok(oauth_browser_manager.has_valid_auth(&server_id).await),
    }
}

/// Revoke OAuth tokens for an MCP server
///
/// Clears all stored tokens (access, refresh, and client secret) from keychain
///
/// # Arguments
/// * `server_id` - MCP server ID
#[tauri::command]
pub fn revoke_mcp_oauth_tokens(
    server_id: String,
    oauth_browser_manager: State<'_, Arc<lr_mcp::oauth_browser::McpOAuthBrowserManager>>,
) -> Result<(), String> {
    oauth_browser_manager
        .revoke_tokens(&server_id)
        .map_err(|e| e.to_string())
}

// ============================================================================
// Inline OAuth Flow Commands (for MCP server creation)
// ============================================================================

// ============================================================================
// MCP Server Capabilities (for permission tree UI)
// ============================================================================

/// Tool information for the permission tree
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct McpToolInfo {
    pub name: String,
    pub description: Option<String>,
}

/// Resource information for the permission tree
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct McpResourceInfo {
    pub uri: String,
    pub name: String,
    pub description: Option<String>,
}

/// Prompt information for the permission tree
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct McpPromptInfo {
    pub name: String,
    pub description: Option<String>,
}

/// MCP server capabilities (tools, resources, prompts)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct McpServerCapabilities {
    pub tools: Vec<McpToolInfo>,
    pub resources: Vec<McpResourceInfo>,
    pub prompts: Vec<McpPromptInfo>,
}

/// Get capabilities (tools, resources, prompts) for an MCP server
///
/// Used by the permission tree UI to display available items for each server.
/// Starts the server if not running. If listing fails on an already-running
/// server (e.g. its upstream session expired), the server is restarted once.
///
/// # Arguments
/// * `server_id` - The MCP server ID
///
/// # Returns
/// * MCP server capabilities (tools, resources, prompts), or an error when
///   the server can't be reached or rejects `tools/list`
#[tauri::command]
pub async fn get_mcp_server_capabilities(
    server_id: String,
    mcp_manager: State<'_, Arc<McpServerManager>>,
) -> Result<McpServerCapabilities, String> {
    tracing::info!("📋 Getting capabilities for MCP server: {}", server_id);

    let was_running = mcp_manager.is_running(&server_id);
    let mut result = discover_server_capabilities(&mcp_manager, &server_id).await;
    if result.is_err() && was_running {
        tracing::warn!(
            "Listing capabilities of running MCP server {} failed ({}); restarting it",
            server_id,
            result
                .as_ref()
                .err()
                .map(String::as_str)
                .unwrap_or_default()
        );
        let _ = mcp_manager.stop_server(&server_id).await;
        result = discover_server_capabilities(&mcp_manager, &server_id).await;
    }

    let capabilities = result?;
    tracing::info!(
        "✅ MCP server {} capabilities: {} tools, {} resources, {} prompts",
        server_id,
        capabilities.tools.len(),
        capabilities.resources.len(),
        capabilities.prompts.len()
    );
    Ok(capabilities)
}

/// Start (if needed), handshake with, and list an MCP server's capabilities.
async fn discover_server_capabilities(
    mcp_manager: &McpServerManager,
    server_id: &str,
) -> Result<McpServerCapabilities, String> {
    use lr_mcp::protocol::JsonRpcRequest;

    if !mcp_manager.is_running(server_id) {
        tracing::info!("MCP server {} not running, starting it now...", server_id);
        mcp_manager
            .start_server(server_id)
            .await
            .map_err(|e| format!("Failed to start MCP server: {}", e))?;
    }

    // Register a request callback so server-initiated requests (sampling/elicitation/roots)
    // during init don't block the server. We decline or return defaults during discovery.
    mcp_manager.set_request_callback(
        server_id,
        std::sync::Arc::new(|request| {
            Box::pin(async move {
                let request_id = request.id.unwrap_or(serde_json::Value::Null);
                match request.method.as_str() {
                    "roots/list" => lr_mcp::protocol::JsonRpcResponse::success(
                        request_id,
                        serde_json::json!({ "roots": [] }),
                    ),
                    "elicitation/create" | "elicitation/requestInput" => {
                        lr_mcp::protocol::JsonRpcResponse::success(
                            request_id,
                            serde_json::json!({ "action": "decline", "content": {} }),
                        )
                    }
                    _ => lr_mcp::protocol::JsonRpcResponse::error(
                        request_id,
                        lr_mcp::protocol::JsonRpcError::custom(
                            -32601,
                            "Not available during server discovery".to_string(),
                            None,
                        ),
                    ),
                }
            })
        }),
    );

    // Complete MCP handshake before querying capabilities.
    // Servers may conditionally register tools based on client capabilities
    // (e.g. only exposing elicitation/sampling demo tools if the client supports them).
    let init_request = JsonRpcRequest::with_id(
        0,
        "initialize".to_string(),
        Some(serde_json::json!({
            "protocolVersion": lr_mcp::protocol::MCP_PROTOCOL_VERSION,
            "capabilities": {
                "sampling": {},
                "elicitation": { "form": {} },
                "roots": { "listChanged": true }
            },
            "clientInfo": {
                "name": "LocalRouter",
                "version": env!("CARGO_PKG_VERSION")
            }
        })),
    );
    if let Err(e) = mcp_manager.send_request(server_id, init_request).await {
        tracing::warn!("Failed to initialize MCP server {}: {}", server_id, e);
    }

    // Send notifications/initialized to complete the handshake
    let init_notification = JsonRpcRequest::new(
        None,
        "notifications/initialized".to_string(),
        Some(serde_json::json!({})),
    );
    if let Err(e) = mcp_manager.send_request(server_id, init_notification).await {
        tracing::warn!(
            "Failed to send initialized notification to {}: {}",
            server_id,
            e
        );
    }

    // tools/list failures are reported; servers without resources/prompts
    // support legitimately reject those lists, so their errors are ignored.
    let tools = list_server_items(mcp_manager, server_id, "tools/list", "tools").await?;
    let resources = list_server_items(mcp_manager, server_id, "resources/list", "resources")
        .await
        .unwrap_or_else(|e| {
            tracing::debug!("resources/list on {} failed: {}", server_id, e);
            Vec::new()
        });
    let prompts = list_server_items(mcp_manager, server_id, "prompts/list", "prompts")
        .await
        .unwrap_or_else(|e| {
            tracing::debug!("prompts/list on {} failed: {}", server_id, e);
            Vec::new()
        });

    let str_field = |item: &serde_json::Value, key: &str| {
        item.get(key)
            .and_then(|v| v.as_str())
            .map(|s| s.to_string())
    };
    Ok(McpServerCapabilities {
        tools: tools
            .iter()
            .map(|t| McpToolInfo {
                name: str_field(t, "name").unwrap_or_default(),
                description: str_field(t, "description"),
            })
            .collect(),
        resources: resources
            .iter()
            .map(|r| McpResourceInfo {
                uri: str_field(r, "uri").unwrap_or_default(),
                name: str_field(r, "name").unwrap_or_default(),
                description: str_field(r, "description"),
            })
            .collect(),
        prompts: prompts
            .iter()
            .map(|p| McpPromptInfo {
                name: str_field(p, "name").unwrap_or_default(),
                description: str_field(p, "description"),
            })
            .collect(),
    })
}

/// Run a paginated MCP list method and collect the items under `key`.
/// A server answering "method not found" has no such items.
async fn list_server_items(
    mcp_manager: &McpServerManager,
    server_id: &str,
    method: &str,
    key: &str,
) -> Result<Vec<serde_json::Value>, String> {
    const MAX_PAGES: usize = 100;
    let mut items = Vec::new();
    let mut cursor: Option<String> = None;
    for page in 0..MAX_PAGES {
        let params = cursor.as_ref().map(|c| serde_json::json!({ "cursor": c }));
        let request =
            lr_mcp::protocol::JsonRpcRequest::with_id(page as u64 + 1, method.to_string(), params);
        let response = mcp_manager
            .send_request(server_id, request)
            .await
            .map_err(|e| format!("{} failed: {}", method, e))?;
        if let Some(error) = response.error {
            if error.code == lr_mcp::protocol::METHOD_NOT_FOUND {
                return Ok(items);
            }
            return Err(format!(
                "{} failed: {} (code {})",
                method, error.message, error.code
            ));
        }
        let Some(result) = response.result else {
            break;
        };
        if let Some(page_items) = result.get(key).and_then(|v| v.as_array()) {
            items.extend(page_items.iter().cloned());
        }
        cursor = result
            .get("nextCursor")
            .and_then(|c| c.as_str())
            .map(|s| s.to_string());
        if cursor.is_none() {
            break;
        }
    }
    Ok(items)
}

// ============================================================================
// MCP Gateway Settings Commands
// ============================================================================

/// Response type for MCP gateway settings
#[derive(Clone, Serialize, Deserialize)]
pub struct McpGatewaySettingsResponse {
    pub sampling: String,
    pub elicitation_mode: String,
}

/// Get the global MCP gateway settings
#[tauri::command]
pub async fn get_mcp_gateway_settings(
    config_manager: State<'_, ConfigManager>,
) -> Result<McpGatewaySettingsResponse, String> {
    let config = config_manager.get();
    let settings = &config.mcp_gateway;

    Ok(McpGatewaySettingsResponse {
        sampling: serde_json::to_value(&settings.sampling)
            .ok()
            .and_then(|v| v.as_str().map(|s| s.to_string()))
            .unwrap_or_else(|| "passthrough".to_string()),
        elicitation_mode: serde_json::to_value(&settings.elicitation_mode)
            .ok()
            .and_then(|v| v.as_str().map(|s| s.to_string()))
            .unwrap_or_else(|| "passthrough".to_string()),
    })
}

/// Update the global MCP gateway settings
#[tauri::command]
pub async fn set_mcp_gateway_settings(
    config_manager: State<'_, ConfigManager>,
    server_manager: State<'_, Arc<ServerManager>>,
    sampling: Option<String>,
    elicitation_mode: Option<String>,
) -> Result<(), String> {
    config_manager
        .update(|config| {
            if let Some(s) = sampling {
                config.mcp_gateway.sampling =
                    serde_json::from_value(serde_json::Value::String(s)).unwrap_or_default();
            }
            if let Some(mode) = elicitation_mode {
                config.mcp_gateway.elicitation_mode =
                    serde_json::from_value(serde_json::Value::String(mode)).unwrap_or_default();
            }
        })
        .map_err(|e| format!("Failed to save settings: {}", e))?;

    // Persist to disk
    config_manager
        .save()
        .await
        .map_err(|e| format!("Failed to save settings: {}", e))?;

    // Update live settings on the gateway so changes take effect without restart
    let settings = config_manager.get().mcp_gateway.clone();
    if let Some(gateway) = server_manager.get_mcp_gateway() {
        gateway.update_gateway_settings(&settings);
    }

    Ok(())
}

#[cfg(test)]
mod auth_update_tests {
    use super::*;
    use serde_json::json;

    fn oauth_value(kind: &str) -> serde_json::Value {
        json!({"type": kind, "client_id": "updated-client", "client_secret": "",
            "auth_url": "https://example.com/authorize", "token_url": "https://example.com/token", "scopes": ["read"]})
    }

    #[test]
    fn absent_null_and_explicit_none_auth_are_accepted() {
        for value in [
            None,
            Some(serde_json::Value::Null),
            Some(json!({"type": "none"})),
        ] {
            assert_eq!(process_auth_config("test", value, None).unwrap(), None);
        }
    }

    #[test]
    fn new_browser_oauth_without_secret_cannot_reuse_a_previous_mode_secret() {
        let config = json!({"type": "oauth_browser"});
        let first = process_auth_config("test", Some(config.clone()), None)
            .unwrap()
            .unwrap();
        let second = process_auth_config("test", Some(config), None)
            .unwrap()
            .unwrap();
        let McpAuthConfig::OAuthBrowser {
            client_secret_ref: first_ref,
            ..
        } = first
        else {
            panic!("wrong auth type")
        };
        let McpAuthConfig::OAuthBrowser {
            client_secret_ref: second_ref,
            ..
        } = second
        else {
            panic!("wrong auth type")
        };
        assert_ne!(first_ref, "test_oauth_browser_secret");
        assert_ne!(first_ref, second_ref);
    }

    #[test]
    fn custom_header_updates_preserve_named_secrets_and_drop_omitted_headers() {
        let existing = McpAuthConfig::CustomHeaders {
            header_refs: [
                ("X-API-Key".into(), "saved-key".into()),
                ("X-Remove".into(), "removed-key".into()),
            ]
            .into(),
        };
        let updated = process_auth_config(
            "test",
            Some(json!({"type": "custom_headers", "headers": {"x-api-key": ""}})),
            Some(&existing),
        )
        .unwrap()
        .unwrap();
        assert_eq!(
            updated,
            McpAuthConfig::CustomHeaders {
                header_refs: [("x-api-key".into(), "saved-key".into())].into()
            }
        );
        let cleared = process_auth_config(
            "test",
            Some(json!({"type": "custom_headers", "headers": {}})),
            Some(&existing),
        )
        .unwrap()
        .unwrap();
        assert_eq!(
            cleared,
            McpAuthConfig::CustomHeaders {
                header_refs: Default::default()
            }
        );
    }

    #[test]
    fn custom_header_updates_reject_invalid_or_case_duplicate_names_and_values() {
        for headers in [
            json!({"Invalid Name": "secret"}),
            json!({"X-Key": "one", "x-key": "two"}),
            json!({"X-Key": "secret\r\ninjected"}),
        ] {
            let error = process_auth_config(
                "test",
                Some(json!({"type": "custom_headers", "headers": headers})),
                None,
            )
            .unwrap_err();
            assert!(!error.contains("secret"));
        }
    }

    #[test]
    fn oauth_spellings_preserve_the_existing_secret_reference() {
        let existing = McpAuthConfig::OAuth {
            client_id: "old-client".into(),
            client_secret_ref: "retained-secret".into(),
            auth_url: "https://example.com/authorize".into(),
            token_url: "https://example.com/token".into(),
            scopes: vec!["old".into()],
        };
        for kind in ["oauth", "o_auth"] {
            let value = oauth_value(kind);
            assert!(serde_json::from_value::<FrontendAuthConfig>(value.clone()).is_ok());
            let auth = process_auth_config("test", Some(value), Some(&existing))
                .unwrap()
                .unwrap();
            let McpAuthConfig::OAuth {
                client_id,
                client_secret_ref,
                scopes,
                ..
            } = &auth
            else {
                panic!("wrong auth type")
            };
            assert_eq!(client_id, "updated-client");
            assert_eq!(client_secret_ref, "retained-secret");
            assert_eq!(scopes, &["read"]);
            let stored = serde_json::to_value(auth).unwrap();
            assert_eq!(stored["type"], "o_auth");
            let mut alias = stored;
            alias["type"] = json!("oauth");
            assert!(serde_json::from_value::<McpAuthConfig>(alias).is_ok());
        }
    }

    #[test]
    fn new_oauth_requires_a_secret() {
        assert!(
            process_auth_config("test", Some(oauth_value("oauth")), None)
                .unwrap_err()
                .contains("Client Secret is required")
        );
    }

    #[test]
    fn browser_oauth_update_preserves_custom_secret_reference() {
        let existing = McpAuthConfig::OAuthBrowser {
            client_id: "client".into(),
            client_secret_ref: "custom-existing-ref".into(),
            auth_url: String::new(),
            token_url: String::new(),
            scopes: vec![],
            redirect_uri: "http://localhost:8080/callback".into(),
            issuer: None,
        };
        let updated = process_auth_config(
            "test",
            Some(json!({"type":"oauth_browser", "client_id":"client"})),
            Some(&existing),
        )
        .unwrap()
        .unwrap();
        let McpAuthConfig::OAuthBrowser {
            client_secret_ref, ..
        } = updated
        else {
            panic!("wrong auth type")
        };
        assert_eq!(client_secret_ref, "custom-existing-ref");
    }
}
