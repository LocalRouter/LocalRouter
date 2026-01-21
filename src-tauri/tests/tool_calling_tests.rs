//! Tool calling integration tests
//!
//! Tests for OpenAI-compatible function calling (tools) across providers.

use localrouter_ai::providers::{
    ChatMessage, ChatMessageContent, CompletionRequest, FunctionDefinition, Tool, ToolChoice,
};
use serde_json::json;

#[test]
fn test_tool_definition_serialization() {
    let tool = Tool {
        tool_type: "function".to_string(),
        function: FunctionDefinition {
            name: "get_weather".to_string(),
            description: Some("Get the current weather in a location".to_string()),
            parameters: json!({
                "type": "object",
                "properties": {
                    "location": {
                        "type": "string",
                        "description": "The city and state, e.g. San Francisco, CA"
                    },
                    "unit": {
                        "type": "string",
                        "enum": ["celsius", "fahrenheit"]
                    }
                },
                "required": ["location"]
            }),
        },
    };

    let serialized = serde_json::to_string(&tool).unwrap();
    assert!(serialized.contains("get_weather"));
    assert!(serialized.contains("function"));

    // Verify it can be deserialized back
    let deserialized: Tool = serde_json::from_str(&serialized).unwrap();
    assert_eq!(deserialized.function.name, "get_weather");
}

#[test]
fn test_tool_choice_auto() {
    let choice = ToolChoice::Auto("auto".to_string());
    let serialized = serde_json::to_string(&choice).unwrap();
    assert_eq!(serialized, r#""auto""#);
}

#[test]
fn test_completion_request_with_tools() {
    let request = CompletionRequest {
        model: "gpt-4".to_string(),
        messages: vec![ChatMessage {
            role: "user".to_string(),
            content: ChatMessageContent::Text("What's the weather in San Francisco?".to_string()),
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
        tools: Some(vec![Tool {
            tool_type: "function".to_string(),
            function: FunctionDefinition {
                name: "get_weather".to_string(),
                description: Some("Get weather information".to_string()),
                parameters: json!({
                    "type": "object",
                    "properties": {
                        "location": {"type": "string"}
                    }
                }),
            },
        }]),
        tool_choice: Some(ToolChoice::Auto("auto".to_string())),
        response_format: None,
        logprobs: None,
        top_logprobs: None,
    };

    // Verify tools are present
    assert!(request.tools.is_some());
    assert_eq!(request.tools.as_ref().unwrap().len(), 1);
    assert!(request.tool_choice.is_some());
}

#[test]
fn test_chat_message_with_tool_calls() {
    let tool_call = localrouter_ai::providers::ToolCall {
        id: "call_abc123".to_string(),
        tool_type: "function".to_string(),
        function: localrouter_ai::providers::FunctionCall {
            name: "get_weather".to_string(),
            arguments: r#"{"location":"San Francisco, CA","unit":"fahrenheit"}"#.to_string(),
        },
    };

    let message = ChatMessage {
        role: "assistant".to_string(),
        content: ChatMessageContent::Text("".to_string()),
        tool_calls: Some(vec![tool_call]),
        tool_call_id: None,
        name: None,
    };

    assert_eq!(message.role, "assistant");
    assert!(message.tool_calls.is_some());
    assert_eq!(message.tool_calls.as_ref().unwrap().len(), 1);
    assert_eq!(
        message.tool_calls.as_ref().unwrap()[0].function.name,
        "get_weather"
    );
}

#[test]
fn test_tool_response_message() {
    let message = ChatMessage {
        role: "tool".to_string(),
        content: ChatMessageContent::Text(r#"{"temperature":72,"conditions":"sunny"}"#.to_string()),
        tool_calls: None,
        tool_call_id: Some("call_abc123".to_string()),
        name: Some("get_weather".to_string()),
    };

    assert_eq!(message.role, "tool");
    assert!(message.tool_call_id.is_some());
    assert_eq!(message.tool_call_id.as_ref().unwrap(), "call_abc123");
}

#[test]
fn test_parallel_tool_calls() {
    // Test that multiple tool calls can be made in a single response
    let tool_calls = vec![
        localrouter_ai::providers::ToolCall {
            id: "call_001".to_string(),
            tool_type: "function".to_string(),
            function: localrouter_ai::providers::FunctionCall {
                name: "get_weather".to_string(),
                arguments: r#"{"location":"San Francisco, CA"}"#.to_string(),
            },
        },
        localrouter_ai::providers::ToolCall {
            id: "call_002".to_string(),
            tool_type: "function".to_string(),
            function: localrouter_ai::providers::FunctionCall {
                name: "get_weather".to_string(),
                arguments: r#"{"location":"New York, NY"}"#.to_string(),
            },
        },
        localrouter_ai::providers::ToolCall {
            id: "call_003".to_string(),
            tool_type: "function".to_string(),
            function: localrouter_ai::providers::FunctionCall {
                name: "get_current_time".to_string(),
                arguments: r#"{"timezone":"America/Los_Angeles"}"#.to_string(),
            },
        },
    ];

    let message = ChatMessage {
        role: "assistant".to_string(),
        content: ChatMessageContent::Text("".to_string()),
        tool_calls: Some(tool_calls),
        tool_call_id: None,
        name: None,
    };

    // Verify multiple tool calls are present
    assert!(message.tool_calls.is_some());
    let calls = message.tool_calls.as_ref().unwrap();
    assert_eq!(calls.len(), 3);

    // Verify each tool call
    assert_eq!(calls[0].id, "call_001");
    assert_eq!(calls[0].function.name, "get_weather");
    assert_eq!(calls[1].id, "call_002");
    assert_eq!(calls[1].function.name, "get_weather");
    assert_eq!(calls[2].id, "call_003");
    assert_eq!(calls[2].function.name, "get_current_time");
}

#[test]
fn test_parallel_tool_responses() {
    // Test that responses for multiple tool calls can be sent back
    let responses = vec![
        ChatMessage {
            role: "tool".to_string(),
            content: ChatMessageContent::Text(
                r#"{"temperature":72,"conditions":"sunny"}"#.to_string(),
            ),
            tool_calls: None,
            tool_call_id: Some("call_001".to_string()),
            name: Some("get_weather".to_string()),
        },
        ChatMessage {
            role: "tool".to_string(),
            content: ChatMessageContent::Text(
                r#"{"temperature":45,"conditions":"rainy"}"#.to_string(),
            ),
            tool_calls: None,
            tool_call_id: Some("call_002".to_string()),
            name: Some("get_weather".to_string()),
        },
        ChatMessage {
            role: "tool".to_string(),
            content: ChatMessageContent::Text(
                r#"{"time":"2026-01-20T15:30:00-08:00"}"#.to_string(),
            ),
            tool_calls: None,
            tool_call_id: Some("call_003".to_string()),
            name: Some("get_current_time".to_string()),
        },
    ];

    // Verify all responses have correct structure
    assert_eq!(responses.len(), 3);
    for (i, response) in responses.iter().enumerate() {
        assert_eq!(response.role, "tool");
        assert!(response.tool_call_id.is_some());
        assert_eq!(
            response.tool_call_id.as_ref().unwrap(),
            &format!("call_00{}", i + 1)
        );
    }
}
