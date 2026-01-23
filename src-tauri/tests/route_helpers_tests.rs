//! Integration tests for route helper functions
//!
//! Tests the shared validation functions used across LLM and MCP endpoints.

use std::sync::Arc;

use chrono::Utc;
use localrouter_ai::clients::{ClientManager, TokenStore};
use localrouter_ai::config::{AppConfig, Client, ConfigManager, McpServerAccess, Strategy};
use localrouter_ai::monitoring::metrics::MetricsCollector;
use localrouter_ai::monitoring::storage::MetricsDatabase;
use localrouter_ai::providers::health::HealthCheckManager;
use localrouter_ai::providers::registry::ProviderRegistry;
use localrouter_ai::router::{RateLimiterManager, Router};
use localrouter_ai::server::routes::helpers::{
    get_client_with_strategy, get_enabled_client, get_enabled_client_from_manager,
};
use localrouter_ai::server::state::AppState;

/// Create a test client with minimal required fields
fn create_test_client(id: &str, name: &str, enabled: bool, strategy_id: &str) -> Client {
    Client {
        id: id.to_string(),
        name: name.to_string(),
        enabled,
        allowed_llm_providers: vec![],
        mcp_server_access: McpServerAccess::None,
        mcp_deferred_loading: false,
        created_at: Utc::now(),
        last_used: None,
        strategy_id: strategy_id.to_string(),
        #[allow(deprecated)]
        routing_config: None,
        roots: None,
        mcp_sampling_enabled: false,
        mcp_sampling_requires_approval: true,
        mcp_sampling_max_tokens: None,
        mcp_sampling_rate_limit: None,
    }
}

/// Create a test strategy with minimal required fields
fn create_test_strategy(id: &str, name: &str) -> Strategy {
    Strategy::new(name.to_string()).with_id(id.to_string())
}

/// Helper trait extension to set strategy ID
trait StrategyExt {
    fn with_id(self, id: String) -> Self;
}

impl StrategyExt for Strategy {
    fn with_id(mut self, id: String) -> Self {
        self.id = id;
        self
    }
}

/// Create a test config with specific clients and strategies
fn create_test_config(clients: Vec<Client>, strategies: Vec<Strategy>) -> AppConfig {
    AppConfig {
        clients,
        strategies,
        ..Default::default()
    }
}

