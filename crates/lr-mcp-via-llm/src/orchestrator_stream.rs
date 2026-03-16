//! Multi-segment streaming orchestrator
//!
//! Streams all agentic loop iterations through a single SSE connection.
//! Between iterations (during tool execution), the stream pauses naturally
//! and SSE keepalive prevents timeout. Intermediate `finish_reason: "tool_calls"`
//! events are suppressed when all tools are MCP-only.

use std::collections::{HashMap, HashSet};
use std::pin::Pin;
use std::sync::Arc;
use std::time::Instant;

use dashmap::DashMap;
use futures::StreamExt;
use parking_lot::RwLock;
use serde_json::{json, Value};

use lr_config::{Client, McpViaLlmConfig};
use lr_mcp::McpGateway;
use lr_providers::{
    ChatMessage, ChatMessageContent, ChunkChoice, ChunkDelta, CompletionChunk, CompletionRequest,
    FunctionCall, FunctionCallDelta, ToolCall, ToolCallDelta,
};
use lr_router::Router;
use lr_types::AppResult;

use crate::gateway_client::GatewayClient;
use crate::manager::McpViaLlmError;
use crate::orchestrator;
use crate::session::{McpViaLlmSession, PendingMixedExecution};

/// Run the agentic loop with multi-segment streaming.
///
/// Returns a stream of `CompletionChunk`s that the caller wraps in SSE.
/// Multiple LLM iterations are streamed through the same connection,
/// with intermediate tool_calls finish reasons suppressed.
#[allow(clippy::too_many_arguments)]
pub async fn run_agentic_loop_streaming(
    gateway: Arc<McpGateway>,
    router: Arc<Router>,
    client: &Client,
    session: Arc<RwLock<McpViaLlmSession>>,
    mut request: CompletionRequest,
    config: &McpViaLlmConfig,
    allowed_servers: Vec<String>,
    pending_executions: Arc<DashMap<String, PendingMixedExecution>>,
    guardrail_gate: Option<crate::manager::GuardrailGate>,
    memory_service: Option<Arc<lr_memory::MemoryService>>,
) -> Result<Pin<Box<dyn futures::Stream<Item = AppResult<CompletionChunk>> + Send>>, McpViaLlmError>
{
    let started_at = Instant::now();
    let timeout = std::time::Duration::from_secs(config.max_loop_timeout_seconds);
    let max_iterations = config.max_loop_iterations;

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
            orchestrator::inject_server_instructions(&mut request, &instructions);
        }
    }

    // Fetch MCP tools
    let mcp_tools = gw_client.list_tools().await?;
    let mut mcp_tool_names: HashSet<String> = mcp_tools.iter().map(|t| t.name.clone()).collect();

    if mcp_tools.is_empty() {
        tracing::info!(
            "MCP via LLM streaming: no MCP tools available for client {}, passing through",
            &client.id[..8.min(client.id.len())]
        );
        request.stream = true;
        let stream = router
            .stream_complete(&client.id, request)
            .await
            .map_err(|e| McpViaLlmError::Gateway(format!("stream passthrough failed: {}", e)))?;
        return Ok(stream);
    }

    tracing::info!(
        "MCP via LLM streaming: {} MCP tools available for client {}",
        mcp_tools.len(),
        &client.id[..8.min(client.id.len())]
    );

    // Merge MCP tools into request
    orchestrator::inject_mcp_tools(&mut request, &mcp_tools);

    // Synthetic tool mappings for prompts
    let mut prompt_tools: HashMap<String, String> = HashMap::new();

    // Expose a single resource_read tool
    if config.expose_resources_as_tools {
        orchestrator::inject_resource_read_tool(&mut request);
        mcp_tool_names.insert(orchestrator::RESOURCE_READ_TOOL_NAME.to_string());
    }

    // Inject MCP prompts
    if config.inject_prompts {
        match gw_client.list_prompts().await {
            Ok(prompts) => {
                if !prompts.is_empty() {
                    // No-arg prompts: resolve and inject as system messages
                    for prompt in prompts.iter().filter(|p| p.arguments.is_empty()) {
                        match gw_client.get_prompt(&prompt.name, json!({})).await {
                            Ok(messages) => {
                                orchestrator::inject_prompt_messages(&mut request, &messages);
                            }
                            Err(e) => {
                                tracing::warn!(
                                    "MCP via LLM streaming: failed to get prompt '{}': {}",
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
                        orchestrator::inject_prompt_tools(
                            &mut request,
                            &param_prompts,
                            &mut prompt_tools,
                        );
                        for name in prompt_tools.keys() {
                            mcp_tool_names.insert(name.clone());
                        }
                    }
                }
            }
            Err(e) => {
                tracing::warn!("MCP via LLM streaming: failed to list prompts: {}", e);
            }
        }
    }

    // Capture state needed for the spawned task
    let client_id = client.id.clone();
    let roots = gw_client.roots().to_vec();
    let servers = gw_client.allowed_servers().to_vec();
    let gw_session_key = session.read().gateway_session_key.clone();
    let perms = orchestrator::GatewayPermissions::from_client_and_session(client, gw_session_key);

    // Channel for streaming chunks to the caller
    let (tx, rx) = tokio::sync::mpsc::channel::<AppResult<CompletionChunk>>(64);

    // Spawn the multi-segment streaming loop
    tokio::spawn(async move {
        let result = streaming_loop(
            gateway,
            router,
            session,
            request,
            &client_id,
            &mcp_tool_names,
            &prompt_tools,
            roots,
            servers,
            &perms,
            tx.clone(),
            started_at,
            timeout,
            max_iterations,
            pending_executions,
            guardrail_gate,
            memory_service,
        )
        .await;

        if let Err(e) = result {
            tracing::error!("MCP via LLM streaming loop error: {}", e);
            let _ = tx
                .send(Err(lr_types::AppError::Internal(format!(
                    "MCP via LLM streaming error: {}",
                    e
                ))))
                .await;
        }
    });

    Ok(Box::pin(tokio_stream::wrappers::ReceiverStream::new(rx)))
}

/// The inner streaming loop that runs in a spawned task.
#[allow(clippy::too_many_arguments)]
async fn streaming_loop(
    gateway: Arc<McpGateway>,
    router: Arc<Router>,
    session: Arc<RwLock<McpViaLlmSession>>,
    mut request: CompletionRequest,
    client_id: &str,
    mcp_tool_names: &HashSet<String>,
    prompt_tools: &HashMap<String, String>,
    roots: Vec<lr_mcp::protocol::Root>,
    servers: Vec<String>,
    permissions: &orchestrator::GatewayPermissions,
    tx: tokio::sync::mpsc::Sender<AppResult<CompletionChunk>>,
    started_at: Instant,
    timeout: std::time::Duration,
    max_iterations: u32,
    pending_executions: Arc<DashMap<String, PendingMixedExecution>>,
    mut guardrail_gate: Option<crate::manager::GuardrailGate>,
    memory_service: Option<Arc<lr_memory::MemoryService>>,
) -> Result<(), McpViaLlmError> {
    let mut iteration: u32 = 0;
    loop {
        // Check max iterations
        let max_iter = max_iterations.max(1);
        if iteration >= max_iter {
            return Err(McpViaLlmError::MaxIterations(max_iter));
        }

        // Check timeout
        if started_at.elapsed() > timeout {
            return Err(McpViaLlmError::Timeout(timeout.as_secs()));
        }

        // Keep session alive during long-running loops
        session.write().touch();

        tracing::info!(
            "MCP via LLM streaming: iteration {} for client {}",
            iteration + 1,
            &client_id[..8.min(client_id.len())]
        );

        // Stream this iteration
        let mut stream_request = request.clone();
        stream_request.stream = true;

        let mut provider_stream = router
            .stream_complete(client_id, stream_request)
            .await
            .map_err(|e| McpViaLlmError::Gateway(format!("stream_complete failed: {}", e)))?;

        // Accumulate the full message from deltas
        let mut accumulated_content = String::new();
        let mut accumulated_tool_calls: Vec<ToolCallAccumulator> = Vec::new();
        let mut accumulated_role = String::from("assistant");
        let mut finish_reason: Option<String> = None;

        // Stream chunks, forwarding non-final ones to the client
        while let Some(chunk_result) = provider_stream.next().await {
            match chunk_result {
                Ok(chunk) => {
                    let mut has_finish = false;

                    if let Some(choice) = chunk.choices.first() {
                        // Accumulate content
                        if let Some(ref content) = choice.delta.content {
                            accumulated_content.push_str(content);
                        }
                        if let Some(ref role) = choice.delta.role {
                            accumulated_role.clone_from(role);
                        }
                        // Accumulate tool call deltas
                        if let Some(ref tool_call_deltas) = choice.delta.tool_calls {
                            accumulate_tool_call_deltas(
                                &mut accumulated_tool_calls,
                                tool_call_deltas,
                            );
                        }
                        if let Some(ref fr) = choice.finish_reason {
                            finish_reason = Some(fr.clone());
                            has_finish = true;
                        }
                    }

                    if has_finish {
                        // Don't forward the finish chunk yet — we need to classify tools first
                    } else {
                        // Forward non-finish chunks to the client
                        if tx.send(Ok(chunk)).await.is_err() {
                            return Ok(()); // Client disconnected
                        }
                    }
                }
                Err(e) => {
                    let _ = tx.send(Err(e)).await;
                    return Ok(());
                }
            }
        }

        // Await guardrail gate before processing the response (first iteration only).
        // This allows guardrails to run in parallel with the LLM streaming call.
        if let Some(gate) = guardrail_gate.take() {
            gate.await
                .map_err(|e| McpViaLlmError::Gateway(format!("Guardrail task panicked: {}", e)))?
                .map_err(McpViaLlmError::GuardrailDenied)?;
        }

        // Reconstruct the full message from accumulated deltas
        let tool_calls = if accumulated_tool_calls.is_empty() {
            None
        } else {
            Some(
                accumulated_tool_calls
                    .iter()
                    .map(|acc| acc.to_tool_call())
                    .collect(),
            )
        };

        let accumulated_message = ChatMessage {
            role: accumulated_role,
            content: ChatMessageContent::Text(accumulated_content),
            tool_calls: tool_calls.clone(),
            tool_call_id: None,
            name: None,
        };

        let fr = finish_reason.as_deref().unwrap_or("stop");

        // Check for tool calls: some providers (e.g., Ollama) send tool_calls in
        // earlier chunks but set finish_reason to "stop" on the final done chunk.
        // So also check accumulated_tool_calls as a fallback.
        let has_accumulated_tools = tool_calls.is_some();
        if fr == "tool_calls" || fr == "tool_use" || has_accumulated_tools {
            if let Some(ref tcs) = tool_calls {
                // Classify: MCP vs client tools
                let (mcp_calls, client_calls): (Vec<&ToolCall>, Vec<&ToolCall>) = tcs
                    .iter()
                    .partition(|tc| mcp_tool_names.contains(&tc.function.name));

                if !client_calls.is_empty() && !mcp_calls.is_empty() {
                    // Mixed tools: spawn MCP in background, store pending, return client tools
                    tracing::info!(
                        "MCP via LLM streaming: mixed tools — {} MCP [{}], {} client [{}] (iteration {})",
                        mcp_calls.len(),
                        mcp_calls.iter().map(|tc| tc.function.name.as_str()).collect::<Vec<_>>().join(", "),
                        client_calls.len(),
                        client_calls.iter().map(|tc| tc.function.name.as_str()).collect::<Vec<_>>().join(", "),
                        iteration + 1
                    );

                    let full_assistant_message = accumulated_message;
                    let client_tool_call_ids: Vec<String> =
                        client_calls.iter().map(|tc| tc.id.clone()).collect();

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
                                    "MCP via LLM streaming: malformed arguments for tool '{}': {}",
                                    tool_name, e
                                );
                                    let err_msg = format!(
                                        "Error: invalid JSON arguments for tool '{}': {}. Raw: {}",
                                        tool_name, e, tool_call.function.arguments
                                    );
                                    let tc_id = tool_call_id.clone();
                                    let handle = tokio::spawn(async move { (tc_id, Err(err_msg)) });
                                    mcp_handles.push(handle);
                                    continue;
                                }
                            };

                        let gw = gateway.clone();
                        let cid = client_id.to_string();
                        let srv = servers.clone();
                        let rts = roots.clone();
                        let p = permissions.clone();

                        let handle = tokio::spawn(async move {
                            let result = orchestrator::execute_mcp_tool_background(
                                &gw, &cid, srv, rts, &p, &tool_name, arguments,
                            )
                            .await;
                            (tool_call_id, result)
                        });
                        mcp_handles.push(handle);
                    }

                    // Note: streaming doesn't provide per-iteration token counts,
                    // so accumulated tokens will be incomplete for the streaming path.
                    // The resumed loop (non-streaming) will track tokens from that point.
                    let pending = PendingMixedExecution {
                        full_assistant_message,
                        mcp_handles,
                        client_tool_call_ids,
                        accumulated_prompt_tokens: 0,
                        accumulated_completion_tokens: 0,
                        mcp_tools_called: mcp_calls
                            .iter()
                            .map(|tc| tc.function.name.clone())
                            .collect(),
                        messages_before_mixed: request.messages.clone(),
                        started_at,
                        accumulated_usage_entries: Vec::new(),
                    };

                    // Store pending execution for later resume
                    pending_executions.insert(client_id.to_string(), pending);

                    // Send finish chunk with client-only tools
                    let finish_chunk = build_finish_chunk_with_tools(
                        &client_calls,
                        &finish_reason.unwrap_or_else(|| "tool_calls".to_string()),
                    );
                    let _ = tx.send(Ok(finish_chunk)).await;
                    return Ok(());
                }

                if !client_calls.is_empty() {
                    // Only client tools — send the finish chunk with client-only tools and stop
                    tracing::info!(
                        "MCP via LLM streaming: {} client tool calls [{}] (iteration {}), finishing stream",
                        client_calls.len(),
                        client_calls.iter().map(|tc| tc.function.name.as_str()).collect::<Vec<_>>().join(", "),
                        iteration + 1
                    );

                    let finish_chunk = build_finish_chunk_with_tools(
                        &client_calls,
                        &finish_reason.unwrap_or_else(|| "tool_calls".to_string()),
                    );
                    let _ = tx.send(Ok(finish_chunk)).await;
                    return Ok(());
                }

                // All MCP tools — suppress the finish, execute tools, continue loop
                tracing::info!(
                    "MCP via LLM streaming: LLM requested {} MCP tools: [{}] (iteration {})",
                    mcp_calls.len(),
                    mcp_calls
                        .iter()
                        .map(|tc| tc.function.name.as_str())
                        .collect::<Vec<_>>()
                        .join(", "),
                    iteration + 1
                );

                request.messages.push(accumulated_message);

                for tool_call in &mcp_calls {
                    let tool_name = &tool_call.function.name;
                    let arguments: Value = match serde_json::from_str(&tool_call.function.arguments)
                    {
                        Ok(v) => v,
                        Err(e) => {
                            tracing::warn!(
                                "MCP via LLM streaming: malformed arguments for tool '{}': {}",
                                tool_name,
                                e
                            );
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
                            continue;
                        }
                    };

                    let tool_start = std::time::Instant::now();
                    tracing::debug!(
                        "MCP via LLM streaming: executing tool '{}' (call_id: {})",
                        tool_name,
                        tool_call.id
                    );

                    let result_content = if tool_name == orchestrator::RESOURCE_READ_TOOL_NAME {
                        // resource_read tool — read MCP resource or skill file
                        let name = arguments.get("name").and_then(|v| v.as_str()).unwrap_or("");
                        match execute_resource_read_background(
                            &gateway,
                            client_id,
                            servers.clone(),
                            roots.clone(),
                            permissions,
                            name,
                        )
                        .await
                        {
                            Ok(content) => content,
                            Err(e) => {
                                format!("Error reading resource '{}': {}", name, e)
                            }
                        }
                    } else if let Some(prompt_name) = prompt_tools.get(tool_name.as_str()) {
                        // Synthetic prompt tool — get the prompt via gateway
                        match execute_prompt_get_background(
                            &gateway,
                            client_id,
                            servers.clone(),
                            roots.clone(),
                            permissions,
                            prompt_name,
                            arguments.clone(),
                        )
                        .await
                        {
                            Ok(content) => content,
                            Err(e) => {
                                format!("Error getting prompt '{}': {}", tool_name, e)
                            }
                        }
                    } else {
                        // Regular MCP tool
                        match orchestrator::execute_mcp_tool_background(
                            &gateway,
                            client_id,
                            servers.clone(),
                            roots.clone(),
                            permissions,
                            tool_name,
                            arguments,
                        )
                        .await
                        {
                            Ok(content) => content,
                            Err(e) => {
                                format!("Error executing tool '{}': {}", tool_name, e)
                            }
                        }
                    };

                    let tool_duration_ms = tool_start.elapsed().as_millis();
                    let is_error = result_content.starts_with("Error ");
                    tracing::info!(
                        "MCP via LLM streaming: tool '{}' completed in {}ms{}",
                        tool_name,
                        tool_duration_ms,
                        if is_error { " (error)" } else { "" }
                    );

                    request.messages.push(ChatMessage {
                        role: "tool".to_string(),
                        content: ChatMessageContent::Text(result_content),
                        tool_calls: None,
                        tool_call_id: Some(tool_call.id.clone()),
                        name: None,
                    });
                }

                // Store updated history
                session
                    .write()
                    .history
                    .set_messages(request.messages.clone());

                iteration += 1;
                continue; // Next iteration
            }
        }

        // No tool calls or finish_reason == "stop" — send the finish chunk and done
        tracing::info!(
            "MCP via LLM streaming: completed after {} iterations",
            iteration + 1
        );

        // Memory: write transcript exchange (fire-and-forget)
        if let Some(ref svc) = memory_service {
            if let Some(path) = session.read().transcript_path.clone() {
                let user_text = request
                    .messages
                    .iter()
                    .rev()
                    .find(|m| m.role == "user")
                    .map(|m| m.content.as_text())
                    .unwrap_or_default();
                let assistant_text = accumulated_message.content.as_text();
                if !user_text.is_empty() && !assistant_text.is_empty() {
                    let svc = svc.clone();
                    tokio::spawn(async move {
                        if let Err(e) =
                            svc.transcript.append_exchange(&path, &user_text, &assistant_text).await
                        {
                            tracing::warn!("Memory: failed to write streaming transcript: {}", e);
                        }
                        svc.touch_session(&path);
                    });
                }
            }
        }

        // Store final history
        {
            let mut s = session.write();
            s.history.set_messages(request.messages.clone());
            s.history.full_messages.push(accumulated_message);
        }

        // Send the final finish chunk
        let finish_chunk = CompletionChunk {
            id: format!("chatcmpl-mcp-{}", uuid::Uuid::new_v4()),
            object: "chat.completion.chunk".to_string(),
            created: std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs() as i64,
            model: String::new(),
            choices: vec![ChunkChoice {
                index: 0,
                delta: ChunkDelta {
                    role: None,
                    content: None,
                    tool_calls: None,
                },
                finish_reason,
            }],
            extensions: None,
        };
        let _ = tx.send(Ok(finish_chunk)).await;

        return Ok(());
    }
}

