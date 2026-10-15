//! Native JSON WebSocket relay; server-side keys never leave the local gateway.
mod policy;
pub mod tickets;
use crate::{
    middleware::{
        client_auth::ClientAuthContext,
        error::{ApiErrorResponse, ApiResult},
    },
    routes::gateway_json::GatewayJson,
    state::{AppState, AuthContext},
};
use axum::{
    extract::{
        ws::{Message, WebSocket, WebSocketUpgrade},
        Query, State,
    },
    http::HeaderMap,
    response::{IntoResponse, Response},
    Extension, Json,
};
use futures::{SinkExt, StreamExt};
use lr_providers::realtime::RealtimeTransport;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use tickets::Ticket;
use utoipa::ToSchema;
const SESSION_SECONDS: u64 = 120;
async fn connect_native(
    request: tokio_tungstenite::tungstenite::handshake::client::Request,
    config: tokio_tungstenite::tungstenite::protocol::WebSocketConfig,
) -> Result<
    (
        tokio_tungstenite::WebSocketStream<
            tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>,
        >,
        tokio_tungstenite::tungstenite::handshake::client::Response,
    ),
    tokio_tungstenite::tungstenite::Error,
> {
    // Both crypto backends are present in the workspace. Do not depend on the
    // optional inspection proxy to initialize the process before a WSS call.
    // install_default preserves an existing caller-selected crypto provider.
    let _ = rustls::crypto::ring::default_provider().install_default();
    tokio_tungstenite::connect_async_with_config(request, Some(config), true).await
}

