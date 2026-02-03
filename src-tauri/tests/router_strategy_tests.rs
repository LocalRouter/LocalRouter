//! Comprehensive tests for router with routing strategies
//!
//! Tests cover:
//! - Strategy-based model validation
//! - Auto-routing with intelligent fallback
//! - Error classification and retry logic
//! - Strategy rate limiting using metrics
//! - Parent lifecycle management
//! - Strategy metrics tracking

use localrouter::config::ConfigManager;
use localrouter::config::{
    AppConfig, AutoModelConfig, AvailableModelsSelection, Client, FirewallRules, McpPermissions,
    McpServerAccess, ModelPermissions, PermissionState, RateLimitTimeWindow, RateLimitType,
    SkillsAccess, SkillsPermissions, Strategy, StrategyRateLimit,
};
use localrouter::monitoring::metrics::MetricsCollector;
use localrouter::monitoring::storage::MetricsDatabase;
use localrouter::providers::registry::ProviderRegistry;
use localrouter::providers::{ChatMessage, ChatMessageContent, CompletionRequest};
use localrouter::router::{RateLimiterManager, Router};
use localrouter::utils::errors::AppError;
use std::sync::Arc;

/// Helper to create a test config with client and strategy
fn create_test_config(
    strategy_id: &str,
    allowed_models: AvailableModelsSelection,
    auto_config: Option<AutoModelConfig>,
    rate_limits: Vec<StrategyRateLimit>,
) -> AppConfig {
    let strategy = Strategy {
        id: strategy_id.to_string(),
        name: "Test Strategy".to_string(),
        parent: None,
        allowed_models,
        auto_config,
        rate_limits,
    };

    let client = Client {
        id: "test-client".to_string(),
        name: "Test Client".to_string(),
        enabled: true,
        strategy_id: strategy_id.to_string(),
        allowed_llm_providers: vec![],
        mcp_server_access: McpServerAccess::None,
        roots: None,
        mcp_sampling_enabled: false,
        mcp_sampling_requires_approval: true,
        mcp_sampling_max_tokens: None,
        mcp_sampling_rate_limit: None,
        firewall: FirewallRules::default(),
        mcp_deferred_loading: false,
        skills_access: SkillsAccess::default(),
        created_at: chrono::Utc::now(),
        last_used: None,
        marketplace_enabled: false,
        mcp_permissions: McpPermissions::default(),
        skills_permissions: SkillsPermissions::default(),
        model_permissions: ModelPermissions::default(),
        marketplace_permission: PermissionState::Off,
    };

    AppConfig {
        strategies: vec![strategy],
        clients: vec![client],
        ..AppConfig::default()
    }
}

/// Helper to create a basic completion request
fn create_test_request(model: &str) -> CompletionRequest {
    CompletionRequest {
        model: model.to_string(),
        messages: vec![ChatMessage {
            role: "user".to_string(),
            content: ChatMessageContent::Text("test".to_string()),
            tool_calls: None,
            tool_call_id: None,
            name: None,
        }],
        temperature: Some(0.7),
        max_tokens: Some(100),
        stream: false,
        top_p: None,
        frequency_penalty: None,
        presence_penalty: None,
        stop: None,
        top_k: None,
        seed: None,
        repetition_penalty: None,
        extensions: None,
        logprobs: None,
        top_logprobs: None,
        response_format: None,
        tool_choice: None,
        tools: None,
    }
}

/// Helper to create a test router
fn create_test_router(config: AppConfig) -> Router {
    let config_manager = Arc::new(ConfigManager::new(
        config,
        std::path::PathBuf::from("/tmp/test_router.yaml"),
    ));

    let provider_registry = Arc::new(ProviderRegistry::new());
    let rate_limiter = Arc::new(RateLimiterManager::new(None));

    // Create test metrics collector with unique DB path
    let metrics_db_path =
        std::env::temp_dir().join(format!("test_router_metrics_{}.db", uuid::Uuid::new_v4()));
    let metrics_db = Arc::new(MetricsDatabase::new(metrics_db_path).unwrap());
    let metrics_collector = Arc::new(MetricsCollector::new(metrics_db));

    Router::new(
        config_manager,
        provider_registry,
        rate_limiter,
        metrics_collector,
    )
}

