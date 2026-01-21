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
        content: ChatMessageContent::Text(
            r#"{"temperature":72,"conditions":"sunny"}"#.to_string(),
        ),
        tool_calls: None,
        tool_call_id: Some("call_abc123".to_string()),
        name: Some("get_weather".to_string()),
    };

    assert_eq!(message.role, "tool");
    assert!(message.tool_call_id.is_some());
    assert_eq!(message.tool_call_id.as_ref().unwrap(), "call_abc123");
}
