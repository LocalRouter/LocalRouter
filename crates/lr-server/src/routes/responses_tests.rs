//! Route-level tests for `POST /v1/responses`: the handler is driven with a
//! scripted provider and a mock MCP server, through the real router, MCP
//! gateway, firewall manager and MCP-via-LLM orchestrator.

use std::any::Any;
use std::collections::{HashMap, VecDeque};
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use axum::extract::{Extension, State};
use axum::http::StatusCode;
use axum::response::IntoResponse;
use parking_lot::Mutex;
use serde_json::{json, Value};

use lr_config::{AppConfig, Client, ConfigManager, FreeTierFallback, PermissionState, Strategy};
use lr_mcp::gateway::access_control::FirewallCheckResult;
use lr_mcp::gateway::firewall::{FirewallApprovalAction, PendingApprovalInfo};
use lr_mcp::gateway::virtual_server::{
    VirtualFirewallResult, VirtualInstructions, VirtualMcpServer, VirtualSessionState,
    VirtualToolCallResult,
};
use lr_mcp::gateway::FirewallDecisionResult;
use lr_providers::factory::{ProviderCategory, ProviderFactory, SetupParameter};
use lr_providers::{
    Capability, ChatMessage, ChatMessageContent, ChunkChoice, ChunkDelta, CompletionChoice,
    CompletionChunk, CompletionRequest, CompletionResponse, FunctionCall, FunctionCallDelta,
    HealthStatus, ModelInfo, ModelProvider, ProviderHealth, TokenUsage, ToolCall, ToolCallDelta,
};
use lr_types::{AppError, AppResult, McpTool};

use super::create_response;
use crate::middleware::client_auth::ClientAuthContext;
use crate::routes::gateway_json::GatewayJson;
use crate::state::{AppState, AuthContext};

const MODEL: &str = "mock/test-model";
const MCP_TOOL: &str = "fs__read";
const CLIENT_TOOL: &str = "get_weather";

// ── Scripted provider ──────────────────────────────────────────────────────

struct ScriptedProvider {
    responses: Mutex<VecDeque<CompletionResponse>>,
    requests: Mutex<Vec<CompletionRequest>>,
    /// Streams end with a usage-only chunk carrying the response's usage
    /// (as OpenAI does with `include_usage`), under the same chunk id as
    /// every other chunk of every stream.
    stream_usage: std::sync::atomic::AtomicBool,
}

fn chunk(delta: ChunkDelta, finish_reason: Option<String>) -> CompletionChunk {
    CompletionChunk {
        id: "chatcmpl-scripted".into(),
        object: "chat.completion.chunk".into(),
        created: 0,
        model: "test-model".into(),
        choices: vec![ChunkChoice {
            index: 0,
            delta,
            finish_reason,
        }],
        extensions: None,
        usage: None,
        provider: None,
    }
}

fn delta(
    role: Option<&str>,
    content: Option<&str>,
    tool_calls: Option<Vec<ToolCallDelta>>,
) -> ChunkDelta {
    ChunkDelta {
        annotations: None,
        role: role.map(str::to_string),
        content: content.map(str::to_string),
        tool_calls,
        reasoning_content: None,
    }
}

/// The way an OpenAI-style upstream streams a response: a role chunk, text
/// in pieces, each tool call as a header delta followed by argument
/// fragments, then a bare finish chunk.
fn as_chunks(response: &CompletionResponse) -> Vec<CompletionChunk> {
    let choice = &response.choices[0];
    let mut chunks = vec![chunk(delta(Some("assistant"), None, None), None)];
    let text = choice.message.content.as_text();
    if !text.is_empty() {
        let middle = text
            .char_indices()
            .nth(text.chars().count() / 2)
            .map_or(0, |(i, _)| i);
        for piece in [&text[..middle], &text[middle..]] {
            if !piece.is_empty() {
                chunks.push(chunk(delta(None, Some(piece), None), None));
            }
        }
    }
    for (index, call) in choice.message.tool_calls.iter().flatten().enumerate() {
        let (head, tail) = call
            .function
            .arguments
            .split_at(call.function.arguments.len() / 2);
        chunks.push(chunk(
            delta(
                None,
                None,
                Some(vec![ToolCallDelta {
                    index: index as u32,
                    id: Some(call.id.clone()),
                    tool_type: Some("function".into()),
                    function: Some(FunctionCallDelta {
                        name: Some(call.function.name.clone()),
                        arguments: Some(head.to_string()),
                    }),
                }]),
            ),
            None,
        ));
        chunks.push(chunk(
            delta(
                None,
                None,
                Some(vec![ToolCallDelta {
                    index: index as u32,
                    id: None,
                    tool_type: None,
                    function: Some(FunctionCallDelta {
                        name: None,
                        arguments: Some(tail.to_string()),
                    }),
                }]),
            ),
            None,
        ));
    }
    chunks.push(chunk(delta(None, None, None), choice.finish_reason.clone()));
    chunks
}

#[async_trait]
impl ModelProvider for ScriptedProvider {
    fn name(&self) -> &str {
        "mock"
    }

    async fn health_check(&self) -> ProviderHealth {
        ProviderHealth {
            status: HealthStatus::Healthy,
            latency_ms: None,
            last_checked: chrono::Utc::now(),
            error_message: None,
        }
    }

    async fn list_models(&self) -> AppResult<Vec<ModelInfo>> {
        Ok(vec![ModelInfo {
            id: "test-model".into(),
            name: "Test Model".into(),
            provider: "mock".into(),
            parameter_count: None,
            context_window: 4096,
            supports_streaming: true,
            capabilities: vec![Capability::Chat],
            detailed_capabilities: None,
        }])
    }

    async fn get_pricing(&self, _model: &str) -> AppResult<lr_providers::PricingInfo> {
        Err(AppError::Provider("not implemented".into()))
    }

    async fn complete(&self, request: CompletionRequest) -> AppResult<CompletionResponse> {
        self.requests.lock().push(request);
        self.responses
            .lock()
            .pop_front()
            .ok_or_else(|| AppError::Provider("no more scripted responses".into()))
    }

    async fn stream_complete(
        &self,
        request: CompletionRequest,
    ) -> AppResult<std::pin::Pin<Box<dyn futures::Stream<Item = AppResult<CompletionChunk>> + Send>>>
    {
        let response = self.complete(request).await?;
        let mut chunks = as_chunks(&response);
        if self.stream_usage.load(std::sync::atomic::Ordering::Relaxed) {
            let mut usage = chunk(delta(None, None, None), None);
            usage.choices.clear();
            usage.usage = Some(response.usage.clone());
            chunks.push(usage);
        }
        Ok(Box::pin(futures::stream::iter(chunks.into_iter().map(Ok))))
    }
}

struct ScriptedFactory(Arc<ScriptedProvider>);

#[async_trait]
impl ProviderFactory for ScriptedFactory {
    fn provider_type(&self) -> &str {
        "mock"
    }
    fn display_name(&self) -> &str {
        "Scripted"
    }
    fn category(&self) -> ProviderCategory {
        ProviderCategory::Local
    }
    fn description(&self) -> &str {
        "Scripted provider for tests"
    }
    fn setup_parameters(&self) -> Vec<SetupParameter> {
        vec![]
    }
    fn create(
        &self,
        _instance_name: String,
        _config: HashMap<String, String>,
    ) -> AppResult<Arc<dyn ModelProvider>> {
        Ok(self.0.clone())
    }
    fn validate_config(&self, _config: &HashMap<String, String>) -> AppResult<()> {
        Ok(())
    }
}

// ── Mock MCP server ────────────────────────────────────────────────────────

#[derive(Clone)]
struct NoState;

impl VirtualSessionState for NoState {
    fn as_any(&self) -> &dyn Any {
        self
    }
    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }
    fn clone_box(&self) -> Box<dyn VirtualSessionState> {
        Box::new(self.clone())
    }
}

#[derive(Default)]
struct FileServer {
    builtin_localrouter: bool,
    calls: Mutex<Vec<(String, Value)>>,
    /// Ask the user before running a tool.
    ask: std::sync::atomic::AtomicBool,
    /// When set, each tool call records how many requests of this client
    /// are in flight (admitted, usage not yet recorded) as it runs.
    in_flight_probe: Mutex<Option<(Arc<lr_router::RateLimiterManager>, String)>>,
    in_flight_seen: Mutex<Vec<u64>>,
}

#[async_trait]
impl VirtualMcpServer for FileServer {
    fn id(&self) -> &str {
        if self.builtin_localrouter {
            lr_mcp::gateway::firewall::ADMIN_MCP_SERVER_ID
        } else {
            "_test_files"
        }
    }
    fn display_name(&self) -> &str {
        "Test files"
    }
    fn owns_tool(&self, tool_name: &str) -> bool {
        tool_name == MCP_TOOL
    }
    fn is_enabled(&self, _client: &Client) -> bool {
        true
    }
    fn list_tools(&self, _state: &dyn VirtualSessionState) -> Vec<McpTool> {
        vec![McpTool {
            name: MCP_TOOL.into(),
            description: Some("Read a file".into()),
            input_schema: json!({"type": "object", "properties": {"path": {"type": "string"}}}),
        }]
    }
    fn check_permissions(
        &self,
        _state: &dyn VirtualSessionState,
        _tool_name: &str,
        _arguments: Option<&Value>,
        _session_approved: bool,
        _session_denied: bool,
    ) -> VirtualFirewallResult {
        if self.ask.load(std::sync::atomic::Ordering::Relaxed) {
            VirtualFirewallResult::Standard(FirewallCheckResult::Ask)
        } else {
            VirtualFirewallResult::Handled(FirewallDecisionResult::Proceed)
        }
    }
    async fn handle_tool_call(
        &self,
        _state: Box<dyn VirtualSessionState>,
        tool_name: &str,
        arguments: Value,
        _client_id: &str,
        _client_name: &str,
    ) -> VirtualToolCallResult {
        self.calls.lock().push((tool_name.to_string(), arguments));
        if let Some((limits, client_id)) = self.in_flight_probe.lock().as_ref() {
            let in_flight = limits.api_key_in_flight(client_id).requests;
            self.in_flight_seen.lock().push(in_flight);
        }
        VirtualToolCallResult::Success(json!({
            "content": [{"type": "text", "text": "FILE-CONTENTS-42"}]
        }))
    }
    fn build_instructions(&self, _state: &dyn VirtualSessionState) -> Option<VirtualInstructions> {
        None
    }
    fn create_session_state(&self, _client: &Client) -> Box<dyn VirtualSessionState> {
        Box::new(NoState)
    }
    fn update_session_state(&self, _state: &mut dyn VirtualSessionState, _client: &Client) {}
    fn all_tool_names(&self) -> Vec<String> {
        vec![MCP_TOOL.into()]
    }
}

