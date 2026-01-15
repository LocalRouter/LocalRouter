//! Common utilities and test helpers for provider testing
//!
//! This module provides reusable components for testing all providers:
//! - Mock server builders for different provider formats
//! - Standard test requests and expected responses
//! - Assertion helpers

use localrouter_ai::providers::*;
use serde_json::json;
use wiremock::{
    matchers::{header, method, path, path_regex},
    Mock, MockServer, ResponseTemplate,
};

/// Standard test request that should work across all providers
pub fn standard_completion_request() -> CompletionRequest {
    CompletionRequest {
        model: "test-model".to_string(),
        messages: vec![
            ChatMessage {
                role: "system".to_string(),
                content: "You are a helpful assistant.".to_string(),
            },
            ChatMessage {
                role: "user".to_string(),
                content: "Say hello".to_string(),
            },
        ],
        temperature: Some(0.7),
        max_tokens: Some(100),
        stream: false,
        top_p: None,
        frequency_penalty: None,
        presence_penalty: None,
        stop: None,
    }
}

/// Standard streaming test request
pub fn standard_streaming_request() -> CompletionRequest {
    CompletionRequest {
        model: "test-model".to_string(),
        messages: vec![ChatMessage {
            role: "user".to_string(),
            content: "Count to 3".to_string(),
        }],
        temperature: Some(0.5),
        max_tokens: Some(50),
        stream: true,
        top_p: None,
        frequency_penalty: None,
        presence_penalty: None,
        stop: None,
    }
}

/// Test cases to run for all providers
#[derive(Debug, Clone)]
pub enum TestCase {
    HealthCheck,
    ListModels,
    Completion,
    StreamingCompletion,
}

/// Assert that a completion response is valid
pub fn assert_valid_completion(response: &CompletionResponse) {
    assert!(!response.id.is_empty(), "Response ID should not be empty");
    assert!(
        !response.model.is_empty(),
        "Response model should not be empty"
    );
    assert!(
        !response.choices.is_empty(),
        "Response should have at least one choice"
    );

    let choice = &response.choices[0];
    assert_eq!(choice.index, 0, "First choice should have index 0");
    assert!(
        !choice.message.content.is_empty(),
        "Message content should not be empty"
    );
    assert_eq!(
        choice.message.role, "assistant",
        "Response role should be assistant"
    );

    assert!(
        response.usage.total_tokens > 0,
        "Total tokens should be greater than 0"
    );
}

// ==================== OPENAI-COMPATIBLE MOCK SERVER ====================

pub struct OpenAICompatibleMockBuilder {
    server: MockServer,
}

impl OpenAICompatibleMockBuilder {
    pub async fn new() -> Self {
        Self {
            server: MockServer::start().await,
        }
    }

    pub fn base_url(&self) -> String {
        self.server.uri()
    }

    /// Mock the /models endpoint (OpenAI format)
    pub async fn mock_list_models(self) -> Self {
        let response = json!({
            "object": "list",
            "data": [
                {
                    "id": "test-model",
                    "object": "model",
                    "created": 1234567890,
                    "owned_by": "test-org"
                },
                {
                    "id": "test-model-large",
                    "object": "model",
                    "created": 1234567890,
                    "owned_by": "test-org"
                }
            ]
        });

        Mock::given(method("GET"))
            .and(path("/models"))
            .respond_with(ResponseTemplate::new(200).set_body_json(response))
            .mount(&self.server)
            .await;

        self
    }

    /// Mock the /chat/completions endpoint (OpenAI format, non-streaming)
    pub async fn mock_completion(self) -> Self {
        let response = json!({
            "id": "chatcmpl-test123",
            "object": "chat.completion",
            "created": 1234567890,
            "model": "test-model",
            "choices": [
                {
                    "index": 0,
                    "message": {
                        "role": "assistant",
                        "content": "Hello! How can I help you today?"
                    },
                    "finish_reason": "stop"
                }
            ],
            "usage": {
                "prompt_tokens": 10,
                "completion_tokens": 9,
                "total_tokens": 19
            }
        });

        Mock::given(method("POST"))
            .and(path("/chat/completions"))
            .respond_with(ResponseTemplate::new(200).set_body_json(response))
            .mount(&self.server)
            .await;

        self
    }

