//! Configuration management module
//!
//! Handles loading, saving, and managing application configuration.
//! Supports file watching and event emission for real-time config updates.

#![allow(dead_code)]

use chrono::Utc;
use lr_types::{AppError, AppResult};
use notify::{Event, EventKind, RecommendedWatcher, RecursiveMode, Watcher};
use parking_lot::RwLock;
use std::path::PathBuf;
use std::sync::Arc;
use tauri::{AppHandle, Emitter};
use tokio::sync::Mutex as AsyncMutex;
use tracing::{debug, error, info};
use uuid::Uuid;

mod migration;
mod storage;
pub mod types;
mod validation;

pub use storage::{load_config, save_config};
pub use types::*;

/// Callback type for syncing clients to external managers
pub type ClientSyncCallback = Arc<dyn Fn(Vec<Client>) + Send + Sync>;

/// Thread-safe configuration manager with file watching and event emission
pub struct ConfigManager {
    config: Arc<RwLock<AppConfig>>,
    config_path: PathBuf,
    app_handle: Option<AppHandle>,
    /// Optional callback to sync clients to ClientManager when config changes
    client_sync_callback: Option<ClientSyncCallback>,
    /// Mutex to serialize disk writes, preventing concurrent save races
    save_mutex: Arc<AsyncMutex<()>>,
}

// Manual Debug implementation since AppHandle doesn't implement Debug
impl std::fmt::Debug for ConfigManager {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ConfigManager")
            .field("config", &self.config)
            .field("config_path", &self.config_path)
            .field("app_handle", &self.app_handle.is_some())
            .field("client_sync_callback", &self.client_sync_callback.is_some())
            .field("save_mutex", &"AsyncMutex<()>")
            .finish()
    }
}

// Manual Clone implementation - callback is cloned by Arc
impl Clone for ConfigManager {
    fn clone(&self) -> Self {
        Self {
            config: self.config.clone(),
            config_path: self.config_path.clone(),
            app_handle: self.app_handle.clone(),
            client_sync_callback: self.client_sync_callback.clone(),
            save_mutex: self.save_mutex.clone(),
        }
    }
}

impl ConfigManager {
    /// Create a new configuration manager
    pub fn new(config: AppConfig, config_path: PathBuf) -> Self {
        Self {
            config: Arc::new(RwLock::new(config)),
            config_path,
            app_handle: None,
            client_sync_callback: None,
            save_mutex: Arc::new(AsyncMutex::new(())),
        }
    }

    /// Load configuration from default location
    pub async fn load() -> AppResult<Self> {
        let config_path = lr_utils::paths::config_file()?;
        let config = load_config(&config_path).await?;
        Ok(Self::new(config, config_path))
    }

    /// Load configuration with custom path
    pub async fn load_from_path(path: PathBuf) -> AppResult<Self> {
        let config = load_config(&path).await?;
        Ok(Self::new(config, path))
    }

    /// Set the Tauri app handle for event emission
    ///
    /// This enables the config manager to emit events to the frontend when the config changes.
    /// Call this during app setup, after the ConfigManager is created.
    pub fn set_app_handle(&mut self, app_handle: AppHandle) {
        self.app_handle = Some(app_handle);
    }

    /// Set a callback to sync clients when config changes
    ///
    /// This callback is invoked whenever clients are modified in the config,
    /// allowing the ClientManager to stay in sync automatically.
    pub fn set_client_sync_callback(&mut self, callback: ClientSyncCallback) {
        self.client_sync_callback = Some(callback);
    }

    /// Sync clients to the registered callback (if any)
    fn sync_clients(&self) {
        if let Some(ref callback) = self.client_sync_callback {
            let clients = self.config.read().clients.clone();
            callback(clients);
        }
    }

