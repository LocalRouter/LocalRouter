//! Agentic loop orchestrator
//!
//! Runs the core loop: call LLM → inspect for tool calls → execute MCP tools
//! → re-call LLM → repeat until final response or limit reached.
//!
//! When mixed tool calls are detected (both MCP and client tools), MCP tools
//! are executed in the background while client tools are returned immediately.

use std::collections::HashSet;
use std::sync::Arc;
use std::time::Instant;

use parking_lot::RwLock;
use serde_json::{json, Value};

use lr_config::{Client, McpViaLlmConfig};
use lr_mcp::protocol::Root;
use lr_mcp::McpGateway;
use lr_providers::{
    ChatMessage, ChatMessageContent, CompletionRequest, CompletionResponse, ToolCall,
};
use lr_router::Router;

use std::collections::HashMap;

use crate::gateway_client::{GatewayClient, McpTool};
use crate::manager::McpViaLlmError;
use crate::session::{McpViaLlmSession, PendingMixedExecution};

/// Bundled gateway permissions for background tool execution.
/// Extracted from a `Client` so background tasks can call `handle_request_with_skills`.
#[derive(Clone)]
pub(crate) struct GatewayPermissions {
    pub session_key: String,
    pub mcp_permissions: lr_config::McpPermissions,
    pub skills_permissions: lr_config::SkillsPermissions,
    pub client_name: String,
    pub marketplace_permission: lr_config::PermissionState,
    pub coding_agent_permission: lr_config::PermissionState,
    pub coding_agent_type: Option<lr_config::CodingAgentType>,
    pub context_management_overrides: Option<lr_config::ContextManagementOverrides>,
    pub mcp_sampling_permission: lr_config::PermissionState,
    pub mcp_elicitation_permission: lr_config::PermissionState,
}

impl GatewayPermissions {
    pub fn from_client_and_session(client: &Client, session_key: String) -> Self {
        Self {
            session_key,
            mcp_permissions: client.mcp_permissions.clone(),
            skills_permissions: client.skills_permissions.clone(),
            client_name: client.name.clone(),
            marketplace_permission: client.marketplace_permission.clone(),
            coding_agent_permission: client.coding_agent_permission.clone(),
            coding_agent_type: client.coding_agent_type,
            context_management_overrides: Some(lr_config::ContextManagementOverrides {
                context_management_enabled: client.context_management_enabled,
                catalog_compression_enabled: client.catalog_compression_enabled,
            }),
            mcp_sampling_permission: client.mcp_sampling_permission.clone(),
            mcp_elicitation_permission: client.mcp_elicitation_permission.clone(),
        }
    }
}

/// Result of the agentic loop
#[allow(clippy::large_enum_variant)]
pub enum OrchestratorResult {
    /// Loop completed — return this response to the client
    Complete(CompletionResponse),
    /// Mixed tools detected — MCP tools running in background, return client tools to client
    PendingMixed {
        /// Response containing only client tool calls
        client_response: CompletionResponse,
        /// Background MCP execution state
        pending: PendingMixedExecution,
    },
}