    /// Mock the /chat/completions endpoint (OpenAI format, streaming)
    pub async fn mock_streaming_completion(self) -> Self {
        let chunks = vec![
            "data: {\"id\":\"chatcmpl-test\",\"object\":\"chat.completion.chunk\",\"created\":1234567890,\"model\":\"test-model\",\"choices\":[{\"index\":0,\"delta\":{\"role\":\"assistant\",\"content\":\"1\"},\"finish_reason\":null}]}\n\n",
            "data: {\"id\":\"chatcmpl-test\",\"object\":\"chat.completion.chunk\",\"created\":1234567890,\"model\":\"test-model\",\"choices\":[{\"index\":0,\"delta\":{\"content\":\" 2\"},\"finish_reason\":null}]}\n\n",
            "data: {\"id\":\"chatcmpl-test\",\"object\":\"chat.completion.chunk\",\"created\":1234567890,\"model\":\"test-model\",\"choices\":[{\"index\":0,\"delta\":{\"content\":\" 3\"},\"finish_reason\":\"stop\"}]}\n\n",
            "data: [DONE]\n\n",
        ];

        let body = chunks.join("");

        Mock::given(method("POST"))
            .and(path("/chat/completions"))
            .respond_with(ResponseTemplate::new(200).set_body_string(body))
            .mount(&self.server)
            .await;

        self
    }

    pub fn into_server(self) -> MockServer {
        self.server
    }
}

// ==================== OLLAMA MOCK SERVER ====================

pub struct OllamaMockBuilder {
    server: MockServer,
}

impl OllamaMockBuilder {
    pub async fn new() -> Self {
        Self {
            server: MockServer::start().await,
        }
    }

    pub fn base_url(&self) -> String {
        self.server.uri()
    }

    /// Mock the /api/tags endpoint (Ollama format for listing models)
    pub async fn mock_list_models(self) -> Self {
        let response = json!({
            "models": [
                {
                    "name": "llama3.3:latest",
                    "modified_at": "2024-01-01T00:00:00Z",
                    "size": 4661224676i64,
                    "digest": "test-digest",
                    "details": {
                        "format": "gguf",
                        "family": "llama",
                        "parameter_size": "70B"
                    }
                }
            ]
        });

        Mock::given(method("GET"))
            .and(path("/api/tags"))
            .respond_with(ResponseTemplate::new(200).set_body_json(response))
            .mount(&self.server)
            .await;

        self
    }

    /// Mock the /api/chat endpoint (Ollama format, non-streaming)
    pub async fn mock_completion(self) -> Self {
        let response = json!({
            "message": {
                "role": "assistant",
                "content": "Hello! How can I help you today?"
            },
            "done": true,
            "final_data": {
                "prompt_eval_count": 10,
                "eval_count": 9
            }
        });

        Mock::given(method("POST"))
            .and(path("/api/chat"))
            .respond_with(ResponseTemplate::new(200).set_body_json(response))
            .mount(&self.server)
            .await;

        self
    }

    /// Mock the /api/chat endpoint (Ollama format, streaming - cumulative content)
    pub async fn mock_streaming_completion(self) -> Self {
        // Ollama sends cumulative content, not deltas
        let chunks = vec![
            json!({
                "message": {"role": "assistant", "content": "1"},
                "done": false
            }),
            json!({
                "message": {"role": "assistant", "content": "1 2"},
                "done": false
            }),
            json!({
                "message": {"role": "assistant", "content": "1 2 3"},
                "done": false
            }),
            json!({
                "message": {"role": "assistant", "content": "1 2 3"},
                "done": true
            }),
        ];

        let body = chunks
            .into_iter()
            .map(|c| format!("{}\n", serde_json::to_string(&c).unwrap()))
            .collect::<String>();

        Mock::given(method("POST"))
            .and(path("/api/chat"))
            .respond_with(ResponseTemplate::new(200).set_body_string(body))
            .mount(&self.server)
            .await;

        self
    }

    pub fn into_server(self) -> MockServer {
        self.server
    }
}

// ==================== ANTHROPIC MOCK SERVER ====================

pub struct AnthropicMockBuilder {
    server: MockServer,
}

impl AnthropicMockBuilder {
    pub async fn new() -> Self {
        Self {
            server: MockServer::start().await,
        }
    }

    pub fn base_url(&self) -> String {
        self.server.uri()
    }

