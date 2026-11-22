//! Configuration management module
//!
//! Handles loading, saving, and managing application configuration.
//! Supports file watching and event emission for real-time config updates.

use chrono::Utc;
use lr_types::{AppError, AppResult};
use parking_lot::RwLock;
use std::path::PathBuf;
use std::sync::Arc;
use tauri::{AppHandle, Emitter};
use tokio::sync::Mutex as AsyncMutex;
use tracing::{debug, error, info, warn};
use uuid::Uuid;

mod upstream_tools;
pub use upstream_tools::*;
mod routing_profiles;
pub use routing_profiles::*;
mod named_routes;
pub use named_routes::*;
mod push_relay;
pub use push_relay::*;
mod client_security;
pub use client_security::*;
mod response_cache;
pub use response_cache::*;
mod adaptive_routing;
pub mod known_client_tools;
mod migration;
mod routing_policy;
pub use adaptive_routing::*;
mod storage;
pub mod types;
pub use routing_policy::*;
mod validation;
mod watcher;

pub use storage::{content_hash, load_config, save_config, ContentHash};
pub use types::*;
pub use watcher::{ConfigWatcher, Debounce, ReloadGate, Seen};

/// Callback type for syncing clients to external managers
pub type ClientSyncCallback = Arc<dyn Fn(Vec<Client>) + Send + Sync>;

/// Emitted (payload: [`ConfigReloaded`]) after the config file was re-read
/// from disk — by the file watcher or a manual reload — and either replaced
/// the running configuration or cleared an earlier rejected edit.
pub const EVENT_CONFIG_RELOADED: &str = "config-reloaded";

/// Emitted (payload: the error message) when the config file on disk could
/// not be applied. The running configuration is unchanged.
pub const EVENT_CONFIG_RELOAD_FAILED: &str = "config-reload-failed";

/// Payload of [`EVENT_CONFIG_RELOADED`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct ConfigReloaded {
    /// Whether the running configuration changed. `false` means the file is
    /// simply valid again (an earlier rejected edit was reverted or fixed
    /// without changing any setting).
    pub applied: bool,
}

/// What looking at the config file on disk led to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReloadOutcome {
    /// The file holds what the app last wrote or loaded.
    Unchanged,
    /// New content that parses to the configuration already running
    /// (a comment or formatting edit).
    Equivalent,
    /// New content, validated and now the running configuration.
    Applied,
    /// The file is not there or is empty (mid-save, or deleted). Nothing
    /// changed.
    Missing,
    /// This broken content was already reported. Nothing changed.
    StillRejected,
}