    /// Start watching the configuration file for changes
    ///
    /// When the config file changes externally (e.g., user edits it), this will:
    /// 1. Reload the configuration from disk
    /// 2. Emit a "config-changed" event to the frontend
    ///
    /// Returns a file watcher that must be kept alive. Drop it to stop watching.
    pub fn start_watching(&self) -> AppResult<RecommendedWatcher> {
        let config_path = self.config_path.clone();
        let config_arc = self.config.clone();
        let app_handle = self.app_handle.clone();

        // Capture the Tokio runtime handle for spawning tasks from the file watcher thread
        let runtime_handle = tokio::runtime::Handle::current();

        let mut watcher =
            notify::recommended_watcher(move |result: Result<Event, notify::Error>| {
                match result {
                    Ok(event) => {
                        // Only respond to modify events
                        if matches!(event.kind, EventKind::Modify(_)) {
                            info!("Configuration file changed, reloading...");

                            // Reload config from disk (blocking operation in event handler)
                            let config_path_clone = config_path.clone();
                            let config_arc_clone = config_arc.clone();
                            let app_handle_clone = app_handle.clone();

                            // Use the captured runtime handle to spawn the task
                            runtime_handle.spawn(async move {
                                match load_config(&config_path_clone).await {
                                    Ok(new_config) => {
                                        // Update in-memory config
                                        *config_arc_clone.write() = new_config.clone();

                                        info!("Configuration reloaded successfully");

                                        // Emit event to frontend
                                        if let Some(handle) = app_handle_clone {
                                            if let Err(e) =
                                                handle.emit("config-changed", &new_config)
                                            {
                                                error!(
                                                    "Failed to emit config-changed event: {}",
                                                    e
                                                );
                                            } else {
                                                debug!("Emitted config-changed event to frontend");
                                            }
                                        }
                                    }
                                    Err(e) => {
                                        error!("Failed to reload configuration: {}", e);
                                    }
                                }
                            });
                        }
                    }
                    Err(e) => {
                        error!("File watch error: {}", e);
                    }
                }
            })
            .map_err(|e| AppError::Config(format!("Failed to create file watcher: {}", e)))?;

        // Watch the config file
        watcher
            .watch(&self.config_path, RecursiveMode::NonRecursive)
            .map_err(|e| AppError::Config(format!("Failed to watch config file: {}", e)))?;

        info!(
            "Started watching configuration file: {:?}",
            self.config_path
        );
        Ok(watcher)
    }

    /// Get a read-only copy of the configuration
    pub fn get(&self) -> AppConfig {
        self.config.read().clone()
    }

    /// Update configuration with a function
    ///
    /// Updates the in-memory configuration and validates it.
    /// To persist changes, call `save()` afterwards.
    /// Emits "config-changed" event to frontend if app handle is set.
    pub fn update<F>(&self, f: F) -> AppResult<()>
    where
        F: FnOnce(&mut AppConfig),
    {
        let updated_config = {
            let mut config = self.config.write();
            // Clone before mutating so we can roll back if validation fails
            let mut new_config = config.clone();
            f(&mut new_config);
            validation::validate_config(&new_config)?;
            *config = new_config.clone();
            new_config
        };

        // Sync clients to ClientManager if callback is registered
        // This ensures in-memory state stays in sync with config
        self.sync_clients();

        // Emit event to frontend
        self.emit_config_changed(&updated_config);

        Ok(())
    }

    /// Save configuration to disk
    ///
    /// Writes the current in-memory configuration to the config file.
    /// Serialized by a mutex to prevent concurrent disk writes from racing.
    /// Does NOT emit event (file watcher will handle that).
    pub async fn save(&self) -> AppResult<()> {
        // Serialize saves: if another save is in progress, wait for it to finish.
        // After acquiring the lock, we clone the latest in-memory config so that
        // queued saves always write the most up-to-date state.
        let _guard = self.save_mutex.lock().await;
        let config = self.config.read().clone();
        save_config(&config, &self.config_path).await
    }

    /// Manually reload configuration from disk
    ///
    /// Useful for forcing a reload without waiting for file watcher.
    /// Emits "config-changed" event to frontend.
    pub async fn reload(&self) -> AppResult<()> {
        let new_config = load_config(&self.config_path).await?;
        *self.config.write() = new_config.clone();

        info!("Configuration reloaded manually");

        // Emit event to frontend
        self.emit_config_changed(&new_config);

        Ok(())
    }

    /// Emit config-changed event to frontend
    fn emit_config_changed(&self, config: &AppConfig) {
        if let Some(ref handle) = self.app_handle {
            if let Err(e) = handle.emit("config-changed", config) {
                error!("Failed to emit config-changed event: {}", e);
            } else {
                debug!("Emitted config-changed event to frontend");
            }
        }
    }

    /// Get the configuration file path
    pub fn config_path(&self) -> &PathBuf {
        &self.config_path
    }

    /// Get global filesystem roots
    ///
    /// Returns a clone of the configured global roots for MCP servers.
    /// Use with per-client roots to determine final roots for a session.
    pub fn get_roots(&self) -> Vec<RootConfig> {
        let config = self.config.read();
        config.roots.clone()
    }