// ── Harness ────────────────────────────────────────────────────────────────

struct Harness {
    state: AppState,
    client_id: String,
    provider: Arc<ScriptedProvider>,
    files: Arc<FileServer>,
    _root: tempfile::TempDir,
}

fn text(content: &str) -> CompletionResponse {
    scripted(content, None)
}

fn calls(tool_calls: &[(&str, &str, &str)]) -> CompletionResponse {
    scripted(
        "",
        Some(
            tool_calls
                .iter()
                .map(|(id, name, arguments)| ToolCall {
                    id: id.to_string(),
                    tool_type: "function".into(),
                    function: FunctionCall {
                        name: name.to_string(),
                        arguments: arguments.to_string(),
                    },
                })
                .collect(),
        ),
    )
}

fn scripted(content: &str, tool_calls: Option<Vec<ToolCall>>) -> CompletionResponse {
    let finish_reason = if tool_calls.is_some() {
        "tool_calls"
    } else {
        "stop"
    };
    CompletionResponse {
        id: uuid::Uuid::new_v4().to_string(),
        object: "chat.completion".into(),
        created: 0,
        model: "test-model".into(),
        provider: "mock".into(),
        choices: vec![CompletionChoice {
            index: 0,
            message: ChatMessage {
                annotations: None,
                role: "assistant".into(),
                content: ChatMessageContent::Text(content.into()),
                tool_calls,
                tool_call_id: None,
                name: None,
                reasoning_content: None,
            },
            finish_reason: Some(finish_reason.into()),
            logprobs: None,
        }],
        usage: TokenUsage {
            prompt_tokens: 10,
            completion_tokens: 5,
            total_tokens: 15,
            prompt_tokens_details: None,
            completion_tokens_details: None,
        },
        system_fingerprint: None,
        service_tier: None,
        extensions: None,
        routellm_win_rate: None,
        request_usage_entries: None,
    }
}

/// A server state with one client. `mcp_via_llm` puts the client in
/// MCP-via-LLM mode with one gateway tool (`fs__read`); `configure` adjusts
/// the client's strategy.
async fn harness(
    mcp_via_llm: bool,
    script: Vec<CompletionResponse>,
    configure: impl FnOnce(&mut Strategy),
) -> Harness {
    let root = tempfile::tempdir().unwrap();
    let mut client = Client::new_with_strategy("responses test client".into(), "strategy".into());
    if mcp_via_llm {
        client.mcp_mode = lr_config::McpMode::ViaLlm;
    }
    let mut strategy = Strategy::new("Responses test".into());
    strategy.id = "strategy".into();
    strategy.parent = Some(client.id.clone());
    configure(&mut strategy);
    let config = AppConfig {
        clients: vec![client.clone()],
        strategies: vec![strategy],
        ..Default::default()
    };
    let config = Arc::new(ConfigManager::new(config, root.path().join("config.yaml")));

    let provider = Arc::new(ScriptedProvider {
        responses: Mutex::new(script.into()),
        requests: Mutex::new(Vec::new()),
        stream_usage: Default::default(),
    });
    let providers = Arc::new(lr_providers::registry::ProviderRegistry::new());
    providers.register_factory(Arc::new(ScriptedFactory(provider.clone())));
    providers
        .create_provider("mock".into(), "mock".into(), HashMap::new())
        .await
        .unwrap();

    let limits = Arc::new(lr_router::RateLimiterManager::new(None));
    let metrics = Arc::new(lr_monitoring::metrics::MetricsCollector::new(Arc::new(
        lr_monitoring::storage::MetricsDatabase::new(root.path().join("metrics.db")).unwrap(),
    )));
    let router = Arc::new(lr_router::Router::new(
        config.clone(),
        providers.clone(),
        limits.clone(),
        metrics.clone(),
        Arc::new(lr_router::FreeTierManager::new(None)),
    ));
    let clients = Arc::new(lr_clients::ClientManager::with_keychain(
        vec![client.clone()],
        Arc::new(lr_api_keys::MockKeychain::new()),
    ));
    let state = AppState::new(
        router,
        limits,
        providers,
        config,
        clients,
        Arc::new(lr_clients::TokenStore::new()),
        metrics,
        None,
    );
    let files = Arc::new(FileServer::default());
    if mcp_via_llm {
        state.mcp_gateway.register_virtual_server(files.clone());
    }
    Harness {
        state,
        client_id: client.id,
        provider,
        files,
        _root: root,
    }
}

/// Call the handler as the server would after authentication.
async fn post_as(state: AppState, client_id: String, body: Value) -> (StatusCode, String) {
    let request = serde_json::from_value(body).expect("valid /v1/responses body");
    let result = create_response(
        State(state),
        Extension(AuthContext {
            api_key_id: client_id.clone(),
            credential_scope: None,
            model_selection: None,
        }),
        Some(Extension(ClientAuthContext { client_id })),
        GatewayJson(request),
    )
    .await;
    let response = match result {
        Ok(response) => response,
        Err(error) => error.into_response(),
    };
    let status = response.status();
    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    (status, String::from_utf8(bytes.to_vec()).unwrap())
}

impl Harness {
    async fn post(&self, body: Value) -> (StatusCode, String) {
        post_as(self.state.clone(), self.client_id.clone(), body).await
    }

    async fn post_json(&self, body: Value) -> (StatusCode, Value) {
        let (status, body) = self.post(body).await;
        (status, serde_json::from_str(&body).expect("JSON body"))
    }

    /// Post a streaming request and return its events in order.
    async fn post_stream(&self, mut body: Value) -> Vec<Value> {
        body["stream"] = json!(true);
        let (status, body) = self.post(body).await;
        assert_eq!(status, StatusCode::OK, "{body}");
        sse_events(&body)
    }