// ============================================================================
// Test 1: Strategy Model Validation
// ============================================================================

#[tokio::test]
async fn test_strategy_allows_specific_model() {
    let allowed_models = AvailableModelsSelection {
        selected_all: false,
        selected_providers: vec![],
        selected_models: vec![("ollama".to_string(), "llama2".to_string())],
    };

    let config = create_test_config("test-strategy", allowed_models, None, vec![]);
    let router = create_test_router(config);

    let request = create_test_request("ollama/llama2");
    let result = router.complete("test-client", request).await;

    // Should fail with provider not found (no providers configured) but NOT with model not allowed
    match result {
        Err(AppError::Router(msg)) => {
            assert!(
                !msg.contains("not allowed"),
                "Model should be allowed, got: {}",
                msg
            );
        }
        Err(e) => panic!("Expected Router error, got: {:?}", e),
        Ok(_) => panic!("Expected error (no provider configured)"),
    }
}

#[tokio::test]
async fn test_strategy_blocks_disallowed_model() {
    let allowed_models = AvailableModelsSelection {
        selected_all: false,
        selected_providers: vec![],
        selected_models: vec![("ollama".to_string(), "llama2".to_string())],
    };

    let config = create_test_config("test-strategy", allowed_models, None, vec![]);
    let router = create_test_router(config);

    let request = create_test_request("openai/gpt-4");
    let result = router.complete("test-client", request).await;

    match result {
        Err(AppError::Router(msg)) => {
            assert!(
                msg.contains("not allowed"),
                "Expected 'not allowed' error, got: {}",
                msg
            );
        }
        Err(e) => panic!("Expected Router error with 'not allowed', got: {:?}", e),
        Ok(_) => panic!("Expected error for disallowed model"),
    }
}

#[tokio::test]
async fn test_strategy_allows_all_provider_models() {
    let allowed_models = AvailableModelsSelection {
        selected_all: false,
        selected_providers: vec!["ollama".to_string()],
        selected_models: vec![],
    };

    let config = create_test_config("test-strategy", allowed_models, None, vec![]);
    let router = create_test_router(config);

    // Any ollama model should be allowed
    let request = create_test_request("ollama/any-model-name");
    let result = router.complete("test-client", request).await;

    // Should fail with provider not found, not with model not allowed
    match result {
        Err(AppError::Router(msg)) => {
            assert!(
                !msg.contains("not allowed"),
                "Model should be allowed, got: {}",
                msg
            );
        }
        Err(e) => panic!("Expected Router error, got: {:?}", e),
        Ok(_) => panic!("Expected error (no provider configured)"),
    }
}

// ============================================================================
// Test 2: Auto-Routing with localrouter/auto
// ============================================================================

#[tokio::test]
async fn test_auto_routing_requires_enabled() {
    let auto_config = AutoModelConfig {
        enabled: false,
        model_name: "localrouter/auto".to_string(),
        prioritized_models: vec![("ollama".to_string(), "llama2".to_string())],
        available_models: vec![],
        routellm_config: None,
    };

    let allowed_models = AvailableModelsSelection {
        selected_all: false,
        selected_providers: vec!["ollama".to_string()],
        selected_models: vec![],
    };

    let config = create_test_config("test-strategy", allowed_models, Some(auto_config), vec![]);
    let router = create_test_router(config);

    let request = create_test_request("localrouter/auto");
    let result = router.complete("test-client", request).await;

    match result {
        Err(AppError::Router(msg)) => {
            assert!(
                msg.contains("disabled"),
                "Expected 'disabled' error, got: {}",
                msg
            );
        }
        Err(e) => panic!("Expected Router error with 'disabled', got: {:?}", e),
        Ok(_) => panic!("Expected error for disabled auto-routing"),
    }
}

