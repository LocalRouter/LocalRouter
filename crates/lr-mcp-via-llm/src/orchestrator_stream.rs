//! Multi-segment streaming orchestrator
//!
//! Streams all agentic loop iterations through a single SSE connection.
//! Between iterations (during tool execution), the stream pauses naturally
//! and SSE keepalive prevents timeout. Intermediate `finish_reason: "tool_calls"`
//! events are suppressed when all tools are MCP-only.

use std::collections::HashSet;
use std::pin::Pin;
use std::sync::Arc;
use std::time::Instant;

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

use crate::gateway_client::{GatewayClient, McpTool};
use crate::manager::McpViaLlmError;
use crate::session::McpViaLlmSession;

/// Run the agentic loop with multi-segment streaming.
///
/// Returns a stream of `CompletionChunk`s that the caller wraps in SSE.
/// Multiple LLM iterations are streamed through the same connection,
/// with intermediate tool_calls finish reasons suppressed.
pub async fn run_agentic_loop_streaming(
    gateway: Arc<McpGateway>,
    router: Arc<Router>,
    client: &Client,
    session: Arc<RwLock<McpViaLlmSession>>,
    mut request: CompletionRequest,
    config: &McpViaLlmConfig,
    allowed_servers: Vec<String>,
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
        gw_client.initialize().await?;
        session.write().gateway_initialized = true;
    }

    // Fetch MCP tools
    let mcp_tools = gw_client.list_tools().await?;
    let mcp_tool_names: HashSet<String> = mcp_tools.iter().map(|t| t.name.clone()).collect();

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
    inject_mcp_tools(&mut request, &mcp_tools);

    // Capture state needed for the spawned task
    let client_id = client.id.clone();
    let roots = gw_client.roots().to_vec();
    let servers = gw_client.allowed_servers().to_vec();

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
            roots,
            servers,
            tx.clone(),
            started_at,
            timeout,
            max_iterations,
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
    roots: Vec<lr_mcp::protocol::Root>,
    servers: Vec<String>,
    tx: tokio::sync::mpsc::Sender<AppResult<CompletionChunk>>,
    started_at: Instant,
    timeout: std::time::Duration,
    max_iterations: u32,
) -> Result<(), McpViaLlmError> {
    for iteration in 0..max_iterations {
        // Check timeout
        if started_at.elapsed() > timeout {
            return Err(McpViaLlmError::Timeout(timeout.as_secs()));
        }

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
                            accumulated_role = role.clone();
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
                        // We'll send it (or suppress it) after the loop
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

        if fr == "tool_calls" || fr == "tool_use" {
            if let Some(ref tcs) = tool_calls {
                // Classify: MCP vs client tools
                let (mcp_calls, client_calls): (Vec<&ToolCall>, Vec<&ToolCall>) = tcs
                    .iter()
                    .partition(|tc| mcp_tool_names.contains(&tc.function.name));

                if !client_calls.is_empty() {
                    // Has client tools — send the finish chunk with client-only tools and stop
                    tracing::info!(
                        "MCP via LLM streaming: {} client tool calls (iteration {}), finishing stream",
                        client_calls.len(),
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
                    "MCP via LLM streaming: executing {} MCP tools (iteration {})",
                    mcp_calls.len(),
                    iteration + 1
                );

                request.messages.push(accumulated_message);

                for tool_call in &mcp_calls {
                    let tool_name = &tool_call.function.name;
                    let arguments: Value =
                        serde_json::from_str(&tool_call.function.arguments).unwrap_or(json!({}));

                    tracing::info!(
                        "MCP via LLM streaming: executing tool '{}' (call_id: {})",
                        tool_name,
                        tool_call.id
                    );

                    let result_content = match crate::orchestrator::execute_mcp_tool_background(
                        &gateway,
                        client_id,
                        servers.clone(),
                        roots.clone(),
                        tool_name,
                        arguments,
                    )
                    .await
                    {
                        Ok(content) => content,
                        Err(e) => format!("Error executing tool '{}': {}", tool_name, e),
                    };

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

                continue; // Next iteration
            }
        }

        // No tool calls or finish_reason == "stop" — send the finish chunk and done
        tracing::info!(
            "MCP via LLM streaming: completed after {} iterations",
            iteration + 1
        );

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

    Err(McpViaLlmError::MaxIterations(max_iterations))
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
            acc.id = id.clone();
        }
        if let Some(ref tt) = delta.tool_type {
            acc.tool_type = tt.clone();
        }
        if let Some(ref func) = delta.function {
            if let Some(ref name) = func.name {
                acc.name.push_str(name);
            }
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

/// Inject MCP tools into the request's tools array (duplicated from orchestrator
/// to avoid making the non-streaming version public just for this)
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

    let mcp_names: HashSet<&str> = mcp_tools.iter().map(|t| t.name.as_str()).collect();

    match &mut request.tools {
        Some(existing) => {
            let before = existing.len();
            existing.retain(|t| !mcp_names.contains(t.function.name.as_str()));
            let shadowed = before - existing.len();
            if shadowed > 0 {
                tracing::warn!(
                    "MCP via LLM streaming: shadowed {} client tools with MCP tools",
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
