//! The server listens only on its configured port: when that port is taken
//! it fails with a clear error instead of moving to another port, and a
//! later start on a free port succeeds and clears the error.

use std::sync::Arc;

use localrouter::clients::{ClientManager, TokenStore};
use localrouter::config::{AppConfig, ConfigManager};
use localrouter::mcp::McpServerManager;
use localrouter::monitoring::metrics::MetricsCollector;
use localrouter::monitoring::storage::MetricsDatabase;
use localrouter::providers::registry::ProviderRegistry;
use localrouter::router::{RateLimiterManager, Router};
use lr_server::manager::ServerDependencies;
use lr_server::{ServerBindError, ServerConfig, ServerManager, ServerStatus};

fn deps() -> ServerDependencies {
    let config_path =
        std::env::temp_dir().join(format!("test_port_in_use_{}.yaml", uuid::Uuid::new_v4()));
    let config_manager = Arc::new(ConfigManager::new(AppConfig::default(), config_path));
    let provider_registry = Arc::new(ProviderRegistry::new());
    let metrics_db_path =
        std::env::temp_dir().join(format!("test_port_in_use_{}.db", uuid::Uuid::new_v4()));
    let metrics_collector = Arc::new(MetricsCollector::new(Arc::new(
        MetricsDatabase::new(metrics_db_path).unwrap(),
    )));
    let rate_limiter = Arc::new(RateLimiterManager::new(None));
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

fn config(port: u16) -> ServerConfig {
    ServerConfig {
        host: "127.0.0.1".to_string(),
        port,
        enable_cors: false,
    }
}

fn free_port() -> u16 {
    std::net::TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port()
}

#[tokio::test]
async fn taken_port_fails_then_a_free_port_starts() {
    let occupant = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let taken = occupant.local_addr().unwrap().port();
    let manager = ServerManager::new();

    let err = manager
        .start(config(taken), deps())
        .await
        .expect_err("a taken port must not be replaced by another one");
    let bind = err
        .downcast_ref::<ServerBindError>()
        .expect("bind failure is a ServerBindError");
    assert_eq!(bind.port, taken);
    assert!(bind.port_in_use, "{bind:?}");
    assert!(bind.message.contains(&format!("Port {taken}")), "{bind}");
    assert_eq!(manager.get_status(), ServerStatus::Stopped);
    assert_eq!(manager.get_actual_port(), None);
    assert!(manager.get_state().is_none());
    assert!(
        manager.get_built_state().is_some(),
        "state stays available to wire into while the port is taken"
    );
    assert_eq!(manager.get_start_error(), Some(bind.message.clone()));

    // Choosing another port starts the same state there and clears the error.
    let port = free_port();
    manager
        .start(config(port), deps())
        .await
        .expect("free port starts");
    assert_eq!(manager.get_status(), ServerStatus::Running);
    assert_eq!(manager.get_actual_port(), Some(port));
    assert_eq!(manager.get_start_error(), None);
    let health = reqwest::get(format!("http://127.0.0.1:{port}/health"))
        .await
        .expect("server answers on the chosen port");
    assert!(health.status().is_success());

    manager.stop().await;
    drop(occupant);
}