#[tokio::test]
async fn test_auto_routing_requires_prioritized_models() {
    let auto_config = AutoModelConfig {
        enabled: true,
        model_name: "localrouter/auto".to_string(),
        prioritized_models: vec![], // Empty list
        available_models: vec![],
        routellm_config: None,
    };

    let allowed_models = AvailableModelsSelection {
        selected_all: false,
        selected_providers: vec!["ollama".to_string()],
        selected_models: vec![],
    };

    let config = create_test_config("test-strategy", allowed_models, Some(auto_config), vec![]);
    let router = create_test_router(config);

    let request = create_test_request("localrouter/auto");
    let result = router.complete("test-client", request).await;

    match result {
        Err(AppError::Router(msg)) => {
            assert!(
                msg.contains("No prioritized models"),
                "Expected 'No prioritized models' error, got: {}",
                msg
            );
        }
        Err(e) => panic!(
            "Expected Router error with 'No prioritized models', got: {:?}",
            e
        ),
        Ok(_) => panic!("Expected error for empty prioritized models"),
    }
}

#[tokio::test]
async fn test_auto_routing_without_config() {
    let allowed_models = AvailableModelsSelection {
        selected_all: false,
        selected_providers: vec!["ollama".to_string()],
        selected_models: vec![],
    };

    let config = create_test_config("test-strategy", allowed_models, None, vec![]);
    let router = create_test_router(config);

    let request = create_test_request("localrouter/auto");
    let result = router.complete("test-client", request).await;

    match result {
        Err(AppError::Router(msg)) => {
            assert!(
                msg.contains("not configured"),
                "Expected 'not configured' error, got: {}",
                msg
            );
        }
        Err(e) => panic!("Expected Router error with 'not configured', got: {:?}", e),
        Ok(_) => panic!("Expected error for missing auto config"),
    }
}

// ============================================================================
// Test 3: Error Classification
// ============================================================================

#[test]
fn test_error_classification_rate_limited() {
    let error = AppError::RateLimitExceeded;
    let router_error = localrouter::router::RouterError::classify(&error, "openai", "gpt-4");

    assert!(router_error.should_retry());
    assert!(router_error.to_log_string().contains("RATE_LIMITED"));
}

#[test]
fn test_error_classification_policy_violation() {
    let error = AppError::Provider("content_policy violation".to_string());
    let router_error = localrouter::router::RouterError::classify(&error, "openai", "gpt-4");

    assert!(router_error.should_retry());
    assert!(router_error.to_log_string().contains("POLICY_VIOLATION"));
}

#[test]
fn test_error_classification_context_length() {
    let error = AppError::Provider("context length exceeded".to_string());
    let router_error = localrouter::router::RouterError::classify(&error, "openai", "gpt-4");

    assert!(router_error.should_retry());
    assert!(router_error
        .to_log_string()
        .contains("CONTEXT_LENGTH_EXCEEDED"));
}

#[test]
fn test_error_classification_unreachable() {
    let error = AppError::Provider("connection timeout".to_string());
    let router_error = localrouter::router::RouterError::classify(&error, "openai", "gpt-4");

    assert!(router_error.should_retry());
    assert!(router_error.to_log_string().contains("UNREACHABLE"));
}

#[test]
fn test_error_classification_io_error() {
    let error = AppError::Io(std::io::Error::new(
        std::io::ErrorKind::ConnectionRefused,
        "connection refused",
    ));
    let router_error = localrouter::router::RouterError::classify(&error, "openai", "gpt-4");

    assert!(router_error.should_retry());
    assert!(router_error.to_log_string().contains("UNREACHABLE"));
}

#[test]
fn test_error_classification_other_non_retryable() {
    let error = AppError::Config("Invalid request".to_string());
    let router_error = localrouter::router::RouterError::classify(&error, "openai", "gpt-4");

    assert!(!router_error.should_retry());
    assert!(router_error.to_log_string().contains("ERROR"));
}

// ============================================================================
// Test 4: Strategy Rate Limiting
// ============================================================================

