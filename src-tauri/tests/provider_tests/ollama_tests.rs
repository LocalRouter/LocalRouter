//! Tests for Ollama provider
//!
//! Ollama has a custom API format that sends cumulative content in streaming mode

use super::common::*;
use futures::StreamExt;
use localrouter::providers::{ollama::OllamaProvider, ModelProvider};

#[tokio::test]
async fn test_ollama_health_check() {
    let mock = OllamaMockBuilder::new().await.mock_list_models().await;

    let provider = OllamaProvider::with_base_url(mock.base_url());

    let health = provider.health_check().await;

    assert_eq!(health.status, localrouter::providers::HealthStatus::Healthy);
    assert!(health.latency_ms.is_some());
    assert!(health.last_checked > chrono::Utc::now() - chrono::Duration::seconds(5));
    assert!(health.error_message.is_none());
}

#[tokio::test]
async fn test_ollama_list_models() {
    let mock = OllamaMockBuilder::new().await.mock_list_models().await;

    let provider = OllamaProvider::with_base_url(mock.base_url());

    let models = provider.list_models().await.unwrap();

    assert!(!models.is_empty());
    assert_eq!(models[0].provider, "ollama");
    assert_eq!(models[0].id, "llama3.3:latest");
    assert_eq!(models[0].name, "llama3.3:latest");
    assert!(models[0].supports_streaming);
}

#[tokio::test]
async fn test_ollama_completion() {
    let mock = OllamaMockBuilder::new().await.mock_completion().await;

    let provider = OllamaProvider::with_base_url(mock.base_url());

    let request = standard_completion_request();
    let response = provider.complete(request).await.unwrap();

    assert_valid_completion(&response);
    assert_eq!(
        response.choices[0].message.content.as_text(),
        "Hello! How can I help you today?"
    );
    assert_eq!(response.choices[0].message.role, "assistant");
}

#[tokio::test]
async fn test_ollama_streaming() {
    let mock = OllamaMockBuilder::new()
        .await
        .mock_streaming_completion()
        .await;

    let provider = OllamaProvider::with_base_url(mock.base_url());

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

    // Ollama sends cumulative content, but our provider converts to deltas
    // So we should get: "1", " 2", " 3"
    let full_content = content_parts.join("");
    assert!(
        full_content.contains("1"),
        "Should contain the number 1, got: {}",
        full_content
    );
    assert!(
        full_content.contains("2"),
        "Should contain the number 2, got: {}",
        full_content
    );
    assert!(
        full_content.contains("3"),
        "Should contain the number 3, got: {}",
        full_content
    );
}

#[tokio::test]
async fn test_ollama_pricing_is_free() {
    let provider = OllamaProvider::new();

    let pricing = provider.get_pricing("any-model").await.unwrap();

    assert_eq!(pricing.input_cost_per_1k, 0.0);
    assert_eq!(pricing.output_cost_per_1k, 0.0);
    assert_eq!(pricing.currency, "USD");
}

#[tokio::test]
async fn test_ollama_provider_name() {
    let provider = OllamaProvider::new();
    assert_eq!(provider.name(), "ollama");
}