    /// Mock the /v1/models endpoint (Anthropic doesn't have this, returns error or empty)
    pub async fn mock_list_models(self) -> Self {
        // Anthropic doesn't have a models endpoint, but the provider should handle this
        Mock::given(method("GET"))
            .and(path("/v1/models"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({"data": []})))
            .mount(&self.server)
            .await;

        self
    }

    /// Mock the /v1/messages endpoint (Anthropic format, non-streaming)
    pub async fn mock_completion(self) -> Self {
        let response = json!({
            "id": "msg_test123",
            "type": "message",
            "role": "assistant",
            "model": "claude-3-sonnet-20240229",
            "content": [
                {
                    "type": "text",
                    "text": "Hello! How can I help you today?"
                }
            ],
            "stop_reason": "end_turn",
            "usage": {
                "input_tokens": 10,
                "output_tokens": 9
            }
        });

        Mock::given(method("POST"))
            .and(path("/v1/messages"))
            .and(header("x-api-key", "test-key"))
            .respond_with(ResponseTemplate::new(200).set_body_json(response))
            .mount(&self.server)
            .await;

        self
    }

    /// Mock the /v1/messages endpoint (Anthropic format, streaming)
    pub async fn mock_streaming_completion(self) -> Self {
        let chunks = vec![
            "event: message_start\ndata: {\"type\":\"message_start\",\"message\":{\"id\":\"msg_test\",\"type\":\"message\",\"role\":\"assistant\"}}\n\n",
            "event: content_block_start\ndata: {\"type\":\"content_block_start\",\"index\":0,\"content_block\":{\"type\":\"text\",\"text\":\"\"}}\n\n",
            "event: content_block_delta\ndata: {\"type\":\"content_block_delta\",\"index\":0,\"delta\":{\"type\":\"text_delta\",\"text\":\"1\"},\"message_id\":\"msg_test\"}\n\n",
            "event: content_block_delta\ndata: {\"type\":\"content_block_delta\",\"index\":0,\"delta\":{\"type\":\"text_delta\",\"text\":\" 2\"},\"message_id\":\"msg_test\"}\n\n",
            "event: content_block_delta\ndata: {\"type\":\"content_block_delta\",\"index\":0,\"delta\":{\"type\":\"text_delta\",\"text\":\" 3\"},\"message_id\":\"msg_test\"}\n\n",
            "event: content_block_stop\ndata: {\"type\":\"content_block_stop\",\"index\":0}\n\n",
            "event: message_stop\ndata: {\"type\":\"message_stop\",\"message_id\":\"msg_test\"}\n\n",
        ];

        let body = chunks.join("");

        Mock::given(method("POST"))
            .and(path("/v1/messages"))
            .and(header("x-api-key", "test-key"))
            .respond_with(ResponseTemplate::new(200).set_body_string(body))
            .mount(&self.server)
            .await;

        self
    }

    pub fn into_server(self) -> MockServer {
        self.server
    }
}

// ==================== GEMINI MOCK SERVER ====================

pub struct GeminiMockBuilder {
    server: MockServer,
}

impl GeminiMockBuilder {
    pub async fn new() -> Self {
        Self {
            server: MockServer::start().await,
        }
    }

    pub fn base_url(&self) -> String {
        // Gemini provider expects base_url to include /v1beta
        format!("{}/v1beta", self.server.uri())
    }

    /// Mock the /v1beta/models endpoint (Gemini format)
    pub async fn mock_list_models(self) -> Self {
        let response = json!({
            "models": [
                {
                    "name": "models/gemini-1.5-flash",
                    "displayName": "Gemini 1.5 Flash",
                    "supportedGenerationMethods": ["generateContent", "streamGenerateContent"],
                    "inputTokenLimit": 1048576,
                    "outputTokenLimit": 8192
                }
            ]
        });

        Mock::given(method("GET"))
            .and(path_regex(r"^/v1beta/models.*"))
            .respond_with(ResponseTemplate::new(200).set_body_json(response))
            .mount(&self.server)
            .await;

        self
    }