    /// Create a client with an auto-created strategy
    pub fn create_client_with_strategy(&self, name: String) -> AppResult<(Client, Strategy)> {
        let client_id = Uuid::new_v4().to_string();
        let strategy = Strategy::new_for_client(client_id.clone(), name.clone());

        let client = Client {
            id: client_id,
            name,
            enabled: true,
            strategy_id: strategy.id.clone(),
            allowed_llm_providers: Vec::new(),
            mcp_server_access: McpServerAccess::None,
            mcp_deferred_loading: false,
            skills_access: SkillsAccess::None,
            created_at: Utc::now(),
            last_used: None,
            roots: None,
            mcp_sampling_enabled: false,
            mcp_sampling_requires_approval: true,
            mcp_sampling_max_tokens: None,
            mcp_sampling_rate_limit: None,
            firewall: FirewallRules::default(),
            marketplace_enabled: false,
            mcp_permissions: McpPermissions::default(),
            skills_permissions: SkillsPermissions::default(),
            model_permissions: ModelPermissions::default(),
            marketplace_permission: PermissionState::default(),
            client_mode: ClientMode::default(),
            template_id: None,
        };

        self.update(|cfg| {
            cfg.clients.push(client.clone());
            cfg.strategies.push(strategy.clone());
        })?;

        Ok((client, strategy))
    }

    /// Delete a client and cascade delete its owned strategies
    pub fn delete_client(&self, client_id: &str) -> AppResult<()> {
        self.update(|cfg| {
            // Collect strategy IDs owned by this client (will be cascade deleted)
            let owned_strategy_ids: Vec<String> = cfg
                .strategies
                .iter()
                .filter(|s| s.parent.as_ref() == Some(&client_id.to_string()))
                .map(|s| s.id.clone())
                .collect();

            // Remove client
            cfg.clients.retain(|c| c.id != client_id);

            // Cascade delete owned strategies
            cfg.strategies
                .retain(|s| s.parent.as_ref() != Some(&client_id.to_string()));

            // Clean up any other clients that reference the deleted strategies
            // (e.g., test clients created by "Try It Out")
            cfg.clients
                .retain(|c| !owned_strategy_ids.contains(&c.strategy_id));
        })?;

        Ok(())
    }

    /// Assign a client to a different strategy (clears parent if selecting non-owned strategy)
    pub fn assign_client_strategy(&self, client_id: &str, new_strategy_id: &str) -> AppResult<()> {
        // First check if client exists
        {
            let cfg = self.config.read();
            if !cfg.clients.iter().any(|c| c.id == client_id) {
                return Err(AppError::Config("Client not found".into()));
            }
        }

        self.update(|cfg| {
            if let Some(client) = cfg.clients.iter_mut().find(|c| c.id == client_id) {
                let old_strategy_id = client.strategy_id.clone();

                // If selecting a different strategy (not own), clear parent from that strategy
                if old_strategy_id != new_strategy_id {
                    if let Some(new_strategy) =
                        cfg.strategies.iter_mut().find(|s| s.id == new_strategy_id)
                    {
                        // Clear parent if it's not the current client
                        if new_strategy.parent.as_ref() != Some(&client_id.to_string()) {
                            new_strategy.parent = None;
                        }
                    }
                }

                client.strategy_id = new_strategy_id.to_string();
            }
        })
    }

    /// Rename a strategy (clears parent if changing from default name)
    pub fn rename_strategy(&self, strategy_id: &str, new_name: &str) -> AppResult<()> {
        // First check if strategy exists
        {
            let cfg = self.config.read();
            if !cfg.strategies.iter().any(|s| s.id == strategy_id) {
                return Err(AppError::Config("Strategy not found".into()));
            }
        }

        self.update(|cfg| {
            if let Some(strategy) = cfg.strategies.iter_mut().find(|s| s.id == strategy_id) {
                // Check if renaming from default pattern
                if let Some(parent_id) = &strategy.parent {
                    if let Some(client) = cfg.clients.iter().find(|c| c.id == *parent_id) {
                        let default_name = format!("{}'s strategy", client.name);
                        if strategy.name == default_name && new_name != default_name {
                            // Clear parent when customizing name
                            strategy.parent = None;
                        }
                    }
                }

                strategy.name = new_name.to_string();
            }
        })
    }
}