/// Run the agentic loop for an MCP via LLM request
///
/// If `guardrail_gate` is provided, it will be awaited after the first LLM call
/// returns but before executing any tools or returning a response. This allows
/// guardrails to run in parallel with the LLM call for lower latency.
#[allow(clippy::too_many_arguments)]
pub async fn run_agentic_loop(
    gateway: Arc<McpGateway>,
    router: &Router,
    client: &Client,
    session: Arc<RwLock<McpViaLlmSession>>,
    mut request: CompletionRequest,
    config: &McpViaLlmConfig,
    allowed_servers: Vec<String>,
    mut guardrail_gate: Option<crate::manager::GuardrailGate>,
    initial_usage_entries: Option<Vec<lr_providers::TokenUsage>>,
    memory_service: Option<Arc<lr_memory::MemoryService>>,
) -> Result<OrchestratorResult, McpViaLlmError> {
    let started_at = Instant::now();
    let timeout = std::time::Duration::from_secs(config.max_loop_timeout_seconds);

    let (gateway_session_key, gateway_initialized) = {
        let s = session.read();
        (s.gateway_session_key.clone(), s.gateway_initialized)
    };

    // Set up gateway client for MCP operations
    let gw_client = GatewayClient::new(&gateway, client, gateway_session_key, allowed_servers);

    // Initialize gateway session if needed
    if !gateway_initialized {
        let instructions = gw_client.initialize().await?;
        session.write().gateway_initialized = true;
        if let Some(instructions) = instructions {
            inject_server_instructions(&mut request, &instructions);
        }
    }

    // Fetch MCP tools
    let mcp_tools = gw_client.list_tools().await?;
    let mut mcp_tool_names: HashSet<String> = mcp_tools.iter().map(|t| t.name.clone()).collect();

    if mcp_tools.is_empty() {
        tracing::info!(
            "MCP via LLM: no MCP tools available for client {}, passing through",
            &client.id[..8.min(client.id.len())]
        );
        // No MCP tools available - just call the router directly (non-streaming)
        // Streaming passthrough is handled by the streaming orchestrator
        request.stream = false;
        let response = router
            .complete(&client.id, request)
            .await
            .map_err(McpViaLlmError::from)?;
        return Ok(OrchestratorResult::Complete(response));
    }

    tracing::info!(
        "MCP via LLM: {} MCP tools available for client {}",
        mcp_tools.len(),
        &client.id[..8.min(client.id.len())]
    );

    // Merge MCP tools into request
    inject_mcp_tools(&mut request, &mcp_tools);

    // Synthetic tool mappings for prompts
    let mut prompt_tools: HashMap<String, String> = HashMap::new(); // tool_name -> prompt_name

    // Expose a single resource_read tool (replaces N per-resource synthetic tools)
    if config.expose_resources_as_tools {
        inject_resource_read_tool(&mut request);
        mcp_tool_names.insert(RESOURCE_READ_TOOL_NAME.to_string());
    }

    // Inject MCP prompts
    if config.inject_prompts {
        match gw_client.list_prompts().await {
            Ok(prompts) => {
                if !prompts.is_empty() {
                    tracing::info!(
                        "MCP via LLM: processing {} prompts ({} no-arg, {} parameterized)",
                        prompts.len(),
                        prompts.iter().filter(|p| p.arguments.is_empty()).count(),
                        prompts.iter().filter(|p| !p.arguments.is_empty()).count(),
                    );

                    // No-arg prompts: resolve and inject as system messages
                    for prompt in prompts.iter().filter(|p| p.arguments.is_empty()) {
                        match gw_client.get_prompt(&prompt.name, json!({})).await {
                            Ok(messages) => {
                                inject_prompt_messages(&mut request, &messages);
                            }
                            Err(e) => {
                                tracing::warn!(
                                    "MCP via LLM: failed to get prompt '{}': {}",
                                    prompt.name,
                                    e
                                );
                            }
                        }
                    }

                    // Parameterized prompts: expose as synthetic tools
                    let param_prompts: Vec<_> = prompts
                        .into_iter()
                        .filter(|p| !p.arguments.is_empty())
                        .collect();
                    if !param_prompts.is_empty() {
                        inject_prompt_tools(&mut request, &param_prompts, &mut prompt_tools);
                        for name in prompt_tools.keys() {
                            mcp_tool_names.insert(name.clone());
                        }
                    }
                }
            }
            Err(e) => {
                tracing::warn!("MCP via LLM: failed to list prompts: {}", e);
            }
        }
    }

    let mut total_prompt_tokens: u64 = 0;
    let mut total_completion_tokens: u64 = 0;
    let mut mcp_tools_called: Vec<String> = Vec::new();
    let mut usage_entries: Vec<lr_providers::TokenUsage> =
        initial_usage_entries.unwrap_or_default();

    let mut iteration: u32 = 0;
    loop {
        // Check max iterations
        let max_iter = config.max_loop_iterations.max(1);
        if iteration >= max_iter {
            return Err(McpViaLlmError::MaxIterations(max_iter));
        }

        // Check timeout
        if started_at.elapsed() > timeout {
            return Err(McpViaLlmError::Timeout(config.max_loop_timeout_seconds));
        }

        // Keep session alive during long-running loops
        session.write().touch();

        tracing::info!(
            "MCP via LLM: iteration {} for client {}",
            iteration + 1,
            &client.id[..8.min(client.id.len())]
        );

        // Always non-streaming for now
        let mut completion_request = request.clone();
        completion_request.stream = false;

        // Call the LLM
        let response = router
            .complete(&client.id, completion_request)
            .await
            .map_err(McpViaLlmError::from)?;

        // Await guardrail gate before processing the response (first iteration only).
        // This allows guardrails to run in parallel with the LLM call.
        if let Some(gate) = guardrail_gate.take() {
            gate.await
                .map_err(|e| McpViaLlmError::Gateway(format!("Guardrail task panicked: {}", e)))?
                .map_err(McpViaLlmError::GuardrailDenied)?;
        }

        // Accumulate usage
        total_prompt_tokens += response.usage.prompt_tokens as u64;
        total_completion_tokens += response.usage.completion_tokens as u64;
        usage_entries.push(response.usage.clone());

        // Inspect the response
        let choice = response
            .choices
            .first()
            .ok_or_else(|| McpViaLlmError::Gateway("No choices in LLM response".to_string()))?;

        // Check for tool calls
        if let Some(ref tool_calls) = choice.message.tool_calls {
            if !tool_calls.is_empty() {
                // Classify: MCP vs client tools
                let (mcp_calls, client_calls): (Vec<&ToolCall>, Vec<&ToolCall>) = tool_calls
                    .iter()
                    .partition(|tc| mcp_tool_names.contains(&tc.function.name));

                if !client_calls.is_empty() && !mcp_calls.is_empty() {
                    // Mixed tools: spawn MCP in background, return client tools
                    tracing::info!(
                        "MCP via LLM: mixed tools detected — {} MCP [{}], {} client [{}] (iteration {})",
                        mcp_calls.len(),
                        mcp_calls.iter().map(|tc| tc.function.name.as_str()).collect::<Vec<_>>().join(", "),
                        client_calls.len(),
                        client_calls.iter().map(|tc| tc.function.name.as_str()).collect::<Vec<_>>().join(", "),
                        iteration + 1
                    );

                    // Capture the full assistant message and client tool call IDs
                    // before we move `response`
                    let full_assistant_message = choice.message.clone();
                    let client_tool_call_ids: Vec<String> =
                        client_calls.iter().map(|tc| tc.id.clone()).collect();
                    let client_tools_owned: Vec<ToolCall> =
                        client_calls.into_iter().cloned().collect();

                    // Capture state needed for background MCP execution
                    let client_id = client.id.clone();
                    let roots = gw_client.roots().to_vec();
                    let servers = gw_client.allowed_servers().to_vec();
                    let gw_session_key = session.read().gateway_session_key.clone();
                    let perms = GatewayPermissions::from_client_and_session(client, gw_session_key);

                    // Spawn background MCP tool executions
                    let mut mcp_handles = Vec::new();
                    for tool_call in &mcp_calls {
                        let tool_name = tool_call.function.name.clone();
                        let tool_call_id = tool_call.id.clone();
                        let arguments: Value =
                            match serde_json::from_str(&tool_call.function.arguments) {
                                Ok(v) => v,
                                Err(e) => {
                                    tracing::warn!(
                                        "MCP via LLM: malformed arguments for tool '{}': {}",
                                        tool_name,
                                        e
                                    );
                                    // Return parse error as tool result so LLM can retry
                                    let err_msg = format!(
                                        "Error: invalid JSON arguments for tool '{}': {}. Raw: {}",
                                        tool_name, e, tool_call.function.arguments
                                    );
                                    let tc_id = tool_call_id.clone();
                                    let handle = tokio::spawn(async move { (tc_id, Err(err_msg)) });
                                    mcp_handles.push(handle);
                                    mcp_tools_called.push(tool_name.clone());
                                    continue;
                                }
                            };

                        let gw = gateway.clone();
                        let cid = client_id.clone();
                        let srv = servers.clone();
                        let rts = roots.clone();
                        let p = perms.clone();

                        mcp_tools_called.push(tool_name.clone());

                        let handle = tokio::spawn(async move {
                            let result = execute_mcp_tool_background(
                                &gw, &cid, srv, rts, &p, &tool_name, arguments,
                            )
                            .await;
                            (tool_call_id, result)
                        });
                        mcp_handles.push(handle);
                    }

                    let pending = PendingMixedExecution {
                        full_assistant_message,
                        mcp_handles,
                        client_tool_call_ids,
                        accumulated_prompt_tokens: total_prompt_tokens,
                        accumulated_completion_tokens: total_completion_tokens,
                        mcp_tools_called: mcp_tools_called.clone(),
                        messages_before_mixed: request.messages.clone(),
                        started_at,
                        accumulated_usage_entries: usage_entries.clone(),
                    };

                    // Build response with only client tool calls
                    let mut client_response = response;
                    if let Some(choice) = client_response.choices.first_mut() {
                        choice.message.tool_calls = Some(client_tools_owned);
                    }

                    let client_response = build_final_response(
                        client_response,
                        total_prompt_tokens,
                        total_completion_tokens,
                        &mcp_tools_called,
                        iteration + 1,
                        usage_entries.clone(),
                    );

                    return Ok(OrchestratorResult::PendingMixed {
                        client_response,
                        pending,
                    });
                }

                if !client_calls.is_empty() {
                    // Only client tools — return them directly
                    tracing::info!(
                        "MCP via LLM: {} client tool calls [{}] (iteration {}), returning to client",
                        client_calls.len(),
                        client_calls.iter().map(|tc| tc.function.name.as_str()).collect::<Vec<_>>().join(", "),
                        iteration + 1
                    );
                    return Ok(OrchestratorResult::Complete(build_final_response(
                        response,
                        total_prompt_tokens,
                        total_completion_tokens,
                        &mcp_tools_called,
                        iteration + 1,
                        usage_entries,
                    )));
                }

                // All MCP tools — execute them and loop
                tracing::info!(
                    "MCP via LLM: LLM requested {} MCP tools: [{}] (iteration {})",
                    mcp_calls.len(),
                    mcp_calls
                        .iter()
                        .map(|tc| tc.function.name.as_str())
                        .collect::<Vec<_>>()
                        .join(", "),
                    iteration + 1
                );

                // Add the assistant message with tool calls to the conversation
                request.messages.push(choice.message.clone());

                for tool_call in &mcp_calls {
                    let tool_name = &tool_call.function.name;
                    let arguments: Value = match serde_json::from_str(&tool_call.function.arguments)
                    {
                        Ok(v) => v,
                        Err(e) => {
                            tracing::warn!(
                                "MCP via LLM: malformed arguments for tool '{}': {}",
                                tool_name,
                                e
                            );
                            // Add parse error as tool result so LLM can retry
                            let error_content = format!(
                                "Error: invalid JSON arguments: {}. Raw: {}",
                                e, tool_call.function.arguments
                            );
                            request.messages.push(ChatMessage {
                                role: "tool".to_string(),
                                content: ChatMessageContent::Text(error_content),
                                tool_calls: None,
                                tool_call_id: Some(tool_call.id.clone()),
                                name: None,
                            });
                            mcp_tools_called.push(tool_name.clone());
                            continue;
                        }
                    };

                    let tool_start = Instant::now();
                    tracing::debug!(
                        "MCP via LLM: executing tool '{}' (call_id: {})",
                        tool_name,
                        tool_call.id
                    );
                    mcp_tools_called.push(tool_name.clone());

                    let result_content = if tool_name == RESOURCE_READ_TOOL_NAME {
                        // resource_read tool — read MCP resource or skill file
                        let name = arguments.get("name").and_then(|v| v.as_str()).unwrap_or("");
                        execute_resource_read(&gw_client, name).await
                    } else if let Some(prompt_name) = prompt_tools.get(tool_name.as_str()) {
                        // Synthetic prompt tool — get the prompt and format as text
                        match gw_client.get_prompt(prompt_name, arguments.clone()).await {
                            Ok(messages) => {
                                // Convert prompt messages to readable text
                                messages
                                    .iter()
                                    .filter_map(|m| {
                                        let role = m
                                            .get("role")
                                            .and_then(|r| r.as_str())
                                            .unwrap_or("system");
                                        let text = m
                                            .get("content")
                                            .and_then(|c| {
                                                // Content can be string or { type: "text", text: "..." }
                                                c.as_str().map(|s| s.to_string()).or_else(|| {
                                                    c.get("text")
                                                        .and_then(|t| t.as_str())
                                                        .map(|s| s.to_string())
                                                })
                                            })
                                            .unwrap_or_default();
                                        if text.is_empty() {
                                            None
                                        } else {
                                            Some(format!("[{}]: {}", role, text))
                                        }
                                    })
                                    .collect::<Vec<_>>()
                                    .join("\n\n")
                            }
                            Err(e) => {
                                format!("Error getting prompt '{}': {}", tool_name, e)
                            }
                        }
                    } else {
                        // Regular MCP tool
                        match gw_client.call_tool(tool_name, arguments).await {
                            Ok(content) => content_to_string(&content),
                            Err(e) => format!("Error executing tool '{}': {}", tool_name, e),
                        }
                    };

                    let tool_duration_ms = tool_start.elapsed().as_millis();
                    let is_error = result_content.starts_with("Error ");
                    tracing::info!(
                        "MCP via LLM: tool '{}' completed in {}ms{}",
                        tool_name,
                        tool_duration_ms,
                        if is_error { " (error)" } else { "" }
                    );

                    // Add tool result message
                    request.messages.push(ChatMessage {
                        role: "tool".to_string(),
                        content: ChatMessageContent::Text(result_content),
                        tool_calls: None,
                        tool_call_id: Some(tool_call.id.clone()),
                        name: None,
                    });
                }

                // Store updated history in session
                session
                    .write()
                    .history
                    .set_messages(request.messages.clone());

                iteration += 1;
                continue; // Next iteration
            }
        }

        // No tool calls — return the final response
        tracing::info!(
            "MCP via LLM: completed after {} iterations, {} MCP tools called",
            iteration + 1,
            mcp_tools_called.len()
        );

        // Store final history
        {
            let mut s = session.write();
            s.history.set_messages(request.messages.clone());
            s.history.full_messages.push(choice.message.clone());
        }

        // Memory: write transcript exchange (fire-and-forget)
        if let Some(ref svc) = memory_service {
            if let Some(path) = session.read().transcript_path.clone() {
                // Extract last user message text
                let user_text = request
                    .messages
                    .iter()
                    .rev()
                    .find(|m| m.role == "user")
                    .map(|m| m.content.as_text())
                    .unwrap_or_default();
                // Extract assistant response text
                let assistant_text = choice.message.content.as_text();
                if !user_text.is_empty() && !assistant_text.is_empty() {
                    let svc = svc.clone();
                    tokio::spawn(async move {
                        if let Err(e) =
                            svc.transcript.append_exchange(&path, &user_text, &assistant_text).await
                        {
                            tracing::warn!("Memory: failed to write transcript: {}", e);
                        }
                        svc.touch_session(&path);
                    });
                }
            }
        }

        return Ok(OrchestratorResult::Complete(build_final_response(
            response,
            total_prompt_tokens,
            total_completion_tokens,
            &mcp_tools_called,
            iteration + 1,
            usage_entries,
        )));
    }
}