/// Create a test AppState with the given config and client manager
fn create_test_state(config: AppConfig, client_manager: Arc<ClientManager>) -> AppState {
    let config_path =
        std::env::temp_dir().join(format!("test_config_{}.yaml", uuid::Uuid::new_v4()));
    let config_manager = Arc::new(ConfigManager::new(config, config_path));
    let token_store = Arc::new(TokenStore::new());

    let health_manager = Arc::new(HealthCheckManager::default());
    let provider_registry = Arc::new(ProviderRegistry::new(health_manager));
    let rate_limiter = Arc::new(RateLimiterManager::new(None));

    let metrics_db_path =
        std::env::temp_dir().join(format!("test_metrics_{}.db", uuid::Uuid::new_v4()));
    let metrics_db = Arc::new(MetricsDatabase::new(metrics_db_path).unwrap());
    let metrics_collector = Arc::new(MetricsCollector::new(metrics_db));

    let router = Arc::new(Router::new(
        config_manager.clone(),
        provider_registry.clone(),
        rate_limiter.clone(),
        metrics_collector.clone(),
    ));

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

// ============================================================================
// Tests for get_enabled_client
// ============================================================================

#[test]
fn test_get_enabled_client_success() {
    let client = create_test_client("test-client-123", "Test Client", true, "default");

    let config = create_test_config(vec![client.clone()], vec![]);
    let client_manager = Arc::new(ClientManager::new(vec![client]));
    let state = create_test_state(config, client_manager);

    // Should successfully get the enabled client
    let result = get_enabled_client(&state, "test-client-123");
    assert!(result.is_ok());
    let retrieved_client = result.unwrap();
    assert_eq!(retrieved_client.id, "test-client-123");
    assert_eq!(retrieved_client.name, "Test Client");
    assert!(retrieved_client.enabled);
}

#[test]
fn test_get_enabled_client_not_found() {
    let config = create_test_config(vec![], vec![]);
    let client_manager = Arc::new(ClientManager::new(vec![]));
    let state = create_test_state(config, client_manager);

    // Should return error for non-existent client
    let result = get_enabled_client(&state, "non-existent-client");
    assert!(result.is_err());

    let err = result.unwrap_err();
    // Check that it's an unauthorized error (401)
    assert!(format!("{:?}", err).contains("Client not found"));
}

#[test]
fn test_get_enabled_client_disabled() {
    let client = create_test_client("disabled-client", "Disabled Client", false, "default");

    let config = create_test_config(vec![client.clone()], vec![]);
    let client_manager = Arc::new(ClientManager::new(vec![client]));
    let state = create_test_state(config, client_manager);

    // Should return error for disabled client
    let result = get_enabled_client(&state, "disabled-client");
    assert!(result.is_err());

    let err = result.unwrap_err();
    // Check that it's a forbidden error (403)
    assert!(format!("{:?}", err).contains("disabled"));
}

// ============================================================================
// Tests for get_client_with_strategy
// ============================================================================

#[test]
fn test_get_client_with_strategy_success() {
    let strategy = create_test_strategy("test-strategy", "Test Strategy");
    let client = create_test_client(
        "client-with-strategy",
        "Client With Strategy",
        true,
        "test-strategy",
    );

    let config = create_test_config(vec![client.clone()], vec![strategy]);
    let client_manager = Arc::new(ClientManager::new(vec![client]));
    let state = create_test_state(config, client_manager);

    // Should successfully get both client and strategy
    let result = get_client_with_strategy(&state, "client-with-strategy");
    assert!(result.is_ok());

    let (retrieved_client, retrieved_strategy) = result.unwrap();
    assert_eq!(retrieved_client.id, "client-with-strategy");
    assert_eq!(retrieved_strategy.id, "test-strategy");
}

#[test]
fn test_get_client_with_strategy_client_not_found() {
    let strategy = create_test_strategy("test-strategy", "Test Strategy");

    let config = create_test_config(vec![], vec![strategy]);
    let client_manager = Arc::new(ClientManager::new(vec![]));
    let state = create_test_state(config, client_manager);

    // Should return error for non-existent client
    let result = get_client_with_strategy(&state, "non-existent");
    assert!(result.is_err());
    assert!(format!("{:?}", result.unwrap_err()).contains("Client not found"));
}

#[test]
fn test_get_client_with_strategy_client_disabled() {
    let strategy = create_test_strategy("test-strategy", "Test Strategy");
    let client = create_test_client("disabled-client", "Disabled Client", false, "test-strategy");

    let config = create_test_config(vec![client.clone()], vec![strategy]);
    let client_manager = Arc::new(ClientManager::new(vec![client]));
    let state = create_test_state(config, client_manager);

    // Should return error for disabled client
    let result = get_client_with_strategy(&state, "disabled-client");
    assert!(result.is_err());
    assert!(format!("{:?}", result.unwrap_err()).contains("disabled"));
}

#[test]
fn test_get_client_with_strategy_strategy_not_found() {
    // Create client with a strategy that doesn't exist
    let client = create_test_client(
        "orphan-client",
        "Orphan Client",
        true,
        "non-existent-strategy",
    );

    let config = create_test_config(vec![client.clone()], vec![]);
    let client_manager = Arc::new(ClientManager::new(vec![client]));
    let state = create_test_state(config, client_manager);

    // Should return error for missing strategy
    let result = get_client_with_strategy(&state, "orphan-client");
    assert!(result.is_err());
    assert!(format!("{:?}", result.unwrap_err()).contains("Strategy"));
}

// ============================================================================
// Tests for get_enabled_client_from_manager
// ============================================================================

#[test]
fn test_get_enabled_client_from_manager_success() {
    // Create client manager with a client
    let client_manager = Arc::new(ClientManager::new(vec![]));

    // Create a new client via the manager (this stores the secret properly)
    let (client_id, _secret, _client) = client_manager
        .create_client("Manager Test Client".to_string())
        .expect("Failed to create client");

    let config = create_test_config(vec![], vec![]);
    let state = create_test_state(config, client_manager);

    // Should successfully get the client from manager
    let result = get_enabled_client_from_manager(&state, &client_id);
    assert!(result.is_ok());

    let retrieved_client = result.unwrap();
    assert_eq!(retrieved_client.id, client_id);
    assert_eq!(retrieved_client.name, "Manager Test Client");
    assert!(retrieved_client.enabled);
}

#[test]
fn test_get_enabled_client_from_manager_not_found() {
    let client_manager = Arc::new(ClientManager::new(vec![]));
    let config = create_test_config(vec![], vec![]);
    let state = create_test_state(config, client_manager);

    // Should return error for non-existent client
    let result = get_enabled_client_from_manager(&state, "non-existent");
    assert!(result.is_err());
    assert!(format!("{:?}", result.unwrap_err()).contains("Client not found"));
}

#[test]
fn test_get_enabled_client_from_manager_disabled() {
    let client_manager = Arc::new(ClientManager::new(vec![]));

    // Create and then disable a client
    let (client_id, _secret, _client) = client_manager
        .create_client("Disabled Manager Client".to_string())
        .expect("Failed to create client");

    client_manager
        .disable_client(&client_id)
        .expect("Failed to disable client");

    let config = create_test_config(vec![], vec![]);
    let state = create_test_state(config, client_manager);

    // Should return error for disabled client
    let result = get_enabled_client_from_manager(&state, &client_id);
    assert!(result.is_err());
    assert!(format!("{:?}", result.unwrap_err()).contains("disabled"));
}

// ============================================================================
// Edge case tests
// ============================================================================

#[test]
fn test_get_enabled_client_multiple_clients() {
    // Create multiple clients
    let clients = vec![
        create_test_client("client-1", "Client One", true, "default"),
        create_test_client("client-2", "Client Two", false, "default"), // disabled
        create_test_client("client-3", "Client Three", true, "default"),
    ];

    let config = create_test_config(clients.clone(), vec![]);
    let client_manager = Arc::new(ClientManager::new(clients));
    let state = create_test_state(config, client_manager);

    // Client 1 should work
    let result1 = get_enabled_client(&state, "client-1");
    assert!(result1.is_ok());
    assert_eq!(result1.unwrap().name, "Client One");

    // Client 2 should fail (disabled)
    let result2 = get_enabled_client(&state, "client-2");
    assert!(result2.is_err());

    // Client 3 should work
    let result3 = get_enabled_client(&state, "client-3");
    assert!(result3.is_ok());
    assert_eq!(result3.unwrap().name, "Client Three");
}

#[test]
fn test_get_client_with_strategy_multiple_strategies() {
    // Create multiple strategies
    let strategies = vec![
        create_test_strategy("strategy-a", "Strategy A"),
        create_test_strategy("strategy-b", "Strategy B"),
    ];

    // Create clients using different strategies
    let clients = vec![
        create_test_client("client-a", "Client A", true, "strategy-a"),
        create_test_client("client-b", "Client B", true, "strategy-b"),
    ];

    let config = create_test_config(clients.clone(), strategies);
    let client_manager = Arc::new(ClientManager::new(clients));
    let state = create_test_state(config, client_manager);

    // Client A should get Strategy A
    let result_a = get_client_with_strategy(&state, "client-a");
    assert!(result_a.is_ok());
    let (client_a, strategy_a) = result_a.unwrap();
    assert_eq!(client_a.id, "client-a");
    assert_eq!(strategy_a.id, "strategy-a");

    // Client B should get Strategy B
    let result_b = get_client_with_strategy(&state, "client-b");
    assert!(result_b.is_ok());
    let (client_b, strategy_b) = result_b.unwrap();
    assert_eq!(client_b.id, "client-b");
    assert_eq!(strategy_b.id, "strategy-b");
}

#[test]
fn test_client_with_mcp_access() {
    // Test that MCP access settings are preserved through the helper
    let mut client = create_test_client("mcp-client", "MCP Client", true, "default");
    client.mcp_server_access =
        McpServerAccess::Specific(vec!["server-1".to_string(), "server-2".to_string()]);

    let config = create_test_config(vec![client.clone()], vec![]);
    let client_manager = Arc::new(ClientManager::new(vec![client]));
    let state = create_test_state(config, client_manager);

    let result = get_enabled_client(&state, "mcp-client");
    assert!(result.is_ok());

    let retrieved = result.unwrap();
    assert!(retrieved.mcp_server_access.can_access("server-1"));
    assert!(retrieved.mcp_server_access.can_access("server-2"));
    assert!(!retrieved.mcp_server_access.can_access("server-3"));
}

#[test]
fn test_client_with_llm_providers() {
    // Test that LLM provider restrictions are preserved through the helper
    let mut client = create_test_client("llm-client", "LLM Client", true, "default");
    client.allowed_llm_providers = vec!["openai".to_string(), "anthropic".to_string()];

    let config = create_test_config(vec![client.clone()], vec![]);
    let client_manager = Arc::new(ClientManager::new(vec![client]));
    let state = create_test_state(config, client_manager);

    let result = get_enabled_client(&state, "llm-client");
    assert!(result.is_ok());

    let retrieved = result.unwrap();
    assert_eq!(retrieved.allowed_llm_providers.len(), 2);
    assert!(retrieved
        .allowed_llm_providers
        .contains(&"openai".to_string()));
    assert!(retrieved
        .allowed_llm_providers
        .contains(&"anthropic".to_string()));
}
