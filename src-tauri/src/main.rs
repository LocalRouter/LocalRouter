// Prevents additional console window on Windows in release
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod cli;
mod ui;
mod updater;

use std::sync::Arc;

use tauri::{Listener, Manager};
use tracing::{debug, error, info, warn};
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};

// Re-exported crate aliases from lib.rs
use localrouter::{
    api_keys, clients, config, marketplace, mcp, monitoring, oauth_browser, oauth_clients,
    providers, routellm, router, server, skills, utils,
};

use lr_providers::factory::{
    AnthropicProviderFactory, CerebrasProviderFactory, CohereProviderFactory,
    DeepInfraProviderFactory, GeminiProviderFactory, GitHubCopilotProviderFactory,
    GroqProviderFactory, LMStudioProviderFactory, MistralProviderFactory, OllamaProviderFactory,
    OpenAICodexProviderFactory, OpenAICompatibleProviderFactory, OpenAIProviderFactory,
    OpenRouterProviderFactory, PerplexityProviderFactory, TogetherAIProviderFactory,
    XAIProviderFactory,
};
use lr_providers::registry::ProviderRegistry;
use lr_server::ServerManager;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // Parse CLI arguments
    let args = cli::Cli::parse_args();

    // Initialize logging (always to stderr for bridge mode)
    init_logging();

    // Branch based on mode
    if args.mcp_bridge {
        run_bridge_mode(args.client_id).await
    } else {
        run_gui_mode().await
    }
}

/// Initialize logging to stderr
///
/// In bridge mode, stdout is reserved for JSON-RPC responses,
/// so all logging must go to stderr.
fn init_logging() {
    tracing_subscriber::registry()
        .with(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "localrouter=info".into()),
        )
        .with(
            tracing_subscriber::fmt::layer().with_writer(std::io::stderr), // Always to stderr
        )
        .init();
}

/// Run in MCP bridge mode (STDIO ↔ HTTP proxy)
///
/// This is a lightweight mode that reads JSON-RPC requests from stdin,
/// forwards them to the running LocalRouter HTTP server, and writes
/// responses back to stdout.
///
/// # Arguments
/// * `client_id` - Optional client ID (auto-detects if None)
///
/// # Returns
/// Ok on clean shutdown, Err on fatal errors
async fn run_bridge_mode(client_id: Option<String>) -> anyhow::Result<()> {
    eprintln!("==========================================================");
    eprintln!("LocalRouter - MCP Bridge Mode");
    eprintln!("==========================================================");
    eprintln!();
    eprintln!("Connecting to LocalRouter server at http://localhost:3625");
    eprintln!("Make sure the LocalRouter GUI is running!");
    eprintln!();

    // Create and run bridge (loads config for client secret only)
    let bridge = mcp::StdioBridge::new(client_id, None).await.map_err(|e| {
        eprintln!("ERROR: Failed to initialize bridge: {}", e);
        eprintln!();
        eprintln!("Common issues:");
        eprintln!("  - LocalRouter GUI not running (start it first)");
        eprintln!("  - Client not configured in config.yaml");
        eprintln!("  - Client secret not found (run GUI once)");
        eprintln!("  - LOCALROUTER_CLIENT_SECRET not set (if using env var)");
        eprintln!();
        e
    })?;

    eprintln!("Bridge ready! Forwarding JSON-RPC requests...");
    eprintln!("==========================================================");
    eprintln!();

    bridge.run().await.map_err(|e| {
        eprintln!("ERROR: Bridge stopped: {}", e);
        e.into()
    })
}