#[derive(Deserialize, Serialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct TicketSession {
    #[serde(rename = "type")]
    pub kind: String,
    pub model: String,
}
#[derive(Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct ExpiresAfter {
    pub anchor: String,
    pub seconds: i64,
}
#[derive(Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct TicketRequest {
    pub session: TicketSession,
    pub expires_after: Option<ExpiresAfter>,
}
#[derive(Serialize, ToSchema)]
pub struct TicketResponse {
    pub value: String,
    pub expires_at: i64,
    pub session: TicketSession,
}
#[derive(Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct ConnectQuery {
    pub model: Option<String>,
}
pub fn credential_tag(state: &AppState, owner: &str) -> ApiResult<String> {
    if !state
        .client_manager
        .get_client(owner)
        .is_some_and(|c| c.credentials_active())
    {
        return Err(ApiErrorResponse::unauthorized(
            "Realtime client credential is inactive",
        ));
    }
    let secret = state
        .client_manager
        .get_secret(owner)
        .map_err(|_| ApiErrorResponse::unauthorized("Realtime credential is unavailable"))?
        .ok_or_else(|| ApiErrorResponse::unauthorized("Realtime credential was revoked"))?;
    Ok(lr_router::explanation::fingerprint(
        json!({"process":state.internal_test_secret.as_str(),"credential":secret}),
    ))
}
fn direct_credential(state: &AppState, owner: &str, headers: &HeaderMap) -> ApiResult<String> {
    let secret = headers
        .get("authorization")
        .and_then(|h| h.to_str().ok())
        .and_then(|h| h.strip_prefix("Bearer "))
        .ok_or_else(|| {
            ApiErrorResponse::unauthorized(
                "Realtime requires a direct local client credential or local ticket",
            )
        })?;
    if state
        .client_manager
        .verify_secret(secret)
        .map_err(|_| ApiErrorResponse::unauthorized("Realtime credential check failed"))?
        .is_none_or(|c| c.id != owner)
    {
        return Err(ApiErrorResponse::unauthorized(
            "Realtime currently requires direct client-secret authentication",
        ));
    }
    credential_tag(state, owner)
}
fn preflight(
    state: &AppState,
    owner: &str,
    model: &str,
) -> ApiResult<(RealtimeTransport, lr_config::Strategy, String, String)> {
    let strategy = state
        .router
        .authorize_native_operation(owner, lr_config::NativeOperation::Realtime)
        .map_err(ApiErrorResponse::from)?;
    if model.len() > 512 || model.chars().any(char::is_whitespace) {
        return Err(ApiErrorResponse::bad_request("Invalid realtime model"));
    }
    let (provider, native) = model
        .split_once('/')
        .filter(|(p, m)| !p.is_empty() && *p != "localrouter" && !m.is_empty() && *m != "auto")
        .ok_or_else(|| {
            ApiErrorResponse::bad_request("Realtime requires an explicit qualified provider/model")
        })?;
    state
        .router
        .authorize_native_resource(owner, lr_config::NativeOperation::Realtime, model)
        .map_err(ApiErrorResponse::from)?;
    crate::documents::enforce_opaque_scan_policy(state, owner).map_err(|_| {
        ApiErrorResponse::bad_request(
            "Realtime audio cannot satisfy the configured blocking content scans",
        )
    })?;
    if lr_types::is_duplicate_hop()
        || state
            .config_manager
            .get()
            .clients
            .iter()
            .any(|c| c.id == owner && c.is_mcp_via_llm())
    {
        return Err(ApiErrorResponse::bad_request(
            "Realtime cannot use automatic MCP or duplicate gateway hops",
        ));
    }
    let transport = state
        .provider_registry
        .get_provider(provider)
        .ok_or_else(|| ApiErrorResponse::bad_request("Realtime provider is unavailable"))?
        .realtime_transport()
        .map_err(ApiErrorResponse::from)?
        .ok_or_else(|| {
            ApiErrorResponse::bad_request("Provider has no verified native realtime mapping")
        })?;
    transport
        .connection(native)
        .map_err(ApiErrorResponse::from)?;
    Ok((transport, strategy, provider.into(), native.into()))
}
#[utoipa::path(post,path="/v1/realtime/client_secrets",tag="realtime",request_body=TicketRequest,responses((status=200,description="Single-use local browser ticket; opening TTL is separate from the 120-second session limit",body=TicketResponse),(status=400,description="Unsupported policy or ticket configuration",body=crate::types::ErrorResponse)),security(("bearer_auth"=[])))]
pub async fn client_secret(
    State(state): State<AppState>,
    Extension(auth): Extension<AuthContext>,
    headers: HeaderMap,
    GatewayJson(request): GatewayJson<TicketRequest>,
) -> ApiResult<Response> {
    if request.session.kind != "realtime" {
        return Err(ApiErrorResponse::bad_request(
            "Only native realtime conversation sessions are supported",
        ));
    }
    let ttl = match request.expires_after {
        Some(expires) if expires.anchor == "created_at" && (10..=60).contains(&expires.seconds) => {
            expires.seconds
        }
        Some(_) => {
            return Err(ApiErrorResponse::bad_request(
                "Local ticket expiry requires created_at and 10–60 seconds",
            ))
        }
        None => 60,
    };
    let tag = direct_credential(&state, &auth.api_key_id, &headers)?;
    let (transport, _, _, _) = preflight(&state, &auth.api_key_id, &request.session.model)?;
    let now = chrono::Utc::now().timestamp();
    let expires_at = now + ttl;
    let value = state
        .realtime_tickets
        .mint(
            Ticket {
                owner: auth.api_key_id,
                model: request.session.model.clone(),
                credential_tag: tag,
                provider_binding: transport.binding(state.internal_test_secret.as_str()),
                expires_at,
            },
            now,
        )
        .map_err(|m| ApiErrorResponse::rate_limited_with_retry(m, 1))?;
    let mut response = Json(TicketResponse {
        value,
        expires_at,
        session: request.session,
    })
    .into_response();
    response
        .headers_mut()
        .insert("cache-control", "no-store".parse().unwrap());
    Ok(response)
}
#[utoipa::path(get,path="/v1/realtime",tag="realtime",params(("model"=Option<String>,Query,description="Explicit configured provider/model; optional only with a pinned local ticket")),responses((status=101,description="Native JSON text/audio WebSocket relay; bounded duration, traffic and responses"),(status=400,description="Unsupported realtime model, policy or configuration",body=crate::types::ErrorResponse)),security(("bearer_auth"=[])))]
pub async fn upgrade(
    State(state): State<AppState>,
    Extension(auth): Extension<AuthContext>,
    client_auth: Option<Extension<ClientAuthContext>>,
    ticket: Option<Extension<Ticket>>,
    Query(query): Query<ConnectQuery>,
    headers: HeaderMap,
    ws: WebSocketUpgrade,
) -> ApiResult<Response> {
    let owner = auth.api_key_id.clone();
    let (model, tag, expected_binding) = if let Some(Extension(ticket)) = ticket {
        if query.model.as_ref().is_some_and(|m| *m != ticket.model) {
            return Err(ApiErrorResponse::bad_request(
                "Realtime ticket model is locked",
            ));
        }
        (
            ticket.model,
            ticket.credential_tag,
            Some(ticket.provider_binding),
        )
    } else {
        (
            query.model.ok_or_else(|| {
                ApiErrorResponse::bad_request("Realtime requires a qualified model")
            })?,
            direct_credential(&state, &owner, &headers)?,
            None,
        )
    };
    if credential_tag(&state, &owner)? != tag {
        return Err(ApiErrorResponse::unauthorized(
            "Realtime credential changed",
        ));
    }
    let (transport, strategy, provider, native) = preflight(&state, &owner, &model)?;
    let binding = transport.binding(state.internal_test_secret.as_str());
    if expected_binding.is_some_and(|b| b != binding) {
        return Err(ApiErrorResponse::bad_request(
            "Realtime ticket provider credential or origin changed",
        ));
    }
    let session = uuid::Uuid::new_v4().to_string();
    let mut monitor = crate::routes::monitor_helpers::emit_llm_call(
        &state,
        client_auth.as_ref(),
        Some(&session),
        "/v1/realtime",
        &model,
        true,
        &json!({"model":model,"session_seconds":SESSION_SECONDS,"frame_limit":policy::MAX_FRAME}),
    );
    let projection:crate::types::ChatCompletionRequest=serde_json::from_value(json!({"model":model,"messages":[{"role":"user","content":"Open a native realtime session"}]})).map_err(|_|ApiErrorResponse::bad_request("Invalid realtime policy projection"))?;
    let expected = serde_json::to_value(&projection).unwrap_or_default();
    let context = crate::routes::pipeline::run_turn_pipeline(
        &state,
        &auth,
        client_auth.as_ref(),
        projection,
        &mut monitor,
        "/v1/realtime",
        session,
        crate::routes::pipeline::PipelineCaps {
            allow_compression: false,
            allow_decision_routing: false,
            parallel_guardrails: false,
        },
    )
    .await?;
    if serde_json::to_value(context.chat_req).ok().as_ref() != Some(&expected) {
        return Err(monitor.capture_err(ApiErrorResponse::bad_request(
            "Realtime policy cannot rewrite the pinned native request",
        )));
    }
    let permit = state
        .realtime_tickets
        .reserve(&owner)
        .map_err(|m| ApiErrorResponse::rate_limited_with_retry(m, 1))?;
    // Upgrade only after approval; the relay checks all credentials and native binding again.
    let context = RelayContext {
        owner,
        model,
        provider,
        native,
        credential_tag: tag,
        binding,
        strategy_id: strategy.id,
        trace_header: lr_types::current_outbound_trace().map(|trace| trace.header_value()),
    };
    Ok(ws
        .protocols(["realtime"])
        .max_message_size(policy::MAX_FRAME)
        .max_frame_size(policy::MAX_FRAME)
        .on_upgrade(move |socket| relay(socket, state, context, permit, monitor)))
}
struct RelayContext {
    owner: String,
    model: String,
    provider: String,
    native: String,
    credential_tag: String,
    binding: String,
    strategy_id: String,
    trace_header: Option<String>,
}
fn live(state: &AppState, context: &RelayContext) -> ApiResult<RealtimeTransport> {
    if credential_tag(state, &context.owner)? != context.credential_tag {
        return Err(ApiErrorResponse::unauthorized(
            "Realtime credential was rotated or revoked",
        ));
    }
    let (transport, strategy, _, _) = preflight(state, &context.owner, &context.model)?;
    if strategy.id != context.strategy_id
        || transport.binding(state.internal_test_secret.as_str()) != context.binding
    {
        return Err(ApiErrorResponse::bad_request(
            "Realtime provider binding or strategy changed",
        ));
    }
    Ok(transport)
}

