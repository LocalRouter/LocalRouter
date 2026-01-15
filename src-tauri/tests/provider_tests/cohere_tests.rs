//! Tests for Cohere provider
//!
//! Cohere uses a custom API v2 format

use super::common::*;
use futures::StreamExt;
use localrouter_ai::providers::{cohere::CohereProvider, ModelProvider};

#[tokio::test]
async fn test_cohere_list_models() {
    let provider = CohereProvider::new("test-key".to_string()).unwrap();

    let models = provider.list_models().await.unwrap();

    // Cohere provider returns a static list of known models
    assert!(!models.is_empty());
    assert!(models.iter().all(|m| m.provider == "cohere"));
    assert!(models
        .iter()
        .any(|m| m.id.contains("command")));
}

#[tokio::test]
async fn test_cohere_completion() {
    let mock = CohereMockBuilder::new().await.mock_completion().await;

    // Note: CohereProvider uses hardcoded base URL
    // TODO: Refactor to accept custom base_url for testing
}

#[tokio::test]
async fn test_cohere_streaming() {
    let mock = CohereMockBuilder::new()
        .await
        .mock_streaming_completion()
        .await;

    // Similar limitation - needs refactoring to accept custom base URL
    // TODO: Refactor to accept custom base_url
}

#[tokio::test]
async fn test_cohere_provider_name() {
    let provider = CohereProvider::new("test-key".to_string()).unwrap();
    assert_eq!(provider.name(), "cohere");
}

#[tokio::test]
async fn test_cohere_pricing() {
    let provider = CohereProvider::new("test-key".to_string()).unwrap();

    // Test pricing for a known model
    let pricing = provider.get_pricing("command-r-plus").await.unwrap();

    assert!(pricing.input_cost_per_1k >= 0.0);
    assert!(pricing.output_cost_per_1k >= 0.0);
    assert_eq!(pricing.currency, "USD");
}