/// Resume an agentic loop after mixed tool execution completes.
///
/// Called when the client returns tool results for client tools while
/// MCP tool results are available from the background execution.
///
/// `incoming_request` is the client's new request (containing tool results).
/// We use it as the base for model/temperature/etc. and replace the messages
/// with our reconstructed history.
#[allow(clippy::too_many_arguments)]
pub async fn resume_after_mixed(
    gateway: Arc<McpGateway>,
    router: &Router,
    client: &Client,
    session: Arc<RwLock<McpViaLlmSession>>,
    mut pending: PendingMixedExecution,
    incoming_request: CompletionRequest,
    client_tool_results: Vec<ChatMessage>,
    config: &McpViaLlmConfig,
    allowed_servers: Vec<String>,
    context_management_config: &lr_config::ContextManagementConfig,
) -> Result<OrchestratorResult, McpViaLlmError> {
    // Take handles out before awaiting (pending has Drop impl that aborts them)
    let mcp_handles = std::mem::take(&mut pending.mcp_handles);

    // Await all MCP background tasks
    let mut mcp_results: Vec<(String, Result<String, String>)> = Vec::new();
    for handle in mcp_handles {
        match handle.await {
            Ok(result) => mcp_results.push(result),
            Err(e) => {
                tracing::error!("MCP via LLM: background task panicked: {}", e);
                mcp_results.push(("unknown".to_string(), Err(format!("Task panicked: {}", e))));
            }
        }
    }

    tracing::info!(
        "MCP via LLM: resuming after mixed execution — {} MCP results, {} client results",
        mcp_results.len(),
        client_tool_results.len()
    );

    // Reconstruct the full message history:
    // 1. Messages before the mixed call
    // 2. The full assistant message (with ALL tool calls)
    // 3. All tool results (MCP + client) in original order
    let full_assistant_message = std::mem::replace(
        &mut pending.full_assistant_message,
        ChatMessage {
            role: String::new(),
            content: ChatMessageContent::Text(String::new()),
            tool_calls: None,
            tool_call_id: None,
            name: None,
        },
    );
    let mut messages = std::mem::take(&mut pending.messages_before_mixed);
    messages.push(full_assistant_message.clone());

    // Get context-mode state for client tool indexing (if available)
    let cm_state_for_indexing = {
        let gw_session_key = session.read().gateway_session_key.clone();
        if let Some(gw_session) = gateway.get_session(&gw_session_key) {
            let gw_read = gw_session.read().await;
            gw_read
                .virtual_server_state
                .get("_context_mode")
                .and_then(|s| {
                    s.as_any()
                        .downcast_ref::<lr_mcp::gateway::context_mode::ContextModeSessionState>()
                        .map(|cm| {
                            (
                                cm.enabled,
                                cm.store.clone(),
                                cm.response_threshold_bytes,
                                cm.search_tool_name.clone(),
                            )
                        })
                })
        } else {
            None
        }
    };

    // Add tool results in the order of the original tool_calls
    let mut client_tool_run_counter: u32 = 0;
    if let Some(ref tool_calls) = full_assistant_message.tool_calls {
        for tc in tool_calls {
            // Check if this is an MCP result
            if let Some((_id, ref result)) = mcp_results.iter().find(|(id, _)| id == &tc.id) {
                let content = match result {
                    Ok(c) => c.clone(),
                    Err(e) => format!("Error executing tool '{}': {}", tc.function.name, e),
                };
                messages.push(ChatMessage {
                    role: "tool".to_string(),
                    content: ChatMessageContent::Text(content),
                    tool_calls: None,
                    tool_call_id: Some(tc.id.clone()),
                    name: None,
                });
            } else if let Some(client_result) = client_tool_results
                .iter()
                .find(|m| m.tool_call_id.as_ref() == Some(&tc.id))
            {
                // Check if we should index this client tool result
                let mut result_to_push = client_result.clone();
                if let Some((cm_enabled, ref store, threshold, ref search_name)) =
                    cm_state_for_indexing
                {
                    if cm_enabled
                        && client.is_client_tool_indexing_eligible(
                            &tc.function.name,
                            context_management_config,
                        )
                    {
                        if let ChatMessageContent::Text(ref text) = client_result.content {
                            // Use a stable incrementing counter keyed by tool name
                            client_tool_run_counter += 1;
                            let run_id = client_tool_run_counter;
                            if let Some(compressed) =
                                lr_mcp::gateway::context_mode::compress_client_tool_response(
                                    store,
                                    &tc.function.name,
                                    run_id,
                                    text,
                                    threshold,
                                    search_name,
                                )
                            {
                                result_to_push = ChatMessage {
                                    content: ChatMessageContent::Text(compressed),
                                    ..client_result.clone()
                                };
                            }
                        }
                    }
                }
                messages.push(result_to_push);
            } else {
                // Missing result — add error placeholder and log warning
                tracing::warn!(
                    "MCP via LLM: no result received for tool call '{}' (tool: '{}') during resume",
                    tc.id,
                    tc.function.name
                );
                messages.push(ChatMessage {
                    role: "tool".to_string(),
                    content: ChatMessageContent::Text(format!(
                        "Error: no result received for tool call '{}' (tool: '{}')",
                        tc.id, tc.function.name
                    )),
                    tool_calls: None,
                    tool_call_id: Some(tc.id.clone()),
                    name: None,
                });
            }
        }
    }

    // Store the reconstructed history
    session.write().history.set_messages(messages.clone());

    // Use the incoming request as base (preserves model, temperature, etc.)
    // and replace messages with our reconstructed history
    let mut request = incoming_request;
    request.messages = messages;
    // Tools will be re-injected by the orchestrator

    // Continue the agentic loop with the reconstructed history
    // No guardrail gate needed — guardrails already completed in the original request.
    let initial_usage_entries = if pending.accumulated_usage_entries.is_empty() {
        None
    } else {
        Some(std::mem::take(&mut pending.accumulated_usage_entries))
    };
    run_agentic_loop(
        gateway,
        router,
        client,
        session,
        request,
        config,
        allowed_servers,
        None,
        initial_usage_entries,
        None, // memory_service not passed through resume path
    )
    .await
}