    /// Run `body` in the background and wait for the approval popup it raises.
    async fn post_until_approval(
        &self,
        body: Value,
    ) -> (
        tokio::task::JoinHandle<(StatusCode, String)>,
        PendingApprovalInfo,
    ) {
        let task = tokio::spawn(post_as(self.state.clone(), self.client_id.clone(), body));
        let firewall = &self.state.mcp_gateway.firewall_manager;
        let pending = tokio::time::timeout(Duration::from_secs(10), async {
            loop {
                if let Some(pending) = firewall.list_pending().into_iter().next() {
                    break pending;
                }
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
        })
        .await
        .expect("an approval popup is requested");
        (task, pending)
    }

    fn decide(&self, pending: &PendingApprovalInfo, action: FirewallApprovalAction) {
        self.state
            .mcp_gateway
            .firewall_manager
            .submit_response(&pending.request_id, action, None)
            .unwrap();
    }

    fn requests(&self) -> Vec<CompletionRequest> {
        self.provider.requests.lock().clone()
    }
}

fn sse_events(body: &str) -> Vec<Value> {
    body.split("\n\n")
        .filter_map(|block| {
            let mut event = None;
            let mut data = None;
            for line in block.lines() {
                if let Some(name) = line.strip_prefix("event: ") {
                    event = Some(name.to_string());
                } else if let Some(payload) = line.strip_prefix("data: ") {
                    data = Some(payload.to_string());
                }
            }
            let data: Value = serde_json::from_str(&data?).expect("event data is JSON");
            assert_eq!(
                event.as_deref(),
                data["type"].as_str(),
                "the SSE event name matches the payload type"
            );
            Some(data)
        })
        .collect()
}

fn event_types(events: &[Value]) -> Vec<&str> {
    events
        .iter()
        .map(|event| event["type"].as_str().unwrap())
        .collect()
}

/// `sequence_number` starts at 0 and has no gaps.
fn assert_sequenced(events: &[Value]) {
    for (index, event) in events.iter().enumerate() {
        assert_eq!(event["sequence_number"], json!(index), "{event}");
    }
}

/// The conversation a request carries, without system messages (the gateway
/// adds its own instructions as one): role, text, the ids of any tool calls,
/// and the call a tool message answers.
fn outline(request: &CompletionRequest) -> Vec<(String, String, Vec<String>, Option<String>)> {
    request
        .messages
        .iter()
        .filter(|m| m.role != "system")
        .map(|m| {
            (
                m.role.clone(),
                m.content.as_text(),
                m.tool_calls
                    .iter()
                    .flatten()
                    .map(|call| call.id.clone())
                    .collect(),
                m.tool_call_id.clone(),
            )
        })
        .collect()
}

fn row(
    role: &str,
    text: &str,
    calls: &[&str],
    answers: Option<&str>,
) -> (String, String, Vec<String>, Option<String>) {
    (
        role.to_string(),
        text.to_string(),
        calls.iter().map(|c| c.to_string()).collect(),
        answers.map(str::to_string),
    )
}

/// Events without the response's `extensions`, whose `mcp_via_llm`
/// metadata names the gateway tools the turn ran (as metadata, not as items).
fn without_metadata(events: &[Value]) -> Vec<Value> {
    events
        .iter()
        .cloned()
        .map(|mut event| {
            if let Some(response) = event.get_mut("response").and_then(Value::as_object_mut) {
                response.remove("extensions");
            }
            event
        })
        .collect()
}

fn function_call_items(output: &Value) -> Vec<&Value> {
    output
        .as_array()
        .unwrap()
        .iter()
        .filter(|item| item["type"] == "function_call")
        .collect()
}

fn client_tool() -> Value {
    json!({
        "type": "function",
        "name": CLIENT_TOOL,
        "description": "Weather for a city",
        "parameters": {"type": "object", "properties": {"city": {"type": "string"}}}
    })
}

// ── Plain requests ─────────────────────────────────────────────────────────

#[tokio::test]
async fn plain_request_returns_the_message_without_any_approval() {
    let h = harness(
        false,
        vec![text("Hello there"), text("Hello there")],
        |_| {},
    )
    .await;

    let (status, body) = h
        .post_json(json!({"model": MODEL, "input": "Hi", "store": false}))
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let id = body["id"].as_str().unwrap();
    assert!(id.starts_with("resp_"));
    assert_eq!(
        body,
        json!({
            "id": id,
            "object": "response",
            "created_at": body["created_at"],
            "status": "completed",
            // The model the provider reports.
            "model": "test-model",
            "output": [{
                "type": "message",
                "id": format!("msg_{id}"),
                "role": "assistant",
                "content": [{"type": "output_text", "text": "Hello there", "annotations": []}],
            }],
            "usage": {"input_tokens": 10, "output_tokens": 5, "total_tokens": 15},
        })
    );

    let events = h
        .post_stream(json!({"model": MODEL, "input": "Hi", "store": false}))
        .await;
    assert_eq!(
        event_types(&events),
        [
            "response.created",
            "response.output_item.added",
            "response.output_text.delta",
            "response.output_text.delta",
            "response.output_item.done",
            "response.completed",
        ]
    );
    assert_sequenced(&events);
    let completed = &events.last().unwrap()["response"];
    assert_eq!(completed["status"], "completed");
    assert_eq!(completed["output"][0]["content"][0]["text"], "Hello there");

    assert!(h
        .state
        .mcp_gateway
        .firewall_manager
        .list_pending()
        .is_empty());
    let sent = h.requests();
    assert_eq!(sent.len(), 2);
    for request in &sent {
        assert_eq!(outline(request), [row("user", "Hi", &[], None)]);
        assert!(request.tools.is_none(), "nothing is injected");
    }
}

/// A client that is not in MCP-via-LLM mode streams its own tool calls as
/// they arrive, exactly as before.
#[tokio::test]
async fn plain_streaming_tool_call_is_forwarded_delta_by_delta() {
    let h = harness(
        false,
        vec![calls(&[("call-w", CLIENT_TOOL, r#"{"city":"Oslo"}"#)])],
        |_| {},
    )
    .await;

    let events = h
        .post_stream(json!({
            "model": MODEL, "input": "Weather in Oslo?", "store": false,
            "tools": [client_tool()],
        }))
        .await;

    assert_eq!(
        event_types(&events),
        [
            "response.created",
            "response.output_item.added",
            "response.function_call_arguments.delta",
            "response.function_call_arguments.delta",
            "response.output_item.done",
            "response.completed",
        ]
    );
    assert_sequenced(&events);
    let done = &events[4]["item"];
    assert_eq!(done["call_id"], "call-w");
    assert_eq!(done["arguments"], r#"{"city":"Oslo"}"#);
}

// ── Approvals ──────────────────────────────────────────────────────────────

/// A model with permission Ask raises the model popup for normal and for
/// MCP-via-LLM clients, streaming or not; Allow lets the request through and
/// Deny stops it before any provider call.
#[tokio::test]
async fn model_permission_ask_is_honoured() {
    for mcp_via_llm in [false, true] {
        for stream in [false, true] {
            for allow in [true, false] {
                let case = format!("mcp_via_llm={mcp_via_llm} stream={stream} allow={allow}");
                let h = harness(mcp_via_llm, vec![text("Approved answer")], |strategy| {
                    strategy.model_permissions.global = PermissionState::Ask;
                })
                .await;

                let (task, pending) = h
                    .post_until_approval(json!({
                        "model": MODEL, "input": "Hi", "stream": stream, "store": false,
                    }))
                    .await;
                assert!(pending.is_model_request, "{case}");
                assert_eq!(pending.is_mcp_via_llm_request, mcp_via_llm, "{case}");
                assert_eq!(pending.client_id, h.client_id, "{case}");
                let shown: Value =
                    serde_json::from_str(pending.full_arguments.as_deref().unwrap()).unwrap();
                let shown_tools: Vec<&str> = shown["tools"]
                    .as_array()
                    .map(|tools| {
                        tools
                            .iter()
                            .filter_map(|t| t["function"]["name"].as_str())
                            .collect()
                    })
                    .unwrap_or_default();
                assert_eq!(
                    shown_tools.contains(&MCP_TOOL),
                    mcp_via_llm,
                    "{case}: the popup shows the request with the gateway's tools: {shown}"
                );
                assert!(
                    h.requests().is_empty(),
                    "{case}: nothing is sent while the popup is open"
                );

                h.decide(
                    &pending,
                    if allow {
                        FirewallApprovalAction::AllowOnce
                    } else {
                        FirewallApprovalAction::Deny
                    },
                );
                let (status, body) = tokio::time::timeout(Duration::from_secs(10), task)
                    .await
                    .unwrap()
                    .unwrap();

                if allow {
                    assert_eq!(status, StatusCode::OK, "{case}: {body}");
                    assert!(body.contains("Approved answer"), "{case}: {body}");
                    assert_eq!(h.requests().len(), 1, "{case}");
                } else {
                    assert_eq!(status, StatusCode::FORBIDDEN, "{case}: {body}");
                    assert!(body.contains("denied by user"), "{case}: {body}");
                    assert!(h.requests().is_empty(), "{case}");
                }
            }
        }
    }
}

/// Edits made in the model popup of an MCP-via-LLM client reach the provider.
#[tokio::test]
async fn model_popup_edits_reach_the_provider_for_mcp_via_llm_clients() {
    let h = harness(true, vec![text("ok")], |strategy| {
        strategy.model_permissions.global = PermissionState::Ask;
    })
    .await;
    let (task, pending) = h
        .post_until_approval(json!({"model": MODEL, "input": "Hi", "store": false}))
        .await;
    h.state
        .mcp_gateway
        .firewall_manager
        .submit_response(
            &pending.request_id,
            FirewallApprovalAction::AllowOnce,
            Some(json!({"temperature": 0.25})),
        )
        .unwrap();
    let (status, body) = task.await.unwrap();
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(h.requests()[0].temperature, Some(0.25));
}

/// `localrouter/auto` with permission Ask raises the auto-router popup.
#[tokio::test]
async fn auto_router_ask_is_honoured() {
    for stream in [false, true] {
        for allow in [true, false] {
            let case = format!("stream={stream} allow={allow}");
            let h = harness(false, vec![text("Routed answer")], |strategy| {
                let auto = strategy.auto_config.as_mut().unwrap();
                auto.permission = PermissionState::Ask;
                auto.prioritized_models = vec![("mock".into(), "test-model".into())];
            })
            .await;

            let (task, pending) = h
                .post_until_approval(json!({
                    "model": "localrouter/auto", "input": "Hi", "stream": stream, "store": false,
                }))
                .await;
            assert!(pending.is_auto_router_request, "{case}");
            h.decide(
                &pending,
                if allow {
                    FirewallApprovalAction::AllowOnce
                } else {
                    FirewallApprovalAction::Deny
                },
            );
            let (status, body) = tokio::time::timeout(Duration::from_secs(10), task)
                .await
                .unwrap()
                .unwrap();
            if allow {
                assert_eq!(status, StatusCode::OK, "{case}: {body}");
                assert!(body.contains("Routed answer"), "{case}: {body}");
            } else {
                assert_eq!(status, StatusCode::FORBIDDEN, "{case}: {body}");
                assert!(h.requests().is_empty(), "{case}");
            }
        }
    }
}

/// A free-tier-only strategy asked for a paid model raises the paid-fallback
/// popup; Allow retries on the paid model and Deny answers 402.
#[tokio::test]
async fn free_tier_fallback_ask_is_honoured() {
    for stream in [false, true] {
        for allow in [true, false] {
            let case = format!("stream={stream} allow={allow}");
            let h = harness(false, vec![text("Paid answer")], |strategy| {
                strategy.free_tier_only = true;
                strategy.free_tier_fallback = FreeTierFallback::Ask;
            })
            .await;

            let (task, pending) = h
                .post_until_approval(json!({
                    "model": MODEL, "input": "Hi", "stream": stream, "store": false,
                }))
                .await;
            assert!(pending.is_free_tier_fallback, "{case}");
            assert!(h.requests().is_empty(), "{case}");
            h.decide(
                &pending,
                if allow {
                    FirewallApprovalAction::AllowOnce
                } else {
                    FirewallApprovalAction::Deny
                },
            );
            let (status, body) = tokio::time::timeout(Duration::from_secs(10), task)
                .await
                .unwrap()
                .unwrap();
            if allow {
                assert_eq!(status, StatusCode::OK, "{case}: {body}");
                assert!(body.contains("Paid answer"), "{case}: {body}");
                assert_eq!(h.requests().len(), 1, "{case}");
            } else {
                assert_eq!(status, StatusCode::PAYMENT_REQUIRED, "{case}: {body}");
                assert!(h.requests().is_empty(), "{case}");
            }
        }
    }
}

#[tokio::test]
async fn localrouter_direct_preview_uses_only_the_existing_internal_test_credential() {
    use axum::{body::Body, extract::ConnectInfo, http::Request};
    use futures::StreamExt;
    use lr_mcp::gateway::firewall::ADMIN_MCP_SERVER_ID;
    use std::net::SocketAddr;
    use std::time::Duration;
    use tokio_util::sync::CancellationToken;
    use tower::ServiceExt;

    let h = harness(false, vec![], |_| {}).await;
    let state = h
        .state
        .with_mcp(Arc::new(lr_mcp::McpServerManager::new_for_test()));
    state
        .mcp_gateway
        .register_virtual_server(Arc::new(FileServer {
            builtin_localrouter: true,
            ..Default::default()
        }));
    let token = state.internal_test_secret.clone();
    let app = crate::build_app(state, false, CancellationToken::new());
    let request = |method, path: &str, peer: &str| {
        Request::builder()
            .method(method)
            .uri(path)
            .header("host", "127.0.0.1")
            .header("authorization", format!("Bearer {token}"))
            .extension(ConnectInfo(peer.parse::<SocketAddr>().unwrap()))
    };

    for (path, peer, target, expected, visible) in [
        ("/", "127.0.0.1:1234", true, StatusCode::OK, true),
        ("/", "127.0.0.1:1234", false, StatusCode::OK, false),
        ("/", "192.0.2.1:1234", true, StatusCode::FORBIDDEN, false),
        (
            "/mcp/localrouter",
            "127.0.0.1:1234",
            true,
            StatusCode::FORBIDDEN,
            false,
        ),
    ] {
        let mut http = request("POST", path, peer).header("content-type", "application/json");
        if target {
            http = http.header("x-mcp-access", ADMIN_MCP_SERVER_ID);
        }
        let response = app
            .clone()
            .oneshot(
                http.body(Body::from(
                    json!({"jsonrpc":"2.0","id":1,"method":"tools/list"}).to_string(),
                ))
                .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), expected, "{path} from {peer}");
        let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        assert_eq!(
            String::from_utf8_lossy(&bytes).contains(MCP_TOOL),
            visible,
            "{path} from {peer}"
        );
    }

    let loopback = "127.0.0.1:1234";
    let stream = app
        .clone()
        .oneshot(
            request("GET", "/", loopback)
                .header("accept", "text/event-stream")
                .header("x-mcp-access", ADMIN_MCP_SERVER_ID)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(stream.status(), StatusCode::OK);
    let mut events = stream.into_body().into_data_stream();
    let endpoint = tokio::time::timeout(Duration::from_secs(5), events.next())
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    let endpoint = std::str::from_utf8(&endpoint)
        .unwrap()
        .lines()
        .find_map(|line| line.strip_prefix("data: "))
        .unwrap();
    assert!(endpoint.starts_with("/?sessionId="));

    let response = app
        .clone()
        .oneshot(
            request("POST", endpoint, loopback)
                .header("content-type", "application/json")
                .header("x-mcp-access", ADMIN_MCP_SERVER_ID)
                .body(Body::from(
                    json!({"jsonrpc":"2.0","id":2,"method":"tools/list"}).to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::ACCEPTED);
    let listed = tokio::time::timeout(Duration::from_secs(5), events.next())
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    assert!(std::str::from_utf8(&listed).unwrap().contains(MCP_TOOL));
}

#[tokio::test]
async fn virtual_tool_arguments_and_results_follow_monitor_capture_policy() {
    use lr_monitor::{MonitorEventData, MonitorEventFilter, MonitorEventType};

    let h = harness(false, vec![], |_| {}).await;
    let state = h
        .state
        .with_mcp(Arc::new(lr_mcp::McpServerManager::new_for_test()));
    state.mcp_gateway.register_virtual_server(h.files);

    let arguments = json!({"path": "/tmp/a"});
    let filter = MonitorEventFilter {
        event_types: Some(vec![MonitorEventType::McpToolCall]),
        ..Default::default()
    };
    for metadata_only in [false, true] {
        state
            .monitor_store
            .set_payload_policy(move |_| metadata_only);
        let result = state
            .mcp_gateway
            .handle_request(
                &h.client_id,
                vec![],
                vec![],
                lr_mcp::protocol::JsonRpcRequest::new(
                    Some(json!(1)),
                    "tools/call".into(),
                    Some(json!({"name": MCP_TOOL, "arguments": arguments})),
                ),
            )
            .await
            .unwrap();
        assert_eq!(
            result.result.unwrap()["content"][0]["text"],
            "FILE-CONTENTS-42"
        );

        let summary = &state.monitor_store.list(0, 1, Some(&filter)).events[0];
        let event = state.monitor_store.get(&summary.id).unwrap();
        assert_eq!(event.payload_redacted, metadata_only);
        let MonitorEventData::McpToolCall {
            arguments: captured,
            response_preview,
            ..
        } = event.data
        else {
            panic!("expected an MCP tool call");
        };
        if metadata_only {
            assert!(captured.is_null());
            assert!(response_preview
                .as_deref()
                .is_none_or(|preview| !preview.contains("FILE-CONTENTS-42")));
        } else {
            assert_eq!(captured, arguments);
            assert_eq!(summary.answer, "FILE-CONTENTS-42");
            assert!(response_preview.unwrap().contains("FILE-CONTENTS-42"));
        }
    }
}

// ── MCP via LLM ────────────────────────────────────────────────────────────

fn tool_loop_script() -> Vec<CompletionResponse> {
    vec![
        calls(&[("call-1", MCP_TOOL, r#"{"path":"/tmp/a"}"#)]),
        text("The file says FILE-CONTENTS-42"),
    ]
}

/// What the provider must have seen across a gateway-executed tool loop.
fn assert_tool_loop_ran(h: &Harness) {
    assert_eq!(
        *h.files.calls.lock(),
        [(MCP_TOOL.to_string(), json!({"path": "/tmp/a"}))],
        "the gateway executed the tool once"
    );
    let sent = h.requests();
    assert_eq!(sent.len(), 2);
    let injected: Vec<&str> = sent[0]
        .tools
        .iter()
        .flatten()
        .map(|t| t.function.name.as_str())
        .collect();
    assert!(injected.contains(&MCP_TOOL), "{injected:?}");
    assert_eq!(
        outline(&sent[1]),
        [
            row("user", "Read /tmp/a", &[], None),
            row("assistant", "", &["call-1"], None),
            row("tool", "FILE-CONTENTS-42", &[], Some("call-1")),
        ]
    );
}

#[tokio::test]
async fn mcp_tool_loop_non_streaming_returns_only_the_final_message() {
    let h = harness(true, tool_loop_script(), |_| {}).await;

    let (status, body) = h
        .post_json(json!({"model": MODEL, "input": "Read /tmp/a"}))
        .await;

    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["status"], "completed");
    let output = body["output"].as_array().unwrap();
    assert_eq!(output.len(), 1, "{body}");
    assert_eq!(output[0]["type"], "message");
    assert_eq!(
        output[0]["content"][0]["text"],
        "The file says FILE-CONTENTS-42"
    );
    assert_eq!(
        body["usage"]["input_tokens"], 20,
        "usage covers both iterations"
    );
    assert_tool_loop_ran(&h);
}

#[tokio::test]
async fn mcp_tool_loop_streaming_hides_the_gateway_executed_call() {
    let h = harness(true, tool_loop_script(), |_| {}).await;

    let events = h
        .post_stream(json!({"model": MODEL, "input": "Read /tmp/a"}))
        .await;

    assert_eq!(
        event_types(&events),
        [
            "response.created",
            "response.output_item.added",
            "response.output_text.delta",
            "response.output_text.delta",
            "response.output_item.done",
            "response.completed",
        ]
    );
    assert_sequenced(&events);
    let message_id = events[1]["item"]["id"].as_str().unwrap();
    assert_eq!(events[1]["item"]["type"], "message");
    assert_eq!(events[1]["output_index"], 0);
    for event in &events[2..4] {
        assert_eq!(event["item_id"], message_id);
        assert_eq!(event["output_index"], 0);
    }
    assert_eq!(events[4]["item"]["id"], message_id);
    let completed = &events[5]["response"];
    assert_eq!(completed["status"], "completed");
    assert_eq!(completed["output"].as_array().unwrap().len(), 1);
    assert_eq!(
        completed["output"][0]["content"][0]["text"],
        "The file says FILE-CONTENTS-42"
    );
    assert!(
        !serde_json::to_string(&without_metadata(&events))
            .unwrap()
            .contains(MCP_TOOL),
        "no event mentions the gateway-executed tool"
    );
    assert_tool_loop_ran(&h);
}

/// A gateway tool whose permission is Ask raises the tool popup mid-turn.
/// Allow runs the tool; Deny tells the model the call was refused and the
/// turn still completes.
#[tokio::test]
async fn mcp_tool_approval_is_honoured() {
    for stream in [false, true] {
        for allow in [true, false] {
            let case = format!("stream={stream} allow={allow}");
            let h = harness(true, tool_loop_script(), |_| {}).await;
            h.files
                .ask
                .store(true, std::sync::atomic::Ordering::Relaxed);

            let (task, pending) = h
                .post_until_approval(json!({
                    "model": MODEL, "input": "Read /tmp/a", "stream": stream, "store": false,
                }))
                .await;
            assert!(!pending.is_model_request, "{case}");
            assert_eq!(pending.tool_name, MCP_TOOL, "{case}");
            assert!(h.files.calls.lock().is_empty(), "{case}");
            h.decide(
                &pending,
                if allow {
                    FirewallApprovalAction::AllowOnce
                } else {
                    FirewallApprovalAction::Deny
                },
            );
            let (status, body) = tokio::time::timeout(Duration::from_secs(10), task)
                .await
                .unwrap()
                .unwrap();

            assert_eq!(status, StatusCode::OK, "{case}: {body}");
            assert!(body.contains("The file says"), "{case}: {body}");
            let sent = h.requests();
            assert_eq!(sent.len(), 2, "{case}");
            let tool_result = sent[1].messages.last().unwrap();
            assert_eq!(tool_result.role, "tool", "{case}");
            assert_eq!(
                tool_result.content.as_text().contains("FILE-CONTENTS-42"),
                allow,
                "{case}: {}",
                tool_result.content.as_text()
            );
            assert_eq!(h.files.calls.lock().len(), usize::from(allow), "{case}");
        }
    }
}

/// The next turn names the response id: the orchestrator session is found by
/// it and the hidden call and result are replayed to the model, even though
/// new `instructions` make the visible history differ from the first turn.
#[tokio::test]
async fn continuation_replays_the_hidden_tool_history() {
    for stream in [false, true] {
        let mut script = tool_loop_script();
        script.push(text("Nothing else"));
        let h = harness(true, script, |_| {}).await;

        let first = json!({"model": MODEL, "input": "Read /tmp/a"});
        let first_id = if stream {
            let events = h.post_stream(first).await;
            events.last().unwrap()["response"]["id"]
                .as_str()
                .unwrap()
                .to_string()
        } else {
            h.post_json(first).await.1["id"]
                .as_str()
                .unwrap()
                .to_string()
        };

        let (status, body) = h
            .post_json(json!({
                "model": MODEL,
                "input": "Anything else?",
                "instructions": "Be brief",
                "previous_response_id": first_id,
            }))
            .await;
        assert_eq!(status, StatusCode::OK, "stream={stream}: {body}");
        assert_eq!(body["output"][0]["content"][0]["text"], "Nothing else");

        let third = &h.requests()[2];
        assert_eq!(
            (
                third.messages[0].role.as_str(),
                third.messages[0].content.as_text().as_str()
            ),
            ("system", "Be brief"),
            "stream={stream}: this turn's instructions lead the conversation"
        );
        assert_eq!(
            outline(third),
            [
                row("user", "Read /tmp/a", &[], None),
                row("assistant", "", &["call-1"], None),
                row("tool", "FILE-CONTENTS-42", &[], Some("call-1")),
                row("assistant", "The file says FILE-CONTENTS-42", &[], None),
                row("user", "Anything else?", &[], None),
            ],
            "stream={stream}"
        );
    }
}

/// A call to one of the client's own tools is surfaced once, complete, with
/// one item id across its events; the client's answer on the next turn
/// reaches the model.
#[tokio::test]
async fn client_tool_call_on_an_mcp_via_llm_client_round_trips() {
    for stream in [false, true] {
        let h = harness(
            true,
            vec![
                calls(&[("call-w", CLIENT_TOOL, r#"{"city":"Oslo"}"#)]),
                text("It is sunny in Oslo"),
            ],
            |_| {},
        )
        .await;
        let first = json!({
            "model": MODEL, "input": "Weather in Oslo?", "tools": [client_tool()],
        });

        let response = if stream {
            let events = h.post_stream(first).await;
            assert_eq!(
                event_types(&events),
                [
                    "response.created",
                    "response.output_item.added",
                    "response.function_call_arguments.delta",
                    "response.output_item.done",
                    "response.completed",
                ]
            );
            assert_sequenced(&events);
            let item_id = events[1]["item"]["id"].as_str().unwrap();
            assert_eq!(events[2]["item_id"], item_id);
            assert_eq!(events[2]["delta"], r#"{"city":"Oslo"}"#);
            assert_eq!(events[3]["item"]["id"], item_id);
            events.last().unwrap()["response"].clone()
        } else {
            h.post_json(first).await.1
        };

        let items = function_call_items(&response["output"]);
        assert_eq!(items.len(), 1, "stream={stream}: {response}");
        assert_eq!(items[0]["call_id"], "call-w");
        assert_eq!(items[0]["name"], CLIENT_TOOL);
        assert_eq!(items[0]["arguments"], r#"{"city":"Oslo"}"#);
        assert!(h.files.calls.lock().is_empty());

        let (status, body) = h
            .post_json(json!({
                "model": MODEL,
                "previous_response_id": response["id"],
                "input": [{"type": "function_call_output", "call_id": "call-w", "output": "sunny"}],
                "tools": [client_tool()],
            }))
            .await;
        assert_eq!(status, StatusCode::OK, "stream={stream}: {body}");
        assert_eq!(
            body["output"][0]["content"][0]["text"],
            "It is sunny in Oslo"
        );
        assert_eq!(
            outline(&h.requests()[1]),
            [
                row("user", "Weather in Oslo?", &[], None),
                row("assistant", "", &["call-w"], None),
                row("tool", "sunny", &[], Some("call-w")),
            ],
            "stream={stream}"
        );
    }
}

/// When the model calls a gateway tool and a client tool in one message, the
/// client is shown only its own call. Its answer resumes the turn with the
/// gateway's result alongside.
#[tokio::test]
async fn mixed_tool_calls_surface_only_the_client_owned_call() {
    for stream in [false, true] {
        let h = harness(
            true,
            vec![
                calls(&[
                    ("call-1", MCP_TOOL, r#"{"path":"/tmp/a"}"#),
                    ("call-w", CLIENT_TOOL, r#"{"city":"Oslo"}"#),
                ]),
                text("Sunny, and the file says FILE-CONTENTS-42"),
            ],
            |_| {},
        )
        .await;
        let first = json!({
            "model": MODEL, "input": "Weather and file?", "tools": [client_tool()],
        });

        let response = if stream {
            let events = h.post_stream(first).await;
            assert!(
                !serde_json::to_string(&without_metadata(&events))
                    .unwrap()
                    .contains(MCP_TOOL),
                "no event mentions the gateway-executed tool"
            );
            events.last().unwrap()["response"].clone()
        } else {
            h.post_json(first).await.1
        };
        let items = function_call_items(&response["output"]);
        assert_eq!(items.len(), 1, "stream={stream}: {response}");
        assert_eq!(items[0]["call_id"], "call-w");
        assert_eq!(items[0]["arguments"], r#"{"city":"Oslo"}"#);

        let (status, body) = h
            .post_json(json!({
                "model": MODEL,
                "previous_response_id": response["id"],
                "input": [{"type": "function_call_output", "call_id": "call-w", "output": "sunny"}],
                "tools": [client_tool()],
            }))
            .await;
        assert_eq!(status, StatusCode::OK, "stream={stream}: {body}");
        assert_eq!(
            body["output"][0]["content"][0]["text"],
            "Sunny, and the file says FILE-CONTENTS-42"
        );
        assert_eq!(h.files.calls.lock().len(), 1, "stream={stream}");

        let resumed = outline(&h.requests()[1]);
        assert_eq!(
            resumed[..2],
            [
                row("user", "Weather and file?", &[], None),
                row("assistant", "", &["call-1", "call-w"], None),
            ],
            "stream={stream}"
        );
        let mut answers: Vec<_> = resumed[2..].to_vec();
        answers.sort();
        assert_eq!(
            answers,
            [
                row("tool", "FILE-CONTENTS-42", &[], Some("call-1")),
                row("tool", "sunny", &[], Some("call-w")),
            ],
            "stream={stream}"
        );
    }
}

// ── Follow-ups: instructions, branching, message items, input items ───────

fn response_id_of(response: &Value) -> String {
    response["id"].as_str().unwrap().to_string()
}

impl Harness {
    /// Post one turn, streaming or not, and return the final response object.
    async fn turn(&self, mut body: Value, stream: bool) -> Value {
        if stream {
            let events = self.post_stream(body).await;
            events.last().unwrap()["response"].clone()
        } else {
            body["stream"] = json!(false);
            let (status, body) = self.post_json(body).await;
            assert_eq!(status, StatusCode::OK, "{body}");
            body
        }
    }

    fn system_texts(&self, request: usize) -> Vec<String> {
        self.requests()[request]
            .messages
            .iter()
            .filter(|m| m.role == "system")
            .map(|m| m.content.as_text())
            .collect()
    }

    fn llm_call_events(&self) -> Vec<lr_monitor::MonitorEvent> {
        let mut events: Vec<_> = self
            .state
            .monitor_store
            .list(0, 1000, None)
            .events
            .into_iter()
            .filter_map(|summary| self.state.monitor_store.get(&summary.id))
            .filter(|event| matches!(event.data, lr_monitor::MonitorEventData::LlmCall { .. }))
            .collect();
        events.sort_by_key(|event| event.sequence);
        events
    }
}

/// `instructions` apply to the request that carries them: a continuation,
/// from the orchestrator session or from stored history, does not run with
/// an earlier turn's instructions.
#[tokio::test]
async fn instructions_are_not_carried_into_later_turns() {
    for stream in [false, true] {
        for expire in [false, true] {
            let case = format!("stream={stream} expire={expire}");
            let mut script = tool_loop_script();
            script.push(text("Nothing else"));
            script.push(text("Still nothing"));
            let h = harness(true, script, |_| {}).await;
            let first = h
                .turn(
                    json!({"model": MODEL, "input": "Read /tmp/a", "instructions": "Be formal"}),
                    stream,
                )
                .await;
            if expire {
                let mut expired = h.state.mcp_via_llm_manager.config();
                expired.session_ttl_seconds = 0;
                h.state.mcp_via_llm_manager.update_config(expired);
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
            let second = h
                .turn(
                    json!({"model": MODEL, "input": "Anything else?",
                           "previous_response_id": response_id_of(&first)}),
                    false,
                )
                .await;
            assert!(
                !h.system_texts(2).contains(&"Be formal".to_string()),
                "{case}: {:?}",
                h.system_texts(2)
            );
            let _ = h
                .turn(
                    json!({"model": MODEL, "input": "And now?", "instructions": "Be brief",
                           "previous_response_id": response_id_of(&second)}),
                    false,
                )
                .await;
            let systems = h.system_texts(3);
            assert_eq!(
                systems.first().map(String::as_str),
                Some("Be brief"),
                "{case}"
            );
            assert!(
                !systems.contains(&"Be formal".to_string()),
                "{case}: {systems:?}"
            );
        }
    }
}

/// Two turns continuing from one response each see that response's
/// history (hidden tool calls included), not each other's.
#[tokio::test]
async fn two_continuations_of_one_response_each_get_its_history() {
    for stream in [false, true] {
        let mut script = tool_loop_script();
        script.push(text("Answer A"));
        script.push(text("Answer B"));
        let h = harness(true, script, |_| {}).await;
        let first = h
            .turn(json!({"model": MODEL, "input": "Read /tmp/a"}), stream)
            .await;
        let id = response_id_of(&first);
        for (question, request) in [("Branch A?", 2), ("Branch B?", 3)] {
            let _ = h
                .turn(
                    json!({"model": MODEL, "input": question, "previous_response_id": id}),
                    stream,
                )
                .await;
            assert_eq!(
                outline(&h.requests()[request]),
                [
                    row("user", "Read /tmp/a", &[], None),
                    row("assistant", "", &["call-1"], None),
                    row("tool", "FILE-CONTENTS-42", &[], Some("call-1")),
                    row("assistant", "The file says FILE-CONTENTS-42", &[], None),
                    row("user", question, &[], None),
                ],
                "stream={stream} {question}"
            );
        }
    }
}

/// A failed turn does not take over the session of the response it
/// continued: a retry from the same response still sees its hidden history.
#[tokio::test]
async fn a_failed_continuation_can_be_retried_from_the_same_response() {
    let h = harness(true, tool_loop_script(), |_| {}).await;
    let first = h
        .turn(json!({"model": MODEL, "input": "Read /tmp/a"}), false)
        .await;
    let id = response_id_of(&first);
    // Nothing left in the script: this turn's LLM call fails.
    let (status, _) = h
        .post(json!({"model": MODEL, "input": "Again?", "previous_response_id": id}))
        .await;
    assert!(!status.is_success());
    h.provider.responses.lock().push_back(text("Retried"));
    let retried = h
        .turn(
            json!({"model": MODEL, "input": "Again?", "previous_response_id": id}),
            false,
        )
        .await;
    assert_eq!(retried["output"][0]["content"][0]["text"], "Retried");
    let last = h.requests().len() - 1;
    assert_eq!(
        outline(&h.requests()[last])[1],
        row("assistant", "", &["call-1"], None),
        "the retry continued from the first turn's hidden history"
    );
}

/// Text the model writes before a gateway-executed call is a message of
/// its own: one `message` item per assistant message, streaming and not,
/// and a continuation still finds the hidden history.
#[tokio::test]
async fn text_before_a_gateway_call_is_its_own_message_item() {
    for stream in [false, true] {
        let script = vec![
            scripted(
                "Let me look.",
                Some(vec![ToolCall {
                    id: "call-1".into(),
                    tool_type: "function".into(),
                    function: FunctionCall {
                        name: MCP_TOOL.into(),
                        arguments: r#"{"path":"/tmp/a"}"#.into(),
                    },
                }]),
            ),
            text("It says FILE-CONTENTS-42"),
            text("Nothing else"),
        ];
        let h = harness(true, script, |_| {}).await;
        let body = json!({"model": MODEL, "input": "Read /tmp/a"});
        let response = if stream {
            let events = h.post_stream(body).await;
            assert_sequenced(&events);
            let added: Vec<&Value> = events
                .iter()
                .filter(|e| e["type"] == "response.output_item.added")
                .collect();
            assert_eq!(added.len(), 2, "{events:?}");
            assert_ne!(added[0]["item"]["id"], added[1]["item"]["id"]);
            assert_eq!(added[0]["output_index"], 0);
            assert_eq!(added[1]["output_index"], 1);
            let done: Vec<&Value> = events
                .iter()
                .filter(|e| e["type"] == "response.output_item.done")
                .collect();
            assert_eq!(done.len(), 2);
            assert_eq!(done[0]["item"]["content"][0]["text"], "Let me look.");
            events.last().unwrap()["response"].clone()
        } else {
            h.post_json(body).await.1
        };
        let output = response["output"].as_array().unwrap();
        let texts: Vec<&Value> = output
            .iter()
            .map(|item| &item["content"][0]["text"])
            .collect();
        assert_eq!(
            texts,
            [&json!("Let me look."), &json!("It says FILE-CONTENTS-42")],
            "stream={stream}: {response}"
        );
        let id = response_id_of(&response);
        let ids: Vec<&Value> = output.iter().map(|item| &item["id"]).collect();
        assert_eq!(
            ids,
            [&json!(format!("msg_{id}")), &json!(format!("msg_{id}_1"))],
            "stream={stream}"
        );

        let _ = h
            .turn(
                json!({"model": MODEL, "input": "More?", "previous_response_id": id}),
                false,
            )
            .await;
        assert_eq!(
            outline(&h.requests()[2]),
            [
                row("user", "Read /tmp/a", &[], None),
                row("assistant", "Let me look.", &["call-1"], None),
                row("tool", "FILE-CONTENTS-42", &[], Some("call-1")),
                row("assistant", "It says FILE-CONTENTS-42", &[], None),
                row("user", "More?", &[], None),
            ],
            "stream={stream}"
        );
    }
}

/// Reasoning items are dropped for a chat upstream; custom tool items
/// become a tool call and its result; an item type with no chat form is
/// rejected.
#[tokio::test]
async fn input_items_are_translated_or_rejected() {
    let h = harness(false, vec![text("Patched")], |_| {}).await;
    let (status, body) = h
        .post_json(json!({
            "model": MODEL,
            "store": false,
            "tools": [{"type": "custom", "name": "apply_patch", "description": "Apply a patch",
                       "format": {"type": "grammar", "syntax": "lark", "definition": "start: /.+/"}}],
            "input": [
                {"type": "message", "role": "user", "content": "Patch it"},
                {"type": "reasoning", "id": "rs_1", "summary": [], "encrypted_content": "opaque"},
                {"type": "custom_tool_call", "call_id": "c1", "name": "apply_patch", "input": "*** Begin Patch"},
                {"type": "custom_tool_call_output", "call_id": "c1", "output": "Done"},
            ],
        }))
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let sent = &h.requests()[0];
    assert_eq!(
        outline(sent),
        [
            row("user", "Patch it", &[], None),
            row("assistant", "", &["c1"], None),
            row("tool", "Done", &[], Some("c1")),
        ]
    );
    let call = &sent.messages[1].tool_calls.as_ref().unwrap()[0];
    assert_eq!(call.function.name, "apply_patch");
    assert_eq!(call.function.arguments, "*** Begin Patch");
    assert!(
        !sent.extensions.as_ref().is_some_and(
            |e| e.contains_key(lr_providers::openai_responses::INPUT_REASONING_EXTENSION)
        ),
        "a chat-completions upstream gets no reasoning items"
    );
    let tools = sent.tools.as_ref().unwrap();
    assert_eq!(tools[0].tool_type, "custom");
    assert_eq!(tools[0].function.parameters["type"], "grammar");

    let (status, body) = h
        .post(json!({
            "model": MODEL,
            "input": [{"type": "web_search_call", "id": "ws_1", "status": "completed"}],
        }))
        .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
    assert!(body.contains("web_search_call"), "{body}");
}

#[test]
fn reasoning_items_are_anchored_to_the_output_that_followed_them() {
    let req: super::CreateResponseRequest = serde_json::from_value(json!({
        "model": MODEL,
        "input": [
            {"type": "message", "role": "user", "content": "Hi"},
            {"type": "reasoning", "summary": [], "encrypted_content": "r1"},
            {"type": "function_call", "call_id": "call-w", "name": CLIENT_TOOL, "arguments": "{}"},
            {"type": "function_call_output", "call_id": "call-w", "output": "sunny"},
            {"type": "reasoning", "summary": [], "encrypted_content": "r2"},
            {"type": "message", "role": "assistant", "content": [{"type": "output_text", "text": "Sunny"}]},
            {"type": "reasoning", "summary": [], "encrypted_content": "dangling"},
        ],
    }))
    .unwrap();
    let chat = super::build_chat_completion_request(&req, vec![], None, vec![]).unwrap();
    let carried = &chat.extensions.as_ref().unwrap()
        [lr_providers::openai_responses::INPUT_REASONING_EXTENSION];
    assert_eq!(
        carried,
        &json!([
            {"call_id": "call-w", "item": {"type": "reasoning", "summary": [], "encrypted_content": "r1"}},
            {"assistant_text": "Sunny", "item": {"type": "reasoning", "summary": [], "encrypted_content": "r2"}},
        ])
    );

    // A Responses upstream gets them back in place.
    let provider = crate::routes::pipeline::convert_to_provider_request(&chat).unwrap();
    let translated = lr_providers::openai_responses::translate_completion_request(&provider, false);
    let kinds: Vec<String> = translated
        .input
        .iter()
        .map(|item| {
            serde_json::to_value(item).unwrap()["type"]
                .as_str()
                .unwrap()
                .to_string()
        })
        .collect();
    assert_eq!(
        kinds,
        [
            "message",
            "reasoning",
            "function_call",
            "function_call_output",
            "reasoning",
            "message"
        ]
    );
}

/// The orchestrator records a streamed MCP-via-LLM turn on the monitor call
/// by call: the request's own event keeps the first call's detail, and the
/// later call's event carries the endpoint and session of the request.
#[tokio::test]
async fn responses_stream_keeps_the_orchestrators_monitor_record() {
    let h = harness(true, tool_loop_script(), |_| {}).await;
    let _ = h
        .post_stream(json!({"model": MODEL, "input": "Read /tmp/a", "store": false}))
        .await;
    let events = h.llm_call_events();
    assert_eq!(events.len(), 2, "{events:?}");
    let lr_monitor::MonitorEventData::LlmCall {
        content_preview, ..
    } = &events[0].data
    else {
        unreachable!()
    };
    assert!(
        content_preview
            .as_deref()
            .is_some_and(|p| p.starts_with("[tool_calls: fs__read]")),
        "{content_preview:?}"
    );
    let lr_monitor::MonitorEventData::LlmCall { endpoint, .. } = &events[1].data else {
        unreachable!()
    };
    assert_eq!(endpoint, "/v1/responses");
    assert_eq!(events[1].session_id, events[0].session_id);
    assert!(events
        .iter()
        .all(|e| e.status == lr_monitor::EventStatus::Complete));
}

// ── Chat completions on an MCP-via-LLM client ──────────────────────────────

impl Harness {
    async fn post_chat(&self, body: Value) -> (StatusCode, String) {
        let request = serde_json::from_value(body).expect("valid chat body");
        let result = crate::routes::chat::chat_completions(
            State(self.state.clone()),
            Extension(AuthContext {
                api_key_id: self.client_id.clone(),
                credential_scope: None,
                model_selection: None,
            }),
            Some(Extension(ClientAuthContext {
                client_id: self.client_id.clone(),
            })),
            GatewayJson(request),
        )
        .await;
        let response = match result {
            Ok(response) => response,
            Err(error) => error.into_response(),
        };
        let status = response.status();
        let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        (status, String::from_utf8(bytes.to_vec()).unwrap())
    }
}

fn chat_chunks(body: &str) -> Vec<Value> {
    body.lines()
        .filter_map(|line| line.strip_prefix("data: "))
        .filter(|data| *data != "[DONE]")
        .map(|data| serde_json::from_str(data).expect("chunk is JSON"))
        .collect()
}

/// What an accumulating chat client ends up with: `(index, id, name,
/// arguments)` per tool call.
fn chat_tool_calls(chunks: &[Value]) -> Vec<(u64, String, String, String)> {
    let mut calls: Vec<(u64, String, String, String)> = Vec::new();
    for chunk in chunks {
        for delta in chunk["choices"][0]["delta"]["tool_calls"]
            .as_array()
            .into_iter()
            .flatten()
        {
            let index = delta["index"].as_u64().unwrap();
            if !calls.iter().any(|c| c.0 == index) {
                calls.push((index, String::new(), String::new(), String::new()));
            }
            let call = calls.iter_mut().find(|c| c.0 == index).unwrap();
            if let Some(id) = delta["id"].as_str() {
                call.1 = id.to_string();
            }
            if let Some(name) = delta["function"]["name"].as_str() {
                call.2 = name.to_string();
            }
            if let Some(arguments) = delta["function"]["arguments"].as_str() {
                call.3.push_str(arguments);
            }
        }
    }
    calls
}

fn chat_client_tool() -> Value {
    json!({"type": "function", "function": {
        "name": CLIENT_TOOL, "description": "Weather for a city",
        "parameters": {"type": "object", "properties": {"city": {"type": "string"}}}}})
}

/// Chat streaming never shows a gateway-executed call, and the client's
/// own call arrives once, numbered among the client's calls, with its
/// arguments intact.
#[tokio::test]
async fn chat_stream_shows_only_the_clients_calls_once() {
    for mixed in [false, true] {
        let mut tool_calls = vec![("call-w", CLIENT_TOOL, r#"{"city":"Oslo"}"#)];
        if mixed {
            tool_calls.insert(0, ("call-1", MCP_TOOL, r#"{"path":"/tmp/a"}"#));
        }
        let h = harness(true, vec![calls(&tool_calls)], |_| {}).await;
        let (status, body) = h
            .post_chat(json!({
                "model": MODEL, "stream": true, "tools": [chat_client_tool()],
                "messages": [{"role": "user", "content": "Weather and file?"}],
            }))
            .await;
        assert_eq!(status, StatusCode::OK, "{body}");
        let chunks = chat_chunks(&body);
        assert!(
            chunks
                .iter()
                .all(|c| !c["choices"].to_string().contains(MCP_TOOL)),
            "mixed={mixed}: {body}"
        );
        assert_eq!(
            chat_tool_calls(&chunks),
            [(
                0,
                "call-w".to_string(),
                CLIENT_TOOL.to_string(),
                r#"{"city":"Oslo"}"#.to_string()
            )],
            "mixed={mixed}"
        );
        let finish: Vec<&Value> = chunks
            .iter()
            .filter(|c| c["choices"][0]["finish_reason"].is_string())
            .collect();
        assert_eq!(finish.len(), 1, "mixed={mixed}");
        assert_eq!(finish[0]["choices"][0]["finish_reason"], "tool_calls");
    }
}

/// A gateway-executed tool loop streams no tool-call deltas at all.
#[tokio::test]
async fn chat_stream_hides_a_gateway_tool_loop() {
    let h = harness(true, tool_loop_script(), |_| {}).await;
    let (status, body) = h
        .post_chat(json!({
            "model": MODEL, "stream": true,
            "messages": [{"role": "user", "content": "Read /tmp/a"}],
        }))
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let chunks = chat_chunks(&body);
    assert!(chat_tool_calls(&chunks).is_empty(), "{body}");
    let text: String = chunks
        .iter()
        .filter_map(|c| c["choices"][0]["delta"]["content"].as_str())
        .collect();
    assert_eq!(text, "The file says FILE-CONTENTS-42");
    assert_tool_loop_ran(&h);
}

/// After a mixed call, the response that resumes the turn reports the
/// gateway tools run before it and the resumed call's own usage, streaming
/// or not.
#[tokio::test]
async fn chat_resume_after_a_mixed_call_reports_the_whole_loop() {
    for stream in [false, true] {
        let h = harness(
            true,
            vec![
                calls(&[
                    ("call-1", MCP_TOOL, r#"{"path":"/tmp/a"}"#),
                    ("call-w", CLIENT_TOOL, r#"{"city":"Oslo"}"#),
                ]),
                text("Sunny, and the file says FILE-CONTENTS-42"),
            ],
            |_| {},
        )
        .await;
        let (status, body) = h
            .post_chat(json!({
                "model": MODEL, "tools": [chat_client_tool()],
                "messages": [{"role": "user", "content": "Weather and file?"}],
            }))
            .await;
        assert_eq!(status, StatusCode::OK, "{body}");
        let mut second = json!({
            "model": MODEL, "tools": [chat_client_tool()], "stream": stream,
            "messages": [
                {"role": "user", "content": "Weather and file?"},
                {"role": "assistant", "content": null, "tool_calls": [{"id": "call-w",
                    "type": "function", "function": {"name": CLIENT_TOOL,
                    "arguments": "{\"city\":\"Oslo\"}"}}]},
                {"role": "tool", "tool_call_id": "call-w", "content": "sunny"},
            ],
        });
        if stream {
            second["stream_options"] = json!({"include_usage": true});
        }
        let (status, body) = h.post_chat(second).await;
        assert_eq!(status, StatusCode::OK, "stream={stream}: {body}");
        let (meta, usage) = if stream {
            let chunks = chat_chunks(&body);
            let meta = chunks
                .iter()
                .find_map(|c| c["extensions"].get("mcp_via_llm").cloned())
                .expect("the resumed loop's metadata is streamed");
            let usage = chunks
                .iter()
                .rev()
                .find_map(|c| c.get("usage").filter(|u| !u.is_null()).cloned())
                .unwrap();
            (meta, usage)
        } else {
            let body: Value = serde_json::from_str(&body).unwrap();
            (
                body["extensions"]["mcp_via_llm"].clone(),
                body["usage"].clone(),
            )
        };
        assert_eq!(
            meta["mcp_tools_called"],
            json!([MCP_TOOL]),
            "stream={stream}"
        );
        assert_eq!(meta["iterations"], 2, "stream={stream}");
        assert_eq!(
            usage["prompt_tokens"], 10,
            "stream={stream}: usage is the resumed call's own"
        );
    }
}

/// A failure inside the orchestrator completes the request's monitor event
/// (it used to stay Pending) and the failed later call's own event, which
/// carries the chat endpoint.
#[tokio::test]
async fn chat_orchestrator_failure_completes_the_monitor_events() {
    // The script ends after the tool call: the second LLM call fails.
    let h = harness(
        true,
        vec![calls(&[("call-1", MCP_TOOL, r#"{"path":"/tmp/a"}"#)])],
        |_| {},
    )
    .await;
    let (status, _) = h
        .post_chat(json!({
            "model": MODEL,
            "messages": [{"role": "user", "content": "Read /tmp/a"}],
        }))
        .await;
    assert!(!status.is_success());
    let events = h.llm_call_events();
    assert_eq!(events.len(), 2, "{events:?}");
    assert!(
        events
            .iter()
            .all(|e| e.status == lr_monitor::EventStatus::Error),
        "{events:?}"
    );
    let lr_monitor::MonitorEventData::LlmCall { endpoint, .. } = &events[1].data else {
        unreachable!()
    };
    assert_eq!(endpoint, "/v1/chat/completions");
    assert_eq!(events[1].session_id, events[0].session_id);
}

/// A streaming client that goes away mid tool loop stops the loop: the
/// tool waiting on its approval popup never runs and no further LLM call
/// is made, for chat and for responses.
#[tokio::test]
async fn client_disconnect_stops_the_tool_loop() {
    for chat in [false, true] {
        let h = harness(true, tool_loop_script(), |_| {}).await;
        h.files
            .ask
            .store(true, std::sync::atomic::Ordering::Relaxed);
        let state = h.state.clone();
        let client_id = h.client_id.clone();
        let auth = AuthContext {
            api_key_id: client_id.clone(),
            credential_scope: None,
            model_selection: None,
        };
        let client_auth = Some(Extension(ClientAuthContext { client_id }));
        let response = if chat {
            let body = json!({"model": MODEL, "stream": true,
                              "messages": [{"role": "user", "content": "Read /tmp/a"}]});
            crate::routes::chat::chat_completions(
                State(state),
                Extension(auth),
                client_auth,
                GatewayJson(serde_json::from_value(body).unwrap()),
            )
            .await
        } else {
            let body =
                json!({"model": MODEL, "stream": true, "store": false, "input": "Read /tmp/a"});
            create_response(
                State(state),
                Extension(auth),
                client_auth,
                GatewayJson(serde_json::from_value(body).unwrap()),
            )
            .await
        }
        .unwrap();
        // Start reading, as a client would, until the tool's popup is up.
        let mut body = response.into_body().into_data_stream();
        let reader = tokio::spawn(async move {
            use futures::StreamExt;
            while body.next().await.is_some() {}
        });
        let firewall = &h.state.mcp_gateway.firewall_manager;
        let pending = tokio::time::timeout(Duration::from_secs(10), async {
            loop {
                if let Some(pending) = firewall.list_pending().into_iter().next() {
                    break pending;
                }
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
        })
        .await
        .expect("the tool's approval popup is requested");

        // The client goes away.
        reader.abort();
        let _ = reader.await;
        tokio::time::sleep(Duration::from_millis(50)).await;
        let _ =
            firewall.submit_response(&pending.request_id, FirewallApprovalAction::AllowOnce, None);
        tokio::time::sleep(Duration::from_millis(100)).await;

        assert!(
            h.files.calls.lock().is_empty(),
            "chat={chat}: the tool did not run"
        );
        assert_eq!(h.requests().len(), 1, "chat={chat}: no further LLM call");
    }
}

// ── Leftovers: guardrail hold, per-call usage, joined chat text, replay ────

/// A streamed MCP-via-LLM turn behind a parallel guardrail check: the
/// client gets nothing before the verdict; a denial shows none of the text,
/// an allow delivers all of it.
#[tokio::test]
async fn streamed_text_waits_for_the_guardrail_verdict() {
    for allow in [true, false] {
        let h = harness(true, vec![text("Guarded answer")], |_| {}).await;
        let client = h.state.config_manager.get().clients[0].clone();
        let allowed_servers = client
            .mcp_permissions
            .allowed_server_ids(&h.state.config_manager.get().mcp_servers);
        let request: CompletionRequest = serde_json::from_value(json!({
            "model": MODEL, "stream": true,
            "messages": [{"role": "user", "content": "Tell me"}],
        }))
        .unwrap();
        let (verdict_tx, verdict_rx) = tokio::sync::oneshot::channel::<Result<(), String>>();
        let gate = tokio::spawn(async move { verdict_rx.await.unwrap() });
        let mut stream = h
            .state
            .mcp_via_llm_manager
            .handle_streaming_request(
                h.state.mcp_gateway.clone(),
                h.state.router.clone(),
                &client,
                request,
                allowed_servers,
                Some(gate),
                None,
                None,
                lr_mcp_via_llm::TurnOptions::default(),
            )
            .await
            .unwrap();
        use futures::StreamExt;
        let early = tokio::time::timeout(Duration::from_millis(200), stream.next()).await;
        assert!(early.is_err(), "allow={allow}: nothing before the verdict");
        assert_eq!(h.requests().len(), 1, "the LLM call ran meanwhile");

        verdict_tx
            .send(if allow {
                Ok(())
            } else {
                Err("blocked by policy".into())
            })
            .unwrap();
        let items: Vec<_> = stream.collect().await;
        let text: String = items
            .iter()
            .filter_map(|item| item.as_ref().ok())
            .filter_map(|c| c.choices.first()?.delta.content.clone())
            .collect();
        let errors: Vec<String> = items
            .iter()
            .filter_map(|item| item.as_ref().err().map(|e| e.to_string()))
            .collect();
        if allow {
            assert_eq!(text, "Guarded answer");
            assert!(errors.is_empty(), "{errors:?}");
        } else {
            assert_eq!(text, "", "denied text never reaches the client");
            assert_eq!(errors.len(), 1, "{errors:?}");
            assert!(errors[0].contains("blocked by policy"), "{errors:?}");
        }
    }
}

/// An upstream that reuses one chunk id for every call of a tool loop
/// still has each call's usage counted: the streamed usage covers both.
#[tokio::test]
async fn streamed_usage_counts_every_llm_call_of_the_loop() {
    // Responses
    let h = harness(true, tool_loop_script(), |_| {}).await;
    h.provider
        .stream_usage
        .store(true, std::sync::atomic::Ordering::Relaxed);
    let events = h
        .post_stream(json!({"model": MODEL, "input": "Read /tmp/a", "store": false}))
        .await;
    let usage = &events.last().unwrap()["response"]["usage"];
    assert_eq!(usage["input_tokens"], 20, "{usage}");
    assert_eq!(usage["output_tokens"], 10, "{usage}");

    // Chat
    let h = harness(true, tool_loop_script(), |_| {}).await;
    h.provider
        .stream_usage
        .store(true, std::sync::atomic::Ordering::Relaxed);
    let (status, body) = h
        .post_chat(json!({
            "model": MODEL, "stream": true, "stream_options": {"include_usage": true},
            "messages": [{"role": "user", "content": "Read /tmp/a"}],
        }))
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let usage = chat_chunks(&body)
        .into_iter()
        .rev()
        .find_map(|c| c.get("usage").filter(|u| !u.is_null()).cloned())
        .unwrap();
    assert_eq!(usage["prompt_tokens"], 20, "{usage}");
    assert_eq!(usage["completion_tokens"], 10, "{usage}");
}

fn look_then_answer_script() -> Vec<CompletionResponse> {
    vec![
        scripted(
            "Let me look.",
            Some(vec![ToolCall {
                id: "call-1".into(),
                tool_type: "function".into(),
                function: FunctionCall {
                    name: MCP_TOOL.into(),
                    arguments: r#"{"path":"/tmp/a"}"#.into(),
                },
            }]),
        ),
        text("It says FILE-CONTENTS-42"),
        text("Nothing else"),
    ]
}

/// Chat completions has one assistant message per choice: it holds the
/// text of every assistant message of the tool loop, separated by a blank
/// line, streaming and not. The client sends that text back, and the next
/// turn still finds the hidden tool history.
#[tokio::test]
async fn chat_reports_a_tool_loops_messages_as_one_joined_message() {
    const JOINED: &str = "Let me look.\n\nIt says FILE-CONTENTS-42";
    for stream in [false, true] {
        let h = harness(true, look_then_answer_script(), |_| {}).await;
        let (status, body) = h
            .post_chat(json!({
                "model": MODEL, "stream": stream,
                "messages": [{"role": "user", "content": "Read /tmp/a"}],
            }))
            .await;
        assert_eq!(status, StatusCode::OK, "{body}");
        let content = if stream {
            chat_chunks(&body)
                .iter()
                .filter_map(|c| {
                    c["choices"][0]["delta"]["content"]
                        .as_str()
                        .map(str::to_string)
                })
                .collect::<String>()
        } else {
            let body: Value = serde_json::from_str(&body).unwrap();
            body["choices"][0]["message"]["content"]
                .as_str()
                .unwrap()
                .to_string()
        };
        assert_eq!(content, JOINED, "stream={stream}");

        let (status, body) = h
            .post_chat(json!({
                "model": MODEL,
                "messages": [
                    {"role": "user", "content": "Read /tmp/a"},
                    {"role": "assistant", "content": JOINED},
                    {"role": "user", "content": "More?"},
                ],
            }))
            .await;
        assert_eq!(status, StatusCode::OK, "{body}");
        assert_eq!(
            outline(&h.requests()[2]),
            [
                row("user", "Read /tmp/a", &[], None),
                row("assistant", "Let me look.", &["call-1"], None),
                row("tool", "FILE-CONTENTS-42", &[], Some("call-1")),
                row("assistant", "It says FILE-CONTENTS-42", &[], None),
                row("user", "More?", &[], None),
            ],
            "stream={stream}: the model sees its messages apart"
        );
    }
}

async fn expire_orchestrator_sessions(h: &Harness) {
    let mut expired = h.state.mcp_via_llm_manager.config();
    expired.session_ttl_seconds = 0;
    h.state.mcp_via_llm_manager.update_config(expired);
    tokio::time::sleep(Duration::from_millis(5)).await;
}

/// With the orchestrator session gone (expired, or the app restarted), a
/// continuation replays the hidden tool history stored with the response.
#[tokio::test]
async fn continuation_after_the_session_expired_replays_the_stored_hidden_history() {
    for stream in [false, true] {
        let mut script = tool_loop_script();
        script.push(text("Nothing else"));
        script.push(text("Still nothing"));
        let h = harness(true, script, |_| {}).await;
        let first = h
            .turn(
                json!({"model": MODEL, "input": "Read /tmp/a", "instructions": "Be formal"}),
                stream,
            )
            .await;

        // Every orchestrator session expires, as after a restart.
        expire_orchestrator_sessions(&h).await;

        let second = h
            .turn(
                json!({"model": MODEL, "input": "Anything else?",
                       "previous_response_id": response_id_of(&first)}),
                stream,
            )
            .await;
        assert_eq!(
            outline(&h.requests()[2]),
            [
                row("user", "Read /tmp/a", &[], None),
                row("assistant", "", &["call-1"], None),
                row("tool", "FILE-CONTENTS-42", &[], Some("call-1")),
                row("assistant", "The file says FILE-CONTENTS-42", &[], None),
                row("user", "Anything else?", &[], None),
            ],
            "stream={stream}"
        );
        assert!(
            !h.system_texts(2).contains(&"Be formal".to_string()),
            "stream={stream}: the first turn's instructions are not replayed"
        );
        assert_eq!(
            h.system_texts(2)
                .iter()
                .filter(|s| s.contains("FILE") || s.starts_with("Be "))
                .count(),
            0
        );

        // The chain goes on from the replayed history.
        expire_orchestrator_sessions(&h).await;
        let _ = h
            .turn(
                json!({"model": MODEL, "input": "And now?",
                       "previous_response_id": response_id_of(&second)}),
                stream,
            )
            .await;
        let third = outline(&h.requests()[3]);
        assert_eq!(
            third[1],
            row("assistant", "", &["call-1"], None),
            "stream={stream}"
        );
        assert_eq!(third.len(), 7, "stream={stream}: {third:?}");
    }
}

/// Reasoning items of an earlier request travel with the stored response:
/// a continuation carries them again, anchored where they were.
#[tokio::test]
async fn reasoning_items_of_earlier_requests_are_replayed_on_continuation() {
    let h = harness(false, vec![text("Sunny"), text("Still sunny")], |_| {}).await;
    let first = h
        .turn(
            json!({
                "model": MODEL,
                "tools": [client_tool()],
                "input": [
                    {"type": "message", "role": "user", "content": "Weather?"},
                    {"type": "reasoning", "summary": [], "encrypted_content": "r1"},
                    {"type": "function_call", "call_id": "call-w", "name": CLIENT_TOOL, "arguments": "{}"},
                    {"type": "function_call_output", "call_id": "call-w", "output": "sunny"},
                ],
            }),
            false,
        )
        .await;
    let second = h
        .turn(
            json!({"model": MODEL, "input": "Tomorrow?",
                   "previous_response_id": response_id_of(&first)}),
            false,
        )
        .await;
    let stored = |id: &str| {
        let row = super::responses_session_store(&h.state)
            .unwrap()
            .get_active(id, &lr_responses_sessions::RetentionConfig::default())
            .unwrap()
            .unwrap();
        lr_responses_sessions::ReplayState::from_json(row.replay_json.as_deref())
    };
    let expected = [json!({"call_id": "call-w",
        "item": {"type": "reasoning", "summary": [], "encrypted_content": "r1"}})];
    assert_eq!(stored(&response_id_of(&first)).reasoning_items, expected);
    // The continuation carried them (a chat upstream drops them before
    // dispatch), so its own row holds them for the turn after.
    assert_eq!(stored(&response_id_of(&second)).reasoning_items, expected);
    assert!(stored(&response_id_of(&second)).hidden_history.is_none());

    // The request a continuation builds carries them in front of its own.
    let continuation: super::CreateResponseRequest = serde_json::from_value(json!({
        "model": MODEL,
        "input": [
            {"type": "reasoning", "summary": [], "encrypted_content": "r2"},
            {"type": "message", "role": "assistant", "content": [{"type": "output_text", "text": "Hm"}]},
        ],
    }))
    .unwrap();
    let chat = super::build_chat_completion_request(&continuation, vec![], None, expected.to_vec())
        .unwrap();
    assert_eq!(
        chat.extensions.as_ref().unwrap()
            [lr_providers::openai_responses::INPUT_REASONING_EXTENSION],
        json!([expected[0].clone(), {"assistant_text": "Hm",
            "item": {"type": "reasoning", "summary": [], "encrypted_content": "r2"}}])
    );
}

/// Every LLM call of a tool loop is admitted on its own and releases its
/// reservation when its usage is recorded: while the gateway runs a tool,
/// nothing of the previous call is in flight; afterwards nothing is, and
/// the client was charged one request per call.
#[tokio::test]
async fn tool_loop_calls_reserve_and_release_admission_one_by_one() {
    for (case, stream) in [
        ("chat", false),
        ("chat", true),
        ("responses", false),
        ("responses", true),
    ] {
        let h = harness(true, tool_loop_script(), |_| {}).await;
        h.state.rate_limiter.add_api_key_limiters(
            h.client_id.clone(),
            vec![lr_router::rate_limit::RateLimiter::new(
                lr_router::rate_limit::RateLimitType::Requests,
                100.0,
                60,
            )],
        );
        *h.files.in_flight_probe.lock() = Some((h.state.rate_limiter.clone(), h.client_id.clone()));
        let (status, body) = if case == "chat" {
            h.post_chat(json!({
                "model": MODEL, "stream": stream,
                "messages": [{"role": "user", "content": "Read /tmp/a"}],
            }))
            .await
        } else {
            h.post(
                json!({"model": MODEL, "stream": stream, "store": false, "input": "Read /tmp/a"}),
            )
            .await
        };
        assert_eq!(status, StatusCode::OK, "{case} stream={stream}: {body}");
        assert_eq!(
            *h.files.in_flight_seen.lock(),
            [0],
            "{case} stream={stream}: the first call's reservation was released before the tool ran"
        );
        // Streams record their usage when they end; give a dropped one a moment.
        tokio::time::sleep(Duration::from_millis(20)).await;
        assert_eq!(
            h.state
                .rate_limiter
                .api_key_in_flight(&h.client_id)
                .requests,
            0,
            "{case} stream={stream}: nothing left in flight"
        );
        let (requests, _, _) = h
            .state
            .rate_limiter
            .get_api_key_usage(&h.client_id, lr_router::rate_limit::RateLimitType::Requests)
            .await
            .expect("usage recorded");
        assert_eq!(
            requests, 2.0,
            "{case} stream={stream}: one request per LLM call"
        );
    }
}
