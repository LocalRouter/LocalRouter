//! Tests for Google Gemini provider
//!
//! Gemini uses the Google Generative Language API format

use super::common::*;
use futures::StreamExt;
use localrouter_ai::providers::{gemini::GeminiProvider, ModelProvider};

#[tokio::test]
async fn test_gemini_with_custom_base_url() {
    let mock = GeminiMockBuilder::new().await.mock_list_models().await;

    let provider = GeminiProvider::with_base_url("test-key".to_string(), mock.base_url());

    let models = provider.list_models().await.unwrap();

    assert!(!models.is_empty());
    assert_eq!(models[0].provider, "gemini");
}

#[tokio::test]
async fn test_gemini_completion() {
    let mock = GeminiMockBuilder::new().await.mock_completion().await;

    let provider = GeminiProvider::with_base_url("test-key".to_string(), mock.base_url());

    let request = standard_completion_request();
    let response = provider.complete(request).await.unwrap();

    assert_valid_completion(&response);
    assert_eq!(
        response.choices[0].message.content.as_text(),
        "Hello! How can I help you today?"
    );
}

#[tokio::test]
async fn test_gemini_streaming() {
    let mock = GeminiMockBuilder::new()
        .await
        .mock_streaming_completion()
        .await;

    let provider = GeminiProvider::with_base_url("test-key".to_string(), mock.base_url());

    let request = standard_streaming_request();
    let mut stream = provider.stream_complete(request).await.unwrap();

    let mut chunks = Vec::new();
    let mut content_parts = Vec::new();

    while let Some(result) = stream.next().await {
        let chunk = result.unwrap();
        assert_eq!(chunk.object, "chat.completion.chunk");
        assert!(!chunk.choices.is_empty());

        if let Some(content) = &chunk.choices[0].delta.content {
            content_parts.push(content.clone());
        }

        chunks.push(chunk);
    }

    assert!(!chunks.is_empty(), "Should receive at least one chunk");

    let full_content = content_parts.join("");
    assert!(
        full_content.contains("1"),
        "Should contain content, got: {}",
        full_content
    );
}

#[tokio::test]
async fn test_gemini_pricing_pro() {
    let provider = GeminiProvider::new("test-key".to_string());

    let pricing = provider.get_pricing("gemini-1.5-pro").await.unwrap();

    assert_eq!(pricing.input_cost_per_1k, 0.00125);
    assert_eq!(pricing.output_cost_per_1k, 0.005);
    assert_eq!(pricing.currency, "USD");
}

#[tokio::test]
async fn test_gemini_pricing_flash() {
    let provider = GeminiProvider::new("test-key".to_string());

    let pricing = provider.get_pricing("gemini-1.5-flash").await.unwrap();

    assert_eq!(pricing.input_cost_per_1k, 0.000075);
    assert_eq!(pricing.output_cost_per_1k, 0.0003);
}

#[tokio::test]
async fn test_gemini_pricing_2_0_flash() {
    let provider = GeminiProvider::new("test-key".to_string());

    let pricing = provider.get_pricing("gemini-2.0-flash").await.unwrap();

    // Free during preview
    assert_eq!(pricing.input_cost_per_1k, 0.0);
    assert_eq!(pricing.output_cost_per_1k, 0.0);
}

#[tokio::test]
async fn test_gemini_provider_name() {
    let provider = GeminiProvider::new("test-key".to_string());
    assert_eq!(provider.name(), "gemini");
}

#[tokio::test]
async fn test_gemini_handles_system_messages() {
    // Test that Gemini correctly converts system messages (prepends to first user message)
    let _provider = GeminiProvider::new("test-key".to_string());

    // System messages should be prepended to first user message
    // This is tested in the unit tests within gemini.rs
}