/// Run in GUI mode (full desktop application)
///
/// This is the default mode that starts the HTTP server, managers,
/// and Tauri desktop window.
async fn run_gui_mode() -> anyhow::Result<()> {
    info!("Starting LocalRouter...");

    // Log configuration directory
    let config_dir =
        lr_utils::paths::config_dir().unwrap_or_else(|_| std::path::PathBuf::from("unknown"));
    #[cfg(debug_assertions)]
    info!("Running in DEVELOPMENT mode");
    #[cfg(not(debug_assertions))]
    info!("Running in PRODUCTION mode");
    info!("Configuration directory: {}", config_dir.display());

    // Initialize managers
    let mut config_manager = config::ConfigManager::load().await.unwrap_or_else(|e| {
        tracing::warn!("Failed to load config, using defaults: {}", e);
        config::ConfigManager::new(
            config::AppConfig::default(),
            lr_utils::paths::config_file().unwrap(),
        )
    });

    // Initialize unified client manager
    // Replaces both API key manager and OAuth client manager
    // Client secrets are stored in OS keychain, only metadata in config
    let client_manager = {
        let config = config_manager.get();
        Arc::new(clients::ClientManager::new(config.clients.clone()))
    };

    // Register client sync callback to keep ClientManager in sync with config
    // This prevents bugs where config changes aren't reflected in ClientManager
    {
        let client_manager_for_sync = client_manager.clone();
        config_manager.set_client_sync_callback(std::sync::Arc::new(move |clients| {
            client_manager_for_sync.sync_clients(clients);
        }));
    }

    // Initialize OAuth token store for short-lived access tokens
    // Tokens are stored in-memory only (1 hour expiry)
    let token_store = Arc::new(clients::TokenStore::new());

    // Initialize OAuth client manager for MCP server authentication
    let oauth_client_manager = {
        let config = config_manager.get();
        Arc::new(oauth_clients::OAuthClientManager::new(
            config.oauth_clients.clone(),
        ))
    };

    // Initialize keychain for secure storage
    let keychain = api_keys::keychain_trait::CachedKeychain::auto().unwrap_or_else(|e| {
        error!("Failed to initialize keychain: {}", e);
        api_keys::keychain_trait::CachedKeychain::system()
    });

    // Initialize MCP server manager
    let mcp_server_manager = {
        let config = config_manager.get();
        let manager = Arc::new(mcp::McpServerManager::new());
        manager.load_configs(config.mcp_servers.clone());
        manager
    };

    // Initialize MCP OAuth managers
    let mcp_oauth_manager = Arc::new(mcp::oauth::McpOAuthManager::new());
    let mcp_oauth_browser_manager = Arc::new(mcp::oauth_browser::McpOAuthBrowserManager::new(
        keychain.clone(),
        mcp_oauth_manager.clone(),
    ));

    // Initialize unified OAuth flow manager for inline OAuth flows
    let oauth_flow_manager = Arc::new(oauth_browser::OAuthFlowManager::new(keychain.clone()));

    // Initialize provider registry
    info!("Initializing provider registry...");
    let provider_registry = Arc::new(ProviderRegistry::new());

    // Register provider factories
    info!("Registering provider factories...");
    provider_registry.register_factory(Arc::new(OllamaProviderFactory));
    provider_registry.register_factory(Arc::new(OpenAIProviderFactory));
    provider_registry.register_factory(Arc::new(OpenAICompatibleProviderFactory));
    provider_registry.register_factory(Arc::new(AnthropicProviderFactory));
    provider_registry.register_factory(Arc::new(GeminiProviderFactory));
    provider_registry.register_factory(Arc::new(OpenRouterProviderFactory));
    provider_registry.register_factory(Arc::new(GroqProviderFactory));
    provider_registry.register_factory(Arc::new(MistralProviderFactory));
    provider_registry.register_factory(Arc::new(CohereProviderFactory));
    provider_registry.register_factory(Arc::new(TogetherAIProviderFactory));
    provider_registry.register_factory(Arc::new(PerplexityProviderFactory));
    provider_registry.register_factory(Arc::new(DeepInfraProviderFactory));
    provider_registry.register_factory(Arc::new(CerebrasProviderFactory));
    provider_registry.register_factory(Arc::new(XAIProviderFactory));
    provider_registry.register_factory(Arc::new(LMStudioProviderFactory));
    // Subscription providers (OAuth-based)
    provider_registry.register_factory(Arc::new(GitHubCopilotProviderFactory));
    provider_registry.register_factory(Arc::new(OpenAICodexProviderFactory));
    info!(
        "Registered {} provider factories",
        provider_registry.list_provider_types().len()
    );

    // On first startup, discover local LLM providers (Ollama, LM Studio)
    {
        let config = config_manager.get();
        if !config.setup_wizard_shown && config.providers.is_empty() {
            info!("First startup detected, discovering local LLM providers...");
            let discovered = providers::factory::discover_local_providers().await;

            if !discovered.is_empty() {
                info!("Discovered {} local provider(s)", discovered.len());
                if let Err(e) = config_manager.update(|cfg| {
                    for provider in &discovered {
                        let provider_config = match provider.provider_type.as_str() {
                            "ollama" => config::ProviderConfig::default_ollama(),
                            "lmstudio" => config::ProviderConfig::default_lmstudio(),
                            _ => continue,
                        };
                        info!(
                            "Auto-configuring discovered provider: {}",
                            provider.instance_name
                        );
                        cfg.providers.push(provider_config);
                    }
                }) {
                    warn!("Failed to save discovered providers to config: {}", e);
                }
            } else {
                info!("No local LLM providers discovered");
            }
        }
    }

    // Load provider instances from configuration
    info!("Loading provider instances from configuration...");
    let providers = config_manager.get().providers;
    for provider_config in providers {
        let provider_type = match provider_config.provider_type {
            config::ProviderType::Ollama => "ollama",
            config::ProviderType::LMStudio => "lmstudio",
            config::ProviderType::OpenAI => "openai",
            config::ProviderType::Anthropic => "anthropic",
            config::ProviderType::Gemini => "gemini",
            config::ProviderType::OpenRouter => "openrouter",
            config::ProviderType::Groq => "groq",
            config::ProviderType::Mistral => "mistral",
            config::ProviderType::Cohere => "cohere",
            config::ProviderType::TogetherAI => "togetherai",
            config::ProviderType::Perplexity => "perplexity",
            config::ProviderType::DeepInfra => "deepinfra",
            config::ProviderType::Cerebras => "cerebras",
            config::ProviderType::XAI => "xai",
            config::ProviderType::Custom => "openai_compatible",
        };

        // Convert provider_config JSON to HashMap
        let mut config_map = std::collections::HashMap::new();
        if let Some(provider_cfg) = provider_config.provider_config {
            if let Some(obj) = provider_cfg.as_object() {
                for (key, value) in obj {
                    if let Some(value_str) = value.as_str() {
                        config_map.insert(key.clone(), value_str.to_string());
                    } else {
                        config_map.insert(key.clone(), value.to_string());
                    }
                }
            }
        }

        // Create the provider instance
        if let Err(e) = provider_registry
            .create_provider(
                provider_config.name.clone(),
                provider_type.to_string(),
                config_map,
            )
            .await
        {
            tracing::warn!("Failed to load provider '{}': {}", provider_config.name, e);
            continue;
        }

        // Set enabled state
        if let Err(e) =
            provider_registry.set_provider_enabled(&provider_config.name, provider_config.enabled)
        {
            tracing::warn!(
                "Failed to set provider '{}' enabled state: {}",
                provider_config.name,
                e
            );
        }
    }
    info!(
        "Loaded {} provider instances",
        config_manager.get().providers.len()
    );

    // Initialize OAuth manager for subscription-based providers
    info!("Initializing OAuth manager...");
    let oauth_storage_path = lr_utils::paths::config_dir()
        .expect("Failed to get config directory")
        .join("oauth_credentials.json");
    let oauth_storage = Arc::new(
        providers::oauth::storage::OAuthStorage::new(oauth_storage_path)
            .await
            .expect("Failed to initialize OAuth storage"),
    );
    let oauth_manager = Arc::new(providers::oauth::OAuthManager::new(oauth_storage));

    // Register OAuth providers
    info!("Registering OAuth providers...");
    oauth_manager.register_provider(Arc::new(
        providers::oauth::github_copilot::GitHubCopilotOAuthProvider::new(),
    ));
    oauth_manager.register_provider(Arc::new(
        providers::oauth::openai_codex::OpenAICodexOAuthProvider::new(keychain.clone()),
    ));
    oauth_manager.register_provider(Arc::new(
        providers::oauth::anthropic_claude::AnthropicClaudeOAuthProvider::new(keychain.clone()),
    ));
    info!("Registered 3 OAuth providers");

    // Initialize rate limiter
    info!("Initializing rate limiter...");
    let rate_limiter = Arc::new(router::RateLimiterManager::new(None));

    // Initialize metrics collector
    info!("Initializing metrics collector...");
    let metrics_db_path = lr_utils::paths::config_dir()
        .unwrap_or_else(|_| std::path::PathBuf::from("."))
        .join("metrics.db");
    let metrics_db = Arc::new(
        monitoring::storage::MetricsDatabase::new(metrics_db_path).unwrap_or_else(|e| {
            tracing::error!("Failed to initialize metrics database: {}", e);
            panic!("Metrics database initialization failed");
        }),
    );
    let metrics_collector = Arc::new(monitoring::metrics::MetricsCollector::new(metrics_db));

    // Initialize RouteLLM intelligent routing service
    info!("Initializing RouteLLM service...");
    let routellm_service = {
        let config = config_manager.get();
        let idle_timeout = config.routellm_settings.idle_timeout_secs;

        match routellm::RouteLLMService::new_with_defaults(idle_timeout) {
            Ok(service) => {
                let service_arc = Arc::new(service);
                // Start auto-unload background task
                tokio::spawn(service_arc.clone().start_auto_unload_task());
                info!(
                    "RouteLLM service initialized with idle timeout: {}s",
                    idle_timeout
                );
                Some(service_arc)
            }
            Err(e) => {
                info!("RouteLLM service not initialized: {}", e);
                None
            }
        }
    };

    // Initialize router
    info!("Initializing router...");
    let config_manager_arc = Arc::new(config_manager.clone());
    let mut app_router = router::Router::new(
        config_manager_arc.clone(),
        provider_registry.clone(),
        rate_limiter.clone(),
        metrics_collector.clone(),
    );

    // Add RouteLLM service to router
    app_router = app_router.with_routellm(routellm_service);
    let app_router = Arc::new(app_router);

    // Initialize server manager and start server
    info!("Initializing web server...");
    let server_manager = Arc::new(ServerManager::new());

    // Get server config from configuration
    let server_config = {
        let config = config_manager.get();
        server::ServerConfig {
            host: config.server.host.clone(),
            port: config.server.port,
            enable_cors: config.server.enable_cors,
        }
    };

    // Start the server (tray_graph_manager will be added later in setup())
    server_manager
        .start(
            server_config,
            lr_server::manager::ServerDependencies {
                router: app_router.clone(),
                mcp_server_manager: mcp_server_manager.clone(),
                rate_limiter: rate_limiter.clone(),
                provider_registry: provider_registry.clone(),
                config_manager: config_manager_arc.clone(),
                client_manager: client_manager.clone(),
                token_store: token_store.clone(),
                metrics_collector: metrics_collector.clone(),
            },
        )
        .await?;

    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_process::init())
        .plugin(tauri_plugin_shell::init())
        .plugin(tauri_plugin_updater::Builder::new().build())
        .setup(move |app| {
            info!("Tauri app initialized");

            // Set app handle on config manager for event emission
            config_manager.set_app_handle(app.handle().clone());

            // File watcher disabled - it was causing duplicate config-changed events
            // when saving from the app. The update() method already emits events.
            // TODO: Re-enable with proper suppression if external file editing is needed
            // let watcher = config_manager.start_watching().map_err(|e| {
            //     tracing::error!("Failed to start config file watcher: {}", e);
            //     e
            // })?;
            // app.manage(watcher);

            // Store managers
            app.manage(config_manager.clone());
            app.manage(client_manager.clone());
            app.manage(token_store.clone());
            app.manage(oauth_client_manager.clone());
            app.manage(mcp_server_manager.clone());
            app.manage(mcp_oauth_manager.clone());
            app.manage(mcp_oauth_browser_manager.clone());
            app.manage(oauth_flow_manager.clone());
            app.manage(provider_registry.clone());
            app.manage(server_manager.clone());
            app.manage(app_router.clone());
            app.manage(rate_limiter.clone());
            app.manage(oauth_manager.clone());
            app.manage(metrics_collector.clone());

            // Initialize skill manager and script executor
            let mut skill_manager = skills::SkillManager::new();
            skill_manager.set_app_handle(app.handle().clone());
            let skills_config = config_manager.get().skills.clone();
            skill_manager.initial_scan(&skills_config.paths, &skills_config.disabled_skills);
            skill_manager.start_cleanup_task();
            let skill_manager = Arc::new(skill_manager);
            let script_executor = Arc::new(skills::executor::ScriptExecutor::new());

            // Start file watcher for skill sources
            let skill_manager_for_watcher = skill_manager.clone();
            let config_manager_for_watcher = config_manager.clone();
            let watcher_paths = skills_config.paths.clone();
            match skills::SkillWatcher::start(
                watcher_paths,
                Arc::new(move |_affected_paths| {
                    let config = config_manager_for_watcher.get();
                    skill_manager_for_watcher
                        .rescan(&config.skills.paths, &config.skills.disabled_skills);
                }),
            ) {
                Ok(watcher) => {
                    app.manage(Arc::new(watcher));
                    info!("Skills file watcher started");
                }
                Err(e) => {
                    warn!("Failed to start skills file watcher: {}", e);
                }
            }

            app.manage(skill_manager.clone());
            app.manage(script_executor.clone());
            info!("Skills system initialized");

            // Initialize marketplace service (always created, checks enabled state internally)
            let marketplace_service: Option<Arc<marketplace::MarketplaceService>> = {
                let config = config_manager.get();
                let data_dir =
                    lr_utils::paths::config_dir().unwrap_or_else(|_| std::path::PathBuf::from("."));

                let service =
                    marketplace::MarketplaceService::new(config.marketplace.clone(), data_dir);
                info!(
                    "Marketplace service initialized (enabled: {})",
                    config.marketplace.enabled
                );
                Some(Arc::new(service))
            };
            app.manage(marketplace_service.clone());

            // Get AppState from server manager and manage it for Tauri commands
            if let Some(app_state) = server_manager.get_state() {
                info!("Managing AppState for Tauri commands");

                // Wire skill support into MCP gateway (uses OnceLock, so &self is fine)
                app_state
                    .mcp_gateway
                    .set_skill_support(skill_manager.clone(), script_executor.clone());
                if skills_config.async_enabled {
                    app_state.mcp_gateway.set_skills_async_enabled(true);
                }
                info!("Skills wired to MCP gateway");

                // Wire marketplace service into MCP gateway if available
                if let Some(ref service) = marketplace_service {
                    app_state
                        .mcp_gateway
                        .set_marketplace_service(service.clone());
                    info!("Marketplace wired to MCP gateway");
                }

                // Set app handle on AppState for event emission
                app_state.set_app_handle(app.handle().clone());

                let app_state = Arc::new(app_state);
                app.manage(app_state.clone());

                // Listen for client permission changes and notify connected MCP clients
                let app_state_for_clients = app_state.clone();
                let config_manager_for_notify = config_manager.clone();
                app.listen("clients-changed", move |_event| {
                    let config = config_manager_for_notify.get();
                    let all_enabled_server_ids: Vec<String> = config
                        .mcp_servers
                        .iter()
                        .filter(|s| s.enabled)
                        .map(|s| s.id.clone())
                        .collect();

                    let broadcast = app_state_for_clients.client_notification_broadcast.clone();
                    app_state_for_clients
                        .mcp_gateway
                        .check_and_notify_permission_changes(
                            &config.clients,
                            &all_enabled_server_ids,
                            |client_id, tools, resources, prompts| {
                                use mcp::gateway::streaming_notifications::StreamingNotificationType;
                                if tools {
                                    let _ = broadcast.send((
                                        client_id.to_string(),
                                        StreamingNotificationType::ToolsListChanged
                                            .to_notification(),
                                    ));
                                }
                                if resources {
                                    let _ = broadcast.send((
                                        client_id.to_string(),
                                        StreamingNotificationType::ResourcesListChanged
                                            .to_notification(),
                                    ));
                                }
                                if prompts {
                                    let _ = broadcast.send((
                                        client_id.to_string(),
                                        StreamingNotificationType::PromptsListChanged
                                            .to_notification(),
                                    ));
                                }
                                info!(
                                    "Sent permission change notifications to client {}: tools={}, resources={}, prompts={}",
                                    client_id, tools, resources, prompts
                                );
                            },
                        );
                });
                info!("Registered clients-changed listener for permission notifications");

                // Spawn firewall approval popup listener
                // Subscribes to MCP notification broadcast and opens popup windows
                // when firewall/approvalRequired notifications arrive
                let app_handle_for_firewall = app.handle().clone();
                let firewall_broadcast_rx =
                    app_state.mcp_notification_broadcast.subscribe();
                tokio::spawn(async move {
                    use tauri::WebviewWindowBuilder;
                    let mut rx = firewall_broadcast_rx;
                    loop {
                        match rx.recv().await {
                            Ok((channel, notification)) => {
                                if channel != "_firewall" {
                                    continue;
                                }
                                // Extract request_id from notification params
                                let request_id = notification
                                    .params
                                    .as_ref()
                                    .and_then(|p| p.get("request_id"))
                                    .and_then(|v| v.as_str())
                                    .unwrap_or("")
                                    .to_string();

                                if request_id.is_empty() {
                                    tracing::warn!(
                                        "Firewall notification missing request_id, skipping"
                                    );
                                    continue;
                                }

                                tracing::info!(
                                    "Opening firewall approval popup for request {}",
                                    request_id
                                );

                                // Rebuild tray menu to show the pending approval
                                if let Err(e) =
                                    crate::ui::tray::rebuild_tray_menu(&app_handle_for_firewall)
                                {
                                    tracing::warn!(
                                        "Failed to rebuild tray menu for firewall: {}",
                                        e
                                    );
                                }
                                if let Some(tgm) = app_handle_for_firewall
                                    .try_state::<Arc<crate::ui::tray::TrayGraphManager>>()
                                {
                                    tgm.notify_activity();
                                }

                                // Create popup window
                                match WebviewWindowBuilder::new(
                                    &app_handle_for_firewall,
                                    format!("firewall-approval-{}", request_id),
                                    tauri::WebviewUrl::App("index.html".into()),
                                )
                                .title("Approval Required")
                                .inner_size(400.0, 320.0)
                                .center()
                                .visible(true)
                                .resizable(false)
                                .decorations(true)
                                .always_on_top(true)
                                .build()
                                {
                                    Ok(window) => {
                                        let _ = window.set_focus();
                                    }
                                    Err(e) => {
                                        tracing::error!(
                                            "Failed to create firewall popup: {}",
                                            e
                                        );
                                    }
                                }
                            }
                            Err(tokio::sync::broadcast::error::RecvError::Lagged(n)) => {
                                tracing::warn!(
                                    "Firewall listener lagged, missed {} notifications",
                                    n
                                );
                            }
                            Err(tokio::sync::broadcast::error::RecvError::Closed) => {
                                tracing::info!("Firewall broadcast channel closed, stopping listener");
                                break;
                            }
                        }
                    }
                });
                info!("Spawned firewall approval popup listener");
            } else {
                error!("Failed to get AppState from server manager");
            }

            // Set up server restart event listener
            let server_manager_clone = server_manager.clone();
            let app_router_clone = app_router.clone();
            let mcp_server_manager_clone = mcp_server_manager.clone();
            let rate_limiter_clone = rate_limiter.clone();
            let provider_registry_clone = provider_registry.clone();
            let client_manager_clone = client_manager.clone();
            let token_store_clone = token_store.clone();
            let metrics_collector_clone = metrics_collector.clone();
            let config_manager_clone = config_manager.clone();
            let app_handle_for_restart = app.handle().clone();
            // Clone health cache for restart handler
            let health_cache_for_restart =
                server_manager.get_state().map(|s| s.health_cache.clone());

            app.listen("server-restart-requested", move |_event| {
                info!("Server restart requested");

                // Get current server configuration
                let server_config = {
                    let config = config_manager_clone.get();
                    server::ServerConfig {
                        host: config.server.host.clone(),
                        port: config.server.port,
                        enable_cors: config.server.enable_cors,
                    }
                };

                // Restart the server (spawn async task)
                let server_manager_clone2 = server_manager_clone.clone();
                let app_router_clone2 = app_router_clone.clone();
                let mcp_server_manager_clone2 = mcp_server_manager_clone.clone();
                let rate_limiter_clone2 = rate_limiter_clone.clone();
                let provider_registry_clone2 = provider_registry_clone.clone();
                let config_manager_clone2 = Arc::new(config_manager_clone.clone());
                let client_manager_clone2 = client_manager_clone.clone();
                let token_store_clone2 = token_store_clone.clone();
                let metrics_collector_clone2 = metrics_collector_clone.clone();
                let app_handle = app_handle_for_restart.clone();
                let health_cache_clone = health_cache_for_restart.clone();

                tokio::spawn(async move {
                    use tauri::Emitter;

                    // Update health cache and emit stopped status before restart
                    if let Some(ref health_cache) = health_cache_clone {
                        health_cache.update_server_status(false, None, None);
                    }
                    let _ = app_handle.emit("server-status-changed", "stopped");

                    match server_manager_clone2
                        .start(
                            server_config.clone(),
                            lr_server::manager::ServerDependencies {
                                router: app_router_clone2,
                                mcp_server_manager: mcp_server_manager_clone2,
                                rate_limiter: rate_limiter_clone2,
                                provider_registry: provider_registry_clone2,
                                config_manager: config_manager_clone2,
                                client_manager: client_manager_clone2,
                                token_store: token_store_clone2,
                                metrics_collector: metrics_collector_clone2,
                            },
                        )
                        .await
                    {
                        Ok(_) => {
                            info!("Server restarted successfully");
                            // Update health cache with new server status
                            if let Some(ref health_cache) = health_cache_clone {
                                health_cache.update_server_status(
                                    true,
                                    Some(server_config.host.clone()),
                                    Some(server_config.port),
                                );
                            }
                            let _ = app_handle.emit("server-status-changed", "running");
                            let _ = app_handle.emit("server-restart-completed", ());
                        }
                        Err(e) => {
                            error!("Failed to restart server: {}", e);
                            // Keep server status as stopped in health cache
                            if let Some(ref health_cache) = health_cache_clone {
                                health_cache.update_server_status(false, None, None);
                            }
                            let _ = app_handle.emit("server-status-changed", "stopped");
                            let _ = app_handle.emit("server-restart-failed", e.to_string());
                        }
                    }
                });
            });

            // Set app handle on server state for event emission
            if let Some(state) = server_manager.get_state() {
                state.set_app_handle(app.handle().clone());

                // Spawn background aggregation task for metrics
                let metrics_db = state.metrics_collector.db();
                tokio::spawn(async move {
                    let _ = monitoring::aggregation_task::spawn_aggregation_task(metrics_db).await;
                });
                info!("Spawned metrics aggregation task");

                // Initialize health cache with current providers and MCP servers
                let health_cache = state.health_cache.clone();
                {
                    use providers::health_cache::ItemHealth;

                    // Initialize providers - disabled ones get disabled status
                    let providers = provider_registry.list_providers();
                    for provider_info in providers {
                        if provider_info.enabled {
                            health_cache.update_provider(
                                &provider_info.instance_name,
                                ItemHealth::pending(provider_info.instance_name.clone()),
                            );
                        } else {
                            health_cache.update_provider(
                                &provider_info.instance_name,
                                ItemHealth::disabled(provider_info.instance_name.clone()),
                            );
                        }
                    }

                    // Initialize MCP servers - disabled ones get disabled status
                    let mcp_configs = mcp_server_manager.list_configs();
                    for config in mcp_configs {
                        if config.enabled {
                            health_cache.update_mcp_server(
                                &config.id,
                                ItemHealth::pending(config.name.clone()),
                            );
                        } else {
                            health_cache
                                .update_mcp_server(&config.id, ItemHealth::disabled(config.name));
                        }
                    }
                }

                // Set server as running with the configured host and port
                let server_config = &config_manager.get().server;
                health_cache.update_server_status(
                    true,
                    Some(server_config.host.clone()),
                    Some(server_config.port),
                );
                info!(
                    "Health cache initialized with {} providers and {} MCP servers",
                    provider_registry.list_providers().len(),
                    mcp_server_manager.list_configs().len()
                );

                // Start periodic health check task if configured
                let health_check_config = config_manager.get().health_check.clone();
                if health_check_config.mode == config::HealthCheckMode::Periodic {
                    let health_cache_for_task = state.health_cache.clone();
                    let provider_registry_for_task = provider_registry.clone();
                    let mcp_server_manager_for_task = mcp_server_manager.clone();
                    let interval_secs = health_check_config.interval_secs;
                    let timeout_secs = health_check_config.timeout_secs;

                    tokio::spawn(async move {
                        use providers::health_cache::ItemHealth;

                        let mut interval =
                            tokio::time::interval(std::time::Duration::from_secs(interval_secs));

                        loop {
                            interval.tick().await;
                            debug!("Running periodic health checks...");

                            // Check all providers
                            let providers = provider_registry_for_task.list_providers();
                            for provider_info in providers {
                                // Skip disabled providers - emit disabled status
                                if !provider_info.enabled {
                                    health_cache_for_task.update_provider(
                                        &provider_info.instance_name,
                                        ItemHealth::disabled(provider_info.instance_name.clone()),
                                    );
                                    continue;
                                }

                                if let Some(provider) = provider_registry_for_task
                                    .get_provider(&provider_info.instance_name)
                                {
                                    let health = tokio::time::timeout(
                                        std::time::Duration::from_secs(timeout_secs),
                                        provider.health_check(),
                                    )
                                    .await;

                                    let item_health = match health {
                                        Ok(h) => {
                                            use providers::HealthStatus;
                                            match h.status {
                                                HealthStatus::Healthy => ItemHealth::healthy(
                                                    provider_info.instance_name.clone(),
                                                    h.latency_ms,
                                                ),
                                                HealthStatus::Degraded => ItemHealth::degraded(
                                                    provider_info.instance_name.clone(),
                                                    h.latency_ms,
                                                    h.error_message
                                                        .unwrap_or_else(|| "Degraded".to_string()),
                                                ),
                                                HealthStatus::Unhealthy => ItemHealth::unhealthy(
                                                    provider_info.instance_name.clone(),
                                                    h.error_message
                                                        .unwrap_or_else(|| "Unhealthy".to_string()),
                                                ),
                                            }
                                        }
                                        Err(_) => ItemHealth::unhealthy(
                                            provider_info.instance_name.clone(),
                                            format!("Health check timeout ({}s)", timeout_secs),
                                        ),
                                    };
                                    health_cache_for_task
                                        .update_provider(&provider_info.instance_name, item_health);
                                }
                            }

                            // Check all MCP servers
                            let mcp_configs = mcp_server_manager_for_task.list_configs();
                            for config in mcp_configs {
                                // Skip disabled MCP servers - emit disabled status
                                if !config.enabled {
                                    health_cache_for_task.update_mcp_server(
                                        &config.id,
                                        ItemHealth::disabled(config.name),
                                    );
                                    continue;
                                }

                                let mcp_server_health = mcp_server_manager_for_task
                                    .get_server_health(&config.id)
                                    .await;
                                use mcp::manager::HealthStatus as McpHealthStatus;
                                let server_id = mcp_server_health.server_id.clone();
                                let server_name = mcp_server_health.server_name.clone();
                                let item_health = match mcp_server_health.status {
                                    McpHealthStatus::Ready => ItemHealth::ready(server_name),
                                    McpHealthStatus::Healthy => ItemHealth::healthy(
                                        server_name,
                                        mcp_server_health.latency_ms,
                                    ),
                                    McpHealthStatus::Unhealthy | McpHealthStatus::Unknown => {
                                        ItemHealth::unhealthy(
                                            server_name,
                                            mcp_server_health
                                                .error
                                                .unwrap_or_else(|| "Unhealthy".to_string()),
                                        )
                                    }
                                };
                                health_cache_for_task.update_mcp_server(&server_id, item_health);
                            }

                            health_cache_for_task.mark_refresh();
                            debug!("Periodic health checks completed");
                        }
                    });
                    info!(
                        "Started periodic health check task (interval: {}s)",
                        interval_secs
                    );
                } else {
                    info!("Health check mode is on-failure, skipping periodic task");
                }
            }

            // Refresh model cache for tray menu
            info!("Refreshing model cache...");
            let provider_registry_clone = provider_registry.clone();
            let app_handle_clone = app.handle().clone();
            tokio::spawn(async move {
                if let Err(e) = provider_registry_clone.refresh_model_cache().await {
                    error!("Failed to refresh model cache: {}", e);
                } else {
                    info!("Model cache refreshed successfully");
                    // Rebuild tray menu with models
                    if let Err(e) = ui::tray::rebuild_tray_menu(&app_handle_clone) {
                        error!("Failed to rebuild tray menu after model refresh: {}", e);
                    }
                }
            });

            // Initialize update notification state
            let update_notification_state = Arc::new(ui::tray::UpdateNotificationState::new());
            app.manage(update_notification_state.clone());

            // Setup system tray
            ui::tray::setup_tray(app)?;

            // Configure window for test mode
            if utils::test_mode::is_test_mode() {
                info!("Running in TEST MODE - configuring window for testing");
                if let Some(window) = app.get_webview_window("main") {
                    // Add [TEST] to window title
                    let _ = window.set_title("LocalRouter [TEST]");
                    // Make window smaller (800x500)
                    let _ = window.set_size(tauri::LogicalSize::new(800.0, 500.0));
                    // Position in bottom-right corner
                    if let Ok(Some(monitor)) = window.current_monitor() {
                        let screen_size = monitor.size();
                        let scale = monitor.scale_factor();
                        // Position 50px from right and bottom edges
                        let x = (screen_size.width as f64 / scale) - 800.0 - 50.0;
                        let y = (screen_size.height as f64 / scale) - 500.0 - 50.0;
                        let _ = window.set_position(tauri::LogicalPosition::new(x, y));
                    }
                    // Minimize the window so it doesn't take focus
                    // User can restore it from taskbar/dock if needed
                    let _ = window.minimize();
                    info!("Test mode window configured: 800x500, bottom-right corner, minimized");
                }
            }

            // Initialize tray graph manager
            info!("Initializing tray graph manager...");
            let ui_config = config_manager.get().ui.clone();
            let tray_graph_manager = Arc::new(ui::tray::TrayGraphManager::new(
                app.handle().clone(),
                ui_config,
            ));
            app.manage(tray_graph_manager.clone());
            info!("Tray graph manager initialized");

            // Set tray graph manager on AppState for request handlers
            if let Some(app_state) = server_manager.get_state() {
                app_state.set_tray_graph_manager(tray_graph_manager.clone());
                info!("Tray graph manager set on AppState");
            }

            // Set up metrics callback to notify graph manager after metrics are recorded
            let tray_graph_manager_for_metrics = tray_graph_manager.clone();
            metrics_collector.set_on_metrics_recorded(move || {
                tray_graph_manager_for_metrics.notify_activity();
            });
            info!("Metrics callback registered with tray graph manager");

            // Listen for server status changes to update tray icon
            let app_handle = app.handle().clone();
            app.listen("server-status-changed", move |event| {
                let status = event.payload();
                info!("Server status changed to: {}", status);
                if let Err(e) = ui::tray::update_tray_icon(&app_handle, status) {
                    error!("Failed to update tray icon: {}", e);
                }
            });

            // Listen for LLM request events to show "active" icon (when graph is disabled)
            let app_handle2 = app.handle().clone();
            let tray_graph_manager_clone = tray_graph_manager.clone();
            app.listen("llm-request", move |_event| {
                // Show "active" icon immediately (only if graph is disabled)
                // When graph is enabled, it will update via metrics callback
                if !tray_graph_manager_clone.is_enabled() {
                    if let Err(e) = ui::tray::update_tray_icon(&app_handle2, "active") {
                        error!("Failed to update tray icon for LLM request: {}", e);
                    }

                    // Restore to "running" after 2 seconds
                    let app_handle_restore = app_handle2.clone();
                    tokio::spawn(async move {
                        tokio::time::sleep(tokio::time::Duration::from_secs(2)).await;
                        let _ = ui::tray::update_tray_icon(&app_handle_restore, "running");
                    });
                }
            });

            // Start background update checker
            info!("Starting background update checker...");
            let app_handle_for_updater = app.handle().clone();
            let config_manager_for_updater = Arc::new(config_manager.clone());
            tokio::spawn(async move {
                updater::start_update_timer(app_handle_for_updater, config_manager_for_updater)
                    .await;
            });
            info!("Background update checker started");

            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            ui::commands::get_config,
            ui::commands::reload_config,
            ui::commands::set_provider_api_key,
            ui::commands::has_provider_api_key,
            ui::commands::delete_provider_api_key,
            ui::commands::list_providers_with_key_status,
            // Provider registry commands
            ui::commands::list_provider_types,
            ui::commands::list_provider_instances,
            ui::commands::create_provider_instance,
            ui::commands::get_provider_config,
            ui::commands::update_provider_instance,
            ui::commands::rename_provider_instance,
            ui::commands::get_provider_api_key,
            ui::commands::remove_provider_instance,
            ui::commands::set_provider_enabled,
            ui::commands::get_providers_health,
            ui::commands::start_provider_health_checks,
            ui::commands::check_single_provider_health,
            // Centralized health cache commands
            ui::commands::get_health_cache,
            ui::commands::refresh_all_health,
            ui::commands::list_provider_models,
            ui::commands::list_all_models,
            ui::commands::list_all_models_detailed,
            ui::commands::get_catalog_stats,
            ui::commands::get_catalog_metadata,
            // Server configuration commands
            ui::commands::get_server_config,
            ui::commands::update_server_config,
            ui::commands::restart_server,
            // Monitoring & statistics commands
            ui::commands::get_aggregate_stats,
            // Metrics commands
            ui::commands_metrics::get_global_metrics,
            ui::commands_metrics::get_api_key_metrics,
            ui::commands_metrics::get_provider_metrics,
            ui::commands_metrics::get_model_metrics,
            ui::commands_metrics::list_tracked_models,
            ui::commands_metrics::list_tracked_providers,
            ui::commands_metrics::list_tracked_api_keys,
            ui::commands_metrics::compare_api_keys,
            ui::commands_metrics::compare_providers,
            ui::commands_metrics::compare_models,
            ui::commands_metrics::get_strategy_metrics,
            ui::commands_metrics::list_tracked_strategies,
            ui::commands_metrics::compare_strategies,
            // MCP metrics commands
            ui::commands_mcp_metrics::get_global_mcp_metrics,
            ui::commands_mcp_metrics::get_client_mcp_metrics,
            ui::commands_mcp_metrics::get_mcp_server_metrics,
            ui::commands_mcp_metrics::get_mcp_method_breakdown,
            ui::commands_mcp_metrics::list_tracked_mcp_clients,
            ui::commands_mcp_metrics::list_tracked_mcp_servers,
            ui::commands_mcp_metrics::compare_mcp_clients,
            ui::commands_mcp_metrics::compare_mcp_servers,
            ui::commands_mcp_metrics::get_mcp_latency_percentiles,
            // Network interface commands
            ui::commands::get_network_interfaces,
            ui::commands::get_executable_path,
            // Server control commands
            ui::commands::get_server_status,
            ui::commands::stop_server,
            // OAuth commands
            ui::commands::list_oauth_providers,
            ui::commands::start_oauth_flow,
            ui::commands::poll_oauth_status,
            ui::commands::cancel_oauth_flow,
            ui::commands::list_oauth_credentials,
            ui::commands::delete_oauth_credentials,
            // OAuth client commands (for MCP)
            ui::commands::list_oauth_clients,
            ui::commands::create_oauth_client,
            ui::commands::get_oauth_client_secret,
            ui::commands::delete_oauth_client,
            ui::commands::update_oauth_client_name,
            ui::commands::toggle_oauth_client_enabled,
            ui::commands::link_mcp_server,
            ui::commands::unlink_mcp_server,
            ui::commands::get_oauth_client_linked_servers,
            // MCP server commands
            ui::commands::list_mcp_servers,
            ui::commands::create_mcp_server,
            ui::commands::delete_mcp_server,
            ui::commands::start_mcp_server,
            ui::commands::stop_mcp_server,
            ui::commands::get_mcp_server_health,
            ui::commands::get_all_mcp_server_health,
            ui::commands::start_mcp_health_checks,
            ui::commands::check_single_mcp_health,
            ui::commands::update_mcp_server_name,
            ui::commands::update_mcp_server_config,
            ui::commands::update_mcp_server,
            ui::commands::toggle_mcp_server_enabled,
            ui::commands::list_mcp_tools,
            ui::commands::call_mcp_tool,
            ui::commands::get_mcp_token_stats,
            // MCP OAuth browser flow commands
            ui::commands::start_mcp_oauth_browser_flow,
            ui::commands::poll_mcp_oauth_browser_status,
            ui::commands::cancel_mcp_oauth_browser_flow,
            ui::commands::discover_mcp_oauth_endpoints,
            ui::commands::test_mcp_oauth_connection,
            ui::commands::revoke_mcp_oauth_tokens,
            // Inline OAuth flow commands (for MCP server creation)
            ui::commands::start_inline_oauth_flow,
            ui::commands::poll_inline_oauth_status,
            ui::commands::cancel_inline_oauth_flow,
            // Unified client management commands
            ui::commands::list_clients,
            ui::commands::create_client,
            ui::commands::delete_client,
            ui::commands::update_client_name,
            ui::commands::toggle_client_enabled,
            ui::commands::rotate_client_secret,
            ui::commands::toggle_client_deferred_loading,
            ui::commands::get_client_value,
            // Strategy management commands
            ui::commands::list_strategies,
            ui::commands::get_strategy,
            ui::commands::create_strategy,
            ui::commands::update_strategy,
            ui::commands::delete_strategy,
            ui::commands::get_clients_using_strategy,
            ui::commands::assign_client_strategy,
            // Firewall approval commands
            ui::commands::submit_firewall_approval,
            ui::commands::list_pending_firewall_approvals,
            ui::commands::get_firewall_approval_details,
            // Unified permission commands
            ui::commands::set_client_mcp_permission,
            ui::commands::set_client_skills_permission,
            ui::commands::set_client_model_permission,
            ui::commands::set_client_marketplace_permission,
            ui::commands::clear_client_mcp_child_permissions,
            ui::commands::clear_client_skills_child_permissions,
            ui::commands::clear_client_model_child_permissions,
            ui::commands::get_mcp_server_capabilities,
            ui::commands::get_skill_tools,
            // OpenAPI documentation commands
            ui::commands::get_openapi_spec,
            // Internal testing commands
            ui::commands::get_internal_test_token,
            ui::commands::create_test_client_for_strategy,
            // Access logs commands
            ui::commands::get_llm_logs,
            ui::commands::get_mcp_logs,
            // Pricing override commands
            ui::commands::get_pricing_override,
            ui::commands::set_pricing_override,
            ui::commands::delete_pricing_override,
            // Tray graph settings commands
            ui::commands::get_tray_graph_settings,
            ui::commands::update_tray_graph_settings,
            // System commands
            ui::commands::get_home_dir,
            // Update checking commands
            ui::commands::get_app_version,
            ui::commands::get_update_config,
            ui::commands::update_update_config,
            ui::commands::mark_update_check_performed,
            ui::commands::skip_update_version,
            ui::commands::set_update_notification,
            // Logging configuration commands
            ui::commands::get_logging_config,
            ui::commands::update_logging_config,
            ui::commands::open_logs_folder,
            // Connection graph commands
            ui::commands::get_active_connections,
            // Setup wizard commands
            ui::commands::get_setup_wizard_shown,
            ui::commands::set_setup_wizard_shown,
            // RouteLLM intelligent routing commands
            ui::commands_routellm::routellm_get_status,
            ui::commands_routellm::routellm_test_prediction,
            ui::commands_routellm::routellm_unload,
            ui::commands_routellm::routellm_download_models,
            ui::commands_routellm::routellm_update_settings,
            ui::commands_routellm::open_routellm_folder,
            // Debug commands (dev only)
            ui::commands::debug_trigger_firewall_popup,
            // File system commands
            ui::commands::open_path,
            // Skills commands
            ui::commands::list_skills,
            ui::commands::get_skill,
            ui::commands::get_skills_config,
            ui::commands::add_skill_source,
            ui::commands::remove_skill_source,
            ui::commands::set_skill_enabled,
            ui::commands::rescan_skills,
            ui::commands::get_skill_files,
            // Marketplace commands
            ui::commands_marketplace::marketplace_get_config,
            ui::commands_marketplace::marketplace_set_enabled,
            ui::commands_marketplace::marketplace_set_registry_url,
            ui::commands_marketplace::marketplace_list_skill_sources,
            ui::commands_marketplace::marketplace_add_skill_source,
            ui::commands_marketplace::marketplace_remove_skill_source,
            ui::commands_marketplace::marketplace_add_default_skill_sources,
            ui::commands_marketplace::marketplace_reset_registry_url,
            ui::commands_marketplace::marketplace_get_cache_status,
            ui::commands_marketplace::marketplace_refresh_cache,
            ui::commands_marketplace::marketplace_clear_mcp_cache,
            ui::commands_marketplace::marketplace_clear_skills_cache,
            ui::commands_marketplace::marketplace_search_mcp_servers,
            ui::commands_marketplace::marketplace_search_skills,
            ui::commands_marketplace::marketplace_install_mcp_server_direct,
            ui::commands_marketplace::marketplace_install_skill_direct,
            ui::commands_marketplace::marketplace_delete_skill,
            ui::commands_marketplace::marketplace_is_skill_from_marketplace,
            ui::commands_marketplace::marketplace_get_pending_install,
            ui::commands_marketplace::marketplace_list_pending_installs,
            ui::commands_marketplace::marketplace_install_respond,
            ui::commands_marketplace::set_client_marketplace_enabled,
            ui::commands_marketplace::get_client_marketplace_enabled,
        ])
        .on_window_event(|window, event| {
            if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                // Prevent the window from closing
                api.prevent_close();

                // Hide the window instead
                if let Err(e) = window.hide() {
                    tracing::error!("Failed to hide window: {}", e);
                }

                tracing::info!("Window close intercepted - app minimized to system tray");
            }
        })
        .run(tauri::generate_context!())
        .expect("error while running tauri application");

    Ok(())
}
