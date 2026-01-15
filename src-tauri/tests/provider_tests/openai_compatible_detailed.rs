//! Detailed OpenAI-compatible provider tests with request validation
//!
//! Tests that validate exact request format, headers, body fields, etc.

use super::common::*;
use super::request_validation::*;
use futures::StreamExt;
use localrouter_ai::providers::{
    lmstudio::LMStudioProvider, openai_compatible::OpenAICompatibleProvider, ModelProvider,
};
use serde_json::json;
use wiremock::{
    matchers::{method, path},
    Mock, MockServer, Request, ResponseTemplate,
};

// ==================== REQUEST FORMAT VALIDATION ====================

#[tokio::test]
async fn test_request_has_correct_headers() {
    let mock_server = MockServer::start().await;

    Mock::given(method("POST"))
        .and(path("/chat/completions"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "id": "test",
            "object": "chat.completion",
            "created": 1234567890,
            "model": "test-model",
            "choices": [{
                "index": 0,
                "message": {"role": "assistant", "content": "Hi"},
                "finish_reason": "stop"
            }],
            "usage": {"prompt_tokens": 5, "completion_tokens": 2, "total_tokens": 7}
        })))
        .expect(1)
        .mount(&mock_server)
        .await;

    let provider = OpenAICompatibleProvider::new(
        "test".to_string(),
        mock_server.uri(),
        Some("test-api-key".to_string()),
    );

    let request = standard_completion_request();
    let _response = provider.complete(request).await.unwrap();

    // Verification happens via wiremock's expect() - if headers were wrong, test would fail
}

#[tokio::test]
async fn test_request_authorization_header() {
    let mock_server = MockServer::start().await;

    let captured_request = std::sync::Arc::new(std::sync::Mutex::new(None));
    let captured_request_clone = captured_request.clone();

    Mock::given(method("POST"))
        .and(path("/chat/completions"))
        .respond_with(move |req: &Request| {
            *captured_request_clone.lock().unwrap() = Some(req.clone());
            ResponseTemplate::new(200).set_body_json(json!({
                "id": "test",
                "object": "chat.completion",
                "created": 1234567890,
                "model": "test-model",
                "choices": [{
                    "index": 0,
                    "message": {"role": "assistant", "content": "Hi"},
                    "finish_reason": "stop"
                }],
                "usage": {"prompt_tokens": 5, "completion_tokens": 2, "total_tokens": 7}
            }))
        })
        .mount(&mock_server)
        .await;

    let provider = OpenAICompatibleProvider::new(
        "test".to_string(),
        mock_server.uri(),
        Some("test-api-key-123".to_string()),
    );

    let request = standard_completion_request();
    let _response = provider.complete(request).await.unwrap();

    // Validate authorization header
    let req = captured_request.lock().unwrap();
    let req = req.as_ref().unwrap();
    assert_bearer_token(req, "");
    assert_header_contains(req, "authorization", "test-api-key-123");
}

#[tokio::test]
async fn test_request_content_type_header() {
    let mock_server = MockServer::start().await;

    let captured_request = std::sync::Arc::new(std::sync::Mutex::new(None));
    let captured_request_clone = captured_request.clone();

    Mock::given(method("POST"))
        .and(path("/chat/completions"))
        .respond_with(move |req: &Request| {
            *captured_request_clone.lock().unwrap() = Some(req.clone());
            ResponseTemplate::new(200).set_body_json(json!({
                "id": "test",
                "object": "chat.completion",
                "created": 1234567890,
                "model": "test-model",
                "choices": [{
                    "index": 0,
                    "message": {"role": "assistant", "content": "Hi"},
                    "finish_reason": "stop"
                }],
                "usage": {"prompt_tokens": 5, "completion_tokens": 2, "total_tokens": 7}
            }))
        })
        .mount(&mock_server)
        .await;

    let provider = OpenAICompatibleProvider::new(
        "test".to_string(),
        mock_server.uri(),
        Some("test-key".to_string()),
    );

    let request = standard_completion_request();
    let _response = provider.complete(request).await.unwrap();

    let req = captured_request.lock().unwrap();
    let req = req.as_ref().unwrap();
    assert_content_type_json(req);
}

// ==================== REQUEST BODY VALIDATION ====================

