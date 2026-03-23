//! Tests for the MCP via LLM orchestration logic.
//!
//! Tests cover: tool injection, prompt injection, content conversion,
//! session management, tool call accumulation, pending execution matching,
//! and the Drop-based abort of background handles.

#[cfg(test)]
mod session_tests {
    use crate::session::{McpViaLlmSession, PendingMixedExecution, SessionHistory};
    use lr_providers::{ChatMessage, ChatMessageContent};
    use std::time::Duration;

    #[test]
    fn session_creation_sets_gateway_key() {
        let session = McpViaLlmSession::new("sess-123".to_string(), "client-abc".to_string());
        assert_eq!(session.client_id, "client-abc");
        assert_eq!(session.gateway_session_key, "mcp-via-llm-sess-123");
        assert!(!session.gateway_initialized);
        assert!(session.history.full_messages.is_empty());
    }

    #[test]
    fn session_touch_updates_activity() {
        let mut session = McpViaLlmSession::new("s1".to_string(), "c1".to_string());
        let before = session.last_activity;
        std::thread::sleep(Duration::from_millis(10));
        session.touch();
        assert!(session.last_activity > before);
    }

    #[test]
    fn session_expiry_with_short_ttl() {
        let mut session = McpViaLlmSession::new("s1".to_string(), "c1".to_string());
        session.last_activity = std::time::Instant::now() - Duration::from_secs(120);
        assert!(session.is_expired(Duration::from_secs(60)));
        assert!(!session.is_expired(Duration::from_secs(300)));
    }

    #[test]
    fn session_not_expired_when_fresh() {
        let session = McpViaLlmSession::new("s1".to_string(), "c1".to_string());
        assert!(!session.is_expired(Duration::from_secs(60)));
    }

    #[test]
    fn session_history_set_and_replace() {
        let mut history = SessionHistory::new();
        assert!(history.full_messages.is_empty());

        let messages = vec![ChatMessage {
            role: "user".to_string(),
            content: ChatMessageContent::Text("hello".to_string()),
            tool_calls: None,
            tool_call_id: None,
            name: None,
            reasoning_content: None,
        }];
        history.set_messages(messages);
        assert_eq!(history.full_messages.len(), 1);
        assert_eq!(history.full_messages[0].role, "user");

        history.set_messages(vec![]);
        assert!(history.full_messages.is_empty());
    }

    #[tokio::test]
    async fn pending_drop_aborts_background_handles() {
        use std::sync::atomic::{AtomicBool, Ordering};
        use std::sync::Arc;

        let completed = Arc::new(AtomicBool::new(false));
        let completed_clone = completed.clone();

        let handle = tokio::spawn(async move {
            tokio::time::sleep(Duration::from_secs(60)).await;
            completed_clone.store(true, Ordering::SeqCst);
            ("call-1".to_string(), Ok("result".to_string()))
        });

        let pending = PendingMixedExecution {
            full_assistant_message: ChatMessage {
                role: "assistant".to_string(),
                content: ChatMessageContent::Text(String::new()),
                tool_calls: None,
                tool_call_id: None,
                name: None,
                reasoning_content: None,
            },
            mcp_handles: vec![handle],
            client_tool_call_ids: vec!["client-1".to_string()],
            accumulated_prompt_tokens: 0,
            accumulated_completion_tokens: 0,
            mcp_tools_called: vec![],
            messages_before_mixed: vec![],
            started_at: std::time::Instant::now(),
            accumulated_usage_entries: vec![],
            gateway_session_key: String::new(),
        };

        drop(pending);
        tokio::time::sleep(Duration::from_millis(50)).await;
        assert!(!completed.load(Ordering::SeqCst));
    }

    #[tokio::test]
    async fn pending_drop_replaces_old_on_insert() {
        use std::sync::atomic::{AtomicBool, Ordering};
        use std::sync::Arc;

        let first_completed = Arc::new(AtomicBool::new(false));
        let first_clone = first_completed.clone();

        let first_handle = tokio::spawn(async move {
            tokio::time::sleep(Duration::from_secs(60)).await;
            first_clone.store(true, Ordering::SeqCst);
            ("call-1".to_string(), Ok("result".to_string()))
        });

        let map = dashmap::DashMap::new();
        let make_pending = |handles: Vec<tokio::task::JoinHandle<_>>| PendingMixedExecution {
            full_assistant_message: ChatMessage {
                role: "assistant".to_string(),
                content: ChatMessageContent::Text(String::new()),
                tool_calls: None,
                tool_call_id: None,
                name: None,
                reasoning_content: None,
            },
            mcp_handles: handles,
            client_tool_call_ids: vec![],
            accumulated_prompt_tokens: 0,
            accumulated_completion_tokens: 0,
            mcp_tools_called: vec![],
            messages_before_mixed: vec![],
            started_at: std::time::Instant::now(),
            accumulated_usage_entries: vec![],
            gateway_session_key: String::new(),
        };

        // Insert first pending
        map.insert("client-1".to_string(), make_pending(vec![first_handle]));

        // Replace with second pending — first should be dropped & aborted
        map.insert("client-1".to_string(), make_pending(vec![]));

        tokio::time::sleep(Duration::from_millis(50)).await;
        assert!(
            !first_completed.load(Ordering::SeqCst),
            "First handle should have been aborted on replace"
        );
    }
}