/// Per-tool timeout for background MCP tool executions (120 seconds)
pub(crate) const TOOL_EXECUTION_TIMEOUT_SECS: u64 = 120;

/// Execute an MCP tool call in the background via the gateway.
///
/// Uses `handle_request_with_skills` so virtual servers (skills, coding agents,
/// marketplace, context-mode) are properly dispatched and firewall popups work.
pub async fn execute_mcp_tool_background(
    gateway: &McpGateway,
    client_id: &str,
    allowed_servers: Vec<String>,
    roots: Vec<Root>,
    permissions: &GatewayPermissions,
    tool_name: &str,
    arguments: Value,
) -> Result<String, String> {
    use lr_mcp::protocol::JsonRpcRequest;

    let request = JsonRpcRequest {
        jsonrpc: "2.0".to_string(),
        id: Some(json!(1)),
        method: "tools/call".to_string(),
        params: Some(json!({
            "name": tool_name,
            "arguments": arguments
        })),
    };

    let timeout = std::time::Duration::from_secs(TOOL_EXECUTION_TIMEOUT_SECS);
    let response = match tokio::time::timeout(
        timeout,
        gateway.handle_request_with_skills(
            client_id,
            Some(&permissions.session_key),
            allowed_servers,
            roots,
            permissions.mcp_permissions.clone(),
            permissions.skills_permissions.clone(),
            permissions.client_name.clone(),
            permissions.marketplace_permission.clone(),
            permissions.coding_agent_permission.clone(),
            permissions.coding_agent_type,
            permissions.context_management_overrides.clone(),
            permissions.mcp_sampling_permission.clone(),
            permissions.mcp_elicitation_permission.clone(),
            request,
        ),
    )
    .await
    {
        Ok(result) => result.map_err(|e| format!("tools/call '{}' failed: {}", tool_name, e))?,
        Err(_) => {
            return Err(format!(
                "tools/call '{}' timed out after {}s",
                tool_name, TOOL_EXECUTION_TIMEOUT_SECS
            ));
        }
    };

    if let Some(error) = response.error {
        return Err(format!(
            "tools/call '{}' error: {}",
            tool_name, error.message
        ));
    }

    let result = response.result.unwrap_or(json!({}));

    // Extract text content from MCP tool result
    if let Some(content) = result.get("content") {
        if let Some(arr) = content.as_array() {
            let texts: Vec<String> = arr
                .iter()
                .filter_map(|c| {
                    if c.get("type").and_then(|t| t.as_str()) == Some("text") {
                        c.get("text")
                            .and_then(|t| t.as_str())
                            .map(|s| s.to_string())
                    } else {
                        None
                    }
                })
                .collect();

            if texts.len() == 1 {
                return Ok(texts.into_iter().next().unwrap());
            } else if !texts.is_empty() {
                return Ok(texts.join("\n"));
            }
        }
    }

    Ok(content_to_string(&result))
}

