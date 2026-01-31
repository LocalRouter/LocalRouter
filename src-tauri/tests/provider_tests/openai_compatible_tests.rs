//! Tests for OpenAI-compatible providers
//!
//! Tests providers that use the OpenAI API format:
//! - OpenAI
//! - OpenRouter
//! - Groq
//! - Mistral
//! - TogetherAI
//! - Perplexity
//! - DeepInfra
//! - Cerebras
//! - xAI
//! - LM Studio

use super::common::*;
use futures::StreamExt;
use localrouter::providers::{
    groq::GroqProvider, lmstudio::LMStudioProvider, openai_compatible::OpenAICompatibleProvider,
    openrouter::OpenRouterProvider, ModelProvider,
};

// ==================== OPENAI TESTS ====================

#[tokio::test]
async fn test_openai_health_check() {
    let mock = OpenAICompatibleMockBuilder::new()
        .await
        .mock_list_models()
        .await;

    // Note: OpenAI provider uses the real API base URL, so we can't easily test it with mock server
    // This test is a placeholder - in practice, we'd need to modify OpenAI provider to accept custom base URL
    // Or use integration tests with real API keys
}

#[tokio::test]
async fn test_openai_list_models() {
    // Similar limitation as above - OpenAI provider uses hardcoded base URL
    // TODO: Refactor OpenAI provider to accept base_url for testing
}

// ==================== OPENAI COMPATIBLE TESTS ====================

#[tokio::test]
async fn test_openai_compatible_health_check() {
    let mock = OpenAICompatibleMockBuilder::new()
        .await
        .mock_list_models()
        .await;

    let provider = OpenAICompatibleProvider::new(
        "test-provider".to_string(),
        mock.base_url(),
        Some("test-key".to_string()),
    );

    let health = provider.health_check().await;

    assert_eq!(
        health.status,
        localrouter::providers::HealthStatus::Healthy
    );
    assert!(health.latency_ms.is_some());
    assert!(health.error_message.is_none());
}

#[tokio::test]
async fn test_openai_compatible_list_models() {
    let mock = OpenAICompatibleMockBuilder::new()
        .await
        .mock_list_models()
        .await;

    let provider = OpenAICompatibleProvider::new(
        "test-provider".to_string(),
        mock.base_url(),
        Some("test-key".to_string()),
    );

    let models = provider.list_models().await.unwrap();

    assert_eq!(models.len(), 2);
    assert_eq!(models[0].id, "test-model");
    assert_eq!(models[0].provider, "test-provider");
    assert!(models[0].supports_streaming);
}

#[tokio::test]
async fn test_openai_compatible_completion() {
    let mock = OpenAICompatibleMockBuilder::new()
        .await
        .mock_completion()
        .await;

    let provider = OpenAICompatibleProvider::new(
        "test-provider".to_string(),
        mock.base_url(),
        Some("test-key".to_string()),
    );

    let request = standard_completion_request();
    let response = provider.complete(request).await.unwrap();

    assert_valid_completion(&response);
    assert_eq!(response.model, "test-model");
    assert_eq!(
        response.choices[0].message.content.as_text(),
        "Hello! How can I help you today?"
    );
}

#[tokio::test]
async fn test_openai_compatible_streaming() {
    let mock = OpenAICompatibleMockBuilder::new()
        .await
        .mock_streaming_completion()
        .await;

    let provider = OpenAICompatibleProvider::new(
        "test-provider".to_string(),
        mock.base_url(),
        Some("test-key".to_string()),
    );

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

    // Verify we got content
    let full_content = content_parts.join("");
    assert!(!full_content.is_empty(), "Should receive some content");

    // Verify last chunk has finish_reason
    let last_chunk = &chunks[chunks.len() - 1];
    assert!(
        last_chunk.choices[0].finish_reason.is_some(),
        "Last chunk should have finish_reason"
    );
}

// ==================== GROQ TESTS ====================

#[tokio::test]
async fn test_groq_completion() {
    let mock = OpenAICompatibleMockBuilder::new()
        .await
        .mock_completion()
        .await;

    let provider = GroqProvider::with_base_url("test-key".to_string(), mock.base_url()).unwrap();

    let request = standard_completion_request();
    let response = provider.complete(request).await.unwrap();

    assert_eq!(response.choices.len(), 1);
    assert_eq!(response.choices[0].message.role, "assistant");
    assert!(!response.choices[0].message.content.is_empty());
}

// ==================== OPENROUTER TESTS ====================

#[tokio::test]
async fn test_openrouter_completion() {
    let mock = OpenAICompatibleMockBuilder::new()
        .await
        .mock_completion()
        .await;

    let provider = OpenRouterProvider::with_base_url("test-key".to_string(), mock.base_url());

    let request = standard_completion_request();
    let response = provider.complete(request).await.unwrap();

    assert_eq!(response.choices.len(), 1);
    assert_eq!(response.choices[0].message.role, "assistant");
    assert!(!response.choices[0].message.content.is_empty());
}

// ==================== LM STUDIO TESTS ====================

#[tokio::test]
async fn test_lmstudio_health_check() {
    let mock = OpenAICompatibleMockBuilder::new()
        .await
        .mock_list_models()
        .await;

    let provider = LMStudioProvider::with_base_url(mock.base_url());

    let health = provider.health_check().await;

    assert_eq!(
        health.status,
        localrouter::providers::HealthStatus::Healthy
    );
}

#[tokio::test]
async fn test_lmstudio_list_models() {
    let mock = OpenAICompatibleMockBuilder::new()
        .await
        .mock_list_models()
        .await;

    let provider = LMStudioProvider::with_base_url(mock.base_url());

    let models = provider.list_models().await.unwrap();

    assert!(!models.is_empty());
    assert_eq!(models[0].provider, "lmstudio");
}

#[tokio::test]
async fn test_lmstudio_completion() {
    let mock = OpenAICompatibleMockBuilder::new()
        .await
        .mock_completion()
        .await;

    let provider = LMStudioProvider::with_base_url(mock.base_url());

    let request = standard_completion_request();
    let response = provider.complete(request).await.unwrap();

    assert_valid_completion(&response);
}

#[tokio::test]
async fn test_lmstudio_streaming() {
    let mock = OpenAICompatibleMockBuilder::new()
        .await
        .mock_streaming_completion()
        .await;

    let provider = LMStudioProvider::with_base_url(mock.base_url());

    let request = standard_streaming_request();
    let mut stream = provider.stream_complete(request).await.unwrap();

    let mut chunk_count = 0;
    while let Some(result) = stream.next().await {
        let _chunk = result.unwrap();
        chunk_count += 1;
    }

    assert!(chunk_count > 0, "Should receive at least one chunk");
}

// ==================== GENERIC OPENAI-COMPATIBLE PROVIDER TESTS ====================

/// Test that all OpenAI-compatible providers handle the same test cases correctly
/// This is a pattern that could be extended to other providers as they're refactored
#[tokio::test]
async fn test_all_openai_compatible_providers_handle_errors_consistently() {
    // Test error handling for providers using OpenAI format
    let mock = OpenAICompatibleMockBuilder::new().await;
    let base_url = mock.base_url();

    let provider =
        OpenAICompatibleProvider::new("test".to_string(), base_url, Some("test-key".to_string()));

    // Test with no mocks - should fail gracefully
    let request = standard_completion_request();
    let result = provider.complete(request).await;

    assert!(
        result.is_err(),
        "Should error when API returns unexpected response"
    );
}
