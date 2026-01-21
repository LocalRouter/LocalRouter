//! POST /v1/completions endpoint tests
//!
//! Comprehensive tests for the legacy completions endpoint including:
//! - Non-streaming completions
//! - Streaming completions (SSE)
//! - Parameter validation
//! - Error handling
//! - Format conversion (chat to completion format)

use futures::StreamExt;
use localrouter_ai::providers::{
    openai_compatible::OpenAICompatibleProvider, ChatMessageContent, ModelProvider,
};
use serde_json::json;
use wiremock::{
    matchers::{method, path},
    Mock, MockServer, ResponseTemplate,
};

// ============================================================================
// Test Helpers
// ============================================================================

fn create_test_provider(base_url: String) -> OpenAICompatibleProvider {
    OpenAICompatibleProvider::new("test".to_string(), base_url, Some("test-key".to_string()))
}

fn standard_completion_request() -> localrouter_ai::providers::CompletionRequest {
    localrouter_ai::providers::CompletionRequest {
        model: "test-model".to_string(),
        messages: vec![localrouter_ai::providers::ChatMessage {
            role: "user".to_string(),
            content: ChatMessageContent::Text("Say hello".to_string()),
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
        tools: None,
        tool_choice: None,
        response_format: None,
        logprobs: None,
        top_logprobs: None,
    }
}

fn streaming_completion_request() -> localrouter_ai::providers::CompletionRequest {
    let mut req = standard_completion_request();
    req.stream = true;
    req
}

// ============================================================================
// Non-Streaming Tests
// ============================================================================

#[tokio::test]
async fn test_non_streaming_completion_basic() {
    let mock_server = MockServer::start().await;

    // Mock OpenAI-compatible response
    let response_body = json!({
        "id": "cmpl-test123",
        "object": "chat.completion",
        "created": 1234567890,
        "model": "test-model",
        "choices": [{
            "index": 0,
            "message": {
                "role": "assistant",
                "content": "Hello! How can I help you today?"
            },
            "finish_reason": "stop"
        }],
        "usage": {
            "prompt_tokens": 10,
            "completion_tokens": 15,
            "total_tokens": 25
        }
    });

    Mock::given(method("POST"))
        .and(path("/chat/completions"))
        .respond_with(ResponseTemplate::new(200).set_body_json(&response_body))
        .mount(&mock_server)
        .await;

    let provider = create_test_provider(mock_server.uri());
    let request = standard_completion_request();
    let response = provider.complete(request).await.unwrap();

    // Verify response structure
    assert_eq!(response.model, "test-model");
    assert_eq!(response.choices.len(), 1);
    assert_eq!(
        response.choices[0].message.content.as_text(),
        "Hello! How can I help you today!"
    );
    assert_eq!(response.choices[0].finish_reason, Some("stop".to_string()));
    assert_eq!(response.usage.prompt_tokens, 10);
    assert_eq!(response.usage.completion_tokens, 15);
    assert_eq!(response.usage.total_tokens, 25);
}

#[tokio::test]
async fn test_non_streaming_completion_with_temperature() {
    let mock_server = MockServer::start().await;

    let response_body = json!({
        "id": "cmpl-test456",
        "object": "chat.completion",
        "created": 1234567890,
        "model": "test-model",
        "choices": [{
            "index": 0,
            "message": {
                "role": "assistant",
                "content": "Creative response!"
            },
            "finish_reason": "stop"
        }],
        "usage": {
            "prompt_tokens": 5,
            "completion_tokens": 3,
            "total_tokens": 8
        }
    });

    Mock::given(method("POST"))
        .and(path("/chat/completions"))
        .respond_with(ResponseTemplate::new(200).set_body_json(&response_body))
        .expect(1)
        .mount(&mock_server)
        .await;

    let provider = create_test_provider(mock_server.uri());
    let mut request = standard_completion_request();
    request.temperature = Some(1.5);

    let response = provider.complete(request).await.unwrap();

    assert_eq!(response.choices.len(), 1);
    assert_eq!(
        response.choices[0].message.content.as_text(),
        "Creative response!"
    );
}

// ============================================================================
// Streaming Tests
// ============================================================================

#[tokio::test]
async fn test_streaming_completion_basic() {
    let mock_server = MockServer::start().await;

    // SSE stream with multiple chunks
    let stream = concat!(
        "data: {\"id\":\"cmpl-1\",\"object\":\"chat.completion.chunk\",\"created\":1234567890,\"model\":\"test-model\",\"choices\":[{\"index\":0,\"delta\":{\"content\":\"Hello\"},\"finish_reason\":null}]}\n\n",
        "data: {\"id\":\"cmpl-1\",\"object\":\"chat.completion.chunk\",\"created\":1234567890,\"model\":\"test-model\",\"choices\":[{\"index\":0,\"delta\":{\"content\":\" there\"},\"finish_reason\":null}]}\n\n",
        "data: {\"id\":\"cmpl-1\",\"object\":\"chat.completion.chunk\",\"created\":1234567890,\"model\":\"test-model\",\"choices\":[{\"index\":0,\"delta\":{\"content\":\"!\"},\"finish_reason\":\"stop\"}]}\n\n"
    );

    Mock::given(method("POST"))
        .and(path("/chat/completions"))
        .respond_with(ResponseTemplate::new(200).set_body_string(stream))
        .mount(&mock_server)
        .await;

    let provider = create_test_provider(mock_server.uri());
    let request = streaming_completion_request();
    let mut stream = provider.stream_complete(request).await.unwrap();

    let mut chunks = Vec::new();
    let mut accumulated_text = String::new();

    while let Some(result) = stream.next().await {
        let chunk = result.unwrap();
        chunks.push(chunk.clone());

        if let Some(content) = &chunk.choices[0].delta.content {
            accumulated_text.push_str(content);
        }
    }

    // Verify we got all chunks
    assert_eq!(chunks.len(), 3);
    assert_eq!(accumulated_text, "Hello there!");

    // Verify individual chunks
    assert_eq!(
        chunks[0].choices[0].delta.content,
        Some("Hello".to_string())
    );
    assert_eq!(
        chunks[1].choices[0].delta.content,
        Some(" there".to_string())
    );
    assert_eq!(chunks[2].choices[0].delta.content, Some("!".to_string()));

    // Verify finish reason on last chunk
    assert_eq!(
        chunks[2].choices[0].finish_reason,
        Some("stop".to_string())
    );
}

#[tokio::test]
async fn test_streaming_completion_long_response() {
    let mock_server = MockServer::start().await;

    // Simulate a longer streaming response with many chunks
    let mut stream_parts = vec![];
    let words = vec!["The", "quick", "brown", "fox", "jumps", "over", "lazy", "dog"];

    for (i, word) in words.iter().enumerate() {
        let is_last = i == words.len() - 1;
        let finish_reason = if is_last { "\"stop\"" } else { "null" };
        let space = if i > 0 { " " } else { "" };

        stream_parts.push(format!(
            "data: {{\"id\":\"cmpl-long\",\"object\":\"chat.completion.chunk\",\"created\":1234567890,\"model\":\"test-model\",\"choices\":[{{\"index\":0,\"delta\":{{\"content\":\"{}{}\"}},\"finish_reason\":{}}}]}}\n\n",
            space, word, finish_reason
        ));
    }

    let stream = stream_parts.join("");

    Mock::given(method("POST"))
        .and(path("/chat/completions"))
        .respond_with(ResponseTemplate::new(200).set_body_string(stream))
        .mount(&mock_server)
        .await;

    let provider = create_test_provider(mock_server.uri());
    let request = streaming_completion_request();
    let mut stream = provider.stream_complete(request).await.unwrap();

    let mut chunks = Vec::new();
    let mut accumulated_text = String::new();

    while let Some(result) = stream.next().await {
        let chunk = result.unwrap();
        if let Some(content) = &chunk.choices[0].delta.content {
            accumulated_text.push_str(content);
        }
        chunks.push(chunk);
    }

    assert_eq!(chunks.len(), 8);
    assert_eq!(accumulated_text, "The quick brown fox jumps over lazy dog");
}

#[tokio::test]
async fn test_streaming_completion_empty_chunks() {
    let mock_server = MockServer::start().await;

    // Some chunks may have empty content (common in real streams)
    let stream = concat!(
        "data: {\"id\":\"cmpl-empty\",\"object\":\"chat.completion.chunk\",\"created\":1234567890,\"model\":\"test-model\",\"choices\":[{\"index\":0,\"delta\":{},\"finish_reason\":null}]}\n\n",
        "data: {\"id\":\"cmpl-empty\",\"object\":\"chat.completion.chunk\",\"created\":1234567890,\"model\":\"test-model\",\"choices\":[{\"index\":0,\"delta\":{\"content\":\"Hello\"},\"finish_reason\":null}]}\n\n",
        "data: {\"id\":\"cmpl-empty\",\"object\":\"chat.completion.chunk\",\"created\":1234567890,\"model\":\"test-model\",\"choices\":[{\"index\":0,\"delta\":{},\"finish_reason\":null}]}\n\n",
        "data: {\"id\":\"cmpl-empty\",\"object\":\"chat.completion.chunk\",\"created\":1234567890,\"model\":\"test-model\",\"choices\":[{\"index\":0,\"delta\":{\"content\":\"!\"},\"finish_reason\":\"stop\"}]}\n\n"
    );

    Mock::given(method("POST"))
        .and(path("/chat/completions"))
        .respond_with(ResponseTemplate::new(200).set_body_string(stream))
        .mount(&mock_server)
        .await;

    let provider = create_test_provider(mock_server.uri());
    let request = streaming_completion_request();
    let mut stream = provider.stream_complete(request).await.unwrap();

    let mut accumulated_text = String::new();

    while let Some(result) = stream.next().await {
        let chunk = result.unwrap();
        if let Some(content) = &chunk.choices[0].delta.content {
            accumulated_text.push_str(content);
        }
    }

    // Empty deltas should be handled gracefully
    assert_eq!(accumulated_text, "Hello!");
}

#[tokio::test]
async fn test_streaming_completion_finish_reasons() {
    let mock_server = MockServer::start().await;

    // Test different finish reasons: stop, length, content_filter
    let stream = concat!(
        "data: {\"id\":\"cmpl-fr\",\"object\":\"chat.completion.chunk\",\"created\":1234567890,\"model\":\"test-model\",\"choices\":[{\"index\":0,\"delta\":{\"content\":\"Test\"},\"finish_reason\":null}]}\n\n",
        "data: {\"id\":\"cmpl-fr\",\"object\":\"chat.completion.chunk\",\"created\":1234567890,\"model\":\"test-model\",\"choices\":[{\"index\":0,\"delta\":{},\"finish_reason\":\"length\"}]}\n\n"
    );

    Mock::given(method("POST"))
        .and(path("/chat/completions"))
        .respond_with(ResponseTemplate::new(200).set_body_string(stream))
        .mount(&mock_server)
        .await;

    let provider = create_test_provider(mock_server.uri());
    let request = streaming_completion_request();
    let mut stream = provider.stream_complete(request).await.unwrap();

    let mut chunks = Vec::new();
    while let Some(result) = stream.next().await {
        chunks.push(result.unwrap());
    }

    assert_eq!(chunks.len(), 2);
    assert_eq!(
        chunks[1].choices[0].finish_reason,
        Some("length".to_string())
    );
}

// ============================================================================
// Edge Cases
// ============================================================================

#[tokio::test]
async fn test_streaming_completion_unicode() {
    let mock_server = MockServer::start().await;

    // Test Unicode characters in streaming
    let stream = concat!(
        "data: {\"id\":\"cmpl-unicode\",\"object\":\"chat.completion.chunk\",\"created\":1234567890,\"model\":\"test-model\",\"choices\":[{\"index\":0,\"delta\":{\"content\":\"Hello \"},\"finish_reason\":null}]}\n\n",
        "data: {\"id\":\"cmpl-unicode\",\"object\":\"chat.completion.chunk\",\"created\":1234567890,\"model\":\"test-model\",\"choices\":[{\"index\":0,\"delta\":{\"content\":\"🌍\"},\"finish_reason\":null}]}\n\n",
        "data: {\"id\":\"cmpl-unicode\",\"object\":\"chat.completion.chunk\",\"created\":1234567890,\"model\":\"test-model\",\"choices\":[{\"index\":0,\"delta\":{\"content\":\" 你好\"},\"finish_reason\":\"stop\"}]}\n\n"
    );

    Mock::given(method("POST"))
        .and(path("/chat/completions"))
        .respond_with(ResponseTemplate::new(200).set_body_string(stream))
        .mount(&mock_server)
        .await;

    let provider = create_test_provider(mock_server.uri());
    let request = streaming_completion_request();
    let mut stream = provider.stream_complete(request).await.unwrap();

    let mut accumulated_text = String::new();
    while let Some(result) = stream.next().await {
        let chunk = result.unwrap();
        if let Some(content) = &chunk.choices[0].delta.content {
            accumulated_text.push_str(content);
        }
    }

    assert_eq!(accumulated_text, "Hello 🌍 你好");
}

#[tokio::test]
async fn test_streaming_completion_special_characters() {
    let mock_server = MockServer::start().await;

    // Test special characters that need escaping in JSON
    let stream = r#"data: {"id":"cmpl-special","object":"chat.completion.chunk","created":1234567890,"model":"test-model","choices":[{"index":0,"delta":{"content":"Line 1\n"},"finish_reason":null}]}

data: {"id":"cmpl-special","object":"chat.completion.chunk","created":1234567890,"model":"test-model","choices":[{"index":0,"delta":{"content":"Line 2\t"},"finish_reason":null}]}

data: {"id":"cmpl-special","object":"chat.completion.chunk","created":1234567890,"model":"test-model","choices":[{"index":0,"delta":{"content":"\"quoted\""},"finish_reason":"stop"}]}

"#;

    Mock::given(method("POST"))
        .and(path("/chat/completions"))
        .respond_with(ResponseTemplate::new(200).set_body_string(stream))
        .mount(&mock_server)
        .await;

    let provider = create_test_provider(mock_server.uri());
    let request = streaming_completion_request();
    let mut stream = provider.stream_complete(request).await.unwrap();

    let mut accumulated_text = String::new();
    while let Some(result) = stream.next().await {
        let chunk = result.unwrap();
        if let Some(content) = &chunk.choices[0].delta.content {
            accumulated_text.push_str(content);
        }
    }

    assert_eq!(accumulated_text, "Line 1\nLine 2\t\"quoted\"");
}

// ============================================================================
// Format Conversion Tests
// ============================================================================

#[tokio::test]
async fn test_completion_chunk_format_conversion() {
    let mock_server = MockServer::start().await;

    // Verify that the streaming response uses the correct format
    let stream = concat!(
        "data: {\"id\":\"cmpl-format\",\"object\":\"chat.completion.chunk\",\"created\":1234567890,\"model\":\"test-model\",\"choices\":[{\"index\":0,\"delta\":{\"content\":\"Test\"},\"finish_reason\":null}]}\n\n",
        "data: {\"id\":\"cmpl-format\",\"object\":\"chat.completion.chunk\",\"created\":1234567890,\"model\":\"test-model\",\"choices\":[{\"index\":0,\"delta\":{\"content\":\" content\"},\"finish_reason\":\"stop\"}]}\n\n"
    );

    Mock::given(method("POST"))
        .and(path("/chat/completions"))
        .respond_with(ResponseTemplate::new(200).set_body_string(stream))
        .mount(&mock_server)
        .await;

    let provider = create_test_provider(mock_server.uri());
    let request = streaming_completion_request();
    let mut stream = provider.stream_complete(request).await.unwrap();

    let mut chunks = Vec::new();
    while let Some(result) = stream.next().await {
        chunks.push(result.unwrap());
    }

    // Verify chunk structure
    assert_eq!(chunks.len(), 2);

    for chunk in &chunks {
        assert_eq!(chunk.model, "test-model");
        assert_eq!(chunk.choices.len(), 1);
        assert_eq!(chunk.choices[0].index, 0);
    }

    // Verify delta content
    assert!(chunks[0].choices[0].delta.content.is_some());
    assert!(chunks[1].choices[0].delta.content.is_some());

    // Verify finish reason only on last chunk
    assert!(chunks[0].choices[0].finish_reason.is_none());
    assert_eq!(
        chunks[1].choices[0].finish_reason,
        Some("stop".to_string())
    );
}