/// Inject MCP tools into the request's tools array
pub(crate) fn inject_mcp_tools(request: &mut CompletionRequest, mcp_tools: &[McpTool]) {
    let provider_tools: Vec<lr_providers::Tool> = mcp_tools
        .iter()
        .map(|t| lr_providers::Tool {
            tool_type: "function".to_string(),
            function: lr_providers::FunctionDefinition {
                name: t.name.clone(),
                description: t.description.clone(),
                parameters: t.input_schema.clone(),
            },
        })
        .collect();

    // Merge: MCP tools take precedence on name conflicts
    let mcp_names: HashSet<&str> = mcp_tools.iter().map(|t| t.name.as_str()).collect();

    match &mut request.tools {
        Some(existing) => {
            // Remove client tools that conflict with MCP tool names
            let before = existing.len();
            existing.retain(|t| !mcp_names.contains(t.function.name.as_str()));
            let shadowed = before - existing.len();
            if shadowed > 0 {
                tracing::warn!(
                    "MCP via LLM: shadowed {} client tools with MCP tools of the same name",
                    shadowed
                );
            }
            existing.extend(provider_tools);
        }
        None => {
            request.tools = Some(provider_tools);
        }
    }
}

/// Build the final response with aggregated usage and metadata
fn build_final_response(
    mut response: CompletionResponse,
    total_prompt_tokens: u64,
    total_completion_tokens: u64,
    mcp_tools_called: &[String],
    iterations: u32,
    usage_entries: Vec<lr_providers::TokenUsage>,
) -> CompletionResponse {
    // Aggregate usage across all iterations
    response.usage.prompt_tokens = total_prompt_tokens as u32;
    response.usage.completion_tokens = total_completion_tokens as u32;
    response.usage.total_tokens = (total_prompt_tokens + total_completion_tokens) as u32;

    // Add metadata extension
    if !mcp_tools_called.is_empty() || iterations > 1 {
        let mut extensions = response.extensions.unwrap_or_default();
        extensions.insert(
            "mcp_via_llm".to_string(),
            json!({
                "iterations": iterations,
                "mcp_tools_called": mcp_tools_called,
                "total_prompt_tokens": total_prompt_tokens,
                "total_completion_tokens": total_completion_tokens,
            }),
        );
        response.extensions = Some(extensions);
    }

    // Only include per-iteration breakdown when there were multiple LLM calls
    if usage_entries.len() > 1 {
        response.request_usage_entries = Some(usage_entries);
    }

    response
}