    /// Mock the generateContent endpoint (Gemini format, non-streaming)
    pub async fn mock_completion(self) -> Self {
        let response = json!({
            "candidates": [
                {
                    "content": {
                        "role": "model",
                        "parts": [
                            {"text": "Hello! How can I help you today?"}
                        ]
                    },
                    "finishReason": "STOP"
                }
            ],
            "usageMetadata": {
                "promptTokenCount": 10,
                "candidatesTokenCount": 9,
                "totalTokenCount": 19
            }
        });

        Mock::given(method("POST"))
            .and(path_regex(r"^/v1beta/models/.*:generateContent.*"))
            .respond_with(ResponseTemplate::new(200).set_body_json(response))
            .mount(&self.server)
            .await;

        self
    }

    /// Mock the streamGenerateContent endpoint (Gemini format, streaming)
    pub async fn mock_streaming_completion(self) -> Self {
        let chunks = vec![
            json!({
                "candidates": [{
                    "content": {
                        "role": "model",
                        "parts": [{"text": "1"}]
                    },
                    "finishReason": null
                }]
            }),
            json!({
                "candidates": [{
                    "content": {
                        "role": "model",
                        "parts": [{"text": " 2"}]
                    },
                    "finishReason": null
                }]
            }),
            json!({
                "candidates": [{
                    "content": {
                        "role": "model",
                        "parts": [{"text": " 3"}]
                    },
                    "finishReason": "STOP"
                }]
            }),
        ];

        let body = chunks
            .into_iter()
            .map(|c| format!("data: {}\n\n", serde_json::to_string(&c).unwrap()))
            .collect::<String>();

        Mock::given(method("POST"))
            .and(path_regex(r"^/v1beta/models/.*:streamGenerateContent.*"))
            .respond_with(ResponseTemplate::new(200).set_body_string(body))
            .mount(&self.server)
            .await;

        self
    }

    pub fn into_server(self) -> MockServer {
        self.server
    }
}

// ==================== COHERE MOCK SERVER ====================

pub struct CohereMockBuilder {
    server: MockServer,
}

impl CohereMockBuilder {
    pub async fn new() -> Self {
        Self {
            server: MockServer::start().await,
        }
    }

    pub fn base_url(&self) -> String {
        self.server.uri()
    }

    /// Mock the /v2/models endpoint (Cohere format) - returns static list
    pub async fn mock_list_models(self) -> Self {
        // Cohere doesn't have a public models endpoint - the provider uses a static list
        Mock::given(method("GET"))
            .and(path("/v2/models"))
            .respond_with(ResponseTemplate::new(404))
            .mount(&self.server)
            .await;

        self
    }

    /// Mock the /v2/chat endpoint (Cohere format, non-streaming)
    pub async fn mock_completion(self) -> Self {
        let response = json!({
            "id": "test-chat-id",
            "message": {
                "role": "assistant",
                "content": [
                    {"type": "text", "text": "Hello! How can I help you today?"}
                ]
            },
            "finish_reason": "COMPLETE",
            "usage": {
                "billed_units": {
                    "input_tokens": 10,
                    "output_tokens": 9
                }
            }
        });

        Mock::given(method("POST"))
            .and(path("/v2/chat"))
            .and(header("Authorization", "Bearer test-key"))
            .respond_with(ResponseTemplate::new(200).set_body_json(response))
            .mount(&self.server)
            .await;

        self
    }

    /// Mock the /v2/chat endpoint (Cohere format, streaming)
    pub async fn mock_streaming_completion(self) -> Self {
        let chunks = vec![
            json!({
                "type": "content-delta",
                "delta": {"message": {"content": {"text": "1"}}}
            }),
            json!({
                "type": "content-delta",
                "delta": {"message": {"content": {"text": " 2"}}}
            }),
            json!({
                "type": "content-delta",
                "delta": {"message": {"content": {"text": " 3"}}}
            }),
            json!({
                "type": "message-end",
                "delta": {
                    "finish_reason": "COMPLETE",
                    "usage": {
                        "billed_units": {"input_tokens": 5, "output_tokens": 6}
                    }
                }
            }),
        ];

        let body = chunks
            .into_iter()
            .map(|c| format!("{}\n", serde_json::to_string(&c).unwrap()))
            .collect::<String>();

        Mock::given(method("POST"))
            .and(path("/v2/chat"))
            .and(header("Authorization", "Bearer test-key"))
            .respond_with(ResponseTemplate::new(200).set_body_string(body))
            .mount(&self.server)
            .await;

        self
    }

    pub fn into_server(self) -> MockServer {
        self.server
    }
}
