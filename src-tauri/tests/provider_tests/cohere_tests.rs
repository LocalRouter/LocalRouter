//! Tests for Cohere provider
//!
//! Cohere uses a custom API v2 format

use super::common::*;
use futures::StreamExt;
use localrouter::providers::{cohere::CohereProvider, ModelProvider};

#[tokio::test]
#[ignore = "requires live API key"]
async fn test_cohere_list_models() {
    let provider = CohereProvider::new("test-key".to_string()).unwrap();

    let models = provider.list_models().await.unwrap();

    // Cohere provider returns a static list of known models
    assert!(!models.is_empty());
    assert!(models.iter().all(|m| m.provider == "cohere"));
    assert!(models.iter().any(|m| m.id.contains("command")));
}

#[tokio::test]
async fn test_cohere_completion() {
    let mock = CohereMockBuilder::new().await.mock_completion().await;

    let provider = CohereProvider::with_base_url("test-key".to_string(), mock.base_url()).unwrap();

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

    let provider = CohereProvider::with_base_url("test-key".to_string(), _mock.base_url()).unwrap();

    let request = standard_streaming_request();
    let mut stream = provider
        .stream_complete(request)
        .await
        .expect("Cohere streams");

    let mut text = String::new();
    let mut finish_reason = None;
    let mut usage = None;
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.expect("chunk");
        for choice in &chunk.choices {
            if let Some(content) = &choice.delta.content {
                text.push_str(content);
            }
            if choice.finish_reason.is_some() {
                finish_reason = choice.finish_reason.clone();
            }
        }
        if chunk.usage.is_some() {
            usage = chunk.usage;
        }
    }
    assert_eq!(text, "1 2 3");
    assert_eq!(finish_reason.as_deref(), Some("stop"));
    let usage = usage.expect("usage from billed_units");
    assert_eq!((usage.prompt_tokens, usage.completion_tokens), (5, 6));
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
