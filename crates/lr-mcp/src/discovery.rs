//! Temporary, user-requested connection discovery. Never invokes server tools.

use crate::oauth::{McpOAuthManager, OAuthDiscoveryResponse};
use crate::protocol::{
    meta_keys, JsonRpcRequest, MCP_PROTOCOL_VERSION, MCP_PROTOCOL_VERSION_STATELESS,
};
use crate::transport::{SseTransport, StdioTransport, Transport};
use lr_types::errors::{AppError, AppResult};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::{collections::HashMap, time::Duration};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DiscoveredAuth {
    None,
    OAuthBrowser,
    Bearer,
    Manual,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct McpConnectionDiscovery {
    pub transport: String,
    pub server_name: Option<String>,
    pub server_version: Option<String>,
    pub protocol_versions: Vec<String>,
    pub capabilities: Option<Value>,
    pub auth_method: DiscoveredAuth,
    pub auth_required: bool,
    pub oauth: Option<OAuthDiscoveryResponse>,
    /// Credential header names only; discovery cannot supply secret values.
    pub suggested_headers: Vec<String>,
    pub warnings: Vec<String>,
}

impl McpConnectionDiscovery {
    fn new(transport: &str) -> Self {
        Self {
            transport: transport.into(),
            server_name: None,
            server_version: None,
            protocol_versions: Vec::new(),
            capabilities: None,
            auth_method: DiscoveredAuth::None,
            auth_required: false,
            oauth: None,
            suggested_headers: Vec::new(),
            warnings: Vec::new(),
        }
    }
}

pub fn discovery_request() -> JsonRpcRequest {
    JsonRpcRequest::with_id(
        1,
        "server/discover".into(),
        Some(json!({
            "_meta": {
                meta_keys::PROTOCOL_VERSION: MCP_PROTOCOL_VERSION_STATELESS,
                meta_keys::CLIENT_INFO: {"name": "LocalRouter", "version": env!("CARGO_PKG_VERSION")},
                meta_keys::CLIENT_CAPABILITIES: {}
            }
        })),
    )
}

fn apply_server_result(discovery: &mut McpConnectionDiscovery, result: &Value) -> AppResult<()> {
    let info = result
        .get("_meta")
        .and_then(|m| m.get(meta_keys::SERVER_INFO))
        .or_else(|| result.get("serverInfo"));
    discovery.server_name = info
        .and_then(|i| i.get("name"))
        .and_then(Value::as_str)
        .map(str::to_string);
    discovery.server_version = info
        .and_then(|i| i.get("version"))
        .and_then(Value::as_str)
        .map(str::to_string);
    discovery.capabilities = result
        .get("capabilities")
        .filter(|v| v.is_object())
        .cloned();
    discovery.protocol_versions = result
        .get("protocolVersions")
        .and_then(Value::as_array)
        .map(|versions| {
            versions
                .iter()
                .filter_map(Value::as_str)
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_else(|| {
            result
                .get("protocolVersion")
                .and_then(Value::as_str)
                .map(|v| vec![v.to_string()])
                .unwrap_or_default()
        });
    if discovery.server_name.is_none()
        || discovery.capabilities.is_none()
        || discovery.protocol_versions.is_empty()
    {
        return Err(AppError::Mcp(
            "The endpoint did not return valid MCP server discovery information".into(),
        ));
    }
    if !discovery
        .protocol_versions
        .iter()
        .any(|v| v == MCP_PROTOCOL_VERSION_STATELESS || v == MCP_PROTOCOL_VERSION)
    {
        discovery.warnings.push("The server advertises no MCP version supported by LocalRouter. Check its compatibility before creating the connection.".into());
    }
    Ok(())
}

async fn identify(
    transport: &dyn Transport,
    discovery: &mut McpConnectionDiscovery,
) -> AppResult<()> {
    // Legacy peers may return -32601, reject the modern protocol header, or require initialization.
    if let Ok(Ok(response)) = tokio::time::timeout(
        Duration::from_secs(5),
        transport.send_request(discovery_request()),
    )
    .await
    {
        if let Some(result) = response.result {
            if apply_server_result(discovery, &result).is_ok() {
                return Ok(());
            }
        }
    }
    let request = JsonRpcRequest::with_id(
        2,
        "initialize".into(),
        Some(json!({
            "protocolVersion": MCP_PROTOCOL_VERSION, "capabilities": {},
            "clientInfo": {"name": "LocalRouter", "version": env!("CARGO_PKG_VERSION")}
        })),
    );
    let response = tokio::time::timeout(Duration::from_secs(10), transport.send_request(request))
        .await
        .map_err(|_| AppError::Mcp("MCP initialization timed out".into()))??;
    let result = response.result.ok_or_else(|| {
        AppError::Mcp("The endpoint did not accept MCP discovery or initialization".into())
    })?;
    apply_server_result(discovery, &result)?;
    // Complete the legacy handshake before disconnecting the temporary connection.
    let _ = tokio::time::timeout(
        Duration::from_secs(2),
        transport.send_request(JsonRpcRequest::new(
            None,
            "notifications/initialized".into(),
            None,
        )),
    )
    .await;
    Ok(())
}

/// Infer URL versus command unless explicitly overridden. All I/O is bounded;
/// temporary transports are closed on success, error, and request cancellation.
pub async fn discover_connection(
    target: &str,
    transport_override: Option<&str>,
    headers: HashMap<String, String>,
    env: HashMap<String, String>,
    cwd: Option<String>,
    oauth_manager: &McpOAuthManager,
) -> AppResult<McpConnectionDiscovery> {
    let target = target.trim();
    if target.is_empty() {
        return Err(AppError::Mcp("Enter an MCP URL or command".into()));
    }
    let is_http = match transport_override {
        Some("http_sse") => true,
        Some("stdio") => false,
        None => {
            target.to_ascii_lowercase().starts_with("https://")
                || target.to_ascii_lowercase().starts_with("http://")
        }
        Some(_) => return Err(AppError::Mcp("Unsupported transport override".into())),
    };
    let mut discovery = McpConnectionDiscovery::new(if is_http { "http_sse" } else { "stdio" });
    if !is_http {
        let parts = shell_words::split(target)
            .map_err(|e| AppError::Mcp(format!("Invalid command: {e}")))?;
        let (program, args) = parts
            .split_first()
            .ok_or_else(|| AppError::Mcp("Enter a command".into()))?;
        let config = lr_config::McpTransportConfig::Stdio {
            command: target.into(),
            args: Vec::new(),
            env: env.clone(),
            cwd,
        };
        let transport = StdioTransport::spawn_in(
            program.clone(),
            args.to_vec(),
            env,
            config.resolve_stdio_cwd(),
        )
        .await?;
        let result = identify(&transport, &mut discovery).await;
        transport.close().await?;
        result?;
        discovery.warnings.push("Required environment variables are not advertised by MCP. Add any credentials the server needs below.".into());
        return Ok(discovery);
    }

    let url = reqwest::Url::parse(target)
        .map_err(|_| AppError::Mcp("Enter a valid HTTP or HTTPS MCP URL".into()))?;
    if !matches!(url.scheme(), "http" | "https")
        || url.host_str().is_none()
        || !url.username().is_empty()
        || url.password().is_some()
    {
        return Err(AppError::Mcp("Use an HTTP or HTTPS URL without embedded credentials; provide credentials in authentication or headers".into()));
    }
    let client = reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .timeout(Duration::from_secs(8))
        .build()
        .map_err(|_| AppError::Mcp("Could not create discovery client".into()))?;
    let mut probe = client
        .post(url)
        .header("Accept", "application/json, text/event-stream")
        .header("Mcp-Method", "server/discover")
        .header("Mcp-Protocol-Version", MCP_PROTOCOL_VERSION_STATELESS)
        .json(&discovery_request());
    for (key, value) in &headers {
        // Validate before probing, and do not include header values in errors.
        let name = reqwest::header::HeaderName::from_bytes(key.as_bytes())
            .map_err(|_| AppError::Mcp("Invalid custom header name".into()))?;
        let value = reqwest::header::HeaderValue::from_str(value)
            .map_err(|_| AppError::Mcp("Invalid custom header value".into()))?;
        probe = probe.header(name, value);
    }
    let probe = probe.send().await.map_err(|_| {
        AppError::Mcp(
            "Could not reach the MCP URL. Check the URL, TLS certificate, and network connection"
                .into(),
        )
    })?;
    let mut status = probe.status();
    let mut challenge_headers = probe.headers().clone();
    drop(probe); // An SSE response body can remain open indefinitely.
    if status.is_redirection() {
        return Err(AppError::Mcp("The URL redirects. Enter the final MCP endpoint URL so custom credentials are sent only to that endpoint".into()));
    }
    // Legacy SSE servers may reserve this URL for GET and advertise their POST
    // message endpoint only on the stream. Check GET authentication in that case.
    if matches!(
        status,
        reqwest::StatusCode::METHOD_NOT_ALLOWED | reqwest::StatusCode::NOT_FOUND
    ) {
        let mut get_probe = client
            .get(target)
            .header("Accept", "application/json, text/event-stream");
        for (key, value) in &headers {
            get_probe = get_probe.header(key, value);
        }
        if let Ok(response) = get_probe.send().await {
            status = response.status();
            challenge_headers = response.headers().clone();
        }
    }
    discovery.auth_required =
        status == reqwest::StatusCode::UNAUTHORIZED || status == reqwest::StatusCode::FORBIDDEN;
    match tokio::time::timeout(Duration::from_secs(10), oauth_manager.discover_oauth_from_headers(target, &challenge_headers)).await {
        Ok(Ok(Some(oauth))) => {
            discovery.auth_method = DiscoveredAuth::OAuthBrowser;
            discovery.oauth = Some(oauth);
        }
        Ok(Ok(None)) => {}
        _ => discovery.warnings.push("OAuth metadata discovery did not complete. You can specify authentication manually or retry.".into()),
    }
    if discovery.oauth.is_none() && discovery.auth_required {
        let bearer = challenge_headers
            .get_all(reqwest::header::WWW_AUTHENTICATE)
            .iter()
            .filter_map(|h| h.to_str().ok())
            .any(|h| {
                h.split(',').any(|part| {
                    part.split_whitespace()
                        .next()
                        .is_some_and(|scheme| scheme.eq_ignore_ascii_case("bearer"))
                })
            });
        discovery.auth_method = if bearer {
            DiscoveredAuth::Bearer
        } else {
            DiscoveredAuth::Manual
        };
        if challenge_headers.contains_key(reqwest::header::WWW_AUTHENTICATE) {
            discovery.suggested_headers.push("Authorization".into());
        }
    }
    if discovery.auth_required {
        discovery.warnings.push("Authentication is required before server identity and capabilities can be checked. You can override the detected authentication below.".into());
        if discovery.oauth.is_none() {
            discovery.warnings.push("No usable OAuth metadata was found. Supply credentials or custom headers from the server documentation.".into());
        }
        return Ok(discovery);
    }
    let transport = SseTransport::connect(target.into(), headers).await?;
    let result = identify(&transport, &mut discovery).await;
    transport.close().await?;
    result?;
    discovery.warnings.push("MCP protocol headers are set automatically. Proprietary API-key headers cannot be inferred; add them below when needed.".into());
    Ok(discovery)
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{
        routing::{get, post},
        Json, Router,
    };
    use std::sync::Arc;

    fn oauth_manager() -> McpOAuthManager {
        McpOAuthManager::new_with_keychain(lr_api_keys::CachedKeychain::new(Arc::new(
            lr_api_keys::MockKeychain::new(),
        )))
    }

    async fn serve(app: Router) -> (String, tokio::task::JoinHandle<()>) {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        let task = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        (url, task)
    }

    #[tokio::test]
    async fn modern_http_discovers_identity_with_manual_headers() {
        let app = Router::new().route("/mcp", post(|headers: axum::http::HeaderMap, Json(request): Json<Value>| async move {
            assert_eq!(headers["X-Api-Key"], "test-only-key");
            assert_eq!(request["method"], "server/discover");
            assert_eq!(headers["Mcp-Method"], "server/discover");
            Json(json!({"jsonrpc":"2.0", "id":request["id"], "result":{
                "resultType":"complete", "protocolVersions":[MCP_PROTOCOL_VERSION_STATELESS],
                "_meta":{meta_keys::SERVER_INFO:{"name":"Modern", "version":"2.0"}},
                "capabilities":{"tools":{}}
            }}))
        }));
        let (origin, task) = serve(app).await;
        let discovery = discover_connection(
            &format!("{origin}/mcp?toolsets=ddsql"),
            None,
            HashMap::from([("X-Api-Key".into(), "test-only-key".into())]),
            HashMap::new(),
            None,
            &oauth_manager(),
        )
        .await
        .unwrap();
        assert_eq!(discovery.server_name.as_deref(), Some("Modern"));
        assert_eq!(discovery.server_version.as_deref(), Some("2.0"));
        assert_eq!(
            discovery.protocol_versions,
            vec![MCP_PROTOCOL_VERSION_STATELESS]
        );
        assert!(matches!(discovery.auth_method, DiscoveredAuth::None));
        task.abort();
    }

    #[tokio::test]
    async fn legacy_http_falls_back_to_initialize_and_keeps_session() {
        use axum::response::IntoResponse;
        let app = Router::new().route("/mcp", post(|headers: axum::http::HeaderMap, Json(request): Json<Value>| async move {
            match request["method"].as_str().unwrap() {
                "server/discover" => Json(json!({"jsonrpc":"2.0","id":request["id"],"error":{"code":-32601,"message":"Unknown method"}})).into_response(),
                "initialize" => ([("Mcp-Session-Id", "discovery-session")], Json(json!({"jsonrpc":"2.0","id":request["id"],"result":{
                    "protocolVersion":MCP_PROTOCOL_VERSION,"serverInfo":{"name":"Legacy","version":"1.0"},"capabilities":{"resources":{}}
                }}))).into_response(),
                "notifications/initialized" => {
                    assert_eq!(headers["Mcp-Session-Id"], "discovery-session");
                    axum::http::StatusCode::ACCEPTED.into_response()
                },
                _ => panic!("Discovery must never call tools"),
            }
        }));
        let (origin, task) = serve(app).await;
        let result = discover_connection(
            &format!("{origin}/mcp"),
            None,
            HashMap::new(),
            HashMap::new(),
            None,
            &oauth_manager(),
        )
        .await
        .unwrap();
        assert_eq!(result.server_name.as_deref(), Some("Legacy"));
        assert_eq!(result.protocol_versions, vec![MCP_PROTOCOL_VERSION]);
        task.abort();
    }

    #[tokio::test]
    async fn post_only_oauth_challenge_uses_custom_metadata_location_and_scope() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let origin = format!("http://{}", listener.local_addr().unwrap());
        let metadata_origin = origin.clone();
        let auth_origin = origin.clone();
        let challenge = format!(
            "Bearer Resource_Metadata = \"{origin}/custom-metadata\", Scope=\"mcp:read mcp:write\""
        );
        let app = Router::new()
            .route("/mcp", post(move || { let challenge = challenge.clone(); async move { (axum::http::StatusCode::UNAUTHORIZED, [("WWW-Authenticate", challenge)]) } }))
            .route("/custom-metadata", get(move || { let origin = metadata_origin.clone(); async move { Json(json!({"authorization_servers":[origin],"scopes_supported":["other"]})) } }))
            .route("/.well-known/oauth-authorization-server", get(move || { let origin = auth_origin.clone(); async move { Json(json!({"issuer":origin,"authorization_endpoint":format!("{origin}/authorize"),"token_endpoint":format!("{origin}/token")})) } }));
        let task = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        let manager = oauth_manager();
        let result = discover_connection(
            &format!("{origin}/mcp"),
            None,
            HashMap::new(),
            HashMap::new(),
            None,
            &manager,
        )
        .await
        .unwrap();
        assert!(result.auth_required);
        assert!(matches!(result.auth_method, DiscoveredAuth::OAuthBrowser));
        assert_eq!(
            result.oauth.unwrap().scopes_supported,
            vec!["mcp:read", "mcp:write"]
        );
        // The normal browser-login path also discovers POST-only challenges.
        assert!(manager
            .discover_oauth(&format!("{origin}/mcp"))
            .await
            .unwrap()
            .is_some());
        task.abort();
    }

    #[tokio::test]
    async fn legacy_sse_endpoint_discovers_via_advertised_message_endpoint() {
        use axum::response::sse::{Event, Sse};
        let sender = Arc::new(parking_lot::Mutex::new(
            None::<tokio::sync::mpsc::UnboundedSender<Value>>,
        ));
        let stream_sender = sender.clone();
        let retained_sender = sender.clone();
        let app = Router::new().route("/sse", get(move || {
            let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<Value>();
            *stream_sender.lock() = Some(tx);
            async move {
                Sse::new(async_stream::stream! {
                    yield Ok::<_, std::convert::Infallible>(Event::default().event("endpoint").data("/messages"));
                    while let Some(response) = rx.recv().await {
                        yield Ok(Event::default().event("message").data(response.to_string()));
                    }
                })
            }
        })).route("/messages", post(move |Json(request): Json<Value>| {
            let sender = sender.clone();
            async move {
                let response = match request["method"].as_str().unwrap() {
                    "server/discover" => Some(json!({"error":{"code":-32601,"message":"Unknown method"}})),
                    "initialize" => Some(json!({"result":{"protocolVersion":MCP_PROTOCOL_VERSION,"serverInfo":{"name":"Legacy SSE"},"capabilities":{}}})),
                    "notifications/initialized" => None,
                    _ => panic!("Unexpected method during discovery"),
                };
                if let Some(mut response) = response {
                    response["jsonrpc"] = json!("2.0");
                    response["id"] = request["id"].clone();
                    sender.lock().as_ref().unwrap().send(response).unwrap();
                }
                axum::http::StatusCode::ACCEPTED
            }
        }));
        let (origin, task) = serve(app).await;
        let result = discover_connection(
            &format!("{origin}/sse"),
            None,
            HashMap::new(),
            HashMap::new(),
            None,
            &oauth_manager(),
        )
        .await
        .unwrap();
        assert_eq!(result.server_name.as_deref(), Some("Legacy SSE"));
        tokio::time::timeout(Duration::from_secs(2), async {
            loop {
                if retained_sender.lock().as_ref().unwrap().is_closed() {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
        })
        .await
        .unwrap();
        task.abort();
    }

    #[tokio::test]
    async fn cancelled_http_connection_does_not_leave_reconnection_task_running() {
        use std::sync::atomic::{AtomicUsize, Ordering};
        let attempts = Arc::new(AtomicUsize::new(0));
        let count = attempts.clone();
        let observed = Arc::new(tokio::sync::Notify::new());
        let signal = observed.clone();
        let app = Router::new().route(
            "/mcp",
            get(move || {
                count.fetch_add(1, Ordering::SeqCst);
                signal.notify_one();
                async { axum::http::StatusCode::SERVICE_UNAVAILABLE }
            }),
        );
        let (origin, server_task) = serve(app).await;
        let connect_task = tokio::spawn(async move {
            SseTransport::connect(format!("{origin}/mcp"), HashMap::new()).await
        });
        tokio::time::timeout(Duration::from_secs(2), observed.notified())
            .await
            .unwrap();
        connect_task.abort();
        let _ = connect_task.await;
        // The transport normally retries its first failed GET after two seconds.
        tokio::time::sleep(Duration::from_millis(2500)).await;
        assert_eq!(attempts.load(Ordering::SeqCst), 1);
        server_task.abort();
    }

    #[tokio::test]
    async fn issuer_mismatch_is_not_used_for_login() {
        let app = Router::new().route("/.well-known/oauth-authorization-server", get(|| async {
            Json(json!({"issuer":"https://other.example", "authorization_endpoint":"https://other.example/authorize", "token_endpoint":"https://other.example/token"}))
        }));
        let (origin, task) = serve(app).await;
        let error = oauth_manager()
            .discover_oauth_from_headers(
                &format!("{origin}/mcp"),
                &reqwest::header::HeaderMap::new(),
            )
            .await
            .unwrap_err();
        assert!(error.to_string().contains("issuer does not match"));
        task.abort();
    }

    #[tokio::test]
    async fn missing_oauth_metadata_preserves_bearer_and_basic_requirements() {
        let app = Router::new()
            .route(
                "/bearer",
                post(|| async {
                    (
                        axum::http::StatusCode::UNAUTHORIZED,
                        [("WWW-Authenticate", "Bearer realm=\"mcp\"")],
                    )
                }),
            )
            .route(
                "/basic",
                post(|| async {
                    (
                        axum::http::StatusCode::UNAUTHORIZED,
                        [("WWW-Authenticate", "Basic realm=\"mcp\"")],
                    )
                }),
            );
        let (origin, task) = serve(app).await;
        for (path, expected) in [("bearer", "bearer"), ("basic", "manual")] {
            let result = discover_connection(
                &format!("{origin}/{path}"),
                None,
                HashMap::new(),
                HashMap::new(),
                None,
                &oauth_manager(),
            )
            .await
            .unwrap();
            assert_eq!(serde_json::to_value(result.auth_method).unwrap(), expected);
            assert!(result.auth_required);
            assert_eq!(result.suggested_headers, vec!["Authorization"]);
            assert!(result.server_name.is_none());
        }
        task.abort();
    }

    #[tokio::test]
    async fn legacy_get_challenge_and_openid_metadata_are_discovered() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let origin = format!("http://{}", listener.local_addr().unwrap());
        let metadata_origin = origin.clone();
        let auth_origin = origin.clone();
        let challenge = format!("Bearer resource_metadata=\"{origin}/metadata\"");
        let app = Router::new()
            .route("/sse", get(move || { let challenge = challenge.clone(); async move { (axum::http::StatusCode::UNAUTHORIZED, [("WWW-Authenticate", challenge)]) } }))
            .route("/metadata", get(move || { let origin = metadata_origin.clone(); async move { Json(json!({"authorization_servers":[format!("{origin}/tenant")]})) } }))
            .route("/tenant/.well-known/openid-configuration", get(move || { let origin = auth_origin.clone(); async move { Json(json!({"issuer":format!("{origin}/tenant"),"authorization_endpoint":format!("{origin}/authorize"),"token_endpoint":format!("{origin}/token")})) } }));
        let task = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        let manager = oauth_manager();
        let result = discover_connection(
            &format!("{origin}/sse"),
            None,
            HashMap::new(),
            HashMap::new(),
            None,
            &manager,
        )
        .await
        .unwrap();
        assert!(matches!(result.auth_method, DiscoveredAuth::OAuthBrowser));
        assert_eq!(
            result.oauth.unwrap().issuer.unwrap(),
            format!("{origin}/tenant")
        );
        assert!(manager
            .discover_oauth(&format!("{origin}/sse"))
            .await
            .unwrap()
            .is_some());
        task.abort();
    }

    #[tokio::test]
    async fn invalid_url_command_and_header_fail_without_exposing_credentials() {
        for (target, override_type) in [
            ("https://user:secret@example.com/mcp", Some("http_sse")),
            ("'unclosed", Some("stdio")),
            ("", None),
            ("http://localhost", Some("invalid")),
        ] {
            assert!(discover_connection(
                target,
                override_type,
                HashMap::new(),
                HashMap::new(),
                None,
                &oauth_manager()
            )
            .await
            .is_err());
        }
        let error = discover_connection(
            "http://127.0.0.1:1/mcp",
            None,
            HashMap::from([("X-Key".into(), "secret\nvalue".into())]),
            HashMap::new(),
            None,
            &oauth_manager(),
        )
        .await
        .unwrap_err()
        .to_string();
        assert!(!error.contains("secret"));
        let mut result = McpConnectionDiscovery::new("http_sse");
        assert!(apply_server_result(&mut result, &json!({"status":"ok"})).is_err());
        let error = discover_connection(
            "http://127.0.0.1:1/mcp",
            None,
            HashMap::new(),
            HashMap::new(),
            None,
            &oauth_manager(),
        )
        .await
        .unwrap_err();
        assert!(error.to_string().contains("Could not reach the MCP URL"));
    }

    #[tokio::test]
    async fn an_http_endpoint_that_is_not_mcp_is_rejected() {
        let (origin, task) =
            serve(Router::new().route("/health", post(|| async { Json(json!({"status":"ok"})) })))
                .await;
        assert!(discover_connection(
            &format!("{origin}/health"),
            None,
            HashMap::new(),
            HashMap::new(),
            None,
            &oauth_manager()
        )
        .await
        .is_err());
        task.abort();
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn stdio_discovery_and_cancellation_clean_up_processes() {
        let directory = std::env::temp_dir().join(format!("lr-discovery-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&directory).unwrap();
        let script = directory.join("server.py");
        std::fs::write(&script, r#"import json, os, sys
open('pid', 'w').write(str(os.getpid()))
assert os.environ['MCP_TEST_ENV'] == 'present'
for line in sys.stdin:
    request = json.loads(line)
    if os.environ.get('MCP_HANG') == '1': continue
    if request['method'] == 'server/discover':
        if os.environ.get('MCP_LEGACY') == '1': response = {'error': {'code': -32601, 'message': 'Unknown'}}
        else: response = {'result': {'protocolVersions': ['2026-07-28'], 'serverInfo': {'name': 'Subprocess'}, 'capabilities': {'tools': {}}}}
    elif request['method'] == 'initialize': response = {'result': {'protocolVersion': '2025-11-25', 'serverInfo': {'name': 'Subprocess'}, 'capabilities': {}}}
    elif request['method'] == 'notifications/initialized': continue
    else: raise Exception('Discovery must not invoke tools')
    print(json.dumps({'jsonrpc':'2.0', 'id':request['id'], **response}), flush=True)
"#).unwrap();
        let command = format!("python3 {}", shell_words::quote(script.to_str().unwrap()));
        let environment = HashMap::from([("MCP_TEST_ENV".into(), "present".into())]);
        let cwd = Some(directory.to_str().unwrap().to_string());
        for legacy in [false, true] {
            let mut env = environment.clone();
            if legacy {
                env.insert("MCP_LEGACY".into(), "1".into());
            }
            let result = discover_connection(
                &command,
                None,
                HashMap::new(),
                env,
                cwd.clone(),
                &oauth_manager(),
            )
            .await
            .unwrap();
            assert_eq!(result.transport, "stdio");
            assert_eq!(result.server_name.as_deref(), Some("Subprocess"));
            let pid = std::fs::read_to_string(directory.join("pid")).unwrap();
            assert!(!std::process::Command::new("kill")
                .args(["-0", &pid])
                .stderr(std::process::Stdio::null())
                .status()
                .unwrap()
                .success());
        }
        std::fs::remove_file(directory.join("pid")).unwrap();
        let directory_copy = directory.clone();
        let task = tokio::spawn(async move {
            let mut env = environment;
            env.insert("MCP_HANG".into(), "1".into());
            discover_connection(&command, None, HashMap::new(), env, cwd, &oauth_manager()).await
        });
        tokio::time::timeout(Duration::from_secs(3), async {
            while !directory_copy.join("pid").exists() {
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
        })
        .await
        .unwrap();
        let pid = std::fs::read_to_string(directory.join("pid")).unwrap();
        task.abort();
        let _ = task.await;
        tokio::time::sleep(Duration::from_millis(100)).await;
        assert!(!std::process::Command::new("kill")
            .args(["-0", &pid])
            .stderr(std::process::Stdio::null())
            .status()
            .unwrap()
            .success());
        std::fs::remove_dir_all(directory).unwrap();
    }
}