/// Thread-safe configuration manager with file watching and event emission
pub struct ConfigManager {
    config: Arc<RwLock<AppConfig>>,
    config_path: PathBuf,
    app_handle: Option<AppHandle>,
    /// Optional callback to sync clients to ClientManager when config changes
    client_sync_callback: Option<ClientSyncCallback>,
    /// Mutex to serialize disk writes, preventing concurrent save races.
    /// Reloads from disk hold it too, so a reload never sees the file from
    /// one side of a save and `reload_gate` from the other.
    save_mutex: Arc<AsyncMutex<()>>,
    /// What the app knows about the file's content; tells its own writes
    /// from outside edits. Read and written only under `save_mutex`.
    reload_gate: Arc<parking_lot::Mutex<ReloadGate>>,
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
            reload_gate: self.reload_gate.clone(),
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
            reload_gate: Arc::new(parking_lot::Mutex::new(ReloadGate::default())),
        }
    }

    /// Load configuration from default location
    pub async fn load() -> AppResult<Self> {
        Self::load_from_path(lr_utils::paths::config_file()?).await
    }

    /// Load configuration with custom path
    pub async fn load_from_path(path: PathBuf) -> AppResult<Self> {
        let config = load_config(&path).await?;
        let manager = Self::new(config, path);
        manager.seed_reload_gate().await;
        Ok(manager)
    }

    /// Record the file's current content as the one the running configuration
    /// corresponds to, unless something already is. Without this, the first
    /// file event after startup would look like an outside edit.
    async fn seed_reload_gate(&self) {
        let _guard = self.save_mutex.lock().await;
        if self.reload_gate.lock().is_seeded() {
            return;
        }
        if let Ok(bytes) = tokio::fs::read(&self.config_path).await {
            self.reload_gate.lock().record_synced(content_hash(&bytes));
        }
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

    /// Start watching the configuration file for outside edits.
    ///
    /// An edit made outside the app (in an editor, by a script, by restoring
    /// a backup) is validated and, if it is good, becomes the running
    /// configuration exactly as a manual [`Self::reload`] would make it. The
    /// app's own saves are recognized by content and do not cause a reload.
    /// See [`Self::reload_from_disk`] for what happens to a bad edit.
    ///
    /// Call after [`Self::set_app_handle`] (the watcher emits through this
    /// instance's handle), inside a Tokio runtime. Watching stops when the
    /// returned handle is dropped.
    pub fn start_watching(&self) -> AppResult<ConfigWatcher> {
        self.start_watching_with(Debounce::default())
    }

    /// [`Self::start_watching`] with explicit debounce timings.
    pub fn start_watching_with(&self, debounce: Debounce) -> AppResult<ConfigWatcher> {
        // A manager that was not loaded from the file (startup fell back to
        // defaults) has nothing recorded yet. If a save holds the mutex right
        // now, that save records its own hash.
        if let Ok(_guard) = self.save_mutex.try_lock() {
            let mut gate = self.reload_gate.lock();
            if !gate.is_seeded() {
                if let Ok(bytes) = std::fs::read(&self.config_path) {
                    gate.record_synced(content_hash(&bytes));
                }
            }
        }
        ConfigWatcher::start(self.clone(), debounce)
    }

    /// Get a read-only copy of the configuration
    pub fn get(&self) -> AppConfig {
        self.config.read().clone()
    }

    /// Read only one client's privacy policy without cloning credentials or
    /// unrelated provider/model configuration for each monitor update.
    pub fn client_privacy(&self, client_id: &str) -> ClientPrivacyConfig {
        self.config
            .read()
            .clients
            .iter()
            .find(|client| client.id == client_id)
            .map(|client| client.security.privacy.clone())
            .unwrap_or_default()
    }

    /// Read one client's remembered secret decisions without copying provider credentials.
    pub fn client_secret_scanning(&self, client_id: &str) -> Option<ClientSecretScanningConfig> {
        self.config
            .read()
            .clients
            .iter()
            .find(|client| client.id == client_id)
            .map(|client| client.secret_scanning.clone())
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

    /// Update machine-recorded derived state, such as remembered upstream
    /// capabilities.
    ///
    /// Validated like [`Self::update`], but emits no `config-changed` event
    /// and does not sync clients: nothing the user configured changed, so the
    /// UI must not reload as if it had. To persist, call `save()` afterwards.
    pub fn update_derived<F>(&self, f: F) -> AppResult<()>
    where
        F: FnOnce(&mut AppConfig),
    {
        let mut config = self.config.write();
        let mut new_config = config.clone();
        f(&mut new_config);
        validation::validate_config(&new_config)?;
        *config = new_config;
        Ok(())
    }

    /// Record upstream capability snapshots on their MCP server entries.
    ///
    /// Only entries whose content actually changed are written (the
    /// observation time alone is not a change), and servers that no longer
    /// exist are ignored. Returns whether anything changed, i.e. whether the
    /// caller needs to `save()`.
    pub fn record_capability_snapshots(
        &self,
        snapshots: &std::collections::HashMap<String, UpstreamCapabilitySnapshot>,
    ) -> AppResult<bool> {
        let changed = |server: &McpServerConfig| {
            snapshots.get(&server.id).is_some_and(|snapshot| {
                !server
                    .capability_snapshot
                    .as_ref()
                    .is_some_and(|existing| existing.same_content(snapshot))
            })
        };
        if !self.config.read().mcp_servers.iter().any(changed) {
            return Ok(false);
        }
        self.update_derived(|cfg| {
            for server in &mut cfg.mcp_servers {
                if changed(server) {
                    server.capability_snapshot = snapshots.get(&server.id).cloned();
                }
            }
        })?;
        Ok(true)
    }

    /// Save configuration to disk
    ///
    /// Writes the current in-memory configuration to the config file.
    /// Serialized by a mutex to prevent concurrent disk writes from racing.
    /// Does NOT emit an event: `update()` already did, and the file watcher
    /// recognizes this write as the app's own.
    pub async fn save(&self) -> AppResult<()> {
        // Serialize saves: if another save is in progress, wait for it to finish.
        // After acquiring the lock, we clone the latest in-memory config so that
        // queued saves always write the most up-to-date state.
        let _guard = self.save_mutex.lock().await;
        self.warn_if_overwriting_an_unloaded_edit().await;
        let config = self.config.read().clone();
        let written = storage::save_config_hashed(&config, &self.config_path).await?;
        // Recorded while still holding the mutex: the watcher takes it before
        // looking at the file, so it finds either the previous content with
        // the previous hash, or this content with this hash — never a mix.
        self.reload_gate.lock().record_synced(written);
        Ok(())
    }

    /// A save that lands while an outside edit is still waiting out the
    /// watcher's debounce replaces that edit. Its content survives in the
    /// timestamped backup the save makes first; say so in the log, since
    /// nothing else would. Call with `save_mutex` held.
    async fn warn_if_overwriting_an_unloaded_edit(&self) -> bool {
        let Ok(bytes) = tokio::fs::read(&self.config_path).await else {
            return false;
        };
        let gate = self.reload_gate.lock();
        let unloaded = gate.is_seeded() && gate.check(&content_hash(&bytes)) == Seen::New;
        drop(gate);
        if unloaded && !bytes.iter().all(u8::is_ascii_whitespace) {
            warn!(
                "Configuration file {:?} was edited outside the app and the edit was not \
                 loaded yet; the app's save replaces it (the edited file is kept as a \
                 settings.yaml.backup.* next to it)",
                self.config_path
            );
            return true;
        }
        false
    }

    /// Manually reload configuration from disk
    ///
    /// Forces a reload even when the file holds what the app last wrote
    /// (discarding in-memory changes that were never saved). The file is
    /// validated first; if it is missing or invalid the running configuration
    /// is kept and the error returned.
    /// Emits "config-changed" event to frontend.
    pub async fn reload(&self) -> AppResult<()> {
        match self.reload_from_disk(true).await? {
            ReloadOutcome::Missing => Err(AppError::Config(format!(
                "Configuration file {:?} is missing or empty",
                self.config_path
            ))),
            _ => Ok(()),
        }
    }

    /// Look at the config file and make it the running configuration if it
    /// holds something new and valid. The one path by which disk content
    /// reaches a running app: the manual reload (`force`) and the file
    /// watcher both come through here.
    ///
    /// - Content the app wrote or already loaded is left alone (unless
    ///   `force`): this is what keeps the app's own saves from reloading it.
    /// - New content is parsed, migrated and validated **before** anything
    ///   changes. If that fails the running configuration stays as it is,
    ///   the file is not touched (no backup is restored over the user's
    ///   edit), [`EVENT_CONFIG_RELOAD_FAILED`] is emitted and the error is
    ///   returned. The same broken content is reported once.
    /// - Valid new content replaces the running configuration, the client
    ///   manager is synced, and "config-changed" plus
    ///   [`EVENT_CONFIG_RELOADED`] are emitted.
    ///
    /// Nothing here writes the config file, so a reload cannot trigger
    /// another reload.
    pub async fn reload_from_disk(&self, force: bool) -> AppResult<ReloadOutcome> {
        // Held across read, validation and swap: a concurrent save either
        // finished before the file is read (and its hash is recorded) or
        // starts after the new configuration is in place.
        let guard = self.save_mutex.lock().await;

        let bytes = match tokio::fs::read(&self.config_path).await {
            Ok(bytes) => bytes,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                debug!(
                    "Config file {:?} is not there; keeping the running configuration",
                    self.config_path
                );
                return Ok(ReloadOutcome::Missing);
            }
            Err(e) => {
                return Err(AppError::Config(format!(
                    "Failed to read configuration file: {}",
                    e
                )))
            }
        };
        let hash = content_hash(&bytes);

        let seen = self.reload_gate.lock().check(&hash);
        match seen {
            Seen::Synced if !force => {
                let recovered = self.reload_gate.lock().clear_rejected();
                drop(guard);
                if recovered {
                    info!("Configuration file is back to the running configuration");
                    self.emit_config_reloaded(false);
                }
                return Ok(ReloadOutcome::Unchanged);
            }
            Seen::Rejected if !force => return Ok(ReloadOutcome::StillRejected),
            _ => {}
        }

        // An editor that truncates and rewrites in place can be caught with
        // an empty file; that is a save in progress, not a configuration —
        // and must never be read as "every setting at its default".
        if bytes.iter().all(u8::is_ascii_whitespace) {
            debug!("Config file is empty; waiting for the rest of the save");
            return Ok(ReloadOutcome::Missing);
        }

        let new_config = match storage::parse_config_bytes(&bytes) {
            Ok(config) => config,
            Err(e) => {
                self.reload_gate.lock().record_rejected(hash);
                drop(guard);
                warn!(
                    "Configuration file changed on disk but was not applied; \
                     keeping the running configuration: {}",
                    e
                );
                self.emit_config_reload_failed(&e.to_string());
                return Err(e);
            }
        };

        let changed = {
            let mut config = self.config.write();
            let changed = *config != new_config;
            if changed {
                *config = new_config.clone();
            }
            changed
        };
        let recovered = self.reload_gate.lock().record_synced(hash);
        drop(guard);

        if !changed {
            debug!("Configuration file changed on disk without changing any setting");
            if recovered {
                self.emit_config_reloaded(false);
            }
            return Ok(ReloadOutcome::Equivalent);
        }

        storage::after_load(&new_config);
        info!("Configuration reloaded from disk");

        // Same order as `update()`: the client manager first, so listeners of
        // the events below already see the new clients.
        self.sync_clients();
        self.emit_config_changed(&new_config);
        self.emit_config_reloaded(true);

        Ok(ReloadOutcome::Applied)
    }

    fn emit_config_reloaded(&self, applied: bool) {
        if let Some(ref handle) = self.app_handle {
            if let Err(e) = handle.emit(EVENT_CONFIG_RELOADED, ConfigReloaded { applied }) {
                error!("Failed to emit {} event: {}", EVENT_CONFIG_RELOADED, e);
            }
        }
    }

    fn emit_config_reload_failed(&self, message: &str) {
        if let Some(ref handle) = self.app_handle {
            if let Err(e) = handle.emit(EVENT_CONFIG_RELOAD_FAILED, message) {
                error!("Failed to emit {} event: {}", EVENT_CONFIG_RELOAD_FAILED, e);
            }
        }
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

        // Generate unique memory folder slug
        let existing_folders: Vec<String> = self
            .get()
            .clients
            .iter()
            .filter_map(|c| c.memory_folder.clone())
            .collect();
        let memory_folder = unique_memory_folder(&name, &existing_folders);

        let client = Client {
            id: client_id,
            name,
            enabled: true,
            strategy_id: strategy.id.clone(),
            security: Default::default(),
            allowed_llm_providers: Vec::new(),
            mcp_server_access: McpServerAccess::None,
            context_management_enabled: None,
            catalog_compression_enabled: None,
            client_tools_indexing: None,
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
            llm_mode: LlmMode::default(),
            mcp_mode: McpMode::default(),
            template_id: None,
            sync_config: false,
            reverse_proxy: None,
            guardrails_enabled: None,
            guardrails: ClientGuardrailsConfig::default(),
            prompt_compression: ClientPromptCompressionConfig::default(),
            json_repair: ClientJsonRepairConfig::default(),
            secret_scanning: ClientSecretScanningConfig::default(),
            coding_agents_permissions: CodingAgentsPermissions::default(),
            coding_agent_permission: PermissionState::default(),
            coding_agent_type: None,
            mcp_sampling_permission: PermissionState::Ask,
            mcp_elicitation_permission: PermissionState::Ask,
            memory_enabled: None,
            memory_folder: Some(memory_folder),
            push_permissions: PushPermissions::default(),
            push_webhook_urls: Vec::new(),
        };

        self.update(|cfg| {
            cfg.clients.push(client.clone());
            cfg.strategies.push(strategy.clone());
            // New clients show up in the tray stats automatically (and the
            // first one turns labels on). Clone has its own push and calls
            // this too; "Try it out" test clients deliberately don't.
            cfg.ui.tray_stats.on_client_created(&client.id);
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

            // Drop its tray stats row so the icon/menu don't keep a ghost panel
            cfg.ui.tray_stats.remove_source(&TraySource::Client {
                id: client_id.to_string(),
            });

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
}

#[cfg(test)]
mod reload_tests;
mod session_routing;
mod ui_state;
pub use session_routing::SessionRoutingConfig;
pub use ui_state::*;
