//! Marketplace module - MCP server and skill discovery/installation
//!
//! Provides built-in tools for AI clients to search and install MCP servers
//! from the official registry and skills from configured GitHub sources.
//! Not a virtual MCP server - these are built-in tools injected into the gateway.

pub mod install;
pub mod install_popup;
pub mod registry;
pub mod skill_sources;
pub mod tools;
pub mod types;

pub use install_popup::MarketplaceInstallManager;
pub use types::*;

use crate::registry::McpRegistryClient;
use crate::skill_sources::SkillSourcesClient;
use lr_config::MarketplaceConfig;
use parking_lot::RwLock;
use serde_json::Value;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tracing::{debug, info, warn};

/// Tool name prefix for marketplace tools
pub const TOOL_PREFIX: &str = "marketplace__";

/// Virtual server ID for marketplace (used in connection graph, not MCP panel)
pub const MARKETPLACE_ID: &str = "marketplace";

/// Check if a tool name is a marketplace tool
pub fn is_marketplace_tool(name: &str) -> bool {
    name.starts_with(TOOL_PREFIX)
}

/// Central marketplace service
pub struct MarketplaceService {
    /// MCP registry client
    registry_client: McpRegistryClient,

    /// Skill sources client
    skill_sources_client: SkillSourcesClient,

    /// Install popup manager
    install_manager: Arc<MarketplaceInstallManager>,

    /// Cached marketplace config
    config: RwLock<MarketplaceConfig>,

    /// Data directory for downloaded skills
    data_dir: PathBuf,

    /// Persistent cache
    cache: RwLock<MarketplaceCache>,

    /// Optional Tauri app handle for event emission
    app_handle: RwLock<Option<tauri::AppHandle>>,
}

impl MarketplaceService {
    /// Create a new marketplace service
    pub fn new(config: MarketplaceConfig, data_dir: PathBuf) -> Self {
        let registry_url = config.registry_url.clone();
        let cache = Self::load_cache(&data_dir);

        Self {
            registry_client: McpRegistryClient::new(registry_url),
            skill_sources_client: SkillSourcesClient::new(config.skill_sources.clone()),
            install_manager: Arc::new(MarketplaceInstallManager::default()),
            config: RwLock::new(config),
            data_dir,
            cache: RwLock::new(cache),
            app_handle: RwLock::new(None),
        }
    }

    /// Create a new marketplace service with broadcast support for popups
    pub fn new_with_broadcast(
        config: MarketplaceConfig,
        data_dir: PathBuf,
        notification_broadcast: Arc<
            tokio::sync::broadcast::Sender<(String, crate::install_popup::JsonRpcNotification)>,
        >,
    ) -> Self {
        let registry_url = config.registry_url.clone();
        let cache = Self::load_cache(&data_dir);

        Self {
            registry_client: McpRegistryClient::new(registry_url),
            skill_sources_client: SkillSourcesClient::new(config.skill_sources.clone()),
            install_manager: Arc::new(MarketplaceInstallManager::new_with_broadcast(
                120,
                notification_broadcast,
            )),
            config: RwLock::new(config),
            data_dir,
            cache: RwLock::new(cache),
            app_handle: RwLock::new(None),
        }
    }

    /// Load cache from disk
    fn load_cache(data_dir: &Path) -> MarketplaceCache {
        let cache_path = data_dir.join("marketplace_cache.json");
        if cache_path.exists() {
            match std::fs::read_to_string(&cache_path) {
                Ok(content) => match serde_json::from_str(&content) {
                    Ok(cache) => {
                        debug!("Loaded marketplace cache from {:?}", cache_path);
                        return cache;
                    }
                    Err(e) => {
                        warn!("Failed to parse marketplace cache: {}", e);
                    }
                },
                Err(e) => {
                    warn!("Failed to read marketplace cache: {}", e);
                }
            }
        }
        MarketplaceCache::default()
    }

    /// Save cache to disk
    fn save_cache(&self) {
        let cache_path = self.data_dir.join("marketplace_cache.json");
        let cache = self.cache.read();
        match serde_json::to_string_pretty(&*cache) {
            Ok(content) => {
                if let Err(e) = std::fs::write(&cache_path, content) {
                    warn!("Failed to write marketplace cache: {}", e);
                } else {
                    debug!("Saved marketplace cache to {:?}", cache_path);
                }
            }
            Err(e) => {
                warn!("Failed to serialize marketplace cache: {}", e);
            }
        }
    }

    /// Set the Tauri app handle for event emission
    pub fn set_app_handle(&self, handle: tauri::AppHandle) {
        *self.app_handle.write() = Some(handle);
    }

    /// Update the marketplace config
    pub fn update_config(&self, config: MarketplaceConfig) {
        let registry_url = config.registry_url.clone();
        self.registry_client.set_registry_url(registry_url);
        self.skill_sources_client
            .set_sources(config.skill_sources.clone());
        *self.config.write() = config;
    }

    /// Get the current config
    pub fn get_config(&self) -> MarketplaceConfig {
        self.config.read().clone()
    }

    /// Check if marketplace is enabled
    pub fn is_enabled(&self) -> bool {
        self.config.read().enabled
    }

    /// Get reference to the install manager
    pub fn install_manager(&self) -> Arc<MarketplaceInstallManager> {
        self.install_manager.clone()
    }

