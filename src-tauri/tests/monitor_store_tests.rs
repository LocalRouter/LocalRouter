//! Monitor store lifetime and per-request event noise.

use std::sync::Arc;

use localrouter::clients::{ClientManager, TokenStore};
use localrouter::config::{AppConfig, ConfigManager};
use localrouter::monitoring::metrics::MetricsCollector;
use localrouter::monitoring::storage::MetricsDatabase;
use localrouter::providers::registry::ProviderRegistry;
use localrouter::router::{RateLimiterManager, Router};
use localrouter::server::manager::ServerDependencies;
use localrouter::server::routes::pipeline::{scan_request_for_secrets, SecretScanOutcome};
use localrouter::server::state::AppState;
use localrouter::server::{ServerConfig, ServerManager};
use lr_mcp::McpServerManager;
use lr_monitor::{EventStatus, MonitorEventData, MonitorEventType};

fn dependencies() -> ServerDependencies {
    let config_path =
        std::env::temp_dir().join(format!("test_monitor_{}.yaml", uuid::Uuid::new_v4()));
    let config_manager = Arc::new(ConfigManager::new(AppConfig::default(), config_path));
    let provider_registry = Arc::new(ProviderRegistry::new());
    let rate_limiter = Arc::new(RateLimiterManager::new(None));
    let metrics_db_path =
        std::env::temp_dir().join(format!("test_monitor_{}.db", uuid::Uuid::new_v4()));
    let metrics_collector = Arc::new(MetricsCollector::new(Arc::new(
        MetricsDatabase::new(metrics_db_path).unwrap(),
    )));
    let router = Arc::new(Router::new(
        config_manager.clone(),
        provider_registry.clone(),
        rate_limiter.clone(),
        metrics_collector.clone(),
        Arc::new(lr_router::FreeTierManager::new(None)),
    ));
    ServerDependencies {
        router,
        mcp_server_manager: Arc::new(McpServerManager::new()),
        rate_limiter,
        provider_registry,
        config_manager,
        client_manager: Arc::new(ClientManager::new(vec![])),
        token_store: Arc::new(TokenStore::new()),
        metrics_collector,
        health_cache: None,
    }
}

fn server_config() -> ServerConfig {
    ServerConfig {
        host: "127.0.0.1".to_string(),
        port: 0,
        enable_cors: false,
    }
}

fn push_marker(state: &AppState) -> String {
    state.monitor_store.push(
        MonitorEventType::PromptCompression,
        None,
        None,
        None,
        MonitorEventData::PromptCompression {
            original_tokens: 10,
            compressed_tokens: 5,
            reduction_percent: 50.0,
            duration_ms: 1,
            method: "test".to_string(),
        },
        EventStatus::Complete,
        Some(1),
    )
}

/// Events outlive a server restart and stay readable while it is stopped:
/// the proxies keep the store they were given at launch, so a restart must
/// not swap in a fresh one.
#[tokio::test]
async fn monitor_store_survives_server_restart() {
    let manager = ServerManager::new();
    manager
        .start(server_config(), dependencies())
        .await
        .expect("server starts");
    let first = manager.get_state().expect("running");
    assert!(Arc::ptr_eq(&first.monitor_store, &manager.monitor_store()));
    let id = push_marker(&first);

    manager.stop().await;
    assert!(manager.get_state().is_none());
    let listed = manager.monitor_store().list(0, 10, None);
    assert_eq!(listed.events.len(), 1);
    assert_eq!(listed.events[0].id, id);

    manager
        .start(server_config(), dependencies())
        .await
        .expect("server restarts");
    let second = manager.get_state().expect("running");
    assert!(Arc::ptr_eq(&second.monitor_store, &first.monitor_store));
    assert!(second.monitor_store.get(&id).is_some());
    manager.stop().await;
}

async fn secret_scan_state() -> AppState {
    let deps = dependencies();
    let manager = ServerManager::new();
    manager
        .start(server_config(), deps)
        .await
        .expect("server starts");
    let state = manager.get_state().expect("running");
    let engine =
        lr_secret_scanner::SecretScanEngine::new(&lr_secret_scanner::SecretScanEngineConfig {
            entropy_threshold: 3.0,
            allowlist: vec![],
            scan_system_messages: false,
        })
        .expect("engine builds");
    *state.secret_scanner.write() = Some(Arc::new(engine));
    state
}

fn chat_body(content: &str) -> serde_json::Value {
    serde_json::json!({
        "model": "gpt-5.5",
        "messages": [{"role": "user", "content": content}]
    })
}

/// A clean secret scan runs on every request, so it records nothing.
#[tokio::test]
async fn clean_secret_scan_records_no_event() {
    let state = secret_scan_state().await;
    let outcome =
        scan_request_for_secrets(&state, "client", "gpt-5.5", &chat_body("hello there")).await;
    assert!(matches!(outcome, SecretScanOutcome::Allow));
    assert_eq!(state.monitor_store.list(0, 10, None).total, 0);
}

/// A scan with findings records one finished event carrying them.
#[tokio::test]
async fn secret_scan_with_findings_records_one_event() {
    let state = secret_scan_state().await;
    scan_request_for_secrets(
        &state,
        "client",
        "gpt-5.5",
        &chat_body("aws key AKIAIOSFODNN7EXAMPLE"),
    )
    .await;
    let listed = state.monitor_store.list(0, 10, None);
    assert_eq!(listed.total, 1);
    let event = state
        .monitor_store
        .get(&listed.events[0].id)
        .expect("event stored");
    assert_eq!(event.event_type, MonitorEventType::SecretScan);
    assert_eq!(event.status, EventStatus::Complete);
    match event.data {
        MonitorEventData::SecretScan {
            findings_count,
            action_taken,
            ..
        } => {
            assert!(findings_count.unwrap_or(0) >= 1);
            assert_eq!(action_taken.as_deref(), Some("notify"));
        }
        other => panic!("unexpected data {other:?}"),
    }
}