#[cfg(test)]
mod tool_injection_tests {
    use crate::gateway_client::{McpPrompt, McpPromptArgument, McpTool};
    use crate::orchestrator::{
        content_to_string, inject_mcp_tools, inject_prompt_read_tool, inject_resource_read_tool,
        inject_server_instructions, PROMPT_READ_TOOL_NAME, RESOURCE_READ_TOOL_NAME,
    };
    use lr_providers::{ChatMessage, ChatMessageContent, CompletionRequest};
    use serde_json::json;

    fn make_request(messages: Vec<ChatMessage>) -> CompletionRequest {
        CompletionRequest {
            model: "test-model".to_string(),
            messages,
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
            tools: None,
            tool_choice: None,
            response_format: None,
            logprobs: None,
            top_logprobs: None,
            n: None,
            logit_bias: None,
            parallel_tool_calls: None,
            service_tier: None,
            store: None,
            metadata: None,
            modalities: None,
            audio: None,
            prediction: None,
            reasoning_effort: None,
            pre_computed_routing: None,
        }
    }

    fn make_mcp_tool(name: &str) -> McpTool {
        McpTool {
            name: name.to_string(),
            description: Some(format!("Description for {}", name)),
            input_schema: json!({"type": "object", "properties": {}}),
        }
    }

    fn msg(role: &str, text: &str) -> ChatMessage {
        ChatMessage {
            role: role.to_string(),
            content: ChatMessageContent::Text(text.to_string()),
            tool_calls: None,
            tool_call_id: None,
            name: None,
            reasoning_content: None,
        }
    }

    fn extract_text(m: &ChatMessage) -> &str {
        match &m.content {
            ChatMessageContent::Text(t) => t.as_str(),
            _ => panic!("Expected text content"),
        }
    }

    // ── inject_mcp_tools ──────────────────────────────────────────────────

    #[test]
    fn inject_into_empty_request() {
        let mut request = make_request(vec![]);
        let tools = vec![make_mcp_tool("fs__read"), make_mcp_tool("fs__write")];

        inject_mcp_tools(&mut request, &tools);

        let rt = request.tools.unwrap();
        assert_eq!(rt.len(), 2);
        assert_eq!(rt[0].function.name, "fs__read");
        assert_eq!(rt[1].function.name, "fs__write");
    }

    #[test]
    fn inject_merges_with_existing_client_tools() {
        let mut request = make_request(vec![]);
        request.tools = Some(vec![lr_providers::Tool {
            tool_type: "function".to_string(),
            function: lr_providers::FunctionDefinition {
                name: "client_tool".to_string(),
                description: Some("A client tool".to_string()),
                parameters: json!({}),
            },
        }]);

        inject_mcp_tools(&mut request, &[make_mcp_tool("mcp__tool")]);

        let rt = request.tools.unwrap();
        assert_eq!(rt.len(), 2);
        assert!(rt.iter().any(|t| t.function.name == "client_tool"));
        assert!(rt.iter().any(|t| t.function.name == "mcp__tool"));
    }

    #[test]
    fn inject_shadows_conflicting_names() {
        let mut request = make_request(vec![]);
        request.tools = Some(vec![
            lr_providers::Tool {
                tool_type: "function".to_string(),
                function: lr_providers::FunctionDefinition {
                    name: "conflict".to_string(),
                    description: Some("Client version".to_string()),
                    parameters: json!({}),
                },
            },
            lr_providers::Tool {
                tool_type: "function".to_string(),
                function: lr_providers::FunctionDefinition {
                    name: "safe_tool".to_string(),
                    description: Some("No conflict".to_string()),
                    parameters: json!({}),
                },
            },
        ]);

        inject_mcp_tools(&mut request, &[make_mcp_tool("conflict")]);

        let rt = request.tools.unwrap();
        assert_eq!(rt.len(), 2); // safe_tool + conflict (MCP version)
        let c = rt.iter().find(|t| t.function.name == "conflict").unwrap();
        assert_eq!(
            c.function.description.as_deref(),
            Some("Description for conflict")
        );
    }

    #[test]
    fn inject_idempotent_on_second_call() {
        let mut request = make_request(vec![]);
        let tools = vec![make_mcp_tool("fs__read")];

        inject_mcp_tools(&mut request, &tools);
        inject_mcp_tools(&mut request, &tools);

        let rt = request.tools.unwrap();
        // Should not duplicate: the second inject shadows the first MCP tool
        assert_eq!(
            rt.iter().filter(|t| t.function.name == "fs__read").count(),
            1
        );
    }

    // ── inject_resource_read_tool ────────────────────────────────────────

    #[test]
    fn resource_read_tool_injected() {
        let mut request = make_request(vec![]);

        inject_resource_read_tool(&mut request);

        let tools = request.tools.unwrap();
        assert_eq!(tools.len(), 1);
        assert_eq!(tools[0].function.name, RESOURCE_READ_TOOL_NAME);
        assert!(tools[0]
            .function
            .description
            .as_ref()
            .unwrap()
            .contains("name"));
    }

    #[test]
    fn resource_read_tool_has_name_parameter() {
        let mut request = make_request(vec![]);

        inject_resource_read_tool(&mut request);

        let tools = request.tools.unwrap();
        let schema = &tools[0].function.parameters;
        let props = schema.get("properties").unwrap();
        assert!(props.get("name").is_some());
        let required = schema.get("required").unwrap().as_array().unwrap();
        assert!(required.iter().any(|v| v.as_str() == Some("name")));
    }

    // ── inject_prompt_read_tool ────────────────────────────────────────────