/// Accumulator for building a complete ToolCall from streaming deltas
struct ToolCallAccumulator {
    #[allow(dead_code)]
    index: u32,
    id: String,
    tool_type: String,
    name: String,
    arguments: String,
}

impl ToolCallAccumulator {
    fn to_tool_call(&self) -> ToolCall {
        ToolCall {
            id: self.id.clone(),
            tool_type: self.tool_type.clone(),
            function: FunctionCall {
                name: self.name.clone(),
                arguments: self.arguments.clone(),
            },
        }
    }
}

/// Accumulate tool call deltas into complete tool calls
fn accumulate_tool_call_deltas(
    accumulators: &mut Vec<ToolCallAccumulator>,
    deltas: &[ToolCallDelta],
) {
    for delta in deltas {
        let idx = delta.index as usize;

        // Extend the accumulator list if needed
        while accumulators.len() <= idx {
            accumulators.push(ToolCallAccumulator {
                index: accumulators.len() as u32,
                id: String::new(),
                tool_type: "function".to_string(),
                name: String::new(),
                arguments: String::new(),
            });
        }

        let acc = &mut accumulators[idx];

        if let Some(ref id) = delta.id {
            acc.id.clone_from(id);
        }
        if let Some(ref tt) = delta.tool_type {
            acc.tool_type.clone_from(tt);
        }
        if let Some(ref func) = delta.function {
            // Name: use assignment (sent once by providers, not split across deltas)
            if let Some(ref name) = func.name {
                if acc.name.is_empty() {
                    acc.name.clone_from(name);
                }
            }
            // Arguments: always append (streamed incrementally)
            if let Some(ref args) = func.arguments {
                acc.arguments.push_str(args);
            }
        }
    }
}