/// Convert a serde_json Value to a string for tool result content
pub(crate) fn content_to_string(value: &Value) -> String {
    match value {
        Value::String(s) => s.clone(),
        other => other.to_string(),
    }
}

/// The single resource_read tool name.
pub(crate) const RESOURCE_READ_TOOL_NAME: &str = "resource_read";

/// Inject a single `resource_read` tool into the request.
///
/// Replaces the old approach of creating N synthetic per-resource tools.
/// Resource names are listed in the welcome message; the LLM calls this
/// tool with the name to fetch content.
pub(crate) fn inject_resource_read_tool(request: &mut CompletionRequest) {
    let tool = lr_providers::Tool {
        tool_type: "function".to_string(),
        function: lr_providers::FunctionDefinition {
            name: RESOURCE_READ_TOOL_NAME.to_string(),
            description: Some(
                "Read a resource by name. MCP resource names are listed in the welcome message. \
                 Skill files can be read as <skill>/<path> (e.g. \"my-skill/scripts/run.sh\"). \
                 If resources are hidden due to compression, use ctx_search to discover them."
                    .to_string(),
            ),
            parameters: json!({
                "type": "object",
                "properties": {
                    "name": {
                        "type": "string",
                        "description": "Resource name or skill file path"
                    }
                },
                "required": ["name"],
                "additionalProperties": false
            }),
        },
    };

    match &mut request.tools {
        Some(existing) => existing.push(tool),
        None => request.tools = Some(vec![tool]),
    }
}

