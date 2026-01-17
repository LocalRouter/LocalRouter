//! MCP server lifecycle management
//!
//! Manages MCP server instances, their lifecycle, and health checks.

use crate::config::{McpServerConfig, McpTransportConfig, McpTransportType};
use crate::mcp::transport::{StdioTransport, Transport};
use crate::mcp::protocol::{JsonRpcRequest, JsonRpcResponse};
use crate::utils::errors::{AppError, AppResult};
use dashmap::DashMap;
use serde::{Deserialize, Serialize};
use std::sync::Arc;

/// MCP server manager
///
/// Manages the lifecycle of MCP server instances.
/// Supports STDIO, SSE, and WebSocket transports.
#[derive(Clone)]
pub struct McpServerManager {
    /// Active STDIO transports (server_id -> transport)
    stdio_transports: Arc<DashMap<String, Arc<StdioTransport>>>,

    /// Server configurations (server_id -> config)
    configs: Arc<DashMap<String, McpServerConfig>>,
}

/// Health status for an MCP server
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct McpServerHealth {
    /// Server ID
    pub server_id: String,

    /// Server name
    pub server_name: String,

    /// Health status
    pub status: HealthStatus,

    /// Error message (if unhealthy)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,

    /// Last health check timestamp
    pub last_check: chrono::DateTime<chrono::Utc>,
}

/// Health status enum
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum HealthStatus {
    Healthy,
    Unhealthy,
    Unknown,
}

impl McpServerManager {
    /// Create a new MCP server manager
    pub fn new() -> Self {
        Self {
            stdio_transports: Arc::new(DashMap::new()),
            configs: Arc::new(DashMap::new()),
        }
    }

    /// Load server configurations from config
    pub fn load_configs(&self, configs: Vec<McpServerConfig>) {
        for config in configs {
            self.configs.insert(config.id.clone(), config);
        }
    }

    /// Add a server configuration
    pub fn add_config(&self, config: McpServerConfig) {
        self.configs.insert(config.id.clone(), config);
    }

    /// Remove a server configuration
    pub fn remove_config(&self, server_id: &str) {
        self.configs.remove(server_id);
    }

    /// Get a server configuration
    pub fn get_config(&self, server_id: &str) -> Option<McpServerConfig> {
        self.configs.get(server_id).map(|c| c.clone())
    }

    /// List all server configurations
    pub fn list_configs(&self) -> Vec<McpServerConfig> {
        self.configs.iter().map(|entry| entry.value().clone()).collect()
    }

    /// Start an MCP server
    ///
    /// # Arguments
    /// * `server_id` - The server ID to start
    ///
    /// # Returns
    /// * Ok(()) if the server started successfully
    /// * Err if the server is not configured or failed to start
    pub async fn start_server(&self, server_id: &str) -> AppResult<()> {
        // Get server config
        let config = self
            .configs
            .get(server_id)
            .ok_or_else(|| AppError::Mcp(format!("Server not found: {}", server_id)))?
            .clone();

        if !config.enabled {
            return Err(AppError::Mcp(format!("Server is disabled: {}", server_id)));
        }

        tracing::info!("Starting MCP server: {} ({})", config.name, server_id);

        match config.transport {
            McpTransportType::Stdio => {
                self.start_stdio_server(server_id, &config).await?;
            }
            McpTransportType::Sse => {
                return Err(AppError::Mcp(
                    "SSE transport not yet implemented".to_string(),
                ));
            }
            McpTransportType::WebSocket => {
                return Err(AppError::Mcp(
                    "WebSocket transport not yet implemented".to_string(),
                ));
            }
        }

        tracing::info!("MCP server started: {}", server_id);
        Ok(())
    }

    /// Start a STDIO MCP server
    async fn start_stdio_server(
        &self,
        server_id: &str,
        config: &McpServerConfig,
    ) -> AppResult<()> {
        // Extract STDIO config
        let (command, args, env) = match &config.transport_config {
            McpTransportConfig::Stdio { command, args, env } => {
                (command.clone(), args.clone(), env.clone())
            }
            _ => {
                return Err(AppError::Mcp(
                    "Invalid transport config for STDIO".to_string(),
                ))
            }
        };

        // Spawn the STDIO transport
        let transport = StdioTransport::spawn(command, args, env).await?;

        // Store the transport
        self.stdio_transports
            .insert(server_id.to_string(), Arc::new(transport));

        Ok(())
    }

