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

    tauri::Builder::default()
        .plugin(tauri_plugin_shell::init())
        .setup(|_app| {
            info!("Tauri app initialized");

            // TODO: Setup system tray
            // ui::tray::setup_tray(app)?;

            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            // TODO: Add Tauri commands here
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");

    Ok(())
}
