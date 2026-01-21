//! HTTP error scenario tests
//!
//! Tests for various HTTP error conditions, status codes, and edge cases

use super::common::*;
use futures::StreamExt;
use localrouter_ai::providers::{openai_compatible::OpenAICompatibleProvider, ModelProvider};
use localrouter_ai::utils::errors::AppError;
use wiremock::{
    matchers::{method, path},
    Mock, MockServer, ResponseTemplate,
};

// ==================== HTTP STATUS CODE TESTS ====================

#[tokio::test]
async fn test_401_unauthorized() {
    let mock_server = MockServer::start().await;

    Mock::given(method("POST"))
        .and(path("/chat/completions"))
        .respond_with(ResponseTemplate::new(401).set_body_json(serde_json::json!({
            "error": {
                "message": "Invalid API key",
                "type": "invalid_request_error",
                "code": "invalid_api_key"
            }
        })))
        .mount(&mock_server)
        .await;

    let provider = OpenAICompatibleProvider::new(
        "test".to_string(),
        mock_server.uri(),
        Some("invalid-key".to_string()),
    );

    let request = standard_completion_request();
    let result = provider.complete(request).await;

    assert!(result.is_err());
    match result.unwrap_err() {
        AppError::Unauthorized => {} // Expected
        other => panic!("Expected Unauthorized error, got: {:?}", other),
    }
}

#[tokio::test]
async fn test_429_rate_limit() {
    let mock_server = MockServer::start().await;

    Mock::given(method("POST"))
        .and(path("/chat/completions"))
        .respond_with(
            ResponseTemplate::new(429)
                .set_body_json(serde_json::json!({
                    "error": {
                        "message": "Rate limit exceeded",
                        "type": "rate_limit_error"
                    }
                }))
                .insert_header("retry-after", "60"),
        )
        .mount(&mock_server)
        .await;

    let provider = OpenAICompatibleProvider::new(
        "test".to_string(),
        mock_server.uri(),
        Some("test-key".to_string()),
    );

    let request = standard_completion_request();
    let result = provider.complete(request).await;

    assert!(result.is_err());
    match result.unwrap_err() {
        AppError::RateLimitExceeded => {} // Expected
        other => panic!("Expected RateLimitExceeded error, got: {:?}", other),
    }
}

#[tokio::test]
async fn test_500_internal_server_error() {
    let mock_server = MockServer::start().await;

    Mock::given(method("POST"))
        .and(path("/chat/completions"))
        .respond_with(ResponseTemplate::new(500).set_body_string("Internal Server Error"))
        .mount(&mock_server)
        .await;

    let provider = OpenAICompatibleProvider::new(
        "test".to_string(),
        mock_server.uri(),
        Some("test-key".to_string()),
    );

    let request = standard_completion_request();
    let result = provider.complete(request).await;

    assert!(result.is_err());
    let err = result.unwrap_err();
    assert!(
        matches!(err, AppError::Provider(_)),
        "Expected Provider error for 500"
    );
}

#[tokio::test]
async fn test_503_service_unavailable() {
    let mock_server = MockServer::start().await;

    Mock::given(method("POST"))
        .and(path("/chat/completions"))
        .respond_with(ResponseTemplate::new(503).set_body_string("Service Unavailable"))
        .mount(&mock_server)
        .await;

    let provider = OpenAICompatibleProvider::new(
        "test".to_string(),
        mock_server.uri(),
        Some("test-key".to_string()),
    );

    let request = standard_completion_request();
    let result = provider.complete(request).await;

    assert!(result.is_err());
}

#[tokio::test]
async fn test_400_bad_request() {
    let mock_server = MockServer::start().await;

    Mock::given(method("POST"))
        .and(path("/chat/completions"))
        .respond_with(ResponseTemplate::new(400).set_body_json(serde_json::json!({
            "error": {
                "message": "Invalid request: 'model' is required",
                "type": "invalid_request_error",
                "param": "model"
            }
        })))
        .mount(&mock_server)
        .await;

    let provider = OpenAICompatibleProvider::new(
        "test".to_string(),
        mock_server.uri(),
        Some("test-key".to_string()),
    );

    let request = standard_completion_request();
    let result = provider.complete(request).await;

    assert!(result.is_err());
}