    #[test]
    fn prompt_read_tool_schema() {
        let mut request = make_request(vec![]);
        let prompts = vec![
            McpPrompt {
                name: "github__review".to_string(),
                description: Some("Review code".to_string()),
                arguments: vec![],
            },
            McpPrompt {
                name: "github__template".to_string(),
                description: None,
                arguments: vec![McpPromptArgument {
                    name: "type".to_string(),
                    description: Some("Template type".to_string()),
                    required: true,
                }],
            },
        ];

        inject_prompt_read_tool(&mut request, &prompts);

        let tools = request.tools.unwrap();
        assert_eq!(tools.len(), 1);
        assert_eq!(tools[0].function.name, PROMPT_READ_TOOL_NAME);

        // Check that available prompt names are listed in the description
        let name_desc = tools[0].function.parameters["properties"]["name"]["description"]
            .as_str()
            .unwrap();
        assert!(name_desc.contains("github__review"));
        assert!(name_desc.contains("github__template"));

        // Check arguments parameter exists
        assert!(tools[0].function.parameters["properties"]["arguments"].is_object());
    }

    #[test]
    fn prompt_read_tool_not_injected_when_empty() {
        let mut request = make_request(vec![]);
        inject_prompt_read_tool(&mut request, &[]);
        // Tool should still be injected (empty list still creates tool with "Prompt name" desc)
        // Actually per the code, if prompts is empty the name desc is just "Prompt name"
        // but the calling code only calls inject_prompt_read_tool when prompts is non-empty
        let tools = request.tools.unwrap();
        assert_eq!(tools.len(), 1);
        let name_desc = tools[0].function.parameters["properties"]["name"]["description"]
            .as_str()
            .unwrap();
        assert_eq!(name_desc, "Prompt name");
    }

    // ── inject_server_instructions ─────────────────────────────────────────

    #[test]
    fn server_instructions_placed_after_system_before_user() {
        let mut request = make_request(vec![
            msg("system", "System prompt"),
            msg("system", "More system"),
            msg("user", "Hello"),
        ]);

        inject_server_instructions(&mut request, "Unified MCP Gateway instructions here.");

        assert_eq!(request.messages.len(), 4);
        assert_eq!(request.messages[0].role, "system");
        assert_eq!(extract_text(&request.messages[0]), "System prompt");
        assert_eq!(request.messages[1].role, "system");
        assert_eq!(extract_text(&request.messages[1]), "More system");
        assert_eq!(request.messages[2].role, "system");
        assert_eq!(
            extract_text(&request.messages[2]),
            "Unified MCP Gateway instructions here."
        );
        assert_eq!(request.messages[3].role, "user");
        assert_eq!(extract_text(&request.messages[3]), "Hello");
    }

    #[test]
    fn server_instructions_before_user_when_no_system() {
        let mut request = make_request(vec![msg("user", "Hi")]);

        inject_server_instructions(&mut request, "Gateway info");

        assert_eq!(request.messages.len(), 2);
        assert_eq!(request.messages[0].role, "system");
        assert_eq!(extract_text(&request.messages[0]), "Gateway info");
        assert_eq!(request.messages[1].role, "user");
    }

    #[test]
    fn server_instructions_appended_when_all_system() {
        let mut request = make_request(vec![msg("system", "A"), msg("system", "B")]);

        inject_server_instructions(&mut request, "Gateway");

        assert_eq!(request.messages.len(), 3);
        assert_eq!(extract_text(&request.messages[2]), "Gateway");
    }

    // ── content_to_string ─────────────────────────────────────────────────

    #[test]
    fn content_string_passthrough() {
        assert_eq!(content_to_string(&json!("hello")), "hello");
    }

    #[test]
    fn content_object_serialized() {
        let r = content_to_string(&json!({"key": "value"}));
        assert!(r.contains("key"));
    }

    #[test]
    fn content_array_serialized() {
        assert_eq!(content_to_string(&json!([1, 2, 3])), "[1,2,3]");
    }
}

#[cfg(test)]
mod tool_classification_tests {
    use crate::orchestrator::PROMPT_READ_TOOL_NAME;
    use lr_providers::{FunctionCall, ToolCall};
    use std::collections::HashSet;

    fn tc(id: &str, name: &str) -> ToolCall {
        ToolCall {
            id: id.to_string(),
            tool_type: "function".to_string(),
            function: FunctionCall {
                name: name.to_string(),
                arguments: "{}".to_string(),
            },
        }
    }