#[tokio::test]
async fn test_strategy_rate_limit_requests() {
    let rate_limits = vec![StrategyRateLimit {
        limit_type: RateLimitType::Requests,
        value: 5.0,
        time_window: RateLimitTimeWindow::Minute,
    }];

    let allowed_models = AvailableModelsSelection {
        selected_all: false,
        selected_providers: vec!["ollama".to_string()],
        selected_models: vec![],
    };

    let config = create_test_config("test-strategy", allowed_models, None, rate_limits);
    let router = create_test_router(config);

    // First request should pass rate limit check (but fail on provider not found)
    let request = create_test_request("ollama/llama2");
    let result = router.complete("test-client", request).await;

    // Should not be rate limited
    match result {
        Err(AppError::RateLimitExceeded) => {
            panic!("Should not be rate limited on first request")
        }
        Err(_) => {} // Expected: provider not found or other router error
        Ok(_) => panic!("Expected error (no provider configured)"),
    }
}

#[tokio::test]
async fn test_strategy_rate_limit_cost_ignores_free_models() {
    let rate_limits = vec![StrategyRateLimit {
        limit_type: RateLimitType::Cost,
        value: 1.0, // $1 limit
        time_window: RateLimitTimeWindow::Hour,
    }];

    let allowed_models = AvailableModelsSelection {
        selected_all: false,
        selected_providers: vec!["ollama".to_string()],
        selected_models: vec![],
    };

    let config = create_test_config("test-strategy", allowed_models, None, rate_limits);
    let router = create_test_router(config);

    // Ollama is free, so cost limit should not apply
    let request = create_test_request("ollama/llama2");
    let result = router.complete("test-client", request).await;

    // Should not be rate limited by cost (avg_cost = 0 for free models)
    match result {
        Err(AppError::RateLimitExceeded) => {
            panic!("Should not be rate limited by cost for free model")
        }
        Err(_) => {} // Expected: provider not found or other router error
        Ok(_) => panic!("Expected error (no provider configured)"),
    }
}

// ============================================================================
// Test 5: Disabled Client
// ============================================================================

#[tokio::test]
async fn test_disabled_client_returns_unauthorized() {
    let strategy = Strategy {
        id: "test-strategy".to_string(),
        name: "Test Strategy".to_string(),
        parent: None,
        allowed_models: AvailableModelsSelection {
            selected_all: false,
            selected_providers: vec!["ollama".to_string()],
            selected_models: vec![],
        },
        auto_config: None,
        rate_limits: vec![],
    };

    let client = Client {
        id: "test-client".to_string(),
        name: "Test Client".to_string(),
        enabled: false, // Disabled
        strategy_id: "test-strategy".to_string(),
        allowed_llm_providers: vec![],
        mcp_server_access: McpServerAccess::None,
        roots: None,
        mcp_sampling_enabled: false,
        mcp_sampling_requires_approval: true,
        mcp_sampling_max_tokens: None,
        mcp_sampling_rate_limit: None,
        firewall: FirewallRules::default(),
        mcp_deferred_loading: false,
        skills_access: SkillsAccess::default(),
        created_at: chrono::Utc::now(),
        last_used: None,
        marketplace_enabled: false,
        mcp_permissions: McpPermissions::default(),
        skills_permissions: SkillsPermissions::default(),
        model_permissions: ModelPermissions::default(),
        marketplace_permission: PermissionState::Off,
    };

    let config = AppConfig {
        strategies: vec![strategy],
        clients: vec![client],
        ..AppConfig::default()
    };

    let router = create_test_router(config);

    let request = create_test_request("ollama/llama2");
    let result = router.complete("test-client", request).await;

    assert!(matches!(result, Err(AppError::Unauthorized)));
}

// ============================================================================
// Test 6: Missing Strategy Reference
// ============================================================================

