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

use tracing::info;
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};

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
    let config_manager = config::ConfigManager::load().await.unwrap_or_else(|e| {
        tracing::warn!("Failed to load config, using defaults: {}", e);
        config::ConfigManager::new(config::AppConfig::default(), config::paths::config_file().unwrap())
    });

    let api_key_manager = api_keys::ApiKeyManager::load().await.unwrap_or_else(|e| {
        tracing::warn!("Failed to load API keys, starting empty: {}", e);
        api_keys::ApiKeyManager::new(vec![])
    });

    tauri::Builder::default()
        .plugin(tauri_plugin_shell::init())
        .manage(config_manager)
        .manage(api_key_manager)
        .setup(|app| {
            info!("Tauri app initialized");

            // Setup system tray
            ui::tray::setup_tray(app)?;

            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            ui::commands::list_api_keys,
            ui::commands::create_api_key,
            ui::commands::list_routers,
            ui::commands::get_config,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");

    Ok(())
}