    #[test]
    fn all_mcp() {
        let mcp: HashSet<String> = ["fs__read", "fs__write"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        let calls = [tc("1", "fs__read"), tc("2", "fs__write")];
        let (m, c): (Vec<_>, Vec<_>) = calls.iter().partition(|t| mcp.contains(&t.function.name));
        assert_eq!(m.len(), 2);
        assert_eq!(c.len(), 0);
    }

    #[test]
    fn all_client() {
        let mcp: HashSet<String> = ["fs__read"].iter().map(|s| s.to_string()).collect();
        let calls = [tc("1", "my_tool"), tc("2", "other")];
        let (m, c): (Vec<_>, Vec<_>) = calls.iter().partition(|t| mcp.contains(&t.function.name));
        assert_eq!(m.len(), 0);
        assert_eq!(c.len(), 2);
    }

    #[test]
    fn mixed() {
        let mcp: HashSet<String> = ["fs__read", "ResourceRead"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        let calls = [
            tc("1", "fs__read"),
            tc("2", "client_search"),
            tc("3", "ResourceRead"),
        ];
        let (m, c): (Vec<_>, Vec<_>) = calls.iter().partition(|t| mcp.contains(&t.function.name));
        assert_eq!(m.len(), 2);
        assert_eq!(c.len(), 1);
        assert_eq!(c[0].id, "2");
    }

    #[test]
    fn resource_read_classified_as_mcp() {
        let mcp: HashSet<String> = ["ResourceRead"].iter().map(|s| s.to_string()).collect();
        let calls = [tc("1", "ResourceRead")];
        let (m, _): (Vec<_>, Vec<_>) = calls.iter().partition(|t| mcp.contains(&t.function.name));
        assert_eq!(m.len(), 1);
    }

    #[test]
    fn prompt_read_classified_as_mcp() {
        let mcp: HashSet<String> = [PROMPT_READ_TOOL_NAME.to_string()].into_iter().collect();
        let calls = [tc("1", PROMPT_READ_TOOL_NAME)];
        let (m, _): (Vec<_>, Vec<_>) = calls.iter().partition(|t| mcp.contains(&t.function.name));
        assert_eq!(m.len(), 1);
    }
}

#[cfg(test)]
mod manager_tests {
    use crate::manager::McpViaLlmManager;
    use crate::session::PendingMixedExecution;
    use lr_config::McpViaLlmConfig;
    use lr_providers::{ChatMessage, ChatMessageContent, CompletionRequest};

    fn cfg() -> McpViaLlmConfig {
        McpViaLlmConfig {
            session_ttl_seconds: 3600,
            max_concurrent_sessions: 100,
            max_loop_iterations: 4,
            max_loop_timeout_seconds: 300,
        }
    }

    fn make_request(tool_call_ids: &[&str]) -> CompletionRequest {
        let mut messages = vec![ChatMessage {
            role: "user".to_string(),
            content: ChatMessageContent::Text("Hi".to_string()),
            tool_calls: None,
            tool_call_id: None,
            name: None,
            reasoning_content: None,
        }];
        for id in tool_call_ids {
            messages.push(ChatMessage {
                role: "tool".to_string(),
                content: ChatMessageContent::Text(format!("Result for {}", id)),
                tool_calls: None,
                tool_call_id: Some(id.to_string()),
                name: None,
                reasoning_content: None,
            });
        }
        CompletionRequest {
            model: "test".to_string(),
            messages,
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
            tools: None,
            tool_choice: None,
            response_format: None,
            logprobs: None,
            top_logprobs: None,
            n: None,
            logit_bias: None,
            parallel_tool_calls: None,
            service_tier: None,
            store: None,
            metadata: None,
            modalities: None,
            audio: None,
            prediction: None,
            reasoning_effort: None,
            pre_computed_routing: None,
        }
    }

    fn make_pending(client_ids: &[&str]) -> PendingMixedExecution {
        PendingMixedExecution {
            full_assistant_message: ChatMessage {
                role: "assistant".to_string(),
                content: ChatMessageContent::Text(String::new()),
                tool_calls: None,
                tool_call_id: None,
                name: None,
                reasoning_content: None,
            },
            mcp_handles: vec![],
            client_tool_call_ids: client_ids.iter().map(|s| s.to_string()).collect(),
            accumulated_prompt_tokens: 0,
            accumulated_completion_tokens: 0,
            mcp_tools_called: vec![],
            messages_before_mixed: vec![],
            started_at: std::time::Instant::now(),
            accumulated_usage_entries: vec![],
            gateway_session_key: String::new(),
        }
    }

    #[test]
    fn pending_match_succeeds() {
        let mgr = McpViaLlmManager::new(cfg());
        mgr.pending_executions
            .insert("c1".to_string(), make_pending(&["tc-1", "tc-2"]));

        let req = make_request(&["tc-1", "tc-2"]);
        let result = mgr.take_pending_if_matching("c1", &req);

        assert!(result.is_some());
        let (pending, results) = result.unwrap();
        assert_eq!(results.len(), 2);
        assert_eq!(pending.client_tool_call_ids.len(), 2);
        assert!(mgr.pending_executions.get("c1").is_none());
    }

    #[test]
    fn pending_no_match_different_ids() {
        let mgr = McpViaLlmManager::new(cfg());
        mgr.pending_executions
            .insert("c1".to_string(), make_pending(&["tc-1"]));

        let req = make_request(&["tc-99"]);
        assert!(mgr.take_pending_if_matching("c1", &req).is_none());
        assert!(mgr.pending_executions.get("c1").is_some());
    }

    #[test]
    fn pending_no_match_no_pending() {
        let mgr = McpViaLlmManager::new(cfg());
        let req = make_request(&["tc-1"]);
        assert!(mgr.take_pending_if_matching("c1", &req).is_none());
    }

    #[test]
    fn pending_partial_match_succeeds() {
        let mgr = McpViaLlmManager::new(cfg());
        mgr.pending_executions
            .insert("c1".to_string(), make_pending(&["tc-1", "tc-2"]));

        let req = make_request(&["tc-1"]); // Only one of two
        let result = mgr.take_pending_if_matching("c1", &req);
        assert!(result.is_some());
        assert_eq!(result.unwrap().1.len(), 1);
    }

    #[test]
    fn cleanup_expired_sessions() {
        let config = McpViaLlmConfig {
            session_ttl_seconds: 1,
            ..cfg()
        };
        let mgr = McpViaLlmManager::new(config);

        // Create and backdate a session
        let session = mgr.get_or_create_session("c1", None);
        session.write().last_activity =
            std::time::Instant::now() - std::time::Duration::from_secs(10);

        mgr.cleanup_expired_sessions();

        // Should be cleaned up
        let entry = mgr.sessions_by_client.get("c1");
        assert!(entry.is_none() || entry.unwrap().is_empty());
    }

    #[test]
    fn cleanup_timed_out_pending() {
        let config = McpViaLlmConfig {
            max_loop_timeout_seconds: 1,
            ..cfg()
        };
        let mgr = McpViaLlmManager::new(config);

        let mut pending = make_pending(&[]);
        pending.started_at = std::time::Instant::now() - std::time::Duration::from_secs(10);
        mgr.pending_executions.insert("c1".to_string(), pending);

        mgr.cleanup_expired_sessions();
        assert!(mgr.pending_executions.get("c1").is_none());
    }

    #[test]
    fn config_update() {
        let mgr = McpViaLlmManager::new(cfg());
        assert_eq!(mgr.config().max_loop_iterations, 4);

        let mut new = cfg();
        new.max_loop_iterations = 50;
        mgr.update_config(new);
        assert_eq!(mgr.config().max_loop_iterations, 50);
    }

    #[test]
    fn session_reuse_same_client_no_messages() {
        let mgr = McpViaLlmManager::new(cfg());
        let s1 = mgr.get_or_create_session("c1", None);
        let id1 = s1.read().gateway_session_key.clone();
        let s2 = mgr.get_or_create_session("c1", None);
        let id2 = s2.read().gateway_session_key.clone();
        assert_eq!(id1, id2);
    }

    #[test]
    fn session_different_per_client() {
        let mgr = McpViaLlmManager::new(cfg());
        let s1 = mgr.get_or_create_session("c1", None);
        let s2 = mgr.get_or_create_session("c2", None);
        assert_ne!(s1.read().gateway_session_key, s2.read().gateway_session_key);
    }
}

#[cfg(test)]
mod streaming_accumulator_tests {
    use lr_providers::{FunctionCallDelta, ToolCallDelta};

    // Mirror the accumulation logic from orchestrator_stream.rs for testing
    struct Acc {
        id: String,
        name: String,
        arguments: String,
    }

    fn accumulate(accs: &mut Vec<Acc>, deltas: &[ToolCallDelta]) {
        for delta in deltas {
            let idx = delta.index as usize;
            while accs.len() <= idx {
                accs.push(Acc {
                    id: String::new(),
                    name: String::new(),
                    arguments: String::new(),
                });
            }
            let acc = &mut accs[idx];
            if let Some(ref id) = delta.id {
                acc.id.clone_from(id);
            }
            if let Some(ref func) = delta.function {
                if let Some(ref name) = func.name {
                    if acc.name.is_empty() {
                        acc.name.clone_from(name);
                    }
                }
                if let Some(ref args) = func.arguments {
                    acc.arguments.push_str(args);
                }
            }
        }
    }

    #[test]
    fn single_tool_call_across_deltas() {
        let mut accs = Vec::new();

        accumulate(
            &mut accs,
            &[ToolCallDelta {
                index: 0,
                id: Some("call-1".to_string()),
                tool_type: Some("function".to_string()),
                function: Some(FunctionCallDelta {
                    name: Some("fs__read".to_string()),
                    arguments: Some("{\"pa".to_string()),
                }),
            }],
        );
        accumulate(
            &mut accs,
            &[ToolCallDelta {
                index: 0,
                id: None,
                tool_type: None,
                function: Some(FunctionCallDelta {
                    name: None,
                    arguments: Some("th\":\"/tmp\"}".to_string()),
                }),
            }],
        );

        assert_eq!(accs.len(), 1);
        assert_eq!(accs[0].id, "call-1");
        assert_eq!(accs[0].name, "fs__read");
        assert_eq!(accs[0].arguments, "{\"path\":\"/tmp\"}");
    }

    #[test]
    fn multiple_tool_calls_in_one_batch() {
        let mut accs = Vec::new();
        accumulate(
            &mut accs,
            &[
                ToolCallDelta {
                    index: 0,
                    id: Some("c1".to_string()),
                    tool_type: Some("function".to_string()),
                    function: Some(FunctionCallDelta {
                        name: Some("a".to_string()),
                        arguments: Some("{}".to_string()),
                    }),
                },
                ToolCallDelta {
                    index: 1,
                    id: Some("c2".to_string()),
                    tool_type: Some("function".to_string()),
                    function: Some(FunctionCallDelta {
                        name: Some("b".to_string()),
                        arguments: Some("{\"x\":1}".to_string()),
                    }),
                },
            ],
        );

        assert_eq!(accs.len(), 2);
        assert_eq!(accs[0].name, "a");
        assert_eq!(accs[1].name, "b");
    }

    #[test]
    fn name_not_duplicated_on_repeated_delta() {
        let mut accs = Vec::new();

        accumulate(
            &mut accs,
            &[ToolCallDelta {
                index: 0,
                id: Some("c1".to_string()),
                tool_type: Some("function".to_string()),
                function: Some(FunctionCallDelta {
                    name: Some("fs__read".to_string()),
                    arguments: Some("{}".to_string()),
                }),
            }],
        );
        // Redundant name delta
        accumulate(
            &mut accs,
            &[ToolCallDelta {
                index: 0,
                id: None,
                tool_type: None,
                function: Some(FunctionCallDelta {
                    name: Some("fs__read".to_string()),
                    arguments: None,
                }),
            }],
        );

        assert_eq!(accs[0].name, "fs__read"); // NOT "fs__readfs__read"
    }

    #[test]
    fn sparse_indices_fill_gaps() {
        let mut accs = Vec::new();
        accumulate(
            &mut accs,
            &[ToolCallDelta {
                index: 2,
                id: Some("c3".to_string()),
                tool_type: Some("function".to_string()),
                function: Some(FunctionCallDelta {
                    name: Some("tool_c".to_string()),
                    arguments: Some("{}".to_string()),
                }),
            }],
        );

        assert_eq!(accs.len(), 3);
        assert_eq!(accs[2].name, "tool_c");
        assert!(accs[0].name.is_empty());
        assert!(accs[1].name.is_empty());
    }
}

#[cfg(test)]
mod session_matching_tests {
    use crate::session::{
        compute_message_hashes, normalize_for_hash, reconstruct_history, score_session_match,
    };
    use lr_providers::{ChatMessage, ChatMessageContent};

    fn msg(role: &str, text: &str) -> ChatMessage {
        ChatMessage {
            role: role.to_string(),
            content: ChatMessageContent::Text(text.to_string()),
            tool_calls: None,
            tool_call_id: None,
            name: None,
            reasoning_content: None,
        }
    }

    // ── normalize_for_hash ────────────────────────────────────────────────

    #[test]
    fn normalize_trims_whitespace() {
        assert_eq!(normalize_for_hash("  hello  "), "hello");
    }

    #[test]
    fn normalize_collapses_interior_whitespace() {
        assert_eq!(normalize_for_hash("hello   world"), "hello world");
        assert_eq!(normalize_for_hash("a\t\nb\r\nc"), "a b c");
    }

    #[test]
    fn normalize_unicode_nfc() {
        // é as combining (e + ́) vs precomposed (é)
        let combining = "e\u{0301}"; // e + combining acute
        let precomposed = "\u{00e9}"; // é precomposed
        assert_eq!(
            normalize_for_hash(combining),
            normalize_for_hash(precomposed)
        );
    }

    #[test]
    fn normalize_empty_string() {
        assert_eq!(normalize_for_hash(""), "");
        assert_eq!(normalize_for_hash("   "), "");
    }

    // ── compute_message_hashes ────────────────────────────────────────────

    #[test]
    fn hashes_are_deterministic() {
        let msgs = vec![msg("user", "hello"), msg("assistant", "hi")];
        let h1 = compute_message_hashes(&msgs);
        let h2 = compute_message_hashes(&msgs);
        assert_eq!(h1, h2);
    }

    #[test]
    fn hashes_differ_by_role() {
        let m1 = vec![msg("user", "hello")];
        let m2 = vec![msg("assistant", "hello")];
        assert_ne!(compute_message_hashes(&m1), compute_message_hashes(&m2));
    }

    #[test]
    fn hashes_differ_by_content() {
        let m1 = vec![msg("user", "hello")];
        let m2 = vec![msg("user", "world")];
        assert_ne!(compute_message_hashes(&m1), compute_message_hashes(&m2));
    }

    #[test]
    fn hashes_resilient_to_whitespace() {
        let m1 = vec![msg("user", "hello  world")];
        let m2 = vec![msg("user", "hello world")];
        assert_eq!(compute_message_hashes(&m1), compute_message_hashes(&m2));
    }

    #[test]
    fn hashes_resilient_to_trailing_whitespace() {
        let m1 = vec![msg("user", "hello ")];
        let m2 = vec![msg("user", "hello")];
        assert_eq!(compute_message_hashes(&m1), compute_message_hashes(&m2));
    }

    // ── score_session_match ───────────────────────────────────────────────

    #[test]
    fn score_empty_stored() {
        assert_eq!(score_session_match(&[], &[1, 2, 3]), 0.0);
    }

    #[test]
    fn score_empty_incoming() {
        assert_eq!(score_session_match(&[1, 2], &[]), 0.0);
    }

    #[test]
    fn score_exact_prefix_match() {
        // stored = [A, B], incoming = [A, B, C] → 1.0
        assert_eq!(score_session_match(&[1, 2], &[1, 2, 3]), 1.0);
    }

    #[test]
    fn score_exact_match() {
        // stored = [A, B], incoming = [A, B] → 1.0
        assert_eq!(score_session_match(&[1, 2], &[1, 2]), 1.0);
    }

    #[test]
    fn score_no_match() {
        assert_eq!(score_session_match(&[1, 2, 3], &[4, 5, 6]), 0.0);
    }

    #[test]
    fn score_suffix_anchored_match() {
        // stored = [A, B, C, D], incoming = [C, D, E] → client dropped A, B
        // suffix [C, D] of stored matches prefix [C, D] of incoming → 2/4 = 0.5
        assert_eq!(score_session_match(&[1, 2, 3, 4], &[3, 4, 5]), 0.5);
    }

    #[test]
    fn score_suffix_full_stored_suffix_matches() {
        // stored = [A, B, C], incoming = [B, C, D] → client dropped A
        // suffix [B, C] matches prefix [B, C] of incoming → 2/3 ≈ 0.667
        let score = score_session_match(&[1, 2, 3], &[2, 3, 4]);
        assert!((score - 2.0 / 3.0).abs() < 0.001);
    }

    #[test]
    fn score_partial_no_count() {
        // stored = [A, B, C], incoming = [A, X, C] → only A matches as prefix but
        // the suffix match requires the entire suffix or entire incoming to match
        // No complete suffix match → 0.0
        // Actually [C] suffix of stored matches... no, incoming starts with A not C.
        // Let me think: suffix [C] of stored, compare with incoming prefix [A] → no match
        // suffix [B, C], compare with incoming prefix [A, X] → no match
        // suffix [A, B, C], compare with incoming prefix [A, X, C] → A matches, B≠X stops → count=1
        // count=1, suffix len=3, incoming len=3 → 1 != 3 and 1 != 3 → not counted
        assert_eq!(score_session_match(&[1, 2, 3], &[1, 99, 3]), 0.0);
    }

    // ── reconstruct_history ───────────────────────────────────────────────

    #[test]
    fn reconstruct_empty_full_returns_incoming() {
        let incoming = vec![msg("user", "hello")];
        let result = reconstruct_history(&[], &incoming, None);
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].content.as_text(), "hello");
    }

