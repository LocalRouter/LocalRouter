//! Tests for Cohere provider
//!
//! Cohere uses a custom API v2 format

use super::common::*;
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

    let provider = CohereProvider::with_base_url(
        "test-key".to_string(),
        mock.base_url(),
    )
    .unwrap();

    let request = standard_completion_request();
    let response = provider.complete(request).await.unwrap();

    assert_eq!(response.choices.len(), 1);
    assert_eq!(response.choices[0].message.role, "assistant");
    assert!(!response.choices[0].message.content.is_empty());
}

#[tokio::test]
async fn test_cohere_streaming() {
    let _mock = CohereMockBuilder::new()
        .await
        .mock_streaming_completion()
        .await;

    let provider = CohereProvider::with_base_url(
        "test-key".to_string(),
        _mock.base_url(),
    )
    .unwrap();

    let request = standard_streaming_request();
    let result = provider.stream_complete(request).await;

    // Cohere streaming is not yet implemented
    assert!(result.is_err(), "Cohere streaming should return an error as it's not implemented");
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