#[tokio::test]
async fn test_client_with_missing_strategy() {
    let client = Client {
        id: "test-client".to_string(),
        name: "Test Client".to_string(),
        enabled: true,
        strategy_id: "non-existent-strategy".to_string(), // References non-existent strategy
        allowed_llm_providers: vec![],
        mcp_server_access: McpServerAccess::None,
        roots: None,
        mcp_sampling_enabled: false,
        mcp_sampling_requires_approval: true,
        mcp_sampling_max_tokens: None,
        mcp_sampling_rate_limit: None,
        firewall: FirewallRules::default(),
        mcp_deferred_loading: false,
        skills_access: SkillsAccess::default(),
        created_at: chrono::Utc::now(),
        last_used: None,
        marketplace_enabled: false,
        mcp_permissions: McpPermissions::default(),
        skills_permissions: SkillsPermissions::default(),
        model_permissions: ModelPermissions::default(),
        marketplace_permission: PermissionState::Off,
    };

    let config = AppConfig {
        strategies: vec![],
        clients: vec![client],
        ..AppConfig::default()
    };

    let router = create_test_router(config);

    let request = create_test_request("ollama/llama2");
    let result = router.complete("test-client", request).await;

    match result {
        Err(AppError::Router(msg)) => {
            assert!(
                msg.contains("Strategy") && msg.contains("not found"),
                "Expected 'Strategy not found' error, got: {}",
                msg
            );
        }
        Err(e) => panic!(
            "Expected Router error with 'Strategy not found', got: {:?}",
            e
        ),
        Ok(_) => panic!("Expected error for missing strategy"),
    }
}

// ============================================================================
// Test 7: Streaming Not Supported for Auto-Routing
// ============================================================================

#[tokio::test]
async fn test_streaming_supports_auto_routing() {
    // Verify that streaming now supports auto-routing (as of recent implementation)
    let auto_config = AutoModelConfig {
        enabled: true,
        model_name: "localrouter/auto".to_string(),
        prioritized_models: vec![("ollama".to_string(), "llama2".to_string())],
        available_models: vec![],
        routellm_config: None,
    };

    let allowed_models = AvailableModelsSelection {
        selected_all: false,
        selected_providers: vec!["ollama".to_string()],
        selected_models: vec![],
    };

    let config = create_test_config("test-strategy", allowed_models, Some(auto_config), vec![]);
    let router = create_test_router(config);

    let request = create_test_request("localrouter/auto");
    let result = router.stream_complete("test-client", request).await;

    match result {
        Err(AppError::Router(msg)) => {
            // Streaming now supports auto-routing, so should NOT error with "not supported"
            assert!(
                !msg.contains("not supported for streaming"),
                "Streaming should support auto-routing now, got: {}",
                msg
            );
            // Expected: provider not found, unreachable, or "failed"
            assert!(
                msg.contains("Provider")
                    || msg.contains("not found")
                    || msg.contains("failed")
                    || msg.contains("UNREACHABLE"),
                "Expected provider-related error, got: {}",
                msg
            );
        }
        Err(e) => {
            // Other errors are acceptable (provider not found, etc.)
            println!("Router returned error (expected): {:?}", e);
        }
        Ok(_) => panic!("Unexpected success without configured providers"),
    }
}

// ============================================================================
// Test 8: Internal Test Token Bypass
// ============================================================================

#[tokio::test]
async fn test_internal_test_token_bypasses_routing() {
    // Create empty config (internal token should bypass)
    let config = AppConfig::default();
    let router = create_test_router(config);

    let request = create_test_request("ollama/llama2");
    let result = router.complete("internal-test", request).await;

    // Should fail with provider not found, not with unauthorized or strategy errors
    match result {
        Err(AppError::Unauthorized) => {
            panic!("Internal test token should bypass auth check")
        }
        Err(AppError::Router(msg)) if msg.contains("Strategy") => {
            panic!("Internal test token should bypass strategy check")
        }
        Err(_) => {} // Expected: provider not found
        Ok(_) => panic!("Expected error (no provider configured)"),
    }
}

#[tokio::test]
async fn test_internal_test_token_requires_provider_prefix() {
    let config = AppConfig::default();
    let router = create_test_router(config);

    // Internal test token requires "provider/model" format
    let request = create_test_request("just-model-name");
    let result = router.complete("internal-test", request).await;

    match result {
        Err(AppError::Router(msg)) => {
            assert!(
                msg.contains("requires provider/model format"),
                "Expected format error, got: {}",
                msg
            );
        }
        Err(e) => panic!(
            "Expected Router error about format requirement, got: {:?}",
            e
        ),
        Ok(_) => panic!("Expected error for missing provider prefix"),
    }
}

// ============================================================================
// Test 9: Model Without Provider Prefix
// ============================================================================