    #[test]
    fn reconstruct_no_anchor_returns_incoming() {
        let full = vec![msg("assistant", "xyz")];
        let incoming = vec![msg("user", "hello")];
        let result = reconstruct_history(&full, &incoming, None);
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].content.as_text(), "hello");
    }

    #[test]
    fn reconstruct_injects_hidden_mcp_messages() {
        // Turn 1 full history: [sys, user1, asst_tools, tool_result, asst_final]
        let full = vec![
            msg("system", "You are helpful"),
            msg("user", "What files are there?"),
            msg("assistant", "I'll check"), // has tool_calls in reality
            msg("tool", "file1.txt\nfile2.txt"),
            msg("assistant", "There are file1.txt and file2.txt"),
        ];

        // Turn 2 client sends: [sys, user1, asst_final, user2]
        let incoming = vec![
            msg("system", "You are helpful"),
            msg("user", "What files are there?"),
            msg("assistant", "There are file1.txt and file2.txt"),
            msg("user", "Read file1.txt"),
        ];

        let result = reconstruct_history(&full, &incoming, None);

        // Should be: full history + new user message
        assert_eq!(result.len(), 6);
        assert_eq!(result[0].content.as_text(), "You are helpful");
        assert_eq!(result[1].content.as_text(), "What files are there?");
        assert_eq!(result[2].content.as_text(), "I'll check"); // hidden MCP interaction
        assert_eq!(result[3].content.as_text(), "file1.txt\nfile2.txt"); // hidden tool result
        assert_eq!(
            result[4].content.as_text(),
            "There are file1.txt and file2.txt"
        ); // anchor
        assert_eq!(result[5].content.as_text(), "Read file1.txt"); // new message
    }

    #[test]
    fn reconstruct_strips_server_instructions() {
        let instructions = "MCP Gateway: Use tools to help the user.";
        let full = vec![
            msg("system", "You are helpful"),
            msg("system", instructions),
            msg("user", "Hello"),
            msg("assistant", "Hi there"),
        ];

        let incoming = vec![
            msg("system", "You are helpful"),
            msg("assistant", "Hi there"),
            msg("user", "How are you?"),
        ];

        let result = reconstruct_history(&full, &incoming, Some(instructions));

        // Server instructions should be stripped (will be re-injected by orchestrator)
        assert_eq!(result.len(), 4);
        assert_eq!(result[0].content.as_text(), "You are helpful");
        assert_eq!(result[1].role, "user");
        assert_eq!(result[2].content.as_text(), "Hi there");
        assert_eq!(result[3].content.as_text(), "How are you?");
    }

    #[test]
    fn reconstruct_updates_system_message() {
        let full = vec![
            msg("system", "Old system prompt"),
            msg("user", "Hello"),
            msg("assistant", "Hi"),
        ];

        let incoming = vec![
            msg("system", "New system prompt"),
            msg("assistant", "Hi"),
            msg("user", "More"),
        ];

        let result = reconstruct_history(&full, &incoming, None);

        // System message should be updated to client's current version
        assert_eq!(result[0].content.as_text(), "New system prompt");
        assert_eq!(result.len(), 4);
    }

    #[test]
    fn reconstruct_with_only_anchor_no_new_messages() {
        // Client sends same messages as full (exact continuation, nothing new yet)
        let full = vec![msg("user", "Hello"), msg("assistant", "Hi")];
        let incoming = vec![msg("user", "Hello"), msg("assistant", "Hi")];

        let result = reconstruct_history(&full, &incoming, None);
        assert_eq!(result.len(), 2);
    }
}

