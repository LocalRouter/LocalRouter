//! Feature Adapter Integration Tests
//!
//! Comprehensive integration tests for all feature adapters including:
//! - Structured Outputs: JSON schema validation
//! - Prompt Caching: Cost optimization through caching
//! - Logprobs: Token probability extraction
//! - JSON Mode: Lightweight JSON validation
//!
//! These tests verify end-to-end functionality with realistic scenarios.

use localrouter_ai::providers::{
    features::{
        json_mode::JsonModeAdapter, logprobs::LogprobsAdapter,
        prompt_caching::PromptCachingAdapter, structured_outputs::StructuredOutputsAdapter,
        FeatureAdapter,
    },
    ChatMessage, ChatMessageContent, CompletionChoice, CompletionRequest, CompletionResponse, PromptTokensDetails,
    TokenUsage,
};
use serde_json::json;
use std::collections::HashMap;

// ============================================================================
// Structured Outputs Integration Tests
// ============================================================================

#[test]
fn test_structured_outputs_person_schema_openai() {
    let adapter = StructuredOutputsAdapter;

    // Person schema: requires name (string) and age (number)
    let schema = json!({
        "type": "object",
        "properties": {
            "name": {"type": "string"},
            "age": {"type": "number"}
        },
        "required": ["name", "age"]
    });

    let mut params = HashMap::new();
    params.insert("schema".to_string(), schema.clone());

    // Create OpenAI request
    let mut request = CompletionRequest {
        model: "gpt-4".to_string(),
        messages: vec![ChatMessage {
            role: "user".to_string(),
            content: ChatMessageContent::Text("Generate a person with name and age".to_string()),
                tool_calls: None,
                tool_call_id: None,
                name: None,
        }],
        temperature: None,
        max_tokens: None,
        stream: false,
        top_p: None,
        frequency_penalty: None,
        presence_penalty: None,
        stop: None,
        top_k: None,
        seed: None,
        repetition_penalty: None,
        extensions: None,
            logprobs: None,
            top_logprobs: None,
            response_format: None,
            tool_choice: None,
            tools: None,
        };

    // Adapt request
    let result = adapter.adapt_request(&mut request, &params);
    assert!(result.is_ok(), "Request adaptation should succeed");

    // Verify response_format was added
    assert!(request.extensions.is_some());
    let extensions = request.extensions.unwrap();
    assert!(extensions.contains_key("response_format"));

    let response_format = extensions.get("response_format").unwrap();
    assert_eq!(response_format["type"], "json_schema");
    assert!(response_format.get("json_schema").is_some());

    // Verify stored schema for validation
    assert!(extensions.contains_key("_structured_outputs_schema"));
}