async fn relay(
    mut client: WebSocket,
    state: AppState,
    context: RelayContext,
    _permit: tickets::SessionPermit,
    monitor: crate::routes::monitor_helpers::LlmCallGuard,
) {
    let started = std::time::Instant::now();
    let mut policy = policy::Policy::default();
    let mut connected = false;
    let work = async {
        use tokio_tungstenite::tungstenite::{
            client::IntoClientRequest, protocol::WebSocketConfig, Message as NativeMessage,
        };
        live(&state, &context)
            .map_err(|_| "Realtime permissions, credentials or provider changed")?;
        let mut dispatch = state
            .router
            .begin_native_dispatch(
                &context.owner,
                lr_config::NativeOperation::Realtime,
                &context.provider,
                &context.native,
                &context.strategy_id,
            )
            .await
            .map_err(|_| "Realtime connection quota or policy rejected the request")?;
        let transport = live(&state, &context)
            .map_err(|_| "Realtime permissions, credentials or provider changed")?;
        let (url, headers) = transport
            .connection(&context.native)
            .map_err(|_| "Invalid native realtime connection")?;
        let mut request = url
            .into_client_request()
            .map_err(|_| "Invalid native WebSocket request")?;
        for (name, value) in headers {
            if let Some(name) = name {
                request.headers_mut().insert(name, value);
            }
        }
        if let Some(trace) = &context.trace_header {
            request.headers_mut().insert(
                lr_types::TRACE_HEADER,
                trace.parse().map_err(|_| "Invalid native trace header")?,
            );
        }
        let config = WebSocketConfig::default()
            .max_message_size(Some(policy::MAX_FRAME))
            .max_frame_size(Some(policy::MAX_FRAME))
            .write_buffer_size(64 * 1024)
            .max_write_buffer_size(2 * policy::MAX_FRAME);
        let (mut native, _) = tokio::time::timeout(
            std::time::Duration::from_secs(10),
            connect_native(request, config),
        )
        .await
        .map_err(|_| "Native realtime connection timed out")?
        .map_err(|_| "Native realtime handshake failed; no reconnect attempted")?;
        connected = true;
        dispatch.success = true;
        drop(dispatch);
        state.record_client_activity(&context.owner);
        state.emit_event("llm-request", "realtime");
        let send = |value: Value| NativeMessage::Text(value.to_string().into());
        tokio::time::timeout(
            std::time::Duration::from_secs(2),
            native.send(send(policy::initial_update())),
        )
        .await
        .map_err(|_| "Native realtime setup timed out")?
        .map_err(|_| "Native realtime setup failed")?;
        // Do not read client content until the native server acknowledges local
        // caps. Early SDK frames remain in the bounded WebSocket/socket buffers.
        let initialize = async {
            for _ in 0..32 {
                let message = native
                    .next()
                    .await
                    .ok_or("Native realtime closed during setup")?
                    .map_err(|_| "Native realtime setup stream failed")?;
                policy.traffic(message.len())?;
                match message {
                    NativeMessage::Text(text) => {
                        let value: Value = serde_json::from_str(&text)
                            .map_err(|_| "Native realtime returned non-JSON text")?;
                        if value["type"] == "error" {
                            return Err("Native provider rejected bounded realtime settings");
                        }
                        policy.server(&value)?;
                        tokio::time::timeout(
                            std::time::Duration::from_secs(2),
                            client.send(Message::Text(text.to_string().into())),
                        )
                        .await
                        .map_err(|_| "Realtime client is not reading")?
                        .map_err(|_| "Realtime client disconnected")?;
                        if policy.ready {
                            return Ok(());
                        }
                    }
                    NativeMessage::Ping(_) | NativeMessage::Pong(_) => {}
                    _ => return Err("Native realtime setup requires JSON text frames"),
                }
            }
            Err("Native realtime did not acknowledge bounded settings")
        };
        tokio::time::timeout(std::time::Duration::from_secs(10), initialize)
            .await
            .map_err(|_| "Native bounded session acknowledgement timed out")??;
        let mut heartbeat = tokio::time::interval(std::time::Duration::from_secs(1));
        heartbeat.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        let outcome = loop {
            tokio::select! {biased;
                _=heartbeat.tick()=>{if live(&state,&context).is_err() {break Err("Realtime permissions, credentials or provider changed");}},
                incoming=client.next(), if policy.ready=>{
                    if live(&state,&context).is_err() {break Err("Realtime permissions, credentials or provider changed");}
                    match incoming {
                        None|Some(Ok(Message::Close(_)))=>break Ok("client_disconnected"),
                        Some(Ok(Message::Text(text)))=>{
                            policy.traffic(text.len())?;
                            let mut value:Value=serde_json::from_str(&text).map_err(|_|"Realtime requires JSON text frames")?;
                            let response=policy.client(&mut value,&context.native,&context.model)?;
                            let mut turn=if response {Some(state.router.begin_native_dispatch(&context.owner,lr_config::NativeOperation::Realtime,&context.provider,&context.native,&context.strategy_id).await.map_err(|_|"Realtime response quota or policy rejected the request")?)}else{None};
                            live(&state,&context).map_err(|_|"Realtime permissions, credentials or provider changed")?;
                            tokio::time::timeout(std::time::Duration::from_secs(2),native.send(send(value))).await.map_err(|_|"Native realtime write timed out")?.map_err(|_|"Native realtime write failed")?;
                            if let Some(turn)=turn.as_mut() {turn.success=true;}
                        }
                        Some(Ok(Message::Ping(bytes)))=>{tokio::time::timeout(std::time::Duration::from_secs(2),client.send(Message::Pong(bytes))).await.map_err(|_|"Realtime client is not reading")?.map_err(|_|"Realtime client disconnected")?;},
                        Some(Ok(Message::Pong(_)))=>{},
                        Some(Ok(Message::Binary(_)))=>break Err("Realtime audio requires base64 JSON text, not binary frames"),
                        Some(Err(_))=>break Err("Realtime client stream failed"),
                    }
                }
                incoming=native.next()=>{
                    if live(&state,&context).is_err() {break Err("Realtime permissions, credentials or provider changed");}
                    match incoming {
                        None|Some(Ok(NativeMessage::Close(_)))=>break Ok("native_disconnected"),
                        Some(Ok(NativeMessage::Text(text)))=>{
                            policy.traffic(text.len())?;
                            let value:Value=serde_json::from_str(&text).map_err(|_|"Native realtime returned non-JSON text")?;
                            if value["type"]=="error" {break Err("Native realtime reported an error");}
                            policy.server(&value)?;
                            tokio::time::timeout(std::time::Duration::from_secs(2),client.send(Message::Text(text.to_string().into()))).await.map_err(|_|"Realtime client is not reading")?.map_err(|_|"Realtime client disconnected")?;
                        }
                        Some(Ok(NativeMessage::Ping(_)|NativeMessage::Pong(_)))=>{},
                        Some(Ok(_))=>break Err("Native realtime requires JSON text frames"),
                        Some(Err(_))=>break Err("Native realtime stream failed; no reconnect attempted"),
                    }
                }
            }
        };
        let _ =
            tokio::time::timeout(std::time::Duration::from_millis(500), native.close(None)).await;
        outcome
    };
    let outcome = tokio::time::timeout(std::time::Duration::from_secs(SESSION_SECONDS), work)
        .await
        .unwrap_or(Err("Local realtime session duration limit reached"));
    let (input, output, reported) = policy.totals();
    let reason = match outcome {
        Ok(reason) => reason,
        Err(reason) => reason,
    };
    let summary = json!({"extensions":{"localrouter.realtime":{"connected":connected,"closed_reason":reason,"response_creations":policy.responses,"usage_snapshots":reported,"usage_complete":false,"response_usage_complete":reported==policy.responses,"reported_input_tokens":input,"reported_output_tokens":output,"cost_usd":null,"cost_source":"unknown","max_session_seconds":SESSION_SECONDS}}});
    crate::routes::monitor_helpers::update_llm_call_response_body(
        &state,
        monitor.event_id(),
        &summary,
    );
    crate::routes::monitor_helpers::update_llm_call_upstream(
        &state,
        monitor.event_id(),
        Some(lr_types::LlmApi::Realtime),
        None,
    );
    monitor.complete(
        &state,
        &context.provider,
        &context.native,
        if outcome.is_ok() { 200 } else { 502 },
        input,
        output,
        None,
        None,
        started.elapsed().as_millis() as u64,
        Some(reason),
        "Native realtime session ended",
        true,
    );
    let final_event = if outcome.is_err() {
        json!({"type":"error","error":{"type":"localrouter_realtime_error","code":"session_closed","message":reason},"extensions":summary["extensions"]})
    } else {
        json!({"type":"localrouter.session.closed","extensions":summary["extensions"]})
    };
    let _ = tokio::time::timeout(
        std::time::Duration::from_millis(500),
        client.send(Message::Text(final_event.to_string().into())),
    )
    .await;
    let _ = tokio::time::timeout(std::time::Duration::from_millis(500), client.close()).await;
}