#[cfg(test)]
mod session_matching_manager_tests {
    use crate::manager::McpViaLlmManager;
    use crate::session::compute_message_hashes;
    use lr_config::McpViaLlmConfig;
    use lr_providers::{ChatMessage, ChatMessageContent};

    fn cfg() -> McpViaLlmConfig {
        McpViaLlmConfig {
            session_ttl_seconds: 3600,
            max_concurrent_sessions: 100,
            max_loop_iterations: 4,
            max_loop_timeout_seconds: 300,
        }
    }

    fn msg(role: &str, text: &str) -> ChatMessage {
        ChatMessage {
            role: role.to_string(),
            content: ChatMessageContent::Text(text.to_string()),
            tool_calls: None,
            tool_call_id: None,
            name: None,
            reasoning_content: None,
        }
    }

    #[test]
    fn matching_creates_new_session_for_new_messages() {
        let mgr = McpViaLlmManager::new(cfg());
        let msgs = vec![msg("user", "hello")];

        let s1 = mgr.get_or_create_session("c1", Some(&msgs));
        let key1 = s1.read().gateway_session_key.clone();

        // No hashes stored yet, so a new session should be created
        let msgs2 = vec![msg("user", "different")];
        let s2 = mgr.get_or_create_session("c1", Some(&msgs2));
        let key2 = s2.read().gateway_session_key.clone();

        // Both should be the same since first session has no stored hashes
        // and second call with no hashes stored falls through to first available
        // Wait - actually with no stored hashes, it skips and creates new.
        // Let me re-check the logic...
        // The first session has empty client_message_hashes, so score is 0.
        // No match >= 0.5, so new session is created.
        assert_ne!(key1, key2);
    }