#[test]
fn test_structured_outputs_response_validation_valid() {
    let adapter = StructuredOutputsAdapter;

    let schema = json!({
        "type": "object",
        "properties": {
            "result": {"type": "string"},
            "count": {"type": "number"}
        },
        "required": ["result", "count"]
    });

    // Create response with valid JSON matching schema
    let mut extensions = HashMap::new();
    extensions.insert("_structured_outputs_schema".to_string(), schema);

    let mut response = CompletionResponse {
        id: "test-id".to_string(),
        object: "chat.completion".to_string(),
        created: 1234567890,
        model: "gpt-4".to_string(),
        provider: "openai".to_string(),
        choices: vec![CompletionChoice {
            index: 0,
            message: ChatMessage {
                role: "assistant".to_string(),
                content: ChatMessageContent::Text(r#"{"result": "success", "count": 42}"#.to_string()),
                tool_calls: None,
                tool_call_id: None,
                name: None,
            },
            finish_reason: Some("stop".to_string()),
            logprobs: None,
        }],
        usage: TokenUsage {
            prompt_tokens: 100,
            completion_tokens: 50,
            total_tokens: 150,
            prompt_tokens_details: None,
            completion_tokens_details: None,
        },
        extensions: Some(extensions),
        routellm_win_rate: None,
    };

    // Validate response
    let result = adapter.adapt_response(&mut response);
    assert!(result.is_ok(), "Valid response should pass validation");

    let feature_data = result.unwrap();
    assert!(feature_data.is_some());

    let data = feature_data.unwrap();
    assert_eq!(data.feature, "structured_outputs");
    assert_eq!(data.data["validated"], true);
}

#[test]
fn test_structured_outputs_response_validation_invalid() {
    let adapter = StructuredOutputsAdapter;

    let schema = json!({
        "type": "object",
        "properties": {
            "name": {"type": "string"}
        },
        "required": ["name"]
    });

    // Create response with JSON that doesn't match schema (missing required field)
    let mut extensions = HashMap::new();
    extensions.insert("_structured_outputs_schema".to_string(), schema);

    let mut response = CompletionResponse {
        id: "test-id".to_string(),
        object: "chat.completion".to_string(),
        created: 1234567890,
        model: "gpt-4".to_string(),
        provider: "openai".to_string(),
        choices: vec![CompletionChoice {
            index: 0,
            message: ChatMessage {
                role: "assistant".to_string(),
                content: ChatMessageContent::Text(r#"{"age": 30}"#.to_string()), // Missing 'name'
                tool_calls: None,
                tool_call_id: None,
                name: None,
            },
            finish_reason: Some("stop".to_string()),
        }],
        usage: TokenUsage {
            prompt_tokens: 100,
            completion_tokens: 50,
            total_tokens: 150,
            prompt_tokens_details: None,
            completion_tokens_details: None,
        },
        extensions: Some(extensions),
        routellm_win_rate: None,
    };

    // Validate response - should fail
    let result = adapter.adapt_response(&mut response);
    assert!(result.is_err(), "Invalid response should fail validation");
    assert!(result
        .unwrap_err()
        .to_string()
        .contains("does not match schema"));
}

// ============================================================================
// Prompt Caching Integration Tests
// ============================================================================

#[test]
fn test_prompt_caching_anthropic_request() {
    let adapter = PromptCachingAdapter;

    let params = HashMap::new();

    // Create Anthropic request with conversation history
    let mut request = CompletionRequest {
        model: "claude-opus-4-5".to_string(),
        messages: vec![
            ChatMessage {
                role: "system".to_string(),
                content: ChatMessageContent::Text("You are a helpful assistant.".to_string()),
                tool_calls: None,
                tool_call_id: None,
                name: None,
            },
            ChatMessage {
                role: "user".to_string(),
                content: ChatMessageContent::Text("What is the capital of France?".to_string()),
                tool_calls: None,
                tool_call_id: None,
                name: None,
            },
            ChatMessage {
                role: "assistant".to_string(),
                content: ChatMessageContent::Text("Paris is the capital of France.".to_string()),
                tool_calls: None,
                tool_call_id: None,
                name: None,
            },
            ChatMessage {
                role: "user".to_string(),
                content: ChatMessageContent::Text("What about Germany?".to_string()),
                tool_calls: None,
                tool_call_id: None,
                name: None,
            },
        ],
        temperature: None,
        max_tokens: None,
        stream: false,
        top_p: None,
        frequency_penalty: None,
        presence_penalty: None,
        stop: None,
        top_k: None,
        seed: None,
        repetition_penalty: None,
        extensions: None,
            top_logprobs: None,
            response_format: None,
            tool_choice: None,
            tools: None,
        };

    // Adapt request
    let result = adapter.adapt_request(&mut request, &params);
    assert!(result.is_ok(), "Request adaptation should succeed");

    // Verify cache configuration was added
    assert!(request.extensions.is_some());
    let extensions = request.extensions.unwrap();
    assert!(extensions.contains_key("_prompt_caching_breakpoints"));
    assert!(extensions.contains_key("_prompt_caching_control"));
}

#[test]
fn test_prompt_caching_cost_savings_calculation() {
    let adapter = PromptCachingAdapter;

    // Simulate Anthropic-style response with cache metrics in extensions.usage
    let mut extensions = HashMap::new();
    extensions.insert(
        "usage".to_string(),
        json!({
            "cache_creation_input_tokens": 500,
            "cache_read_input_tokens": 500,
            "input_tokens": 100
        }),
    );

    let mut response = CompletionResponse {
        id: "test-id".to_string(),
        object: "chat.completion".to_string(),
        created: 1234567890,
        model: "claude-opus-4-5".to_string(),
        provider: "anthropic".to_string(),
        choices: vec![CompletionChoice {
            index: 0,
            message: ChatMessage {
                role: "assistant".to_string(),
                content: ChatMessageContent::Text("Response content".to_string()),
                tool_calls: None,
                tool_call_id: None,
                name: None,
            },
            finish_reason: Some("stop".to_string()),
            logprobs: None,
        }],
        usage: TokenUsage {
            prompt_tokens: 100, // Regular prompt tokens
            completion_tokens: 50,
            total_tokens: 1150,
            prompt_tokens_details: Some(PromptTokensDetails {
                cached_tokens: None,
                cache_creation_tokens: Some(500), // Created 500 tokens
                cache_read_tokens: Some(500),     // Read 500 cached tokens
            }),
            completion_tokens_details: None,
        },
        extensions: Some(extensions),
        routellm_win_rate: None,
    };

    // Validate response and extract cache data
    let result = adapter.adapt_response(&mut response);
    assert!(result.is_ok(), "Response adaptation should succeed");

    let feature_data = result.unwrap();
    assert!(feature_data.is_some());

    let data = feature_data.unwrap();
    assert_eq!(data.feature, "prompt_caching");
    assert_eq!(data.data["cache_creation_input_tokens"], 500);
    assert_eq!(data.data["cache_read_input_tokens"], 500);

    // Verify cost savings calculation
    // Cache read: 0.1x cost = 90% savings
    // Full cost: 100 + 500 + 500 = 1100 tokens
    // Cached cost: 100 + 500 + (500 * 0.1) = 650 tokens
    // Savings: (1100 - 650) / 1100 = ~41%
    let savings_str = data.data["cache_savings_percent"].as_str().unwrap();
    // Parse "41.0%" -> 41.0
    let savings: f64 = savings_str.trim_end_matches('%').parse().unwrap();
    assert!(
        savings > 40.0 && savings < 42.0,
        "Cache savings should be ~41%, got {}",
        savings
    );
}

// ============================================================================
// Logprobs Integration Tests
// ============================================================================

#[test]
fn test_logprobs_openai_request() {
    let adapter = LogprobsAdapter;

    let mut params = HashMap::new();
    params.insert("top_logprobs".to_string(), json!(5));

    // Create OpenAI request
    let mut request = CompletionRequest {
        model: "gpt-4".to_string(),
        messages: vec![ChatMessage {
            role: "user".to_string(),
            content: ChatMessageContent::Text("Say hello".to_string()),
                tool_calls: None,
                tool_call_id: None,
                name: None,
        }],
        temperature: None,
        max_tokens: None,
        stream: false,
        top_p: None,
        frequency_penalty: None,
        presence_penalty: None,
        stop: None,
        top_k: None,
        seed: None,
        repetition_penalty: None,
        extensions: None,
            top_logprobs: None,
            response_format: None,
            tool_choice: None,
            tools: None,
        };

    // Adapt request
    let result = adapter.adapt_request(&mut request, &params);
    assert!(result.is_ok(), "Request adaptation should succeed");

    // Verify logprobs configuration
    assert!(request.extensions.is_some());
    let extensions = request.extensions.unwrap();
    assert_eq!(extensions["_logprobs_enabled"], true);
    assert_eq!(extensions["_logprobs_top_count"], 5);
}

#[test]
fn test_logprobs_response_extraction() {
    let adapter = LogprobsAdapter;

    // Create response with logprobs data
    let logprobs_data = json!({
        "content": [
            {
                "token": "Hello",
                "logprob": -0.0001,
                "bytes": [72, 101, 108, 108, 111],
                "top_logprobs": [
                    {"token": "Hello", "logprob": -0.0001},
                    {"token": "Hi", "logprob": -2.5},
                    {"token": "Hey", "logprob": -3.2}
                ]
            },
            {
                "token": " world",
                "logprob": -0.0002,
                "bytes": [32, 119, 111, 114, 108, 100],
                "top_logprobs": [
                    {"token": " world", "logprob": -0.0002},
                    {"token": " there", "logprob": -4.1}
                ]
            }
        ]
    });

    let mut extensions = HashMap::new();
    extensions.insert("logprobs".to_string(), logprobs_data);

    let mut response = CompletionResponse {
        id: "test-id".to_string(),
        object: "chat.completion".to_string(),
        created: 1234567890,
        model: "gpt-4".to_string(),
        provider: "openai".to_string(),
        choices: vec![CompletionChoice {
            index: 0,
            message: ChatMessage {
                role: "assistant".to_string(),
                content: ChatMessageContent::Text("Hello world".to_string()),
                tool_calls: None,
                tool_call_id: None,
                name: None,
            },
            finish_reason: Some("stop".to_string()),
            logprobs: None,
        }],
        usage: TokenUsage {
            prompt_tokens: 10,
            completion_tokens: 2,
            total_tokens: 12,
            prompt_tokens_details: None,
            completion_tokens_details: None,
        },
        extensions: Some(extensions),
        routellm_win_rate: None,
    };

    // Extract logprobs
    let result = adapter.adapt_response(&mut response);
    assert!(result.is_ok(), "Logprobs extraction should succeed");

    let feature_data = result.unwrap();
    assert!(feature_data.is_some());

    let data = feature_data.unwrap();
    assert_eq!(data.feature, "logprobs");
    assert!(data.data.get("logprobs").is_some());
    assert!(data.data["logprobs"].get("content").is_some());
    assert_eq!(data.data["token_count"], 2);

    // Verify average confidence calculation
    // (-0.0001 + -0.0002) / 2 = -0.00015
    let avg_confidence = data.data["average_confidence"].as_f64().unwrap();
    assert!((avg_confidence - (-0.00015)).abs() < 0.0001);
}

#[test]
fn test_logprobs_various_top_values() {
    let adapter = LogprobsAdapter;

    // Test with different top_logprobs values
    for top_n in [0, 1, 5, 10, 20] {
        let mut params = HashMap::new();
        params.insert("top_logprobs".to_string(), json!(top_n));

        let mut request = CompletionRequest {
            model: "gpt-4".to_string(),
            messages: vec![ChatMessage {
                role: "user".to_string(),
                content: ChatMessageContent::Text("Test".to_string()),
                tool_calls: None,
                tool_call_id: None,
                name: None,
            }],
            temperature: None,
            max_tokens: None,
            stream: false,
            top_p: None,
            frequency_penalty: None,
            presence_penalty: None,
            stop: None,
            top_k: None,
            seed: None,
            repetition_penalty: None,
            extensions: None,
            top_logprobs: None,
            response_format: None,
            tool_choice: None,
            tools: None,
        };

        let result = adapter.adapt_request(&mut request, &params);
        assert!(result.is_ok(), "Should accept top_logprobs={}", top_n);

        let extensions = request.extensions.unwrap();
        assert_eq!(extensions["_logprobs_enabled"], true);

        // top_logprobs=0 means no top alternatives, so _logprobs_top_count won't be set
        if top_n > 0 {
            assert_eq!(extensions["_logprobs_top_count"], top_n);
        }
    }
}

#[test]
fn test_logprobs_invalid_top_value() {
    let adapter = LogprobsAdapter;

    // Test with invalid top_logprobs value (> 20)
    let mut params = HashMap::new();
    params.insert("top_logprobs".to_string(), json!(25));

    let result = adapter.validate_params(&params);
    assert!(result.is_err(), "Should reject top_logprobs > 20");
    assert!(result.unwrap_err().to_string().contains("between 0 and 20"));
}

// ============================================================================
// JSON Mode Integration Tests
// ============================================================================

#[test]
fn test_json_mode_openai() {
    let adapter = JsonModeAdapter;

    let params = HashMap::new();

    // Create OpenAI request
    let mut request = CompletionRequest {
        model: "gpt-4".to_string(),
        messages: vec![ChatMessage {
            role: "user".to_string(),
            content: ChatMessageContent::Text("Generate a JSON object with a greeting".to_string()),
                tool_calls: None,
                tool_call_id: None,
                name: None,
        }],
        temperature: None,
        max_tokens: None,
        stream: false,
        top_p: None,
        frequency_penalty: None,
        presence_penalty: None,
        stop: None,
        top_k: None,
        seed: None,
        repetition_penalty: None,
        extensions: None,
            logprobs: None,
            top_logprobs: None,
            response_format: None,
            tool_choice: None,
            tools: None,
        };

    // Adapt request
    let result = adapter.adapt_request(&mut request, &params);
    assert!(result.is_ok(), "Request adaptation should succeed");

    // Verify response_format was set
    assert!(request.extensions.is_some());
    let extensions = request.extensions.unwrap();
    assert!(extensions.contains_key("response_format"));
    assert_eq!(extensions["response_format"]["type"], "json_object");
}

#[test]
fn test_json_mode_anthropic() {
    let adapter = JsonModeAdapter;

    let params = HashMap::new();

    // Create Anthropic request
    let mut request = CompletionRequest {
        model: "claude-3-opus".to_string(),
        messages: vec![ChatMessage {
            role: "user".to_string(),
            content: ChatMessageContent::Text("Generate a JSON object".to_string()),
                tool_calls: None,
                tool_call_id: None,
                name: None,
        }],
        temperature: None,
        max_tokens: None,
        stream: false,
        top_p: None,
        frequency_penalty: None,
        presence_penalty: None,
        stop: None,
        top_k: None,
        seed: None,
        repetition_penalty: None,
        extensions: None,
            logprobs: None,
            top_logprobs: None,
            response_format: None,
            tool_choice: None,
            tools: None,
        };

    // Adapt request
    let result = adapter.adapt_request(&mut request, &params);
    assert!(result.is_ok(), "Request adaptation should succeed");

    // Verify system message was added for Anthropic
    assert_eq!(request.messages.len(), 2);
    assert_eq!(request.messages[0].role, "system");
    assert!(request.messages[0].content.as_text().contains("valid JSON"));
}

#[test]
fn test_json_mode_validation_valid() {
    let adapter = JsonModeAdapter;

    // Create response with valid JSON
    let mut extensions = HashMap::new();
    extensions.insert("_json_mode_validation".to_string(), json!(true));

    let mut response = CompletionResponse {
        id: "test-id".to_string(),
        object: "chat.completion".to_string(),
        created: 1234567890,
        model: "gpt-4".to_string(),
        provider: "openai".to_string(),
        choices: vec![CompletionChoice {
            index: 0,
            message: ChatMessage {
                role: "assistant".to_string(),
                content: ChatMessageContent::Text(r#"{"greeting": "Hello, World!", "count": 42}"#.to_string()),
                tool_calls: None,
                tool_call_id: None,
                name: None,
            },
            finish_reason: Some("stop".to_string()),
            logprobs: None,
        }],
        usage: TokenUsage {
            prompt_tokens: 20,
            completion_tokens: 10,
            total_tokens: 30,
            prompt_tokens_details: None,
            completion_tokens_details: None,
        },
        extensions: Some(extensions),
        routellm_win_rate: None,
    };

    // Validate response
    let result = adapter.adapt_response(&mut response);
    assert!(result.is_ok(), "Valid JSON should pass validation");

    let feature_data = result.unwrap();
    assert!(feature_data.is_some());

    let data = feature_data.unwrap();
    assert_eq!(data.feature, "json_mode");
    assert_eq!(data.data["validated"], true);
    assert_eq!(data.data["choices_validated"], 1);
}

#[test]
fn test_json_mode_validation_invalid() {
    let adapter = JsonModeAdapter;

    // Create response with invalid JSON
    let mut extensions = HashMap::new();
    extensions.insert("_json_mode_validation".to_string(), json!(true));

    let mut response = CompletionResponse {
        id: "test-id".to_string(),
        object: "chat.completion".to_string(),
        created: 1234567890,
        model: "gpt-4".to_string(),
        provider: "openai".to_string(),
        choices: vec![CompletionChoice {
            index: 0,
            message: ChatMessage {
                role: "assistant".to_string(),
                content: ChatMessageContent::Text("This is not valid JSON at all".to_string()),
                tool_calls: None,
                tool_call_id: None,
                name: None,
            },
            finish_reason: Some("stop".to_string()),
        }],
        usage: TokenUsage {
            prompt_tokens: 20,
            completion_tokens: 10,
            total_tokens: 30,
            prompt_tokens_details: None,
            completion_tokens_details: None,
        },
        extensions: Some(extensions),
        routellm_win_rate: None,
    };

    // Validate response - should fail
    let result = adapter.adapt_response(&mut response);
    assert!(result.is_err(), "Invalid JSON should fail validation");
    assert!(result.unwrap_err().to_string().contains("not valid JSON"));
}

#[test]
fn test_json_mode_all_providers() {
    let adapter = JsonModeAdapter;

    let test_models = vec![
        ("gpt-4", "openai"),
        ("gpt-3.5-turbo", "openai"),
        ("claude-3-opus", "anthropic"),
        ("claude-sonnet-3-5", "anthropic"),
        ("gemini-pro", "gemini"),
        ("gemini-1.5-pro", "gemini"),
        ("openai/gpt-4", "openrouter"),
    ];

    for (model, expected_provider) in test_models {
        let mut request = CompletionRequest {
            model: model.to_string(),
            messages: vec![ChatMessage {
                role: "user".to_string(),
                content: ChatMessageContent::Text("Test".to_string()),
                tool_calls: None,
                tool_call_id: None,
                name: None,
            }],
            temperature: None,
            max_tokens: None,
            stream: false,
            top_p: None,
            frequency_penalty: None,
            presence_penalty: None,
            stop: None,
            top_k: None,
            seed: None,
            repetition_penalty: None,
            extensions: None,
            top_logprobs: None,
            response_format: None,
            tool_choice: None,
            tools: None,
        };

        let result = adapter.adapt_request(&mut request, &HashMap::new());
        assert!(
            result.is_ok(),
            "JSON mode should support {} ({})",
            model,
            expected_provider
        );
    }
}

// ============================================================================
// Cross-Feature Integration Tests
// ============================================================================

#[test]
fn test_structured_outputs_with_prompt_caching() {
    // Test that structured outputs and prompt caching can work together
    // Note: Using Claude model because prompt caching only works with Anthropic
    let structured_adapter = StructuredOutputsAdapter;
    let caching_adapter = PromptCachingAdapter;

    let schema = json!({
        "type": "object",
        "properties": {
            "answer": {"type": "string"}
        },
        "required": ["answer"]
    });

    let mut structured_params = HashMap::new();
    structured_params.insert("schema".to_string(), schema);

    let caching_params = HashMap::new();

    // Create request with Claude model (supports both features)
    let mut request = CompletionRequest {
        model: "claude-3-opus".to_string(),
        messages: vec![
            ChatMessage {
                role: "system".to_string(),
                content: ChatMessageContent::Text("You are a helpful assistant.".to_string()),
                tool_calls: None,
                tool_call_id: None,
                name: None,
            },
            ChatMessage {
                role: "user".to_string(),
                content: ChatMessageContent::Text("What is 2+2?".to_string()),
                tool_calls: None,
                tool_call_id: None,
                name: None,
            },
        ],
        temperature: None,
        max_tokens: None,
        stream: false,
        top_p: None,
        frequency_penalty: None,
        presence_penalty: None,
        stop: None,
        top_k: None,
        seed: None,
        repetition_penalty: None,
        extensions: None,
            logprobs: None,
            top_logprobs: None,
            response_format: None,
            tool_choice: None,
            tools: None,
        };

    // Apply both adapters
    let result1 = structured_adapter.adapt_request(&mut request, &structured_params);
    assert!(result1.is_ok(), "Structured outputs adapter should succeed");

    let result2 = caching_adapter.adapt_request(&mut request, &caching_params);
    assert!(result2.is_ok(), "Prompt caching adapter should succeed");

    // Verify both features are configured
    assert!(request.extensions.is_some());
    let extensions = request.extensions.unwrap();
    assert!(
        extensions.contains_key("_structured_outputs_schema"),
        "Should have structured outputs config"
    );
    assert!(
        extensions.contains_key("_prompt_caching_breakpoints"),
        "Should have caching config"
    );
}

#[test]
fn test_json_mode_with_logprobs() {
    // Test that JSON mode and logprobs can work together
    let json_adapter = JsonModeAdapter;
    let logprobs_adapter = LogprobsAdapter;

    let json_params = HashMap::new();

    let mut logprobs_params = HashMap::new();
    logprobs_params.insert("top_logprobs".to_string(), json!(3));

    // Create request
    let mut request = CompletionRequest {
        model: "gpt-4".to_string(),
        messages: vec![ChatMessage {
            role: "user".to_string(),
            content: ChatMessageContent::Text("Generate JSON".to_string()),
                tool_calls: None,
                tool_call_id: None,
                name: None,
        }],
        temperature: None,
        max_tokens: None,
        stream: false,
        top_p: None,
        frequency_penalty: None,
        presence_penalty: None,
        stop: None,
        top_k: None,
        seed: None,
        repetition_penalty: None,
        extensions: None,
            logprobs: None,
            top_logprobs: None,
            response_format: None,
            tool_choice: None,
            tools: None,
        };

    // Apply both adapters
    let result1 = json_adapter.adapt_request(&mut request, &json_params);
    assert!(result1.is_ok(), "JSON mode adapter should succeed");

    let result2 = logprobs_adapter.adapt_request(&mut request, &logprobs_params);
    assert!(result2.is_ok(), "Logprobs adapter should succeed");

    // Verify both features are configured
    assert!(request.extensions.is_some());
    let extensions = request.extensions.unwrap();
    assert!(
        extensions.contains_key("response_format"),
        "Should have JSON mode config"
    );
    assert!(
        extensions.contains_key("_logprobs_enabled"),
        "Should have logprobs config"
    );
    assert_eq!(extensions["_logprobs_top_count"], 3);
}