#[tokio::test]
async fn test_model_without_provider_finds_from_individual_models() {
    let allowed_models = AvailableModelsSelection {
        selected_all: false,
        selected_providers: vec![],
        selected_models: vec![("ollama".to_string(), "llama2".to_string())],
    };

    let config = create_test_config("test-strategy", allowed_models, None, vec![]);
    let router = create_test_router(config);

    // Request just "llama2" without provider prefix
    let request = create_test_request("llama2");
    let result = router.complete("test-client", request).await;

    // Should resolve provider from individual_models and fail with provider not found (not "not allowed")
    match result {
        Err(AppError::Router(msg)) => {
            assert!(
                !msg.contains("not allowed"),
                "Model should be resolved from individual_models, got: {}",
                msg
            );
        }
        Err(e) => panic!("Expected Router error, got: {:?}", e),
        Ok(_) => panic!("Expected error (no provider configured)"),
    }
}

#[tokio::test]
async fn test_model_without_provider_not_in_allowed() {
    let allowed_models = AvailableModelsSelection {
        selected_all: false,
        selected_providers: vec![],
        selected_models: vec![("ollama".to_string(), "llama2".to_string())],
    };

    let config = create_test_config("test-strategy", allowed_models, None, vec![]);
    let router = create_test_router(config);

    // Request model not in allowed list
    let request = create_test_request("gpt-4");
    let result = router.complete("test-client", request).await;

    match result {
        Err(AppError::Router(msg)) => {
            assert!(
                msg.contains("not allowed"),
                "Expected 'not allowed' error, got: {}",
                msg
            );
        }
        Err(e) => panic!("Expected Router error with 'not allowed', got: {:?}", e),
        Ok(_) => panic!("Expected error for model not in allowed list"),
    }
}

// ============================================================================
// Test 10: Bug Tests - Conservative Cost Estimate
// ============================================================================

#[tokio::test]
async fn test_bug_conservative_cost_estimate_for_free_models() {
    // Bug: get_pre_estimate_for_strategy returns (1000.0, 0.01) when no recent data
    // This incorrectly counts $0.01 for free models on the first request

    // Create metrics collector
    let metrics_db_path =
        std::env::temp_dir().join(format!("test_bug_cost_{}.db", uuid::Uuid::new_v4()));
    let metrics_db = Arc::new(MetricsDatabase::new(metrics_db_path).unwrap());
    let metrics_collector = Arc::new(MetricsCollector::new(metrics_db));

    // Get pre-estimate for strategy with no history
    let (avg_tokens, avg_cost) =
        metrics_collector.get_pre_estimate_for_strategy("test-strategy", 10);

    // The bug: avg_cost should be 0.0 for strategies with no history
    // but it returns 0.01 as "conservative estimate"
    println!(
        "Conservative estimate: tokens={}, cost=${}",
        avg_tokens, avg_cost
    );

    // This assertion will FAIL, demonstrating the bug
    // Expected: 0.0 (no data = can't estimate, assume free)
    // Actual: 0.01 (hardcoded conservative estimate)
    assert_eq!(
        avg_cost, 0.0,
        "Conservative cost estimate should be 0.0 for strategies with no history, got {}",
        avg_cost
    );
}

// ============================================================================
// Test 11: Bug Tests - Model ID Normalization
// ============================================================================

#[tokio::test]
async fn test_bug_model_id_with_tag_suffix() {
    // Bug: Ollama returns "llama2:latest" but user requests "llama2"
    // Should match after normalization

    let allowed_models = AvailableModelsSelection {
        selected_all: false,
        selected_providers: vec![],
        selected_models: vec![("ollama".to_string(), "llama2:latest".to_string())],
    };

    let config = create_test_config("test-strategy", allowed_models, None, vec![]);
    let router = create_test_router(config);

    // Request "llama2" (without tag) - should resolve to ollama provider
    let request = create_test_request("llama2");
    let result = router.complete("test-client", request).await;

    // Should fail with provider not found (not with "not allowed")
    match result {
        Err(AppError::Router(msg)) => {
            assert!(
                !msg.contains("not allowed"),
                "Model should be allowed after normalization, got: {}",
                msg
            );
        }
        Err(e) => panic!("Expected Router error, got: {:?}", e),
        Ok(_) => panic!("Expected error (no provider configured)"),
    }
}

