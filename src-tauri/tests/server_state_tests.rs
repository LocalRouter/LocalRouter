//! Server state across restarts, and per-request monitor event noise.

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

async fn health(manager: &ServerManager) -> reqwest::StatusCode {
    let port = manager.get_actual_port().expect("listening");
    reqwest::get(format!("http://127.0.0.1:{port}/health"))
        .await
        .expect("server answers")
        .status()
}

/// A restart serves the state built by the first start, so everything wired
/// into it at launch (and held by the proxies and UI commands) stays live, and
/// monitor events stay readable while the server is stopped.
#[tokio::test]
async fn restart_serves_the_same_state() {
    let manager = ServerManager::new();
    assert!(manager.monitor_store().is_none());
    manager
        .start(server_config(), dependencies())
        .await
        .expect("server starts");
    assert!(health(&manager).await.is_success());
    let first = manager.get_state().expect("running");
    let id = push_marker(&first);
    let engine =
        lr_secret_scanner::SecretScanEngine::new(&lr_secret_scanner::SecretScanEngineConfig {
            entropy_threshold: 3.0,
            allowlist: vec![],
            scan_system_messages: false,
        })
        .expect("engine builds");
    *first.secret_scanner.write() = Some(Arc::new(engine));

    manager.stop().await;
    assert!(manager.get_state().is_none());
    let store = manager.monitor_store().expect("built");
    assert!(Arc::ptr_eq(&store, &first.monitor_store));
    assert_eq!(store.list(0, 10, None).events[0].id, id);

    manager
        .start(server_config(), dependencies())
        .await
        .expect("server restarts");
    assert!(health(&manager).await.is_success());
    let second = manager.get_state().expect("running");
    assert!(Arc::ptr_eq(&second.monitor_store, &first.monitor_store));
    assert!(Arc::ptr_eq(&second.mcp_gateway, &first.mcp_gateway));
    assert!(second.secret_scanner.read().is_some());
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
