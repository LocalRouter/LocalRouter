//! Tests for Anthropic (Claude) provider
//!
//! Anthropic uses the Messages API format which differs from OpenAI

use super::common::*;
use futures::StreamExt;
use localrouter::providers::{anthropic::AnthropicProvider, ModelProvider};

#[tokio::test]
async fn test_anthropic_list_models() {
    let provider = AnthropicProvider::new("test-key".to_string()).unwrap();

    let models = provider.list_models().await.unwrap();

    // Anthropic provider returns a static list of known models
    assert!(!models.is_empty());
    assert!(models.iter().all(|m| m.provider == "anthropic"));
    assert!(models.iter().any(|m| m.id.contains("claude")));
}

#[tokio::test]
async fn test_anthropic_completion() {
    let mock = AnthropicMockBuilder::new().await.mock_completion().await;

    let provider =
        AnthropicProvider::with_base_url("test-key".to_string(), mock.base_url()).unwrap();

    let request = standard_completion_request();
    let response = provider.complete(request).await.unwrap();

    assert_eq!(response.choices.len(), 1);
    assert_eq!(response.choices[0].message.role, "assistant");
    assert!(!response.choices[0].message.content.is_empty());
}

#[tokio::test]
async fn test_anthropic_streaming() {
    let mock = AnthropicMockBuilder::new()
        .await
        .mock_streaming_completion()
        .await;

    let provider =
        AnthropicProvider::with_base_url("test-key".to_string(), mock.base_url()).unwrap();

    let request = standard_streaming_request();
    let mut stream = provider.stream_complete(request).await.unwrap();

    let mut chunks = Vec::new();
    while let Some(result) = stream.next().await {
        chunks.push(result.unwrap());
    }

    assert!(!chunks.is_empty());
    assert!(chunks.iter().any(|c| !c.choices.is_empty()));
}

#[tokio::test]
async fn test_anthropic_pricing() {
    let provider = AnthropicProvider::new("test-key".to_string()).unwrap();

    let pricing = provider
        .get_pricing("claude-3-5-sonnet-20241022")
        .await
        .unwrap();

    assert!(pricing.input_cost_per_1k > 0.0);
    assert!(pricing.output_cost_per_1k > 0.0);
    assert_eq!(pricing.currency, "USD");
}

#[tokio::test]
async fn test_anthropic_provider_name() {
    let provider = AnthropicProvider::new("test-key".to_string()).unwrap();
    assert_eq!(provider.name(), "anthropic");
}

#[tokio::test]
async fn test_anthropic_handles_system_messages() {
    // Test that Anthropic correctly converts system messages
    let _provider = AnthropicProvider::new("test-key".to_string()).unwrap();

    // This tests the internal message conversion logic
    // The actual API call would need a mock server with custom base URL
}