#[tokio::test]
async fn test_404_not_found() {
    let mock_server = MockServer::start().await;

    Mock::given(method("POST"))
        .and(path("/chat/completions"))
        .respond_with(ResponseTemplate::new(404).set_body_string("Not Found"))
        .mount(&mock_server)
        .await;

    let provider = OpenAICompatibleProvider::new(
        "test".to_string(),
        mock_server.uri(),
        Some("test-key".to_string()),
    );

    let request = standard_completion_request();
    let result = provider.complete(request).await;

    assert!(result.is_err());
}

// ==================== MALFORMED RESPONSE TESTS ====================

#[tokio::test]
async fn test_malformed_json_response() {
    let mock_server = MockServer::start().await;

    Mock::given(method("POST"))
        .and(path("/chat/completions"))
        .respond_with(ResponseTemplate::new(200).set_body_string("{invalid json"))
        .mount(&mock_server)
        .await;

    let provider = OpenAICompatibleProvider::new(
        "test".to_string(),
        mock_server.uri(),
        Some("test-key".to_string()),
    );

    let request = standard_completion_request();
    let result = provider.complete(request).await;

    assert!(result.is_err());
    let err = result.unwrap_err();
    assert!(
        matches!(err, AppError::Provider(_)),
        "Expected Provider error for malformed JSON"
    );
}

#[tokio::test]
async fn test_empty_response() {
    let mock_server = MockServer::start().await;

    Mock::given(method("POST"))
        .and(path("/chat/completions"))
        .respond_with(ResponseTemplate::new(200).set_body_string(""))
        .mount(&mock_server)
        .await;

    let provider = OpenAICompatibleProvider::new(
        "test".to_string(),
        mock_server.uri(),
        Some("test-key".to_string()),
    );

    let request = standard_completion_request();
    let result = provider.complete(request).await;

    assert!(result.is_err());
}

#[tokio::test]
async fn test_missing_required_fields() {
    let mock_server = MockServer::start().await;

    // Response missing 'choices' field
    Mock::given(method("POST"))
        .and(path("/chat/completions"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "id": "test-id",
            "object": "chat.completion",
            "created": 1234567890,
            "model": "test-model"
            // Missing 'choices' and 'usage'
        })))
        .mount(&mock_server)
        .await;

    let provider = OpenAICompatibleProvider::new(
        "test".to_string(),
        mock_server.uri(),
        Some("test-key".to_string()),
    );

    let request = standard_completion_request();
    let result = provider.complete(request).await;

    assert!(result.is_err());
}

#[tokio::test]
async fn test_empty_choices_array() {
    let mock_server = MockServer::start().await;

    Mock::given(method("POST"))
        .and(path("/chat/completions"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "id": "test-id",
            "object": "chat.completion",
            "created": 1234567890,
            "model": "test-model",
            "choices": [], // Empty array
            "usage": {
                "prompt_tokens": 10,
                "completion_tokens": 0,
                "total_tokens": 10
            }
        })))
        .mount(&mock_server)
        .await;

    let provider = OpenAICompatibleProvider::new(
        "test".to_string(),
        mock_server.uri(),
        Some("test-key".to_string()),
    );

    let request = standard_completion_request();
    let result = provider.complete(request).await;

    // Empty choices should return an error
    // An empty choices array means the API didn't generate any response,
    // which is an error condition that should be surfaced to the caller
    assert!(
        result.is_err(),
        "Provider should return error when choices array is empty"
    );

    let err = result.unwrap_err();
    assert!(
        matches!(err, AppError::Provider(_)),
        "Expected Provider error for empty choices"
    );
}

// ==================== NETWORK ERROR TESTS ====================

