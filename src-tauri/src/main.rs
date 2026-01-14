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

use tauri::Manager;
use tracing::info;
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};

use providers::factory::{
    AnthropicProviderFactory, GeminiProviderFactory, OllamaProviderFactory,
    OpenAICompatibleProviderFactory, OpenAIProviderFactory, OpenRouterProviderFactory,
};
use providers::health::HealthCheckManager;
use providers::registry::ProviderRegistry;

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

    // TODO: Load configuration
    // TODO: Initialize web server
    // TODO: Initialize provider manager
    // TODO: Initialize router
    // TODO: Initialize monitoring

    // Initialize managers
    let mut config_manager = config::ConfigManager::load().await.unwrap_or_else(|e| {
        tracing::warn!("Failed to load config, using defaults: {}", e);
        config::ConfigManager::new(config::AppConfig::default(), config::paths::config_file().unwrap())
    });

    let api_key_manager = api_keys::ApiKeyManager::load().await.unwrap_or_else(|e| {
        tracing::warn!("Failed to load API keys, starting empty: {}", e);
        api_keys::ApiKeyManager::new(vec![])
    });

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
            app.manage(config_manager);
            app.manage(api_key_manager);
            app.manage(provider_registry.clone());
            app.manage(health_manager.clone());

            // Setup system tray
            ui::tray::setup_tray(app)?;

            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            ui::commands::list_api_keys,
            ui::commands::create_api_key,
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
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");

    Ok(())
}