/// Execute a resource_read call — resolves to MCP resource or skill file.
async fn execute_resource_read(gw_client: &GatewayClient<'_>, name: &str) -> String {
    if name.is_empty() {
        return "Error: missing 'name' parameter".to_string();
    }

    // Try reading as an MCP resource first (by namespaced name)
    match gw_client.read_resource(name).await {
        Ok(content) => content,
        Err(_) => {
            // Not found as MCP resource — try as skill file
            // Skill files use the pattern: <skill_name>/<subpath>
            if let Some(slash_pos) = name.find('/') {
                let skill_name = &name[..slash_pos];
                let subpath = &name[slash_pos + 1..];
                if !subpath.is_empty() {
                    // Try reading via the gateway's skill file reader
                    match gw_client.read_skill_file(skill_name, subpath).await {
                        Ok(content) => return content,
                        Err(e) => {
                            return format!(
                                "Error: resource '{}' not found as MCP resource or skill file: {}",
                                name, e
                            );
                        }
                    }
                }
            }
            format!(
                "Error: resource '{}' not found. Check the welcome message for available resource names.",
                name
            )
        }
    }
}

/// Inject parameterized MCP prompts as synthetic function tools.
pub(crate) fn inject_prompt_tools(
    request: &mut CompletionRequest,
    prompts: &[crate::gateway_client::McpPrompt],
    prompt_tools: &mut HashMap<String, String>,
) {
    let tools: Vec<lr_providers::Tool> = prompts
        .iter()
        .map(|p| {
            let tool_name = format!("mcp_prompt__{}", p.name);
            prompt_tools.insert(tool_name.clone(), p.name.clone());

            let description = p
                .description
                .clone()
                .unwrap_or_else(|| format!("Get the '{}' prompt template", p.name));

            // Build JSON Schema properties from prompt arguments
            let mut properties = serde_json::Map::new();
            let mut required = Vec::new();
            for arg in &p.arguments {
                let mut prop = serde_json::Map::new();
                prop.insert("type".to_string(), json!("string"));
                if let Some(ref desc) = arg.description {
                    prop.insert("description".to_string(), json!(desc));
                }
                properties.insert(arg.name.clone(), Value::Object(prop));
                if arg.required {
                    required.push(json!(arg.name));
                }
            }

            let parameters = json!({
                "type": "object",
                "properties": properties,
                "required": required
            });

            lr_providers::Tool {
                tool_type: "function".to_string(),
                function: lr_providers::FunctionDefinition {
                    name: tool_name,
                    description: Some(description),
                    parameters,
                },
            }
        })
        .collect();

    match &mut request.tools {
        Some(existing) => existing.extend(tools),
        None => request.tools = Some(tools),
    }
}