#[tokio::test]
async fn test_connection_refused() {
    // Use a port that's not listening
    let provider = OpenAICompatibleProvider::new(
        "test".to_string(),
        "http://localhost:9999".to_string(),
        Some("test-key".to_string()),
    );

    let request = standard_completion_request();
    let result = provider.complete(request).await;

    assert!(result.is_err());
    let err = result.unwrap_err();
    assert!(matches!(err, AppError::Provider(_)));
}

#[tokio::test]
async fn test_invalid_url() {
    let provider = OpenAICompatibleProvider::new(
        "test".to_string(),
        "not-a-valid-url".to_string(),
        Some("test-key".to_string()),
    );

    let request = standard_completion_request();
    let result = provider.complete(request).await;

    assert!(result.is_err());
}

// ==================== TIMEOUT TESTS ====================

// Note: Timeout tests are tricky with wiremock, would need real async delay testing

// ==================== STREAMING ERROR TESTS ====================

#[tokio::test]
async fn test_streaming_connection_drop() {
    let mock_server = MockServer::start().await;

    // Send partial stream then close connection
    let partial_stream = "data: {\"id\":\"test\",\"object\":\"chat.completion.chunk\",\"created\":1234567890,\"model\":\"test\",\"choices\":[{\"index\":0,\"delta\":{\"content\":\"Hello\"},\"finish_reason\":null}]}\n\n";

    Mock::given(method("POST"))
        .and(path("/chat/completions"))
        .respond_with(ResponseTemplate::new(200).set_body_string(partial_stream))
        .mount(&mock_server)
        .await;

    let provider = OpenAICompatibleProvider::new(
        "test".to_string(),
        mock_server.uri(),
        Some("test-key".to_string()),
    );

    let request = standard_streaming_request();
    let mut stream = provider.stream_complete(request).await.unwrap();

    let mut chunk_count = 0;
    while let Some(result) = stream.next().await {
        // Should get at least one chunk
        result.unwrap();
        chunk_count += 1;
    }

    assert!(chunk_count > 0, "Should receive at least one chunk");
    // Stream ends without finish_reason due to connection drop
}

#[tokio::test]
async fn test_streaming_invalid_json_chunk() {
    let mock_server = MockServer::start().await;

    let stream_with_invalid = "data: {\"id\":\"test\",\"object\":\"chat.completion.chunk\",\"created\":1234567890,\"model\":\"test\",\"choices\":[{\"index\":0,\"delta\":{\"content\":\"Hello\"},\"finish_reason\":null}]}\n\ndata: {invalid json}\n\n";

    Mock::given(method("POST"))
        .and(path("/chat/completions"))
        .respond_with(ResponseTemplate::new(200).set_body_string(stream_with_invalid))
        .mount(&mock_server)
        .await;

    let provider = OpenAICompatibleProvider::new(
        "test".to_string(),
        mock_server.uri(),
        Some("test-key".to_string()),
    );

    let request = standard_streaming_request();
    let mut stream = provider.stream_complete(request).await.unwrap();

    let mut chunks = Vec::new();
    while let Some(result) = stream.next().await {
        chunks.push(result);
    }

    // First chunk should succeed
    assert!(chunks[0].is_ok());
    // Second chunk should error
    assert!(chunks.len() >= 2);
    assert!(chunks[1].is_err());
}

#[tokio::test]
async fn test_streaming_empty_stream() {
    let mock_server = MockServer::start().await;

    Mock::given(method("POST"))
        .and(path("/chat/completions"))
        .respond_with(ResponseTemplate::new(200).set_body_string(""))
        .mount(&mock_server)
        .await;

    let provider = OpenAICompatibleProvider::new(
        "test".to_string(),
        mock_server.uri(),
        Some("test-key".to_string()),
    );

    let request = standard_streaming_request();
    let mut stream = provider.stream_complete(request).await.unwrap();

    let mut chunk_count = 0;
    while let Some(_result) = stream.next().await {
        chunk_count += 1;
    }

    assert_eq!(chunk_count, 0, "Empty stream should yield no chunks");
}

