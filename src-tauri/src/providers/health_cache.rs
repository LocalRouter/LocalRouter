//! Centralized health cache for providers and MCP servers
//!
//! This module provides a centralized cache for health check results,
//! enabling the frontend to display aggregate health status without
//! triggering health checks on every request.

use chrono::{DateTime, Utc};
use parking_lot::RwLock;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;
use tauri::{AppHandle, Emitter};
use tracing::{debug, info};

/// Aggregate health status for the entire system
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum AggregateHealthStatus {
    /// Server is down - critical failure
    Red,
    /// All systems operational
    Green,
    /// Some issues detected (providers/MCPs unhealthy or degraded)
    Yellow,
}

impl AggregateHealthStatus {
    /// Convert to a string suitable for display
    pub fn as_str(&self) -> &'static str {
        match self {
            AggregateHealthStatus::Red => "red",
            AggregateHealthStatus::Green => "green",
            AggregateHealthStatus::Yellow => "yellow",
        }
    }
}

/// Individual item health status
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ItemHealthStatus {
    /// Item is healthy and operational
    Healthy,
    /// Item is degraded (high latency, partial failures)
    Degraded,
    /// Item is unhealthy (failed health check)
    Unhealthy,
    /// Item is ready (MCP server started, not yet checked)
    Ready,
    /// Item health check is pending (not yet checked)
    Pending,
}

impl Default for ItemHealthStatus {
    fn default() -> Self {
        Self::Pending
    }
}

/// Health information for an individual provider or MCP server
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ItemHealth {
    /// Name/ID of the item
    pub name: String,
    /// Health status
    pub status: ItemHealthStatus,
    /// Response latency in milliseconds (if available)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub latency_ms: Option<u64>,
    /// Error message (if unhealthy)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    /// When the health was last checked
    pub last_checked: DateTime<Utc>,
}

impl ItemHealth {
    /// Create a new pending health item
    pub fn pending(name: String) -> Self {
        Self {
            name,
            status: ItemHealthStatus::Pending,
            latency_ms: None,
            error: None,
            last_checked: Utc::now(),
        }
    }

    /// Create a healthy item
    pub fn healthy(name: String, latency_ms: Option<u64>) -> Self {
        Self {
            name,
            status: ItemHealthStatus::Healthy,
            latency_ms,
            error: None,
            last_checked: Utc::now(),
        }
    }

    /// Create a degraded item
    pub fn degraded(name: String, latency_ms: Option<u64>, reason: String) -> Self {
        Self {
            name,
            status: ItemHealthStatus::Degraded,
            latency_ms,
            error: Some(reason),
            last_checked: Utc::now(),
        }
    }

    /// Create an unhealthy item
    pub fn unhealthy(name: String, error: String) -> Self {
        Self {
            name,
            status: ItemHealthStatus::Unhealthy,
            latency_ms: None,
            error: Some(error),
            last_checked: Utc::now(),
        }
    }

    /// Create a ready item (MCP server started)
    pub fn ready(name: String) -> Self {
        Self {
            name,
            status: ItemHealthStatus::Ready,
            latency_ms: None,
            error: None,
            last_checked: Utc::now(),
        }
    }
}

/// Cached health state for the entire system
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HealthCacheState {
    /// Whether the server is running
    pub server_running: bool,
    /// Server port (if running)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub server_port: Option<u16>,
    /// Provider health states (keyed by provider name)
    pub providers: HashMap<String, ItemHealth>,
    /// MCP server health states (keyed by server ID)
    pub mcp_servers: HashMap<String, ItemHealth>,
    /// Last time a full refresh was performed
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_refresh: Option<DateTime<Utc>>,
    /// Calculated aggregate status
    pub aggregate_status: AggregateHealthStatus,
}

impl Default for HealthCacheState {
    fn default() -> Self {
        Self {
            server_running: false,
            server_port: None,
            providers: HashMap::new(),
            mcp_servers: HashMap::new(),
            last_refresh: None,
            aggregate_status: AggregateHealthStatus::Red,
        }
    }
}

