// Prevents additional console window on Windows in release
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod api_keys;
mod config;
mod monitoring;
mod providers;
mod router;
mod server;
mod ui;
mod utils;

use std::sync::Arc;

use parking_lot::RwLock;
use tauri::{Listener, Manager};
use tokio::task::JoinHandle;
use tracing::{error, info};
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};

use providers::factory::{
    AnthropicProviderFactory, GeminiProviderFactory, OllamaProviderFactory,
    OpenAICompatibleProviderFactory, OpenAIProviderFactory, OpenRouterProviderFactory,
};
use providers::health::HealthCheckManager;
use providers::registry::ProviderRegistry;

/// Manages the web server task
struct ServerManager {
    task_handle: Option<JoinHandle<()>>,
}

impl ServerManager {
    fn new() -> Self {
        Self { task_handle: None }
    }

    /// Start the web server in a background task
    fn start(
        &mut self,
        config: server::ServerConfig,
        router: Arc<router::Router>,
        api_key_manager: api_keys::ApiKeyManager,
        rate_limiter: Arc<router::RateLimiterManager>,
        provider_registry: Arc<ProviderRegistry>,
    ) {
        // Cancel previous task if running
        if let Some(handle) = self.task_handle.take() {
            handle.abort();
        }

        // Spawn new server task
        let handle = tokio::spawn(async move {
            if let Err(e) = server::start_server(
                config,
                router,
                api_key_manager,
                rate_limiter,
                provider_registry,
            )
            .await
            {
                error!("Server error: {}", e);
            }
        });

        self.task_handle = Some(handle);
    }

    /// Stop the web server
    fn stop(&mut self) {
        if let Some(handle) = self.task_handle.take() {
            handle.abort();
        }
    }
}

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

    // Initialize provider registry
    info!("Initializing provider registry...");
    let health_manager = Arc::new(HealthCheckManager::default());
    let provider_registry = Arc::new(ProviderRegistry::new(health_manager.clone()));

    // Register provider factories
    info!("Registering provider factories...");
    provider_registry.register_factory(Arc::new(OllamaProviderFactory));
    provider_registry.register_factory(Arc::new(OpenAIProviderFactory));
    provider_registry.register_factory(Arc::new(OpenAICompatibleProviderFactory));
    provider_registry.register_factory(Arc::new(AnthropicProviderFactory));
    provider_registry.register_factory(Arc::new(GeminiProviderFactory));
    provider_registry.register_factory(Arc::new(OpenRouterProviderFactory));
    info!("Registered 6 provider factories");

    // TODO: Load provider instances from configuration
    // provider_registry.load_from_config(...).await?;

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
    let server_manager = Arc::new(RwLock::new(ServerManager::new()));

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
    server_manager.write().start(
        server_config,
        app_router.clone(),
        api_key_manager.clone(),
        rate_limiter.clone(),
        provider_registry.clone(),
    );

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
            app.manage(provider_registry.clone());
            app.manage(health_manager.clone());
            app.manage(server_manager.clone());

            // Set up server restart event listener
            let server_manager_clone = server_manager.clone();
            let app_router_clone = app_router.clone();
            let api_key_manager_clone = api_key_manager.clone();
            let rate_limiter_clone = rate_limiter.clone();
            let provider_registry_clone = provider_registry.clone();
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

                // Restart the server
                server_manager_clone.write().start(
                    server_config,
                    app_router_clone.clone(),
                    api_key_manager_clone.clone(),
                    rate_limiter_clone.clone(),
                    provider_registry_clone.clone(),
                );

                info!("Server restarted successfully");
            });

            // Setup system tray
            ui::tray::setup_tray(app)?;

            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            ui::commands::list_api_keys,
            ui::commands::create_api_key,
            ui::commands::get_api_key_value,
            ui::commands::delete_api_key,
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
            ui::commands::remove_provider_instance,
            ui::commands::set_provider_enabled,
            ui::commands::get_providers_health,
            ui::commands::list_provider_models,
            ui::commands::list_all_models,
            // Server configuration commands
            ui::commands::get_server_config,
            ui::commands::update_server_config,
            ui::commands::restart_server,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");

    Ok(())
}