/// Build a finish chunk containing only the specified tool calls
fn build_finish_chunk_with_tools(tool_calls: &[&ToolCall], finish_reason: &str) -> CompletionChunk {
    let tool_call_deltas: Vec<ToolCallDelta> = tool_calls
        .iter()
        .enumerate()
        .map(|(i, tc)| ToolCallDelta {
            index: i as u32,
            id: Some(tc.id.clone()),
            tool_type: Some(tc.tool_type.clone()),
            function: Some(FunctionCallDelta {
                name: Some(tc.function.name.clone()),
                arguments: Some(tc.function.arguments.clone()),
            }),
        })
        .collect();

    CompletionChunk {
        id: format!("chatcmpl-mcp-{}", uuid::Uuid::new_v4()),
        object: "chat.completion.chunk".to_string(),
        created: std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs() as i64,
        model: String::new(),
        choices: vec![ChunkChoice {
            index: 0,
            delta: ChunkDelta {
                role: None,
                content: None,
                tool_calls: Some(tool_call_deltas),
            },
            finish_reason: Some(finish_reason.to_string()),
        }],
        extensions: None,
    }
}

/// Execute a resource read in the background via the gateway.
///
/// Tries MCP resource read by name first, then falls back to skill file read
/// for names matching `<skill>/<subpath>`.
async fn execute_resource_read_background(
    gateway: &lr_mcp::McpGateway,
    client_id: &str,
    allowed_servers: Vec<String>,
    roots: Vec<lr_mcp::protocol::Root>,
    permissions: &orchestrator::GatewayPermissions,
    name: &str,
) -> Result<String, String> {
    use lr_mcp::protocol::JsonRpcRequest;

    if name.is_empty() {
        return Err("missing 'name' parameter".to_string());
    }

    // Try as MCP resource first (by name)
    let request = JsonRpcRequest {
        jsonrpc: "2.0".to_string(),
        id: Some(json!(1)),
        method: "resources/read".to_string(),
        params: Some(json!({ "name": name })),
    };

    let timeout = std::time::Duration::from_secs(orchestrator::TOOL_EXECUTION_TIMEOUT_SECS);
    let response = match tokio::time::timeout(
        timeout,
        gateway.handle_request_with_skills(
            client_id,
            Some(&permissions.session_key),
            allowed_servers.clone(),
            roots.clone(),
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
        Ok(result) => result.map_err(|e| format!("resources/read '{}' failed: {}", name, e))?,
        Err(_) => return Err(format!("resources/read '{}' timed out", name)),
    };

    if response.error.is_none() {
        let result = response.result.unwrap_or(json!({}));
        // Extract text content: { contents: [{ uri, text, mimeType }] }
        if let Some(contents) = result.get("contents").and_then(|c| c.as_array()) {
            let texts: Vec<String> = contents
                .iter()
                .filter_map(|c| {
                    c.get("text")
                        .and_then(|t| t.as_str())
                        .map(|s| s.to_string())
                })
                .collect();
            if !texts.is_empty() {
                return Ok(texts.join("\n"));
            }
        }
        return Ok(orchestrator::content_to_string(&result));
    }

    // Not found as MCP resource — try as skill file (<skill_name>/<subpath>)
    if let Some(slash_pos) = name.find('/') {
        let skill_name = &name[..slash_pos];
        let subpath = &name[slash_pos + 1..];
        if !subpath.is_empty() {
            let skill_request = JsonRpcRequest {
                jsonrpc: "2.0".to_string(),
                id: Some(json!(1)),
                method: "tools/call".to_string(),
                params: Some(json!({
                    "name": "skill_read_file",
                    "arguments": { "skill": skill_name, "path": subpath }
                })),
            };

            let skill_response = match tokio::time::timeout(
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
                    skill_request,
                ),
            )
            .await
            {
                Ok(result) => result.map_err(|e| format!("skill file read failed: {}", e))?,
                Err(_) => return Err(format!("skill file read '{}' timed out", name)),
            };

            if let Some(error) = skill_response.error {
                return Err(format!(
                    "resource '{}' not found as MCP resource or skill file: {}",
                    name, error.message
                ));
            }

            let result = skill_response.result.unwrap_or(json!({}));
            return Ok(orchestrator::content_to_string(&result));
        }
    }

    Err(format!(
        "resource '{}' not found. Check the welcome message for available resource names.",
        name
    ))
}