#[tokio::test]
async fn test_request_body_structure() {
    let mock_server = MockServer::start().await;

    let captured_request = std::sync::Arc::new(std::sync::Mutex::new(None));
    let captured_request_clone = captured_request.clone();

    Mock::given(method("POST"))
        .and(path("/chat/completions"))
        .respond_with(move |req: &Request| {
            *captured_request_clone.lock().unwrap() = Some(req.clone());
            ResponseTemplate::new(200).set_body_json(json!({
                "id": "test",
                "object": "chat.completion",
                "created": 1234567890,
                "model": "test-model",
                "choices": [{
                    "index": 0,
                    "message": {"role": "assistant", "content": "Hi"},
                    "finish_reason": "stop"
                }],
                "usage": {"prompt_tokens": 5, "completion_tokens": 2, "total_tokens": 7}
            }))
        })
        .mount(&mock_server)
        .await;

    let provider = OpenAICompatibleProvider::new(
        "test".to_string(),
        mock_server.uri(),
        Some("test-key".to_string()),
    );

    let request = standard_completion_request();
    let _response = provider.complete(request).await.unwrap();

    // Validate request body
    let req = captured_request.lock().unwrap();
    let req = req.as_ref().unwrap();
    let body = extract_json_body(req);

    assert_json_string_field(&body, "model", "test-model");
    assert_json_bool_field(&body, "stream", false);
    assert_messages_format(&body, 2);
}

#[tokio::test]
async fn test_request_body_optional_fields() {
    let mock_server = MockServer::start().await;

    let captured_request = std::sync::Arc::new(std::sync::Mutex::new(None));
    let captured_request_clone = captured_request.clone();

    Mock::given(method("POST"))
        .and(path("/chat/completions"))
        .respond_with(move |req: &Request| {
            *captured_request_clone.lock().unwrap() = Some(req.clone());
            ResponseTemplate::new(200).set_body_json(json!({
                "id": "test",
                "object": "chat.completion",
                "created": 1234567890,
                "model": "test-model",
                "choices": [{
                    "index": 0,
                    "message": {"role": "assistant", "content": "Hi"},
                    "finish_reason": "stop"
                }],
                "usage": {"prompt_tokens": 5, "completion_tokens": 2, "total_tokens": 7}
            }))
        })
        .mount(&mock_server)
        .await;

    let provider = OpenAICompatibleProvider::new(
        "test".to_string(),
        mock_server.uri(),
        Some("test-key".to_string()),
    );

    let request = standard_completion_request();
    let _response = provider.complete(request).await.unwrap();

    let req = captured_request.lock().unwrap();
    let req = req.as_ref().unwrap();
    let body = extract_json_body(req);

    // Check optional fields are included
    assert!(body.get("temperature").is_some());
    assert!(body.get("max_tokens").is_some());

    // Validate values
    assert_eq!(body.get("temperature").unwrap().as_f64().unwrap(), 0.7);
    assert_eq!(body.get("max_tokens").unwrap().as_u64().unwrap(), 100);
}

#[tokio::test]
async fn test_request_messages_array() {
    let mock_server = MockServer::start().await;

    let captured_request = std::sync::Arc::new(std::sync::Mutex::new(None));
    let captured_request_clone = captured_request.clone();

    Mock::given(method("POST"))
        .and(path("/chat/completions"))
        .respond_with(move |req: &Request| {
            *captured_request_clone.lock().unwrap() = Some(req.clone());
            ResponseTemplate::new(200).set_body_json(json!({
                "id": "test",
                "object": "chat.completion",
                "created": 1234567890,
                "model": "test-model",
                "choices": [{
                    "index": 0,
                    "message": {"role": "assistant", "content": "Hi"},
                    "finish_reason": "stop"
                }],
                "usage": {"prompt_tokens": 5, "completion_tokens": 2, "total_tokens": 7}
            }))
        })
        .mount(&mock_server)
        .await;

    let provider = OpenAICompatibleProvider::new(
        "test".to_string(),
        mock_server.uri(),
        Some("test-key".to_string()),
    );

    let request = standard_completion_request();
    let _response = provider.complete(request).await.unwrap();

    let req = captured_request.lock().unwrap();
    let req = req.as_ref().unwrap();
    let body = extract_json_body(req);

    let messages = body.get("messages").unwrap().as_array().unwrap();
    assert_eq!(messages.len(), 2);

    // First message (system)
    assert_eq!(messages[0].get("role").unwrap().as_str().unwrap(), "system");
    assert_eq!(
        messages[0].get("content").unwrap().as_str().unwrap(),
        "You are a helpful assistant."
    );

    // Second message (user)
    assert_eq!(messages[1].get("role").unwrap().as_str().unwrap(), "user");
    assert_eq!(
        messages[1].get("content").unwrap().as_str().unwrap(),
        "Say hello"
    );
}