#[tokio::test]
async fn test_streaming_malformed_sse() {
    let mock_server = MockServer::start().await;

    // SSE without "data: " prefix
    let malformed_sse = "{\"id\":\"test\"}\n\n";

    Mock::given(method("POST"))
        .and(path("/chat/completions"))
        .respond_with(ResponseTemplate::new(200).set_body_string(malformed_sse))
        .mount(&mock_server)
        .await;

    let provider = OpenAICompatibleProvider::new(
        "test".to_string(),
        mock_server.uri(),
        Some("test-key".to_string()),
    );

    let request = standard_streaming_request();
    let mut stream = provider.stream_complete(request).await.unwrap();

    let mut chunk_count = 0;
    while let Some(_result) = stream.next().await {
        chunk_count += 1;
    }

    // Should not yield any chunks since SSE format is wrong
    assert_eq!(chunk_count, 0);
}

#[tokio::test]
async fn test_streaming_only_done_marker() {
    let mock_server = MockServer::start().await;

    let only_done = "data: [DONE]\n\n";

    Mock::given(method("POST"))
        .and(path("/chat/completions"))
        .respond_with(ResponseTemplate::new(200).set_body_string(only_done))
        .mount(&mock_server)
        .await;

    let provider = OpenAICompatibleProvider::new(
        "test".to_string(),
        mock_server.uri(),
        Some("test-key".to_string()),
    );

    let request = standard_streaming_request();
    let mut stream = provider.stream_complete(request).await.unwrap();

    let mut chunk_count = 0;
    while let Some(_result) = stream.next().await {
        chunk_count += 1;
    }

    // [DONE] marker should be skipped
    assert_eq!(chunk_count, 0);
}

// ==================== CONTENT VALIDATION TESTS ====================

#[tokio::test]
async fn test_unicode_content() {
    let mock_server = MockServer::start().await;

    Mock::given(method("POST"))
        .and(path("/chat/completions"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "id": "test-id",
            "object": "chat.completion",
            "created": 1234567890,
            "model": "test-model",
            "choices": [{
                "index": 0,
                "message": {
                    "role": "assistant",
                    "content": "Hello 世界! 🌍 Привет мир"
                },
                "finish_reason": "stop"
            }],
            "usage": {
                "prompt_tokens": 10,
                "completion_tokens": 9,
                "total_tokens": 19
            }
        })))
        .mount(&mock_server)
        .await;

    let provider = OpenAICompatibleProvider::new(
        "test".to_string(),
        mock_server.uri(),
        Some("test-key".to_string()),
    );

    let request = standard_completion_request();
    let response = provider.complete(request).await.unwrap();

    assert_eq!(
        response.choices[0].message.content.as_text(),
        "Hello 世界! 🌍 Привет мир"
    );
}

#[tokio::test]
async fn test_very_long_content() {
    let mock_server = MockServer::start().await;

    let long_content = "x".repeat(100_000);

    Mock::given(method("POST"))
        .and(path("/chat/completions"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "id": "test-id",
            "object": "chat.completion",
            "created": 1234567890,
            "model": "test-model",
            "choices": [{
                "index": 0,
                "message": {
                    "role": "assistant",
                    "content": long_content
                },
                "finish_reason": "length"
            }],
            "usage": {
                "prompt_tokens": 10,
                "completion_tokens": 100000,
                "total_tokens": 100010
            }
        })))
        .mount(&mock_server)
        .await;

    let provider = OpenAICompatibleProvider::new(
        "test".to_string(),
        mock_server.uri(),
        Some("test-key".to_string()),
    );

    let request = standard_completion_request();
    let response = provider.complete(request).await.unwrap();

    assert_eq!(response.choices[0].message.content.as_text().len(), 100_000);
    assert_eq!(
        response.choices[0].finish_reason,
        Some("length".to_string())
    );
}