/// Called only by the authentication layer on a realtime GET. Other API routes
/// never accept these credentials. Strip the credential-bearing subprotocol
/// before passing the authenticated request to the upgrade handler.
pub fn authenticate_ticket(
    state: &AppState,
    request: &mut axum::http::Request<axum::body::Body>,
) -> ApiResult<Option<Ticket>> {
    if request.method() != axum::http::Method::GET
        || !matches!(request.uri().path(), "/v1/realtime" | "/realtime")
    {
        return Ok(None);
    }
    let authorization = request
        .headers()
        .get("authorization")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
        .filter(|v| v.starts_with("ek_lr_"));
    let mut protocols = Vec::new();
    for header in request.headers().get_all("sec-websocket-protocol") {
        let header = header.to_str().map_err(|_| {
            ApiErrorResponse::unauthorized("Invalid realtime authentication protocol")
        })?;
        if header.len() > 1024 {
            return Err(ApiErrorResponse::unauthorized(
                "Invalid realtime authentication protocol",
            ));
        }
        protocols.extend(header.split(',').map(str::trim));
    }
    let tokens: Vec<_> = protocols
        .iter()
        .filter_map(|p| p.strip_prefix("openai-insecure-api-key."))
        .collect();
    if tokens.len() > 1
        || (!tokens.is_empty() && request.headers().contains_key("authorization"))
        || request.headers().get_all("authorization").iter().count() > 1
    {
        return Err(ApiErrorResponse::unauthorized(
            "Ambiguous realtime credentials",
        ));
    }
    if !tokens.is_empty() && !protocols.contains(&"realtime") {
        return Err(ApiErrorResponse::unauthorized(
            "Browser realtime tickets require the realtime subprotocol",
        ));
    }
    let token = authorization.or_else(|| tokens.first().copied());
    let Some(token) = token else {
        return Ok(None);
    };
    let ticket = state
        .realtime_tickets
        .take(token, chrono::Utc::now().timestamp())
        .ok_or_else(|| {
            ApiErrorResponse::unauthorized("Realtime ticket is invalid, expired or already used")
        })?;
    if credential_tag(state, &ticket.owner)? != ticket.credential_tag {
        return Err(ApiErrorResponse::unauthorized(
            "Realtime ticket credential was rotated or revoked",
        ));
    }
    if !tokens.is_empty() {
        request
            .headers_mut()
            .insert("sec-websocket-protocol", "realtime".parse().unwrap());
    }
    request.headers_mut().remove("authorization");
    Ok(Some(ticket))
}