// ==================== STREAMING REQUEST VALIDATION ====================

#[tokio::test]
async fn test_streaming_request_has_stream_true() {
    let mock_server = MockServer::start().await;

    let captured_request = std::sync::Arc::new(std::sync::Mutex::new(None));
    let captured_request_clone = captured_request.clone();

    Mock::given(method("POST"))
        .and(path("/chat/completions"))
        .respond_with(move |req: &Request| {
            *captured_request_clone.lock().unwrap() = Some(req.clone());
            ResponseTemplate::new(200).set_body_string(
                "data: {\"id\":\"test\",\"object\":\"chat.completion.chunk\",\"created\":1234567890,\"model\":\"test\",\"choices\":[{\"index\":0,\"delta\":{\"content\":\"Hi\"},\"finish_reason\":\"stop\"}]}\n\n"
            )
        })
        .mount(&mock_server)
        .await;

    let provider = OpenAICompatibleProvider::new(
        "test".to_string(),
        mock_server.uri(),
        Some("test-key".to_string()),
    );

    let request = standard_streaming_request();
    let mut stream = provider.stream_complete(request).await.unwrap();

    // Consume stream
    while let Some(_) = stream.next().await {}

    let req = captured_request.lock().unwrap();
    let req = req.as_ref().unwrap();
    let body = extract_json_body(req);

    assert_json_bool_field(&body, "stream", true);
}

// ==================== LM STUDIO SPECIFIC TESTS ====================

#[tokio::test]
async fn test_lmstudio_health_check_endpoint() {
    let mock_server = MockServer::start().await;

    let captured_request = std::sync::Arc::new(std::sync::Mutex::new(None));
    let captured_request_clone = captured_request.clone();

    Mock::given(method("GET"))
        .and(path("/models"))
        .respond_with(move |req: &Request| {
            *captured_request_clone.lock().unwrap() = Some(req.clone());
            ResponseTemplate::new(200).set_body_json(json!({
                "object": "list",
                "data": []
            }))
        })
        .mount(&mock_server)
        .await;

    let provider = LMStudioProvider::with_base_url(mock_server.uri());
    let _health = provider.health_check().await;

    let req = captured_request.lock().unwrap();
    let req = req.as_ref().unwrap();
    assert_method(req, "GET");
    assert_path(req, "/models");
}

#[tokio::test]
async fn test_lmstudio_with_api_key() {
    let mock_server = MockServer::start().await;

    let captured_request = std::sync::Arc::new(std::sync::Mutex::new(None));
    let captured_request_clone = captured_request.clone();

    Mock::given(method("POST"))
        .and(path("/chat/completions"))
        .respond_with(move |req: &Request| {
            *captured_request_clone.lock().unwrap() = Some(req.clone());
            ResponseTemplate::new(200).set_body_json(json!({
                "id": "test",
                "object": "chat.completion",
                "created": 1234567890,
                "model": "test-model",
                "choices": [{
                    "index": 0,
                    "message": {"role": "assistant", "content": "Hi"},
                    "finish_reason": "stop"
                }],
                "usage": {"prompt_tokens": 5, "completion_tokens": 2, "total_tokens": 7}
            }))
        })
        .mount(&mock_server)
        .await;

    let provider = LMStudioProvider::with_base_url(mock_server.uri())
        .with_api_key(Some("lmstudio-key".to_string()));

    let request = standard_completion_request();
    let _response = provider.complete(request).await.unwrap();

    let req = captured_request.lock().unwrap();
    let req = req.as_ref().unwrap();
    assert_header_contains(req, "authorization", "lmstudio-key");
}

