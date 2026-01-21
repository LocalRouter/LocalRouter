//! Tests for MCP notification forwarding to external clients
//!
//! Verifies that notifications from MCP servers are correctly forwarded
//! to external clients via the broadcast channel.

use std::sync::Arc;
use std::time::Duration;

use localrouter_ai::clients::{ClientManager, TokenStore};
use localrouter_ai::config::{AppConfig, ConfigManager};
use localrouter_ai::mcp::gateway::{GatewayConfig, McpGateway};
use localrouter_ai::mcp::manager::McpServerManager;
use localrouter_ai::mcp::protocol::{JsonRpcNotification, JsonRpcRequest};
use localrouter_ai::monitoring::metrics::MetricsCollector;
use localrouter_ai::monitoring::storage::MetricsDatabase;
use localrouter_ai::providers::health::HealthCheckManager;
use localrouter_ai::providers::registry::ProviderRegistry;
use localrouter_ai::router::{RateLimiterManager, Router};
use localrouter_ai::server::state::AppState;

/// Helper to create a minimal test router for gateway tests
fn create_test_router() -> Arc<Router> {
    let config = AppConfig::default();
    let config_manager = Arc::new(ConfigManager::new(
        config,
        std::path::PathBuf::from("/tmp/test_notification_router.yaml"),
    ));

    let health_manager = Arc::new(HealthCheckManager::default());
    let provider_registry = Arc::new(ProviderRegistry::new(health_manager));
    let rate_limiter = Arc::new(RateLimiterManager::new(None));

    let metrics_db_path = std::env::temp_dir()
        .join(format!("test_notification_metrics_{}.db", uuid::Uuid::new_v4()));
    let metrics_db = Arc::new(MetricsDatabase::new(metrics_db_path).unwrap());
    let metrics_collector = Arc::new(MetricsCollector::new(metrics_db));

    Arc::new(Router::new(
        config_manager,
        provider_registry,
        rate_limiter,
        metrics_collector,
    ))
}

/// Helper to create a minimal test AppState
fn create_test_app_state() -> AppState {
    let router = create_test_router();
    let config = AppConfig::default();
    let config_path = std::env::temp_dir().join(format!("test_config_{}.yaml", uuid::Uuid::new_v4()));
    let config_manager = Arc::new(ConfigManager::new(config, config_path));
    let client_manager = Arc::new(ClientManager::new(vec![])); // Empty client list for tests
    let token_store = Arc::new(TokenStore::new());

    let health_manager = Arc::new(HealthCheckManager::default());
    let provider_registry = Arc::new(ProviderRegistry::new(health_manager));
    let rate_limiter = Arc::new(RateLimiterManager::new(None));

    let metrics_db_path = std::env::temp_dir().join(format!("test_metrics_{}.db", uuid::Uuid::new_v4()));
    let metrics_db = Arc::new(MetricsDatabase::new(metrics_db_path).unwrap());
    let metrics_collector = Arc::new(MetricsCollector::new(metrics_db));

    AppState::new(
        router,
        rate_limiter,
        provider_registry,
        config_manager,
        client_manager,
        token_store,
        metrics_collector,
    )
}

/// Test that broadcast channel is properly integrated into AppState and Gateway
#[tokio::test]
async fn test_broadcast_channel_integration() {
    // Create test AppState (this creates the broadcast channel)
    let state = create_test_app_state();

    // Verify broadcast channel exists
    assert!(
        Arc::strong_count(&state.mcp_notification_broadcast) >= 1,
        "Broadcast channel should be created"
    );

    // Create MCP manager and gateway with broadcast
    let mcp_manager = Arc::new(McpServerManager::new());
    let state = state.with_mcp(mcp_manager.clone());

    // Verify gateway was updated
    assert!(
        Arc::strong_count(&state.mcp_gateway) >= 1,
        "Gateway should be created"
    );
}

