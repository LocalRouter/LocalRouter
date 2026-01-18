// Prevents additional console window on Windows in release
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod api_keys;
mod clients;
mod config;
mod mcp;
mod monitoring;
mod oauth_clients;
mod providers;
mod router;
mod server;
mod ui;
mod utils;

use std::sync::Arc;

use tauri::{Listener, Manager};
use tracing::{error, info};
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};

use providers::factory::{
    AnthropicProviderFactory, CerebrasProviderFactory, CohereProviderFactory,
    DeepInfraProviderFactory, GeminiProviderFactory, GroqProviderFactory,
    LMStudioProviderFactory, MistralProviderFactory, OllamaProviderFactory,
    OpenAICompatibleProviderFactory, OpenAIProviderFactory, OpenRouterProviderFactory,
    PerplexityProviderFactory, TogetherAIProviderFactory, XAIProviderFactory,
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
    let config_dir = config::paths::config_dir().unwrap_or_else(|_| std::path::PathBuf::from("unknown"));
    #[cfg(debug_assertions)]
    info!("Running in DEVELOPMENT mode");
    #[cfg(not(debug_assertions))]
    info!("Running in PRODUCTION mode");
    info!("Configuration directory: {}", config_dir.display());

    // Initialize managers
    let mut config_manager = config::ConfigManager::load().await.unwrap_or_else(|e| {
        tracing::warn!("Failed to load config, using defaults: {}", e);
        config::ConfigManager::new(config::AppConfig::default(), config::paths::config_file().unwrap())
    });

    // Initialize API key manager with keys from config
    // Actual API keys are stored in OS keychain, only metadata in config
    let api_key_manager = {
        let config = config_manager.get();
        api_keys::ApiKeyManager::new(config.api_keys.clone())
    };

    // Initialize OAuth client manager for MCP
    // Actual client secrets are stored in OS keychain, only metadata in config
    let oauth_client_manager = {
        let config = config_manager.get();
        oauth_clients::OAuthClientManager::new(config.oauth_clients.clone())
    };

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
            tracing::warn!(
                "Failed to load provider '{}': {}",
                provider_config.name,
                e
            );
            continue;
        }

        // Set enabled state
        if let Err(e) = provider_registry.set_provider_enabled(
            &provider_config.name,
            provider_config.enabled,
        ) {
            tracing::warn!(
                "Failed to set provider '{}' enabled state: {}",
                provider_config.name,
                e
            );
        }
    }
    info!("Loaded {} provider instances", config_manager.get().providers.len());

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
    oauth_manager.register_provider(Arc::new(providers::oauth::github_copilot::GitHubCopilotOAuthProvider::new()));
    oauth_manager.register_provider(Arc::new(providers::oauth::openai_codex::OpenAICodexOAuthProvider::new()));
    oauth_manager.register_provider(Arc::new(providers::oauth::anthropic_claude::AnthropicClaudeOAuthProvider::new()));
    info!("Registered 3 OAuth providers");

    // Initialize rate limiter
    info!("Initializing rate limiter...");
    let rate_limiter = Arc::new(router::RateLimiterManager::new(None));

    // Initialize router
    info!("Initializing router...");
    let config_manager_arc = Arc::new(config_manager.clone());
    let app_router = Arc::new(router::Router::new(
        config_manager_arc.clone(),
        provider_registry.clone(),
        rate_limiter.clone(),
    ));

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

    // Start the server
    server_manager
        .start(
            server_config,
            crate::server::manager::ServerDependencies {
                router: app_router.clone(),
                api_key_manager: api_key_manager.clone(),
                oauth_client_manager: oauth_client_manager.clone(),
                mcp_server_manager: mcp_server_manager.clone(),
                rate_limiter: rate_limiter.clone(),
                provider_registry: provider_registry.clone(),
                client_manager: client_manager.clone(),
                token_store: token_store.clone(),
            },
        )
        .await?;

    tauri::Builder::default()
        .plugin(tauri_plugin_shell::init())
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
            app.manage(api_key_manager.clone());
            app.manage(oauth_client_manager.clone());
            app.manage(client_manager.clone());
            app.manage(token_store.clone());
            app.manage(mcp_server_manager.clone());
            app.manage(provider_registry.clone());
            app.manage(health_manager.clone());
            app.manage(server_manager.clone());
            app.manage(app_router.clone());
            app.manage(rate_limiter.clone());
            app.manage(oauth_manager.clone());

            // Set up server restart event listener
            let server_manager_clone = server_manager.clone();
            let app_router_clone = app_router.clone();
            let api_key_manager_clone = api_key_manager.clone();
            let oauth_client_manager_clone = oauth_client_manager.clone();
            let mcp_server_manager_clone = mcp_server_manager.clone();
            let rate_limiter_clone = rate_limiter.clone();
            let provider_registry_clone = provider_registry.clone();
            let client_manager_clone = client_manager.clone();
            let token_store_clone = token_store.clone();
            let config_manager_clone = config_manager.clone();

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
                let api_key_manager_clone2 = api_key_manager_clone.clone();
                let oauth_client_manager_clone2 = oauth_client_manager_clone.clone();
                let mcp_server_manager_clone2 = mcp_server_manager_clone.clone();
                let rate_limiter_clone2 = rate_limiter_clone.clone();
                let provider_registry_clone2 = provider_registry_clone.clone();
                let client_manager_clone2 = client_manager_clone.clone();
                let token_store_clone2 = token_store_clone.clone();

                tokio::spawn(async move {
                    match server_manager_clone2
                        .start(
                            server_config,
                            crate::server::manager::ServerDependencies {
                                router: app_router_clone2,
                                api_key_manager: api_key_manager_clone2,
                                oauth_client_manager: oauth_client_manager_clone2,
                                mcp_server_manager: mcp_server_manager_clone2,
                                rate_limiter: rate_limiter_clone2,
                                provider_registry: provider_registry_clone2,
                                client_manager: client_manager_clone2,
                                token_store: token_store_clone2,
                            },
                        )
                        .await
                    {
                        Ok(_) => info!("Server restarted successfully"),
                        Err(e) => error!("Failed to restart server: {}", e),
                    }
                });
            });

            // Set app handle on server state for event emission
            if let Some(state) = server_manager.get_state() {
                state.set_app_handle(app.handle().clone());
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

            // Setup system tray
            ui::tray::setup_tray(app)?;

            // Listen for server status changes to update tray icon
            let app_handle = app.handle().clone();
            app.listen("server-status-changed", move |event| {
                let status = event.payload();
                info!("Server status changed to: {}", status);
                if let Err(e) = ui::tray::update_tray_icon(&app_handle, status) {
                    error!("Failed to update tray icon: {}", e);
                }
            });

            // Listen for LLM request events to blink tray icon
            let app_handle2 = app.handle().clone();
            app.listen("llm-request", move |_event| {
                if let Err(e) = ui::tray::update_tray_icon(&app_handle2, "active") {
                    error!("Failed to update tray icon for LLM request: {}", e);
                }
            });

            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            ui::commands::list_api_keys,
            ui::commands::create_api_key,
            ui::commands::get_api_key_value,
            ui::commands::delete_api_key,
            ui::commands::update_api_key_model,
            ui::commands::update_api_key_name,
            ui::commands::toggle_api_key_enabled,
            ui::commands::rotate_api_key,
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
            ui::commands_metrics::compare_api_keys,
            ui::commands_metrics::compare_providers,
            // Network interface commands
            ui::commands::get_network_interfaces,
            // Server control commands
            ui::commands::get_server_status,
            ui::commands::start_server,
            ui::commands::stop_server,
            // OAuth commands
            ui::commands::list_oauth_providers,
            ui::commands::start_oauth_flow,
            ui::commands::poll_oauth_status,
            ui::commands::cancel_oauth_flow,
            ui::commands::list_oauth_credentials,
            ui::commands::delete_oauth_credentials,
            // Routing strategy commands
            ui::commands::get_routing_config,
            ui::commands::update_prioritized_list,
            ui::commands::set_routing_strategy,
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
            ui::commands::toggle_mcp_server_enabled,
            ui::commands::list_mcp_tools,
            ui::commands::call_mcp_tool,
            // Unified client management commands
            ui::commands::list_clients,
            ui::commands::create_client,
            ui::commands::delete_client,
            ui::commands::update_client_name,
            ui::commands::toggle_client_enabled,
            ui::commands::add_client_llm_provider,
            ui::commands::remove_client_llm_provider,
            ui::commands::add_client_mcp_server,
            ui::commands::remove_client_mcp_server,
            // OpenAPI documentation commands
            ui::commands::get_openapi_spec,
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