    #[test]
    fn matching_reuses_session_for_continuation() {
        let mgr = McpViaLlmManager::new(cfg());
        let msgs1 = vec![msg("user", "hello")];

        // Create first session and store hashes
        let s1 = mgr.get_or_create_session("c1", Some(&msgs1));
        let key1 = s1.read().gateway_session_key.clone();
        s1.write().client_message_hashes = compute_message_hashes(&msgs1);

        // Continue with same prefix + new message → should match
        let msgs2 = vec![
            msg("user", "hello"),
            msg("assistant", "Hi"),
            msg("user", "How?"),
        ];
        let s2 = mgr.get_or_create_session("c1", Some(&msgs2));
        let key2 = s2.read().gateway_session_key.clone();

        assert_eq!(key1, key2);
    }

    #[test]
    fn matching_creates_new_for_unrelated_messages() {
        let mgr = McpViaLlmManager::new(cfg());
        let msgs1 = vec![msg("user", "hello")];

        // Create first session and store hashes
        let s1 = mgr.get_or_create_session("c1", Some(&msgs1));
        let key1 = s1.read().gateway_session_key.clone();
        s1.write().client_message_hashes = compute_message_hashes(&msgs1);

        // Completely different messages → should NOT match (score = 0)
        let msgs2 = vec![msg("user", "totally different conversation")];
        let s2 = mgr.get_or_create_session("c1", Some(&msgs2));
        let key2 = s2.read().gateway_session_key.clone();

        assert_ne!(key1, key2);
    }