    /// Stop an MCP server
    ///
    /// # Arguments
    /// * `server_id` - The server ID to stop
    ///
    /// # Returns
    /// * Ok(()) if the server stopped successfully
    /// * Err if the server is not running or failed to stop
    pub async fn stop_server(&self, server_id: &str) -> AppResult<()> {
        tracing::info!("Stopping MCP server: {}", server_id);

        // Try to stop STDIO transport
        if let Some((_, transport)) = self.stdio_transports.remove(server_id) {
            transport.close().await?;
            tracing::info!("MCP STDIO server stopped: {}", server_id);
            return Ok(());
        }

        // Server not running
        Err(AppError::Mcp(format!("Server not running: {}", server_id)))
    }

    /// Send a JSON-RPC request to an MCP server
    ///
    /// # Arguments
    /// * `server_id` - The server ID to send the request to
    /// * `request` - The JSON-RPC request
    ///
    /// # Returns
    /// * The JSON-RPC response from the server
    pub async fn send_request(
        &self,
        server_id: &str,
        request: JsonRpcRequest,
    ) -> AppResult<JsonRpcResponse> {
        // Check if server is running
        if let Some(transport) = self.stdio_transports.get(server_id) {
            return transport.send_request(request).await;
        }

        Err(AppError::Mcp(format!("Server not running: {}", server_id)))
    }

    /// Get the health status of an MCP server
    ///
    /// # Arguments
    /// * `server_id` - The server ID to check
    ///
    /// # Returns
    /// * The health status
    pub async fn get_server_health(&self, server_id: &str) -> McpServerHealth {
        let config = self.get_config(server_id);

        let (status, error) = if let Some(transport) = self.stdio_transports.get(server_id) {
            if transport.is_healthy().await {
                (HealthStatus::Healthy, None)
            } else {
                (
                    HealthStatus::Unhealthy,
                    Some("Process not running".to_string()),
                )
            }
        } else {
            (HealthStatus::Unhealthy, Some("Not started".to_string()))
        };

        McpServerHealth {
            server_id: server_id.to_string(),
            server_name: config.map(|c| c.name).unwrap_or_else(|| "Unknown".to_string()),
            status,
            error,
            last_check: chrono::Utc::now(),
        }
    }

    /// Get health status for all servers
    pub async fn get_all_health(&self) -> Vec<McpServerHealth> {
        let mut health_statuses = Vec::new();

        for entry in self.configs.iter() {
            let server_id = entry.key();
            let health = self.get_server_health(server_id).await;
            health_statuses.push(health);
        }

        health_statuses
    }

    /// Check if a server is running
    pub fn is_running(&self, server_id: &str) -> bool {
        self.stdio_transports.contains_key(server_id)
    }

    /// Shutdown all servers
    pub async fn shutdown_all(&self) {
        tracing::info!("Shutting down all MCP servers");

        for entry in self.stdio_transports.iter() {
            let server_id = entry.key();
            if let Err(e) = self.stop_server(server_id).await {
                tracing::error!("Failed to stop server {}: {}", server_id, e);
            }
        }
    }
}

impl Default for McpServerManager {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::McpServerConfig;
    use std::collections::HashMap;

    #[test]
    fn test_manager_creation() {
        let manager = McpServerManager::new();
        assert_eq!(manager.list_configs().len(), 0);
    }

    #[test]
    fn test_add_remove_config() {
        let manager = McpServerManager::new();

        let config = McpServerConfig::new(
            "Test Server".to_string(),
            McpTransportType::Stdio,
            McpTransportConfig::Stdio {
                command: "echo".to_string(),
                args: vec![],
                env: HashMap::new(),
            },
        );

        let server_id = config.id.clone();

        manager.add_config(config.clone());
        assert_eq!(manager.list_configs().len(), 1);
        assert!(manager.get_config(&server_id).is_some());

        manager.remove_config(&server_id);
        assert_eq!(manager.list_configs().len(), 0);
        assert!(manager.get_config(&server_id).is_none());
    }

    #[tokio::test]
    async fn test_health_check_not_running() {
        let manager = McpServerManager::new();

        let config = McpServerConfig::new(
            "Test Server".to_string(),
            McpTransportType::Stdio,
            McpTransportConfig::Stdio {
                command: "echo".to_string(),
                args: vec![],
                env: HashMap::new(),
            },
        );

        let server_id = config.id.clone();
        manager.add_config(config);

        let health = manager.get_server_health(&server_id).await;
        assert_eq!(health.status, HealthStatus::Unhealthy);
        assert!(health.error.is_some());
    }
}