/// Execute a prompt get in the background via the gateway.
async fn execute_prompt_get_background(
    gateway: &lr_mcp::McpGateway,
    client_id: &str,
    allowed_servers: Vec<String>,
    roots: Vec<lr_mcp::protocol::Root>,
    permissions: &orchestrator::GatewayPermissions,
    prompt_name: &str,
    arguments: Value,
) -> Result<String, String> {
    use lr_mcp::protocol::JsonRpcRequest;

    let request = JsonRpcRequest {
        jsonrpc: "2.0".to_string(),
        id: Some(json!(1)),
        method: "prompts/get".to_string(),
        params: Some(json!({
            "name": prompt_name,
            "arguments": arguments
        })),
    };

    let timeout = std::time::Duration::from_secs(orchestrator::TOOL_EXECUTION_TIMEOUT_SECS);
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
        Ok(result) => result.map_err(|e| format!("prompts/get '{}' failed: {}", prompt_name, e))?,
        Err(_) => return Err(format!("prompts/get '{}' timed out", prompt_name)),
    };

    if let Some(error) = response.error {
        return Err(format!(
            "prompts/get '{}' error: {}",
            prompt_name, error.message
        ));
    }

    let result = response.result.unwrap_or(json!({}));

    // Extract messages and format as text
    if let Some(messages) = result.get("messages").and_then(|m| m.as_array()) {
        let texts: Vec<String> = messages
            .iter()
            .filter_map(|m| {
                let role = m.get("role").and_then(|r| r.as_str()).unwrap_or("system");
                let text = m.get("content").and_then(|c| {
                    c.as_str().map(|s| s.to_string()).or_else(|| {
                        c.get("text")
                            .and_then(|t| t.as_str())
                            .map(|s| s.to_string())
                    })
                })?;
                if text.is_empty() {
                    None
                } else {
                    Some(format!("[{}]: {}", role, text))
                }
            })
            .collect();
        if !texts.is_empty() {
            return Ok(texts.join("\n\n"));
        }
    }

    Ok(orchestrator::content_to_string(&result))
}
