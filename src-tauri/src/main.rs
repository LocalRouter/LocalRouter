// Prevents additional console window on Windows in release
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod api_keys;
mod catalog;
mod clients;
mod config;
mod mcp;
mod monitoring;
mod oauth_clients;
mod providers;
mod routellm;
mod router;
mod server;
mod ui;
mod updater;
mod utils;

use std::sync::Arc;

use tauri::{Listener, Manager};
use tracing::{error, info};
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};

use providers::factory::{
    AnthropicProviderFactory, CerebrasProviderFactory, CohereProviderFactory,
    DeepInfraProviderFactory, GeminiProviderFactory, GroqProviderFactory, LMStudioProviderFactory,
    MistralProviderFactory, OllamaProviderFactory, OpenAICompatibleProviderFactory,
    OpenAIProviderFactory, OpenRouterProviderFactory, PerplexityProviderFactory,
    TogetherAIProviderFactory, XAIProviderFactory,
};
use providers::health::HealthCheckManager;
use providers::registry::ProviderRegistry;
use server::ServerManager;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // Initialize logging
    tracing_subscriber::registry()
        .with(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "localrouter_ai=info".into()),
        )
        .with(tracing_subscriber::fmt::layer())
        .init();

    info!("Starting LocalRouter AI...");

    // Log configuration directory
    let config_dir =
        config::paths::config_dir().unwrap_or_else(|_| std::path::PathBuf::from("unknown"));
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
            config::paths::config_file().unwrap(),
        )
    });

    // Ensure default strategy exists and all clients have strategy_id assigned
    info!("Ensuring default strategy exists...");
    config_manager
        .ensure_default_strategy()
        .await
        .unwrap_or_else(|e| {
            tracing::error!("Failed to ensure default strategy: {}", e);
        });

    // Initialize unified client manager
    // Replaces both API key manager and OAuth client manager
    // Client secrets are stored in OS keychain, only metadata in config
    let client_manager = {
        let config = config_manager.get();
        Arc::new(clients::ClientManager::new(config.clients.clone()))
    };

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

    // Initialize MCP server manager
    let mcp_server_manager = {
        let config = config_manager.get();
        let manager = Arc::new(mcp::McpServerManager::new());
        manager.load_configs(config.mcp_servers.clone());
        manager
    };

    // Initialize provider registry
    info!("Initializing provider registry...");
    let health_manager = Arc::new(HealthCheckManager::default());
    let provider_registry = Arc::new(ProviderRegistry::new(health_manager.clone()));

    // Start background health check task
    info!("Starting background health checks...");
    let _health_task = health_manager.clone().start_background_task();

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
    info!("Registered 15 provider factories");

    // Load provider instances from configuration
    info!("Loading provider instances from configuration...");
    let providers = config_manager.get().providers;
    for provider_config in providers {
        let provider_type = match provider_config.provider_type {
            config::ProviderType::Ollama => "ollama",
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
    let oauth_storage_path = config::paths::config_dir()
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
        providers::oauth::openai_codex::OpenAICodexOAuthProvider::new(),
    ));
    oauth_manager.register_provider(Arc::new(
        providers::oauth::anthropic_claude::AnthropicClaudeOAuthProvider::new(),
    ));
    info!("Registered 3 OAuth providers");

    // Initialize rate limiter
    info!("Initializing rate limiter...");
    let rate_limiter = Arc::new(router::RateLimiterManager::new(None));

    // Initialize metrics collector
    info!("Initializing metrics collector...");
    let metrics_db_path = config::paths::config_dir()
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
                let _ = service_arc.clone().start_auto_unload_task();
                info!("RouteLLM service initialized with idle timeout: {}s", idle_timeout);
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
            crate::server::manager::ServerDependencies {
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
        .plugin(tauri_plugin_shell::init())
        .plugin(tauri_plugin_updater::Builder::new().build())
        .setup(move |app| {
            info!("Tauri app initialized");

            // Set app handle on config manager for event emission
            config_manager.set_app_handle(app.handle().clone());

            // Start watching config file for changes
            let watcher = config_manager.start_watching().map_err(|e| {
                tracing::error!("Failed to start config file watcher: {}", e);
                e
            })?;

            // Store watcher to keep it alive (if it's dropped, watching stops)
            app.manage(watcher);

            // Store managers
            app.manage(config_manager.clone());
            app.manage(client_manager.clone());
            app.manage(token_store.clone());
            app.manage(oauth_client_manager.clone());
            app.manage(mcp_server_manager.clone());
            app.manage(provider_registry.clone());
            app.manage(health_manager.clone());
            app.manage(server_manager.clone());
            app.manage(app_router.clone());
            app.manage(rate_limiter.clone());
            app.manage(oauth_manager.clone());
            app.manage(metrics_collector.clone());

            // Get AppState from server manager and manage it for Tauri commands
            if let Some(app_state) = server_manager.get_state() {
                info!("Managing AppState for Tauri commands");

                // Set app handle on AppState for event emission
                app_state.set_app_handle(app.handle().clone());

                app.manage(Arc::new(app_state));
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

                tokio::spawn(async move {
                    use tauri::Emitter;

                    match server_manager_clone2
                        .start(
                            server_config,
                            crate::server::manager::ServerDependencies {
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
                            let _ = app_handle.emit("server-restart-completed", ());
                        }
                        Err(e) => {
                            error!("Failed to restart server: {}", e);
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
                updater::start_update_timer(app_handle_for_updater, config_manager_for_updater).await;
            });
            info!("Background update checker started");

            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            ui::commands::list_routers,
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
            ui::commands::remove_provider_instance,
            ui::commands::set_provider_enabled,
            ui::commands::get_providers_health,
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
            ui::commands::update_mcp_server_name,
            ui::commands::update_mcp_server_config,
            ui::commands::toggle_mcp_server_enabled,
            ui::commands::list_mcp_tools,
            ui::commands::call_mcp_tool,
            ui::commands::get_mcp_token_stats,
            // Unified client management commands
            ui::commands::list_clients,
            ui::commands::create_client,
            ui::commands::delete_client,
            ui::commands::update_client_name,
            ui::commands::toggle_client_enabled,
            ui::commands::toggle_client_deferred_loading,
            ui::commands::add_client_llm_provider,
            ui::commands::remove_client_llm_provider,
            ui::commands::add_client_mcp_server,
            ui::commands::remove_client_mcp_server,
            // Client routing configuration commands
            ui::commands::set_client_routing_strategy,
            ui::commands::set_client_forced_model,
            ui::commands::update_client_available_models,
            ui::commands::update_client_prioritized_models,
            ui::commands::get_client_value,
            // Strategy management commands
            ui::commands::list_strategies,
            ui::commands::get_strategy,
            ui::commands::create_strategy,
            ui::commands::update_strategy,
            ui::commands::delete_strategy,
            ui::commands::get_clients_using_strategy,
            ui::commands::assign_client_strategy,
            // OpenAPI documentation commands
            ui::commands::get_openapi_spec,
            // Internal testing commands
            ui::commands::get_internal_test_token,
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
            // RouteLLM intelligent routing commands
            ui::commands_routellm::routellm_get_status,
            ui::commands_routellm::routellm_test_prediction,
            ui::commands_routellm::routellm_unload,
            ui::commands_routellm::routellm_download_models,
            ui::commands_routellm::routellm_update_settings,
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
