//! Agentic loop orchestrator
//!
//! Runs the core loop: call LLM → inspect for tool calls → execute MCP tools
//! → re-call LLM → repeat until final response or limit reached.

use std::collections::HashSet;
use std::sync::Arc;
use std::time::Instant;

use parking_lot::RwLock;
use serde_json::{json, Value};

use lr_config::{Client, McpViaLlmConfig};
use lr_mcp::McpGateway;
use lr_providers::{
    ChatMessage, ChatMessageContent, CompletionRequest, CompletionResponse, FunctionCall, ToolCall,
};
use lr_router::Router;

use crate::gateway_client::{GatewayClient, McpTool};
use crate::manager::McpViaLlmError;
use crate::session::McpViaLlmSession;

/// Run the agentic loop for an MCP via LLM request
pub async fn run_agentic_loop(
    gateway: &McpGateway,
    router: &Router,
    client: &Client,
    session: Arc<RwLock<McpViaLlmSession>>,
    mut request: CompletionRequest,
    config: &McpViaLlmConfig,
    allowed_servers: Vec<String>,
) -> Result<CompletionResponse, McpViaLlmError> {
    let started_at = Instant::now();
    let timeout = std::time::Duration::from_secs(config.max_loop_timeout_seconds);

    let (gateway_session_key, gateway_initialized) = {
        let s = session.read();
        (s.gateway_session_key.clone(), s.gateway_initialized)
    };

    // Set up gateway client for MCP operations
    let gw_client = GatewayClient::new(gateway, client, gateway_session_key, allowed_servers);

    // Initialize gateway session if needed
    if !gateway_initialized {
        gw_client.initialize().await?;
        session.write().gateway_initialized = true;
    }

    // Fetch MCP tools
    let mcp_tools = gw_client.list_tools().await?;
    let mcp_tool_names: HashSet<String> = mcp_tools.iter().map(|t| t.name.clone()).collect();

    if mcp_tools.is_empty() {
        tracing::info!(
            "MCP via LLM: no MCP tools available for client {}, passing through",
            &client.id[..8.min(client.id.len())]
        );
        // No MCP tools available - just call the router directly
        request.stream = false;
        return router
            .complete(&client.id, request)
            .await
            .map_err(McpViaLlmError::from);
    }

    tracing::info!(
        "MCP via LLM: {} MCP tools available for client {}",
        mcp_tools.len(),
        &client.id[..8.min(client.id.len())]
    );

    // Merge MCP tools into request
    inject_mcp_tools(&mut request, &mcp_tools);

    let mut total_prompt_tokens: u64 = 0;
    let mut total_completion_tokens: u64 = 0;
    let mut mcp_tools_called: Vec<String> = Vec::new();

    for iteration in 0..config.max_loop_iterations {
        // Check timeout
        if started_at.elapsed() > timeout {
            return Err(McpViaLlmError::Timeout(config.max_loop_timeout_seconds));
        }

        tracing::info!(
            "MCP via LLM: iteration {} for client {}",
            iteration + 1,
            &client.id[..8.min(client.id.len())]
        );

        // Always non-streaming for Phase 1
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

                if !client_calls.is_empty() {
                    // Phase 1: return client tool calls to the client
                    tracing::info!(
                        "MCP via LLM: {} client tool calls detected (iteration {}), returning to client",
                        client_calls.len(),
                        iteration + 1
                    );
                    return Ok(build_final_response(
                        response,
                        total_prompt_tokens,
                        total_completion_tokens,
                        &mcp_tools_called,
                        iteration + 1,
                    ));
                }

                // All MCP tools — execute them and loop
                // Add the assistant message with tool calls to the conversation
                request.messages.push(choice.message.clone());

                for tool_call in &mcp_calls {
                    let tool_name = &tool_call.function.name;
                    let arguments: Value =
                        serde_json::from_str(&tool_call.function.arguments).unwrap_or(json!({}));

                    tracing::info!(
                        "MCP via LLM: executing tool '{}' (call_id: {})",
                        tool_name,
                        tool_call.id
                    );
                    mcp_tools_called.push(tool_name.clone());

                    let result_content = match gw_client.call_tool(tool_name, arguments).await {
                        Ok(content) => content_to_string(&content),
                        Err(e) => format!("Error executing tool '{}': {}", tool_name, e),
                    };

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

        return Ok(build_final_response(
            response,
            total_prompt_tokens,
            total_completion_tokens,
            &mcp_tools_called,
            iteration + 1,
        ));
    }

    Err(McpViaLlmError::MaxIterations(config.max_loop_iterations))
}

/// Inject MCP tools into the request's tools array
fn inject_mcp_tools(request: &mut CompletionRequest, mcp_tools: &[McpTool]) {
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
fn content_to_string(value: &Value) -> String {
    match value {
        Value::String(s) => s.clone(),
        other => other.to_string(),
    }
}
