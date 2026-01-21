//! Provider-specific tool calling integration tests
//!
//! Tests for tool calling request/response structures.

use localrouter_ai::providers::{
    ChatMessage, ChatMessageContent, CompletionRequest, FunctionCall, FunctionDefinition, Tool,
    ToolCall, ToolChoice,
};
use serde_json::json;

#[test]
fn test_tool_call_message_structure() {
    // Test that ChatMessage correctly holds tool calls
    let message = ChatMessage {
        role: "assistant".to_string(),
        content: ChatMessageContent::Text("".to_string()),
        tool_calls: Some(vec![
            ToolCall {
                id: "call_1".to_string(),
                tool_type: "function".to_string(),
                function: FunctionCall {
                    name: "get_weather".to_string(),
                    arguments: r#"{"location":"San Francisco"}"#.to_string(),
                },
            },
            ToolCall {
                id: "call_2".to_string(),
                tool_type: "function".to_string(),
                function: FunctionCall {
                    name: "get_time".to_string(),
                    arguments: r#"{"timezone":"America/Los_Angeles"}"#.to_string(),
                },
            },
        ]),
        tool_call_id: None,
        name: None,
    };

    assert_eq!(message.role, "assistant");
    assert!(message.tool_calls.is_some());
    assert_eq!(message.tool_calls.as_ref().unwrap().len(), 2);
}

#[test]
fn test_tool_response_message_structure() {
    // Test that ChatMessage correctly represents tool responses
    let message = ChatMessage {
        role: "tool".to_string(),
        content: ChatMessageContent::Text(r#"{"temperature":72,"conditions":"sunny"}"#.to_string()),
        tool_calls: None,
        tool_call_id: Some("call_123".to_string()),
        name: Some("get_weather".to_string()),
    };

    assert_eq!(message.role, "tool");
    assert!(message.tool_call_id.is_some());
    assert_eq!(message.tool_call_id.as_ref().unwrap(), "call_123");
    assert_eq!(message.name.as_ref().unwrap(), "get_weather");
}

#[test]
fn test_completion_request_with_tools() {
    let request = CompletionRequest {
        model: "gpt-4".to_string(),
        messages: vec![ChatMessage {
            role: "user".to_string(),
            content: ChatMessageContent::Text("What's the weather?".to_string()),
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
                description: Some("Get current weather".to_string()),
                parameters: json!({
                    "type": "object",
                    "properties": {
                        "location": {"type": "string"}
                    },
                    "required": ["location"]
                }),
            },
        }]),
        tool_choice: Some(ToolChoice::Auto("auto".to_string())),
        response_format: None,
        logprobs: None,
        top_logprobs: None,
    };

    assert!(request.tools.is_some());
    assert_eq!(request.tools.as_ref().unwrap().len(), 1);
    assert!(request.tool_choice.is_some());
}

#[test]
fn test_tool_call_json_serialization() {
    let tool_call = ToolCall {
        id: "call_abc123".to_string(),
        tool_type: "function".to_string(),
        function: FunctionCall {
            name: "get_weather".to_string(),
            arguments: r#"{"location":"San Francisco","unit":"celsius"}"#.to_string(),
        },
    };

    let serialized = serde_json::to_string(&tool_call).unwrap();
    assert!(serialized.contains("call_abc123"));
    assert!(serialized.contains("get_weather"));
    assert!(serialized.contains("function"));

    // Verify it can be deserialized back
    let deserialized: ToolCall = serde_json::from_str(&serialized).unwrap();
    assert_eq!(deserialized.id, "call_abc123");
    assert_eq!(deserialized.function.name, "get_weather");
}

#[test]
fn test_tool_definition_serialization() {
    let tool = Tool {
        tool_type: "function".to_string(),
        function: FunctionDefinition {
            name: "calculate".to_string(),
            description: Some("Perform calculation".to_string()),
            parameters: json!({
                "type": "object",
                "properties": {
                    "expression": {"type": "string"}
                }
            }),
        },
    };

    let serialized = serde_json::to_string(&tool).unwrap();
    let deserialized: Tool = serde_json::from_str(&serialized).unwrap();
    assert_eq!(deserialized.function.name, "calculate");
}

#[test]
fn test_message_with_both_content_and_tool_calls() {
    // Some models may return both text content and tool calls
    let message = ChatMessage {
        role: "assistant".to_string(),
        content: ChatMessageContent::Text("Let me check that for you.".to_string()),
        tool_calls: Some(vec![ToolCall {
            id: "call_1".to_string(),
            tool_type: "function".to_string(),
            function: FunctionCall {
                name: "search".to_string(),
                arguments: r#"{"query":"weather"}"#.to_string(),
            },
        }]),
        tool_call_id: None,
        name: None,
    };

    assert!(!message.content.as_text().is_empty());
    assert!(message.tool_calls.is_some());
}

#[test]
fn test_conversation_with_tool_calling_flow() {
    // Simulate a full conversation with tool calling
    let messages = vec![
        // 1. User asks a question
        ChatMessage {
            role: "user".to_string(),
            content: ChatMessageContent::Text("What's the weather in Tokyo?".to_string()),
            tool_calls: None,
            tool_call_id: None,
            name: None,
        },
        // 2. Assistant decides to use a tool
        ChatMessage {
            role: "assistant".to_string(),
            content: ChatMessageContent::Text("".to_string()),
            tool_calls: Some(vec![ToolCall {
                id: "call_tokyo".to_string(),
                tool_type: "function".to_string(),
                function: FunctionCall {
                    name: "get_weather".to_string(),
                    arguments: r#"{"location":"Tokyo","unit":"celsius"}"#.to_string(),
                },
            }]),
            tool_call_id: None,
            name: None,
        },
        // 3. Tool returns a result
        ChatMessage {
            role: "tool".to_string(),
            content: ChatMessageContent::Text(
                r#"{"temperature":18,"conditions":"cloudy"}"#.to_string(),
            ),
            tool_calls: None,
            tool_call_id: Some("call_tokyo".to_string()),
            name: Some("get_weather".to_string()),
        },
        // 4. Assistant synthesizes the response
        ChatMessage {
            role: "assistant".to_string(),
            content: ChatMessageContent::Text(
                "In Tokyo, it's currently 18°C and cloudy.".to_string(),
            ),
            tool_calls: None,
            tool_call_id: None,
            name: None,
        },
    ];

    // Verify the conversation structure
    assert_eq!(messages.len(), 4);
    assert_eq!(messages[0].role, "user");
    assert_eq!(messages[1].role, "assistant");
    assert!(messages[1].tool_calls.is_some());
    assert_eq!(messages[2].role, "tool");
    assert!(messages[2].tool_call_id.is_some());
    assert_eq!(messages[3].role, "assistant");
    assert!(messages[3].tool_calls.is_none());
}