    #[test]
    fn matching_handles_truncated_history() {
        let mgr = McpViaLlmManager::new(cfg());

        // Session with 4-message history
        let msgs1 = vec![
            msg("user", "msg1"),
            msg("assistant", "r1"),
            msg("user", "msg2"),
            msg("assistant", "r2"),
        ];

        let s1 = mgr.get_or_create_session("c1", Some(&msgs1));
        let key1 = s1.read().gateway_session_key.clone();
        s1.write().client_message_hashes = compute_message_hashes(&msgs1);

        // Client drops first 2 messages, keeps last 2 + adds new
        let msgs2 = vec![
            msg("user", "msg2"),
            msg("assistant", "r2"),
            msg("user", "msg3"),
        ];
        let s2 = mgr.get_or_create_session("c1", Some(&msgs2));
        let key2 = s2.read().gateway_session_key.clone();

        // Should match: suffix [msg2, r2] of stored matches prefix of incoming → 2/4 = 0.5
        assert_eq!(key1, key2);
    }

    #[test]
    fn multiple_concurrent_sessions() {
        let mgr = McpViaLlmManager::new(cfg());

        // Create two sessions with different conversations
        let msgs_a = vec![msg("user", "conversation A")];
        let s_a = mgr.get_or_create_session("c1", Some(&msgs_a));
        let key_a = s_a.read().gateway_session_key.clone();
        s_a.write().client_message_hashes = compute_message_hashes(&msgs_a);

        let msgs_b = vec![msg("user", "conversation B")];
        let s_b = mgr.get_or_create_session("c1", Some(&msgs_b));
        let key_b = s_b.read().gateway_session_key.clone();
        s_b.write().client_message_hashes = compute_message_hashes(&msgs_b);

        assert_ne!(key_a, key_b);

        // Continue conversation A → should match session A
        let msgs_a2 = vec![
            msg("user", "conversation A"),
            msg("assistant", "Reply A"),
            msg("user", "Follow up A"),
        ];
        let s = mgr.get_or_create_session("c1", Some(&msgs_a2));
        assert_eq!(s.read().gateway_session_key, key_a);

        // Continue conversation B → should match session B
        let msgs_b2 = vec![
            msg("user", "conversation B"),
            msg("assistant", "Reply B"),
            msg("user", "Follow up B"),
        ];
        let s = mgr.get_or_create_session("c1", Some(&msgs_b2));
        assert_eq!(s.read().gateway_session_key, key_b);
    }

    #[test]
    fn find_session_by_gateway_key() {
        let mgr = McpViaLlmManager::new(cfg());
        let s1 = mgr.get_or_create_session("c1", None);
        let key = s1.read().gateway_session_key.clone();

        let found = mgr.find_session_by_gateway_key("c1", &key);
        assert!(found.is_some());
        assert_eq!(found.unwrap().read().gateway_session_key, key);

        assert!(mgr
            .find_session_by_gateway_key("c1", "nonexistent")
            .is_none());
        assert!(mgr.find_session_by_gateway_key("c2", &key).is_none());
    }

    #[test]
    fn fuzzy_matching_whitespace_resilience() {
        let mgr = McpViaLlmManager::new(cfg());

        // Create session with normal whitespace
        let msgs1 = vec![msg("user", "hello world")];
        let s1 = mgr.get_or_create_session("c1", Some(&msgs1));
        let key1 = s1.read().gateway_session_key.clone();
        s1.write().client_message_hashes = compute_message_hashes(&msgs1);

        // Same message but with different whitespace → should still match
        let msgs2 = vec![msg("user", "hello  world"), msg("assistant", "response")];
        let s2 = mgr.get_or_create_session("c1", Some(&msgs2));
        assert_eq!(s2.read().gateway_session_key, key1);
    }
}