#[cfg(test)]
mod tests {
    #[tokio::test]
    async fn wss_crypto_initializes_without_proxy_startup() {
        const CHILD: &str = "LOCALROUTER_REALTIME_TLS_TEST_CHILD";
        if std::env::var_os(CHILD).is_none() {
            let result = std::process::Command::new(std::env::current_exe().unwrap())
                .args([
                    "--exact",
                    "realtime::tests::wss_crypto_initializes_without_proxy_startup",
                    "--nocapture",
                ])
                .env(CHILD, "1")
                .output()
                .unwrap();
            let stdout = String::from_utf8_lossy(&result.stdout);
            assert!(
                result.status.success(),
                "{stdout}\n{}",
                String::from_utf8_lossy(&result.stderr)
            );
            assert!(
                stdout.contains("running 1 test"),
                "isolated TLS regression did not run: {stdout}"
            );
            return;
        }
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        use tokio_tungstenite::tungstenite::{
            client::IntoClientRequest, protocol::WebSocketConfig,
        };
        assert!(
            rustls::crypto::CryptoProvider::get_default().is_none(),
            "test must not inherit proxy initialization"
        );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut hello = [0u8; 4096];
            let count = socket.read(&mut hello).await.unwrap();
            assert!(count > 0);
            assert_eq!(hello[0], 22, "native client must actually attempt TLS");
            socket
                .write_all(
                    b"HTTP/1.1 400 Bad Request\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
                )
                .await
                .unwrap();
        });
        let request = format!("wss://{addr}/realtime?model=native")
            .into_client_request()
            .unwrap();
        let result = tokio::time::timeout(
            std::time::Duration::from_secs(3),
            super::connect_native(request, WebSocketConfig::default()),
        )
        .await
        .unwrap();
        assert!(
            result.is_err(),
            "a non-TLS peer must never be accepted as secure"
        );
        server.await.unwrap();
        assert!(rustls::crypto::CryptoProvider::get_default().is_some());
    }
}