#[tokio::test]
async fn test_bug_model_id_with_provider_prefix() {
    // Bug: OpenAI-compatible returns "openai/gpt-4" but user requests "gpt-4"
    // Should match after normalization

    let allowed_models = AvailableModelsSelection {
        selected_all: false,
        selected_providers: vec![],
        selected_models: vec![("openai".to_string(), "openai/gpt-4".to_string())],
    };

    let config = create_test_config("test-strategy", allowed_models, None, vec![]);
    let router = create_test_router(config);

    // Request "gpt-4" (without provider prefix) - should resolve to openai provider
    let request = create_test_request("gpt-4");
    let result = router.complete("test-client", request).await;

    // Should fail with provider not found (not with "not allowed")
    match result {
        Err(AppError::Router(msg)) => {
            assert!(
                !msg.contains("not allowed"),
                "Model should be allowed after normalization, got: {}",
                msg
            );
        }
        Err(e) => panic!("Expected Router error, got: {:?}", e),
        Ok(_) => panic!("Expected error (no provider configured)"),
    }
}

#[tokio::test]
async fn test_bug_model_id_with_both_prefix_and_suffix() {
    // Bug: Provider returns "provider/model:tag" but user requests "model"
    // Should match after stripping both prefix and suffix

    let allowed_models = AvailableModelsSelection {
        selected_all: false,
        selected_providers: vec![],
        selected_models: vec![("ollama".to_string(), "ollama/llama2:7b".to_string())],
    };

    let config = create_test_config("test-strategy", allowed_models, None, vec![]);
    let router = create_test_router(config);

    // Request "llama2" - should resolve to ollama provider after normalization
    let request = create_test_request("llama2");
    let result = router.complete("test-client", request).await;

    // Should fail with provider not found (not with "not allowed")
    match result {
        Err(AppError::Router(msg)) => {
            assert!(
                !msg.contains("not allowed"),
                "Model should be allowed after normalization, got: {}",
                msg
            );
        }
        Err(e) => panic!("Expected Router error, got: {:?}", e),
        Ok(_) => panic!("Expected error (no provider configured)"),
    }
}

// ============================================================================
// Test 12: Edge Cases
// ============================================================================

#[tokio::test]
async fn test_empty_allowed_models_blocks_all() {
    let allowed_models = AvailableModelsSelection {
        selected_all: false,
        selected_providers: vec![],
        selected_models: vec![],
    };

    let config = create_test_config("test-strategy", allowed_models, None, vec![]);
    let router = create_test_router(config);

    let request = create_test_request("ollama/llama2");
    let result = router.complete("test-client", request).await;

    match result {
        Err(AppError::Router(msg)) => {
            assert!(
                msg.contains("not allowed"),
                "Expected 'not allowed' error, got: {}",
                msg
            );
        }
        Err(e) => panic!("Expected Router error with 'not allowed', got: {:?}", e),
        Ok(_) => panic!("Expected error for empty allowed models"),
    }
}

#[tokio::test]
async fn test_nonexistent_client_returns_unauthorized() {
    let config = AppConfig::default();
    let router = create_test_router(config);

    let request = create_test_request("ollama/llama2");
    let result = router.complete("nonexistent-client", request).await;

    assert!(matches!(result, Err(AppError::Unauthorized)));
}

// ============================================================================
// Test 13: Auto-Routing Fallback Behavior
// ============================================================================

// Note: These tests verify the fallback logic configuration and flow.
// Full integration testing with mock providers would require additional
// test infrastructure to simulate provider failures.

