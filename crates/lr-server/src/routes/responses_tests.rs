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
        Ok(Box::pin(futures::stream::iter(
            as_chunks(&response).into_iter().map(Ok),
        )))
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
            "response.incomplete",
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
    use std::net::SocketAddr;
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
        let mut request = Request::post(path)
            .header("host", "127.0.0.1")
            .header("authorization", format!("Bearer {token}"))
            .header("content-type", "application/json")
            .extension(ConnectInfo(peer.parse::<SocketAddr>().unwrap()));
        if target {
            request = request.header(
                "x-mcp-access",
                lr_mcp::gateway::firewall::ADMIN_MCP_SERVER_ID,
            );
        }
        let response = app
            .clone()
            .oneshot(
                request
                    .body(Body::from(
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
        !serde_json::to_string(&events).unwrap().contains(MCP_TOOL),
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

/// With the orchestrator session gone, the continuation replays the history
/// stored for the response. It holds the turn as the client saw it, so no
/// assistant message carries a tool call that nothing answers.
#[tokio::test]
async fn continuation_from_stored_history_is_a_valid_conversation() {
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

        // Expire every orchestrator session.
        let mut expired = h.state.mcp_via_llm_manager.config();
        expired.session_ttl_seconds = 0;
        h.state.mcp_via_llm_manager.update_config(expired);
        tokio::time::sleep(Duration::from_millis(5)).await;

        let (status, body) = h
            .post_json(json!({
                "model": MODEL,
                "input": "Anything else?",
                "previous_response_id": first_id,
            }))
            .await;
        assert_eq!(status, StatusCode::OK, "stream={stream}: {body}");

        assert_eq!(
            outline(&h.requests()[2]),
            [
                row("user", "Read /tmp/a", &[], None),
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
                    "response.incomplete",
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
                !serde_json::to_string(&events).unwrap().contains(MCP_TOOL),
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