/// Inject no-argument prompt messages into the request as system messages.
/// These are prepended to the conversation before user messages.
pub(crate) fn inject_prompt_messages(request: &mut CompletionRequest, prompt_messages: &[Value]) {
    // Find the first non-system message index to insert before it
    let insert_idx = request
        .messages
        .iter()
        .position(|m| m.role != "system")
        .unwrap_or(request.messages.len());

    let mut offset = 0;
    for msg in prompt_messages {
        let role = msg
            .get("role")
            .and_then(|r| r.as_str())
            .unwrap_or("system")
            .to_string();

        let text = msg
            .get("content")
            .and_then(|c| {
                // Content can be a string or { type: "text", text: "..." }
                c.as_str().map(|s| s.to_string()).or_else(|| {
                    c.get("text")
                        .and_then(|t| t.as_str())
                        .map(|s| s.to_string())
                })
            })
            .unwrap_or_default();

        if !text.is_empty() {
            request.messages.insert(
                insert_idx + offset,
                ChatMessage {
                    role,
                    content: ChatMessageContent::Text(text),
                    tool_calls: None,
                    tool_call_id: None,
                    name: None,
                },
            );
            offset += 1;
        }
    }
}

/// Inject the unified MCP gateway instructions as a system message.
/// Placed after all existing system messages but before the first non-system message.
pub(crate) fn inject_server_instructions(request: &mut CompletionRequest, instructions: &str) {
    let insert_idx = request
        .messages
        .iter()
        .position(|m| m.role != "system")
        .unwrap_or(request.messages.len());

    request.messages.insert(
        insert_idx,
        ChatMessage {
            role: "system".to_string(),
            content: ChatMessageContent::Text(instructions.to_string()),
            tool_calls: None,
            tool_call_id: None,
            name: None,
        },
    );
}