/// Manager for centralized health caching
pub struct HealthCacheManager {
    /// Cached health state
    cache: Arc<RwLock<HealthCacheState>>,
    /// Tauri app handle for emitting events (set after app initialization)
    app_handle: Arc<RwLock<Option<AppHandle>>>,
}

impl HealthCacheManager {
    /// Create a new health cache manager
    pub fn new() -> Self {
        Self {
            cache: Arc::new(RwLock::new(HealthCacheState::default())),
            app_handle: Arc::new(RwLock::new(None)),
        }
    }

    /// Set the Tauri app handle for event emission
    pub fn set_app_handle(&self, handle: AppHandle) {
        *self.app_handle.write() = Some(handle);
    }

    /// Get the current cached health state
    pub fn get(&self) -> HealthCacheState {
        self.cache.read().clone()
    }

    /// Get the aggregate health status
    pub fn aggregate_status(&self) -> AggregateHealthStatus {
        self.cache.read().aggregate_status
    }

    /// Update server status
    pub fn update_server_status(&self, running: bool, port: Option<u16>) {
        {
            let mut cache = self.cache.write();
            cache.server_running = running;
            cache.server_port = port;
            self.recalculate_aggregate_status(&mut cache);
        }
        self.emit_status_changed();
        info!(
            "Health cache updated: server_running={}, port={:?}",
            running, port
        );
    }

    /// Update provider health
    pub fn update_provider(&self, name: &str, health: ItemHealth) {
        {
            let mut cache = self.cache.write();
            cache.providers.insert(name.to_string(), health.clone());
            self.recalculate_aggregate_status(&mut cache);
        }
        self.emit_status_changed();
        debug!("Health cache updated: provider={}, status={:?}", name, health.status);
    }

    /// Update MCP server health
    pub fn update_mcp_server(&self, id: &str, health: ItemHealth) {
        {
            let mut cache = self.cache.write();
            cache.mcp_servers.insert(id.to_string(), health.clone());
            self.recalculate_aggregate_status(&mut cache);
        }
        self.emit_status_changed();
        debug!("Health cache updated: mcp_server={}, status={:?}", id, health.status);
    }

    /// Initialize providers with pending status
    pub fn init_providers(&self, names: Vec<String>) {
        let mut cache = self.cache.write();
        for name in names {
            cache.providers.insert(name.clone(), ItemHealth::pending(name));
        }
        self.recalculate_aggregate_status(&mut cache);
        debug!("Health cache initialized {} providers", cache.providers.len());
    }

    /// Initialize MCP servers with pending status
    pub fn init_mcp_servers(&self, configs: Vec<(String, String)>) {
        let mut cache = self.cache.write();
        for (id, name) in configs {
            cache.mcp_servers.insert(id.clone(), ItemHealth::pending(name));
        }
        self.recalculate_aggregate_status(&mut cache);
        debug!("Health cache initialized {} MCP servers", cache.mcp_servers.len());
    }

    /// Remove a provider from the cache
    pub fn remove_provider(&self, name: &str) {
        {
            let mut cache = self.cache.write();
            cache.providers.remove(name);
            self.recalculate_aggregate_status(&mut cache);
        }
        self.emit_status_changed();
    }

    /// Remove an MCP server from the cache
    pub fn remove_mcp_server(&self, id: &str) {
        {
            let mut cache = self.cache.write();
            cache.mcp_servers.remove(id);
            self.recalculate_aggregate_status(&mut cache);
        }
        self.emit_status_changed();
    }

    /// Mark the time of a full refresh
    pub fn mark_refresh(&self) {
        let mut cache = self.cache.write();
        cache.last_refresh = Some(Utc::now());
    }

