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
pub async fn run_agentic_loop(
    gateway: Arc<McpGateway>,
    router: &Router,
    client: &Client,
    session: Arc<RwLock<McpViaLlmSession>>,
    mut request: CompletionRequest,
    config: &McpViaLlmConfig,
    allowed_servers: Vec<String>,
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
        gw_client.initialize().await?;
        session.write().gateway_initialized = true;
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

    // Synthetic tool mappings for resources and prompts
    let mut resource_tools: HashMap<String, String> = HashMap::new(); // tool_name -> uri
    let mut prompt_tools: HashMap<String, String> = HashMap::new(); // tool_name -> prompt_name

    // Expose MCP resources as synthetic tools (lazy fetch)
    if config.expose_resources_as_tools {
        match gw_client.list_resources().await {
            Ok(resources) => {
                if !resources.is_empty() {
                    tracing::info!(
                        "MCP via LLM: exposing {} resources as tools",
                        resources.len()
                    );
                    inject_resource_tools(&mut request, &resources, &mut resource_tools);
                    // Add synthetic names to MCP tool set so they're classified as server-side
                    for name in resource_tools.keys() {
                        mcp_tool_names.insert(name.clone());
                    }
                }
            }
            Err(e) => {
                tracing::warn!("MCP via LLM: failed to list resources: {}", e);
            }
        }
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

        // Accumulate usage
        total_prompt_tokens += response.usage.prompt_tokens as u64;
        total_completion_tokens += response.usage.completion_tokens as u64;

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

                    // Spawn background MCP tool executions
                    let mut mcp_handles = Vec::new();
                    for tool_call in &mcp_calls {
                        let tool_name = tool_call.function.name.clone();
                        let tool_call_id = tool_call.id.clone();
                        let arguments: Value = serde_json::from_str(&tool_call.function.arguments)
                            .unwrap_or(json!({}));

                        let gw = gateway.clone();
                        let cid = client_id.clone();
                        let srv = servers.clone();
                        let rts = roots.clone();

                        mcp_tools_called.push(tool_name.clone());

                        let handle = tokio::spawn(async move {
                            let result = execute_mcp_tool_background(
                                &gw, &cid, srv, rts, &tool_name, arguments,
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
                    let arguments: Value =
                        serde_json::from_str(&tool_call.function.arguments).unwrap_or(json!({}));

                    let tool_start = Instant::now();
                    tracing::debug!(
                        "MCP via LLM: executing tool '{}' (call_id: {})",
                        tool_name,
                        tool_call.id
                    );
                    mcp_tools_called.push(tool_name.clone());

                    let result_content = if let Some(uri) = resource_tools.get(tool_name.as_str()) {
                        // Synthetic resource tool — read the resource
                        match gw_client.read_resource(uri).await {
                            Ok(content) => content,
                            Err(e) => {
                                format!("Error reading resource '{}': {}", tool_name, e)
                            }
                        }
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

        return Ok(OrchestratorResult::Complete(build_final_response(
            response,
            total_prompt_tokens,
            total_completion_tokens,
            &mcp_tools_called,
            iteration + 1,
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

    // Add tool results in the order of the original tool_calls
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
                messages.push(client_result.clone());
            } else {
                // Missing result — add error placeholder
                messages.push(ChatMessage {
                    role: "tool".to_string(),
                    content: ChatMessageContent::Text(format!(
                        "Error: no result received for tool call '{}'",
                        tc.id
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
    run_agentic_loop(
        gateway,
        router,
        client,
        session,
        request,
        config,
        allowed_servers,
    )
    .await
}

/// Execute an MCP tool call in the background via the gateway directly.
pub async fn execute_mcp_tool_background(
    gateway: &McpGateway,
    client_id: &str,
    allowed_servers: Vec<String>,
    roots: Vec<Root>,
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

    let response = gateway
        .handle_request(client_id, allowed_servers, roots, request)
        .await
        .map_err(|e| format!("tools/call '{}' failed: {}", tool_name, e))?;

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

    response
}

/// Convert a serde_json Value to a string for tool result content
pub(crate) fn content_to_string(value: &Value) -> String {
    match value {
        Value::String(s) => s.clone(),
        other => other.to_string(),
    }
}

/// Inject MCP resources as synthetic function tools.
/// Each resource becomes a no-argument tool that reads the resource on demand.
pub(crate) fn inject_resource_tools(
    request: &mut CompletionRequest,
    resources: &[crate::gateway_client::McpResource],
    resource_tools: &mut HashMap<String, String>,
) {
    let tools: Vec<lr_providers::Tool> = resources
        .iter()
        .map(|r| {
            // Synthetic tool name: mcp_resource__<namespaced_name>
            let tool_name = format!("mcp_resource__{}", r.name);
            resource_tools.insert(tool_name.clone(), r.uri.clone());

            let description = r.description.clone().unwrap_or_else(|| {
                format!(
                    "Read the resource '{}'{}",
                    r.name,
                    r.mime_type
                        .as_ref()
                        .map(|m| format!(" ({})", m))
                        .unwrap_or_default()
                )
            });

            lr_providers::Tool {
                tool_type: "function".to_string(),
                function: lr_providers::FunctionDefinition {
                    name: tool_name,
                    description: Some(description),
                    parameters: json!({"type": "object", "properties": {}}),
                },
            }
        })
        .collect();

    match &mut request.tools {
        Some(existing) => existing.extend(tools),
        None => request.tools = Some(tools),
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