    /// Get the data directory for marketplace-installed skills
    pub fn skills_data_dir(&self) -> PathBuf {
        self.data_dir.join("marketplace-skills")
    }

    /// List the 4 marketplace tools as JSON tool definitions
    pub fn list_tools(&self) -> Vec<Value> {
        tools::list_tools()
    }

    /// Handle a marketplace tool call
    pub async fn handle_tool_call(
        &self,
        tool_name: &str,
        arguments: Value,
        client_id: &str,
        client_name: &str,
    ) -> Result<Value, MarketplaceError> {
        tools::handle_tool_call(self, tool_name, arguments, client_id, client_name).await
    }

    /// Search MCP servers from the registry (uses cache if available)
    pub async fn search_mcp_servers(
        &self,
        query: &str,
        limit: Option<u32>,
    ) -> Result<Vec<McpServerListing>, MarketplaceError> {
        info!(
            "Searching MCP servers: query='{}', limit={:?}",
            query, limit
        );

        // Check cache first
        {
            let cache = self.cache.read();
            if let Some(cached) = cache.get_mcp_servers(query) {
                info!("Using cached MCP servers for query '{}'", query);
                let mut result = cached.clone();
                // Apply limit
                if let Some(limit) = limit {
                    result.truncate(limit as usize);
                }
                return Ok(result);
            }
        }

        // Fetch from registry
        let servers = self.registry_client.search(query, limit).await?;

        // Cache the results
        {
            let mut cache = self.cache.write();
            cache.cache_mcp_servers(query.to_string(), servers.clone());
        }
        self.save_cache();

        Ok(servers)
    }

    /// Search MCP servers bypassing cache (force refresh)
    pub async fn search_mcp_servers_fresh(
        &self,
        query: &str,
        limit: Option<u32>,
    ) -> Result<Vec<McpServerListing>, MarketplaceError> {
        info!(
            "Fetching fresh MCP servers: query='{}', limit={:?}",
            query, limit
        );

        let servers = self.registry_client.search(query, limit).await?;

        // Cache the results
        {
            let mut cache = self.cache.write();
            cache.cache_mcp_servers(query.to_string(), servers.clone());
        }
        self.save_cache();

        Ok(servers)
    }

    /// Search skills from configured sources (uses cache if available)
    pub async fn search_skills(
        &self,
        query: Option<&str>,
        source: Option<&str>,
    ) -> Result<Vec<SkillListing>, MarketplaceError> {
        info!("Searching skills: query={:?}, source={:?}", query, source);
        self.skill_sources_client
            .search_with_cache(query, source, &self.cache, || self.save_cache())
            .await
    }

    /// Search skills bypassing cache (force refresh)
    pub async fn search_skills_fresh(
        &self,
        query: Option<&str>,
        source: Option<&str>,
    ) -> Result<Vec<SkillListing>, MarketplaceError> {
        info!(
            "Fetching fresh skills: query={:?}, source={:?}",
            query, source
        );
        self.skill_sources_client
            .search_fresh(query, source, &self.cache, || self.save_cache())
            .await
    }

    /// Get cache status info
    pub fn get_cache_status(&self) -> CacheStatus {
        let cache = self.cache.read();
        CacheStatus {
            mcp_last_refresh: cache.mcp_last_refresh,
            skills_last_refresh: cache.skills_last_refresh,
            mcp_cached_queries: cache.mcp_cache.len(),
            skills_cached_sources: cache.skills_cache.len(),
        }
    }

    /// Clear all caches and refresh
    pub async fn refresh_all(&self) -> Result<(), MarketplaceError> {
        info!("Refreshing all marketplace caches");

        // Clear caches
        {
            let mut cache = self.cache.write();
            cache.clear_mcp_cache();
            cache.clear_skills_cache();
        }
        self.save_cache();

        // Refresh MCP with default query
        let _ = self.search_mcp_servers_fresh("mcp", Some(50)).await;

        // Refresh skills (will be done by search_skills_fresh)
        let _ = self.search_skills_fresh(None, None).await;

        Ok(())
    }

    /// Clear MCP cache only
    pub fn clear_mcp_cache(&self) {
        let mut cache = self.cache.write();
        cache.clear_mcp_cache();
        drop(cache);
        self.save_cache();
    }

    /// Clear skills cache only
    pub fn clear_skills_cache(&self) {
        let mut cache = self.cache.write();
        cache.clear_skills_cache();
        drop(cache);
        self.save_cache();
    }

    /// Emit marketplace-changed event to frontend
    #[allow(dead_code)]
    fn emit_marketplace_changed(&self) {
        if let Some(ref handle) = *self.app_handle.read() {
            use tauri::Emitter;
            if let Err(e) = handle.emit("marketplace-changed", ()) {
                warn!("Failed to emit marketplace-changed event: {}", e);
            }
        }
    }
}

impl Clone for MarketplaceService {
    fn clone(&self) -> Self {
        Self {
            registry_client: self.registry_client.clone(),
            skill_sources_client: self.skill_sources_client.clone(),
            install_manager: self.install_manager.clone(),
            config: RwLock::new(self.config.read().clone()),
            data_dir: self.data_dir.clone(),
            cache: RwLock::new(self.cache.read().clone()),
            app_handle: RwLock::new(self.app_handle.read().clone()),
        }
    }
}