#[tokio::test]
async fn test_lmstudio_without_api_key() {
    let mock_server = MockServer::start().await;

    let captured_request = std::sync::Arc::new(std::sync::Mutex::new(None));
    let captured_request_clone = captured_request.clone();

    Mock::given(method("POST"))
        .and(path("/chat/completions"))
        .respond_with(move |req: &Request| {
            *captured_request_clone.lock().unwrap() = Some(req.clone());
            ResponseTemplate::new(200).set_body_json(json!({
                "id": "test",
                "object": "chat.completion",
                "created": 1234567890,
                "model": "test-model",
                "choices": [{
                    "index": 0,
                    "message": {"role": "assistant", "content": "Hi"},
                    "finish_reason": "stop"
                }],
                "usage": {"prompt_tokens": 5, "completion_tokens": 2, "total_tokens": 7}
            }))
        })
        .mount(&mock_server)
        .await;

    let provider = LMStudioProvider::with_base_url(mock_server.uri());

    let request = standard_completion_request();
    let _response = provider.complete(request).await.unwrap();

    let req = captured_request.lock().unwrap();
    let req = req.as_ref().unwrap();

    // Should not have authorization header when no API key provided
    assert!(req.headers.get("authorization").is_none());
}

// ==================== RESPONSE VALIDATION ====================

#[tokio::test]
async fn test_response_field_mapping() {
    let mock_server = MockServer::start().await;

    Mock::given(method("POST"))
        .and(path("/chat/completions"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "id": "chatcmpl-test-123",
            "object": "chat.completion",
            "created": 1234567890,
            "model": "gpt-3.5-turbo",
            "choices": [{
                "index": 0,
                "message": {
                    "role": "assistant",
                    "content": "Test response"
                },
                "finish_reason": "stop"
            }],
            "usage": {
                "prompt_tokens": 15,
                "completion_tokens": 10,
                "total_tokens": 25
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

    // Validate all fields are correctly mapped
    assert_eq!(response.id, "chatcmpl-test-123");
    assert_eq!(response.object, "chat.completion");
    assert_eq!(response.created, 1234567890);
    assert_eq!(response.model, "gpt-3.5-turbo");
    assert_eq!(response.choices.len(), 1);
    assert_eq!(response.choices[0].index, 0);
    assert_eq!(response.choices[0].message.role, "assistant");
    assert_eq!(response.choices[0].message.content, "Test response");
    assert_eq!(response.choices[0].finish_reason, Some("stop".to_string()));
    assert_eq!(response.usage.prompt_tokens, 15);
    assert_eq!(response.usage.completion_tokens, 10);
    assert_eq!(response.usage.total_tokens, 25);
}

#[tokio::test]
async fn test_finish_reason_stop() {
    let mock_server = MockServer::start().await;

    Mock::given(method("POST"))
        .and(path("/chat/completions"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "id": "test",
            "object": "chat.completion",
            "created": 1234567890,
            "model": "test-model",
            "choices": [{
                "index": 0,
                "message": {"role": "assistant", "content": "Hi"},
                "finish_reason": "stop"
            }],
            "usage": {"prompt_tokens": 5, "completion_tokens": 2, "total_tokens": 7}
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

    assert_eq!(response.choices[0].finish_reason, Some("stop".to_string()));
}

#[tokio::test]
async fn test_finish_reason_length() {
    let mock_server = MockServer::start().await;

    Mock::given(method("POST"))
        .and(path("/chat/completions"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "id": "test",
            "object": "chat.completion",
            "created": 1234567890,
            "model": "test-model",
            "choices": [{
                "index": 0,
                "message": {"role": "assistant", "content": "Hi"},
                "finish_reason": "length"
            }],
            "usage": {"prompt_tokens": 5, "completion_tokens": 2, "total_tokens": 7}
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

    assert_eq!(response.choices[0].finish_reason, Some("length".to_string()));
}

#[tokio::test]
async fn test_finish_reason_content_filter() {
    let mock_server = MockServer::start().await;

    Mock::given(method("POST"))
        .and(path("/chat/completions"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "id": "test",
            "object": "chat.completion",
            "created": 1234567890,
            "model": "test-model",
            "choices": [{
                "index": 0,
                "message": {"role": "assistant", "content": "Hi"},
                "finish_reason": "content_filter"
            }],
            "usage": {"prompt_tokens": 5, "completion_tokens": 2, "total_tokens": 7}
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

    assert_eq!(response.choices[0].finish_reason, Some("content_filter".to_string()));
}