#[tokio::test]
async fn test_auto_routing_fallback_configuration() {
    // Verify that auto-routing is configured to try multiple models in order
    let auto_config = AutoModelConfig {
        enabled: true,
        model_name: "localrouter/auto".to_string(),
        prioritized_models: vec![
            ("ollama".to_string(), "model1".to_string()),
            ("ollama".to_string(), "model2".to_string()),
            ("ollama".to_string(), "model3".to_string()),
        ],
        available_models: vec![],
        routellm_config: None,
    };

    let allowed_models = AvailableModelsSelection {
        selected_all: false,
        selected_providers: vec!["ollama".to_string()],
        selected_models: vec![],
    };

    let config = create_test_config("test-strategy", allowed_models, Some(auto_config), vec![]);
    let router = create_test_router(config);

    let request = create_test_request("localrouter/auto");
    let result = router.complete("test-client", request).await;

    // Without real providers, this will fail at provider execution
    // In a full integration test, we would mock providers to:
    // 1. Make model1 fail with RateLimitExceeded
    // 2. Make model2 fail with PolicyViolation
    // 3. Make model3 succeed
    // Then verify model3 was called and response received

    match result {
        Err(AppError::Router(msg)) => {
            // Expected: provider not found or similar
            assert!(
                msg.contains("Provider") || msg.contains("not found") || msg.contains("failed"),
                "Expected provider-related error, got: {}",
                msg
            );
        }
        Err(e) => {
            // Other errors are also acceptable for this test
            println!("Router returned error (expected): {:?}", e);
        }
        Ok(_) => panic!("Unexpected success without configured providers"),
    }
}

#[tokio::test]
async fn test_auto_routing_strategy_rate_limits_checked_per_model() {
    // Verify that strategy rate limits are checked for each model attempt
    // This prevents the first model from being tried if its rate limit is exceeded

    use localrouter::config::RateLimitTimeWindow;
    use localrouter::config::RateLimitType;

    let auto_config = AutoModelConfig {
        enabled: true,
        model_name: "localrouter/auto".to_string(),
        prioritized_models: vec![
            ("ollama".to_string(), "model1".to_string()),
            ("ollama".to_string(), "model2".to_string()),
        ],
        available_models: vec![],
        routellm_config: None,
    };

    let allowed_models = AvailableModelsSelection {
        selected_all: false,
        selected_providers: vec!["ollama".to_string()],
        selected_models: vec![],
    };

    // Set an impossible rate limit (0 requests per minute)
    let rate_limits = vec![localrouter::config::StrategyRateLimit {
        limit_type: RateLimitType::Requests,
        value: 0.0, // Zero requests allowed
        time_window: RateLimitTimeWindow::Minute,
    }];

    let config = create_test_config(
        "test-strategy",
        allowed_models,
        Some(auto_config),
        rate_limits,
    );
    let router = create_test_router(config);

    let request = create_test_request("localrouter/auto");
    let result = router.complete("test-client", request).await;

    // Should fail with rate limit exceeded before trying any models
    match result {
        Err(AppError::RateLimitExceeded) => {
            // Expected: rate limit check prevents any model from being tried
        }
        Err(AppError::Router(msg)) if msg.contains("failed") || msg.contains("rate") => {
            // Also acceptable: wrapped in Router error
            println!("Rate limit error (expected): {}", msg);
        }
        Err(e) => panic!("Expected RateLimitExceeded or Router error, got: {:?}", e),
        Ok(_) => panic!("Expected rate limit error"),
    }
}

#[test]
fn test_router_error_should_retry_logic() {
    // Verify that retryable errors are correctly classified
    use localrouter::router::RouterError;

    let retryable_errors = vec![
        RouterError::RateLimited {
            provider: "test".to_string(),
            model: "test".to_string(),
            retry_after_secs: 60,
        },
        RouterError::PolicyViolation {
            provider: "test".to_string(),
            model: "test".to_string(),
            reason: "test".to_string(),
        },
        RouterError::ContextLengthExceeded {
            provider: "test".to_string(),
            model: "test".to_string(),
            max_tokens: 1000,
        },
        RouterError::Unreachable {
            provider: "test".to_string(),
            model: "test".to_string(),
        },
    ];

    for error in retryable_errors {
        assert!(
            error.should_retry(),
            "Error {:?} should be retryable",
            error
        );
    }

    // Non-retryable error
    let non_retryable = RouterError::Other {
        provider: "test".to_string(),
        model: "test".to_string(),
        error: "validation failed".to_string(),
    };
    assert!(
        !non_retryable.should_retry(),
        "Other errors should not be retryable"
    );
}