    /// Recalculate the aggregate status based on current state
    fn recalculate_aggregate_status(&self, cache: &mut HealthCacheState) {
        // If server is not running, always red
        if !cache.server_running {
            cache.aggregate_status = AggregateHealthStatus::Red;
            return;
        }

        let mut has_issues = false;

        // Check providers (only count non-pending items)
        for (_name, health) in &cache.providers {
            match health.status {
                ItemHealthStatus::Unhealthy => {
                    has_issues = true;
                }
                ItemHealthStatus::Degraded => {
                    has_issues = true;
                }
                ItemHealthStatus::Healthy | ItemHealthStatus::Ready | ItemHealthStatus::Pending => {
                    // OK or not yet checked
                }
            }
        }

        // Check MCP servers (only count non-pending items)
        for (_id, health) in &cache.mcp_servers {
            match health.status {
                ItemHealthStatus::Unhealthy => {
                    has_issues = true;
                }
                ItemHealthStatus::Degraded => {
                    has_issues = true;
                }
                ItemHealthStatus::Healthy | ItemHealthStatus::Ready | ItemHealthStatus::Pending => {
                    // OK or not yet checked
                }
            }
        }

        cache.aggregate_status = if has_issues {
            AggregateHealthStatus::Yellow
        } else {
            AggregateHealthStatus::Green
        };
    }

    /// Emit health-status-changed event to frontend
    fn emit_status_changed(&self) {
        if let Some(handle) = self.app_handle.read().as_ref() {
            let state = self.cache.read().clone();
            if let Err(e) = handle.emit("health-status-changed", &state) {
                tracing::error!("Failed to emit health-status-changed event: {}", e);
            }
        }
    }
}

impl Default for HealthCacheManager {
    fn default() -> Self {
        Self::new()
    }
}

impl Clone for HealthCacheManager {
    fn clone(&self) -> Self {
        Self {
            cache: self.cache.clone(),
            app_handle: self.app_handle.clone(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_aggregate_status_server_down() {
        let manager = HealthCacheManager::new();
        manager.update_server_status(false, None);
        assert_eq!(manager.aggregate_status(), AggregateHealthStatus::Red);
    }

    #[test]
    fn test_aggregate_status_all_healthy() {
        let manager = HealthCacheManager::new();
        manager.update_server_status(true, Some(3625));
        manager.update_provider("openai", ItemHealth::healthy("openai".to_string(), Some(100)));
        manager.update_provider("anthropic", ItemHealth::healthy("anthropic".to_string(), Some(150)));
        assert_eq!(manager.aggregate_status(), AggregateHealthStatus::Green);
    }

    #[test]
    fn test_aggregate_status_provider_unhealthy() {
        let manager = HealthCacheManager::new();
        manager.update_server_status(true, Some(3625));
        manager.update_provider("openai", ItemHealth::healthy("openai".to_string(), Some(100)));
        manager.update_provider("anthropic", ItemHealth::unhealthy("anthropic".to_string(), "Connection refused".to_string()));
        assert_eq!(manager.aggregate_status(), AggregateHealthStatus::Yellow);
    }

    #[test]
    fn test_aggregate_status_mcp_degraded() {
        let manager = HealthCacheManager::new();
        manager.update_server_status(true, Some(3625));
        manager.update_mcp_server("fs-server", ItemHealth::degraded("fs-server".to_string(), Some(3000), "High latency".to_string()));
        assert_eq!(manager.aggregate_status(), AggregateHealthStatus::Yellow);
    }

    #[test]
    fn test_init_providers() {
        let manager = HealthCacheManager::new();
        manager.init_providers(vec!["openai".to_string(), "anthropic".to_string()]);
        let state = manager.get();
        assert_eq!(state.providers.len(), 2);
        assert_eq!(state.providers.get("openai").unwrap().status, ItemHealthStatus::Pending);
    }

    #[test]
    fn test_init_mcp_servers() {
        let manager = HealthCacheManager::new();
        manager.init_mcp_servers(vec![
            ("id1".to_string(), "Server 1".to_string()),
            ("id2".to_string(), "Server 2".to_string()),
        ]);
        let state = manager.get();
        assert_eq!(state.mcp_servers.len(), 2);
        assert_eq!(state.mcp_servers.get("id1").unwrap().name, "Server 1");
    }

    #[test]
    fn test_remove_provider() {
        let manager = HealthCacheManager::new();
        manager.update_server_status(true, Some(3625));
        manager.update_provider("openai", ItemHealth::healthy("openai".to_string(), Some(100)));
        assert_eq!(manager.get().providers.len(), 1);
        manager.remove_provider("openai");
        assert_eq!(manager.get().providers.len(), 0);
    }
}