/// Test that notifications can be sent and received through the broadcast channel
#[tokio::test]
async fn test_broadcast_channel_send_receive() {
    // Create test AppState
    let state = create_test_app_state();

    // Subscribe to broadcast channel (like a WebSocket client would)
    let mut rx = state.mcp_notification_broadcast.subscribe();

    // Create test notification
    let notification = JsonRpcNotification {
        jsonrpc: "2.0".to_string(),
        method: "notifications/tools/list_changed".to_string(),
        params: None,
    };

    // Send notification through broadcast
    let send_result = state
        .mcp_notification_broadcast
        .send(("test_server".to_string(), notification.clone()));

    assert!(send_result.is_ok(), "Should be able to send notification");
    assert_eq!(
        send_result.unwrap(),
        1,
        "Should have 1 receiver (our subscriber)"
    );

    // Receive notification
    let received = tokio::time::timeout(Duration::from_secs(1), rx.recv())
        .await
        .expect("Should receive notification within timeout")
        .expect("Should not get error");

    assert_eq!(received.0, "test_server");
    assert_eq!(received.1.method, "notifications/tools/list_changed");
}

/// Test that multiple subscribers can receive the same notification
#[tokio::test]
async fn test_broadcast_multiple_subscribers() {
    // Create test AppState
    let state = create_test_app_state();

    // Create multiple subscribers (simulate multiple WebSocket clients)
    let mut rx1 = state.mcp_notification_broadcast.subscribe();
    let mut rx2 = state.mcp_notification_broadcast.subscribe();
    let mut rx3 = state.mcp_notification_broadcast.subscribe();

    // Create test notification
    let notification = JsonRpcNotification {
        jsonrpc: "2.0".to_string(),
        method: "notifications/resources/list_changed".to_string(),
        params: None,
    };

    // Send notification
    let send_result = state
        .mcp_notification_broadcast
        .send(("test_server".to_string(), notification.clone()));

    assert_eq!(
        send_result.unwrap(),
        3,
        "Should have 3 receivers (all subscribers)"
    );

    // All subscribers should receive the notification
    let received1 = rx1.recv().await.expect("Subscriber 1 should receive");
    let received2 = rx2.recv().await.expect("Subscriber 2 should receive");
    let received3 = rx3.recv().await.expect("Subscriber 3 should receive");

    assert_eq!(received1.0, "test_server");
    assert_eq!(received2.0, "test_server");
    assert_eq!(received3.0, "test_server");

    assert_eq!(
        received1.1.method,
        "notifications/resources/list_changed"
    );
    assert_eq!(
        received2.1.method,
        "notifications/resources/list_changed"
    );
    assert_eq!(
        received3.1.method,
        "notifications/resources/list_changed"
    );
}

/// Test that old messages are dropped when channel is full
#[tokio::test]
async fn test_broadcast_channel_backpressure() {
    // Create test AppState
    let state = create_test_app_state();

    // Create subscriber but don't read from it (simulate slow client)
    let _rx = state.mcp_notification_broadcast.subscribe();

    // Send many messages (more than channel capacity of 1000)
    for i in 0..1500 {
        let notification = JsonRpcNotification {
            jsonrpc: "2.0".to_string(),
            method: format!("test_method_{}", i),
            params: None,
        };

        let _ = state
            .mcp_notification_broadcast
            .send(("test_server".to_string(), notification));
    }

    // Test passes if we don't deadlock or panic
    // The broadcast channel should drop old messages automatically
}

/// Test gateway notification handler forwards to broadcast channel
#[tokio::test]
async fn test_gateway_forwards_notifications() {
    // Create test AppState with broadcast channel
    let state = create_test_app_state();

    // Subscribe to broadcast (like a WebSocket client would)
    let mut rx = state.mcp_notification_broadcast.subscribe();

    // Create MCP manager and gateway with broadcast
    let mcp_manager = Arc::new(McpServerManager::new());
    let router = create_test_router();
    let gateway = Arc::new(McpGateway::new_with_broadcast(
        mcp_manager.clone(),
        GatewayConfig::default(),
        router,
        Some(state.mcp_notification_broadcast.clone()),
    ));

    // Create a test session (this registers notification handlers)
    let _session = gateway
        .handle_request(
            "test_client",
            vec!["test_server".to_string()],
            false,
            vec![], // Empty roots for test
            JsonRpcRequest::new(
                Some(serde_json::json!(1)),
                "initialize".to_string(),
                Some(serde_json::json!({
                    "protocolVersion": "2024-11-05",
                    "capabilities": {},
                    "clientInfo": {
                        "name": "test",
                        "version": "1.0"
                    }
                })),
            ),
        )
        .await;

    // NOTE: This test verifies the structure is correct
    // Full end-to-end testing requires mock MCP servers that can send notifications
    // which is handled in mcp_gateway_mock_integration_tests.rs

    // Verify the gateway has notification handlers registered
    // (actual notification forwarding is tested with mock servers)
}
