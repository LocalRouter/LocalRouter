//! Complete installed-client tests: synthetic credentials, local TLS provider,
//! real AppFirewall and MITM transport, and actual Codex tool execution.
//! No request in this fixture can be forwarded to a public provider.
use super::{
    secret_policy_tests::{fixture, KEY},
    AppFirewall,
};
use async_trait::async_trait;
use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
use bytes::Bytes;
use futures_util::{SinkExt, StreamExt};
use http_body_util::{BodyExt, Full};
use hyper::{body::Incoming, service::service_fn, Request, Response};
use hyper_util::rt::TokioIo;
use lr_api_keys::{keychain_trait::SystemKeychain, KeychainStorage};
use lr_proxy::{
    active::ActiveInterceptor,
    cert::CertAuthority,
    interceptor::{ClientCtx, ConnectDecision, ObservedExchange, ProxyInterceptor, RequestAction},
    passive::PassiveInterceptor,
    resolver::StaticResolver,
    ProxyManager,
};
use serde_json::{json, Value};
use std::{
    collections::HashSet,
    convert::Infallible,
    sync::{Arc, Mutex},
    time::Duration,
};
use tokio::net::TcpListener;
use tokio_rustls::TlsAcceptor;

/// Delegate every request to the production interceptor. Only this local
/// destination can be reached, including auth/model preflights.
struct LocalOnly(ActiveInterceptor);
#[async_trait]
impl ProxyInterceptor for LocalOnly {
    fn on_connect(&self, host: &str, _: &ClientCtx) -> ConnectDecision {
        if host == "localhost" {
            ConnectDecision::Mitm
        } else {
            ConnectDecision::Reject("fixture permits localhost only")
        }
    }
    async fn on_request(&self, ex: &ObservedExchange) -> RequestAction {
        self.0.on_request(ex).await
    }
    fn begin(&self, ex: &ObservedExchange) -> Option<String> {
        self.0.begin(ex)
    }
    async fn on_response(&self, ex: &ObservedExchange) {
        self.0.on_response(ex).await
    }
}

/// Exercise native credential storage without touching production services.
struct TestKeychain {
    service: String,
    accounts: Mutex<HashSet<String>>,
}
impl TestKeychain {
    fn new() -> Self {
        Self {
            service: format!("LocalRouter-Lifecycle-Test-{}", uuid::Uuid::new_v4()),
            accounts: Mutex::default(),
        }
    }
}
impl KeychainStorage for TestKeychain {
    fn store(&self, _: &str, account: &str, value: &str) -> lr_types::AppResult<()> {
        self.accounts.lock().unwrap().insert(account.into());
        SystemKeychain.store(&self.service, account, value)
    }
    fn get(&self, _: &str, account: &str) -> lr_types::AppResult<Option<String>> {
        SystemKeychain.get(&self.service, account)
    }
    fn delete(&self, _: &str, account: &str) -> lr_types::AppResult<()> {
        SystemKeychain.delete(&self.service, account)
    }
}
impl Drop for TestKeychain {
    fn drop(&mut self) {
        for account in self.accounts.get_mut().unwrap().iter() {
            SystemKeychain
                .delete(&self.service, account)
                .expect("remove synthetic native credential");
        }
    }
}

#[derive(Default)]
struct Evidence {
    requests: Vec<Value>,
    transports: Vec<&'static str>,
    tool_output_seen: bool,
    marker: Option<String>,
    stages: usize,
    compressed: usize,
    tools: Vec<Value>,
    lite_seen: bool,
    custom_output_seen: bool,
    plaintext_after_disable: bool,
}
struct Provider {
    evidence: Mutex<Evidence>,
    firewall: Arc<AppFirewall>,
    transition: bool,
    websockets: bool,
    disable_after_first: bool,
}

fn find_marker(value: &Value) -> Option<String> {
    match value {
        Value::String(s) => {
            let start = s.find("<localrouter-redacted ")?;
            let end = start + s[start..].find('>')? + 1;
            let mut tag = s[start..end].to_owned();
            for _ in 0..4 {
                if !tag.contains("\\\"") {
                    break;
                }
                match serde_json::from_str::<String>(&format!("\"{tag}\"")) {
                    Ok(decoded) => tag = decoded,
                    Err(_) => break,
                }
            }
            Some(tag)
        }
        Value::Array(a) => a.iter().find_map(find_marker),
        Value::Object(o) => o.values().find_map(find_marker),
        _ => None,
    }
}
fn function_tool(value: &Value) -> Option<(String, Option<String>)> {
    for tool in value.get("tools")?.as_array()? {
        match tool["type"].as_str() {
            Some("function")
                if matches!(
                    tool["name"].as_str(),
                    Some("exec_command" | "shell_command" | "shell")
                ) =>
            {
                return Some((tool["name"].as_str()?.into(), None))
            }
            Some("namespace") => {
                if let Some((name, _)) = function_tool(&json!({"tools":tool["tools"]})) {
                    return Some((name, tool["name"].as_str().map(str::to_owned)));
                }
            }
            _ => {}
        }
    }
    None
}
fn custom_exec(value: &Value, namespace: Option<&str>) -> Option<(Value, Option<String>)> {
    for tool in value.get("tools")?.as_array()? {
        if tool["type"] == "custom" && tool["name"] == "exec" {
            return Some((tool.clone(), namespace.map(str::to_owned)));
        }
        if tool["type"] == "namespace" {
            if let Some(found) = custom_exec(&json!({"tools":tool["tools"]}), tool["name"].as_str())
            {
                return Some(found);
            }
        }
    }
    None
}
impl Provider {
    fn respond(&self, request: Value, transport: &'static str, compressed: bool) -> Vec<Value> {
        let mut state = self.evidence.lock().unwrap();
        let warmup = request["generate"] == false;
        let stage = state.stages;
        let protected = self.firewall.config_manager.get().secret_scanning.action
            == lr_config::SecretScanAction::Redact;
        state.plaintext_after_disable |=
            self.disable_after_first && stage > 0 && request.to_string().contains(KEY);
        if protected {
            assert!(
                !request.to_string().contains(KEY),
                "original reached synthetic provider in Redact mode"
            );
        }
        if compressed {
            state.compressed += 1;
        }
        if let Some(marker) = find_marker(&request) {
            if let Some(previous) = state.marker.as_ref() {
                assert_eq!(
                    previous, &marker,
                    "references must stay stable through the conversation and resume"
                );
            }
            state.marker = Some(marker);
        }
        state.custom_output_seen |= request.to_string().contains("custom_tool_call_output");
        if request.to_string().contains("function_call_output")
            || request.to_string().contains("custom_tool_call_output")
        {
            state.tool_output_seen = true;
        }
        let mut declared = request
            .get("tools")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        if let Some(input) = request.get("input").and_then(Value::as_array) {
            for item in input
                .iter()
                .filter(|item| item["type"] == "additional_tools")
            {
                state.lite_seen = true;
                if let Some(tools) = item.get("tools").and_then(Value::as_array) {
                    declared.extend(tools.iter().cloned());
                }
            }
        }
        if !declared.is_empty() {
            state.tools = declared;
        }
        let effective = json!({"tools":state.tools});
        state.requests.push(request.clone());
        state.transports.push(transport);
        let id = format!("opaque-provider-response-{}", state.requests.len());
        eprintln!("Local synthetic provider: transport={transport}, stage={stage}, warmup={warmup}, protected={protected}, previous={}", request["previous_response_id"]);
        let mut events = vec![json!({"type":"response.created","response":{"id":id}})];
        let mut completed_output = Vec::new();
        if !warmup {
            let marker = state.marker.clone().unwrap_or_else(|| KEY.into());
            let mut item = if stage < 2 {
                let command = if stage == 0 {
                    format!("printf '%s' 'aws_secret_access_key={marker}'")
                } else {
                    format!("printf '%s' '{marker}' > restored.env")
                };
                if let Some((name, namespace)) = function_tool(&effective) {
                    let args = if name == "shell" {
                        json!({"command":["/bin/sh","-c",command]})
                    } else if name == "shell_command" {
                        json!({"command":command,"timeout_ms":1000,"login":false})
                    } else {
                        json!({"cmd":command,"max_output_tokens":200,"login":false})
                    };
                    let mut item = json!({"type":"function_call","call_id":format!("call_{stage}"),"name":name,"arguments":args.to_string()});
                    if let Some(namespace) = namespace {
                        item["namespace"] = json!(namespace);
                    }
                    item
                } else {
                    let (tool, namespace) = custom_exec(&effective, None)
                        .expect("actual Codex must expose execution tools");
                    let description = tool["description"].as_str().unwrap_or_default();
                    let (method, args) = if description.contains("exec_command") {
                        (
                            "exec_command",
                            json!({"cmd":command,"max_output_tokens":200,"login":false}),
                        )
                    } else if description.contains("shell_command") {
                        (
                            "shell_command",
                            json!({"command":command,"timeout_ms":1000,"login":false}),
                        )
                    } else {
                        panic!("Code-mode description exposes no shell executor")
                    };
                    let mut item = json!({"type":"custom_tool_call","call_id":format!("call_{stage}"),"name":"exec","input":format!("text(JSON.stringify(await tools.{method}({args})));")});
                    if let Some(namespace) = namespace {
                        item["namespace"] = json!(namespace);
                    }
                    item
                }
            } else {
                json!({"type":"message","id":"message-final","role":"assistant","content":[{"type":"output_text","text":format!("Completed: {marker}")} ]})
            };
            let item_id = format!("item-{}", state.requests.len());
            item["id"] = json!(item_id);
            let kind = item["type"].as_str().unwrap();
            let mut added = item.clone();
            let (channel, field, text) = if kind == "custom_tool_call" {
                added["input"] = json!("");
                (
                    "custom_tool_call_input",
                    "input",
                    item["input"].as_str().unwrap().to_owned(),
                )
            } else if kind == "function_call" {
                added["arguments"] = json!("");
                (
                    "function_call_arguments",
                    "arguments",
                    item["arguments"].as_str().unwrap().to_owned(),
                )
            } else {
                added["content"] = json!([]);
                (
                    "output_text",
                    "text",
                    item["content"][0]["text"].as_str().unwrap().to_owned(),
                )
            };
            events.push(json!({"type":"response.output_item.added","output_index":0,"item":added}));
            if kind == "message" {
                events.push(json!({"type":"response.content_part.added","item_id":item_id,"output_index":0,"content_index":0,"part":{"type":"output_text","text":"","annotations":[]}}));
            }
            // Split every returned reference across many real protocol deltas.
            for chunk in text.as_bytes().chunks(7) {
                events.push(json!({"type":format!("response.{channel}.delta"),"item_id":item_id,"output_index":0,"content_index":0,"delta":std::str::from_utf8(chunk).unwrap()}));
            }
            let mut done = json!({"type":format!("response.{channel}.done"),"item_id":item_id,"output_index":0,"content_index":0});
            done[field] = json!(text);
            events.push(done);
            if kind == "message" {
                events.push(json!({"type":"response.content_part.done","item_id":item_id,"output_index":0,"content_index":0,"part":item["content"][0]}));
            }
            events.push(json!({"type":"response.output_item.done","output_index":0,"item":item}));
            completed_output.push(item);
            state.stages += 1;
            if (self.transition || self.disable_after_first) && stage == 0 {
                self.firewall
                    .config_manager
                    .update(|config| {
                        config.secret_scanning.action = if self.disable_after_first {
                            lr_config::SecretScanAction::Off
                        } else {
                            lr_config::SecretScanAction::Redact
                        }
                    })
                    .unwrap();
            }
        }
        events.push(json!({"type":"response.completed","response":{"id":id,"output":completed_output,"usage":{"input_tokens":1,"output_tokens":1,"total_tokens":2}}}));
        events
    }
}

async fn provider_request(
    mut req: Request<Incoming>,
    provider: Arc<Provider>,
) -> Result<Response<Full<Bytes>>, Infallible> {
    if req
        .headers()
        .get("upgrade")
        .is_some_and(|v| v == "websocket")
    {
        if !provider.websockets {
            return Ok(Response::builder()
                .status(426)
                .body(Full::default())
                .unwrap());
        }
        let key = req.headers()["sec-websocket-key"].as_bytes();
        let accept = tokio_tungstenite::tungstenite::handshake::derive_accept_key(key);
        let upgrade = hyper::upgrade::on(&mut req);
        tokio::spawn(async move {
            let upgraded = upgrade.await.unwrap();
            let mut ws = tokio_tungstenite::WebSocketStream::from_raw_socket(
                TokioIo::new(upgraded),
                tokio_tungstenite::tungstenite::protocol::Role::Server,
                None,
            )
            .await;
            while let Some(Ok(message)) = ws.next().await {
                if let tokio_tungstenite::tungstenite::Message::Text(text) = message {
                    let request: Value = serde_json::from_str(&text).unwrap();
                    if request["type"] == "response.cancel" {
                        continue;
                    }
                    for event in provider.respond(request, "websocket", false) {
                        ws.send(tokio_tungstenite::tungstenite::Message::Text(
                            event.to_string(),
                        ))
                        .await
                        .unwrap();
                    }
                }
            }
        });
        return Ok(Response::builder()
            .status(101)
            .header("connection", "upgrade")
            .header("upgrade", "websocket")
            .header("sec-websocket-accept", accept)
            .body(Full::default())
            .unwrap());
    }
    let path = req.uri().path();
    if req.method() == "GET" && path.ends_with("/mcp") {
        return Ok(Response::builder()
            .status(405)
            .body(Full::default())
            .unwrap());
    }
    eprintln!("Local synthetic endpoint: {} {path}", req.method());
    if path.ends_with("/accounts/check") {
        let body = json!({"accounts":[{"id":"synthetic-account", "can_access_with_session":true, "plan_type":"pro", "workspace_backend_origin":"NO_CONSTRAINT", "account_routing_override":"NO_CONSTRAINT"}],"default_account_id":"synthetic-account"});
        return Ok(Response::builder()
            .header("content-type", "application/json")
            .body(Full::new(Bytes::from(body.to_string())))
            .unwrap());
    }
    if req.method() == "GET" {
        return Ok(Response::builder()
            .header("content-type", "application/json")
            .body(Full::new(Bytes::from(json!({"models":[]}).to_string())))
            .unwrap());
    }
    if !path.ends_with("/responses") {
        // ChatGPT Apps' optional MCP bootstrap is independent of inference.
        // Supply an empty synthetic server; no external tool is contacted.
        let body = req.into_body().collect().await.unwrap().to_bytes();
        let rpc: Value = serde_json::from_slice(&body).unwrap_or(Value::Null);
        let result = match rpc["method"].as_str() {
            Some("initialize") => {
                json!({"protocolVersion":"2025-03-26","capabilities":{"tools":{}},"serverInfo":{"name":"synthetic-empty-apps","version":"1"}})
            }
            Some("tools/list") => json!({"tools":[]}),
            _ => json!({}),
        };
        if rpc.get("id").is_none() {
            return Ok(Response::builder()
                .status(204)
                .body(Full::default())
                .unwrap());
        }
        return Ok(Response::builder()
            .header("content-type", "application/json")
            .body(Full::new(Bytes::from(
                json!({"jsonrpc":"2.0","id":rpc["id"],"result":result}).to_string(),
            )))
            .unwrap());
    }
    let compressed = req.headers().contains_key("content-encoding");
    let headers = req.headers().clone();
    let body = req.into_body().collect().await.unwrap().to_bytes();
    let decoded = if headers.get("content-encoding").is_some_and(|v| v == "zstd") {
        zstd::stream::decode_all(body.as_ref()).unwrap()
    } else {
        body.to_vec()
    };
    let request = serde_json::from_slice(&decoded).unwrap();
    let events = provider.respond(request, "http", compressed);
    let stream = events
        .into_iter()
        .map(|event| format!("data: {event}\n\n"))
        .collect::<String>();
    Ok(Response::builder()
        .header("content-type", "text/event-stream")
        .body(Full::new(Bytes::from(stream)))
        .unwrap())
}

async fn start_provider(
    provider: Arc<Provider>,
) -> (u16, rustls::RootCertStore, tokio::task::JoinHandle<()>) {
    lr_proxy::tls::ensure_crypto_provider();
    let key = rcgen::KeyPair::generate().unwrap();
    let cert = rcgen::CertificateParams::new(vec!["localhost".into()])
        .unwrap()
        .self_signed(&key)
        .unwrap();
    let mut roots = rustls::RootCertStore::empty();
    roots.add(cert.der().clone()).unwrap();
    let key = rustls_pki_types::PrivateKeyDer::Pkcs8(rustls_pki_types::PrivatePkcs8KeyDer::from(
        key.serialize_der(),
    ));
    let tls = rustls::ServerConfig::builder()
        .with_no_client_auth()
        .with_single_cert(vec![cert.der().clone()], key)
        .unwrap();
    let acceptor = TlsAcceptor::from(Arc::new(tls));
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let handle = tokio::spawn(async move {
        loop {
            let (tcp, _) = listener.accept().await.unwrap();
            let acceptor = acceptor.clone();
            let provider = provider.clone();
            tokio::spawn(async move {
                let Ok(tls) = acceptor.accept(tcp).await else {
                    return;
                };
                hyper::server::conn::http1::Builder::new()
                    .serve_connection(
                        TokioIo::new(tls),
                        service_fn(move |request| provider_request(request, provider.clone())),
                    )
                    .with_upgrades()
                    .await
                    .ok();
            });
        }
    });
    (port, roots, handle)
}

struct RunningProxy {
    url: String,
    ca_pem: String,
    stop: Option<tokio::sync::oneshot::Sender<()>>,
    task: tokio::task::JoinHandle<()>,
}
impl Drop for RunningProxy {
    fn drop(&mut self) {
        if let Some(tx) = self.stop.take() {
            let _ = tx.send(());
        }
        self.task.abort();
    }
}
async fn start_proxy(
    root: &std::path::Path,
    firewall: Arc<AppFirewall>,
    client_id: &str,
    roots: rustls::RootCertStore,
) -> RunningProxy {
    let ca = Arc::new(CertAuthority::load_or_create(&root.join("ca")).unwrap());
    let ca_pem = ca.ca_pem().to_string();
    let interceptor = Arc::new(LocalOnly(ActiveInterceptor::new(
        PassiveInterceptor::new(Arc::new(lr_monitor::MonitorEventStore::new(128))),
        firewall,
    )));
    let resolver = Arc::new(StaticResolver {
        client_id: client_id.to_owned(),
        secret: "synthetic-proxy".into(),
        proxy_enabled: true,
    });
    let manager = ProxyManager::with_upstream_roots(ca, interceptor, resolver, roots).unwrap();
    let listener = ProxyManager::bind("127.0.0.1", 0).await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let (stop_tx, stop_rx) = tokio::sync::oneshot::channel::<()>();
    let proxy = tokio::spawn(async move {
        manager
            .serve(listener, async {
                stop_rx.await.ok();
            })
            .await;
    });
    RunningProxy {
        url: format!("http://{client_id}:synthetic-proxy@127.0.0.1:{port}"),
        ca_pem,
        stop: Some(stop_tx),
        task: proxy,
    }
}

async fn run_installed_codex(transition: bool, websockets: bool, disable_after_first: bool) {
    let binary = std::env::var("LOCALROUTER_TEST_CODEX")
        .expect("set LOCALROUTER_TEST_CODEX to installed Codex CLI");
    let root = tempfile::tempdir().unwrap();
    let (storage, storage_label): (Arc<dyn KeychainStorage>, &str) =
        match std::env::var("LOCALROUTER_TEST_KEYCHAIN").as_deref() {
            Ok("mock") => (Arc::new(lr_api_keys::MockKeychain::new()), "mock"),
            Ok("native") | Err(_) => (Arc::new(TestKeychain::new()), "native"),
            _ => panic!("LOCALROUTER_TEST_KEYCHAIN must be native or mock"),
        };
    let (firewall, request) = fixture(root.path(), lr_config::LlmMode::Proxy, storage);
    if transition {
        firewall
            .config_manager
            .update(|c| c.secret_scanning.action = lr_config::SecretScanAction::Off)
            .unwrap();
    }
    let firewall = Arc::new(firewall);
    let provider = Arc::new(Provider {
        evidence: Mutex::default(),
        firewall: firewall.clone(),
        transition,
        websockets,
        disable_after_first,
    });
    let (provider_port, roots, upstream) = start_provider(provider.clone()).await;
    let running = start_proxy(root.path(), firewall, &request.client_id, roots).await;
    let cert_path = root.path().join("ca.pem");
    std::fs::write(&cert_path, &running.ca_pem).unwrap();
    let home = root.path().join("codex");
    let isolated_home = root.path().join("isolated-home");
    std::fs::create_dir_all(&isolated_home).unwrap();
    let workspace = root.path().join("workspace");
    std::fs::create_dir_all(&home).unwrap();
    std::fs::create_dir_all(&workspace).unwrap();
    let claims = json!({"exp":4102444800_u64,"email":"synthetic@example.invalid","https://api.openai.com/auth":{"chatgpt_plan_type":"pro","chatgpt_account_id":"synthetic-account"}});
    let token = format!(
        "{}.{}.synthetic",
        URL_SAFE_NO_PAD.encode(r#"{"alg":"RS256","typ":"JWT"}"#),
        URL_SAFE_NO_PAD.encode(claims.to_string())
    );
    std::fs::write(home.join("auth.json"),json!({"auth_mode":"chatgpt","OPENAI_API_KEY":null,"tokens":{"id_token":token,"access_token":token,"refresh_token":"synthetic","account_id":"synthetic-account"},"last_refresh":chrono::Utc::now().to_rfc3339()}).to_string()).unwrap();
    let proxy_url = running.url.clone();
    for resume in [false, true] {
        let mut command = tokio::process::Command::new(&binary);
        command
            .env_clear()
            .env("PATH", std::env::var("PATH").unwrap())
            .env("HOME", &isolated_home)
            .env("XDG_CONFIG_HOME", isolated_home.join(".config"))
            .env("XDG_DATA_HOME", isolated_home.join(".local/share"))
            .env("CODEX_HOME", &home)
            .env("CODEX_CA_CERTIFICATE", &cert_path)
            .env("HTTPS_PROXY", &proxy_url)
            .env("HTTP_PROXY", &proxy_url)
            .env("ALL_PROXY", &proxy_url)
            .env("NO_PROXY", "")
            .args([
                "exec",
                "--ignore-user-config",
                "--ignore-rules",
                "--skip-git-repo-check",
                "--json",
                "-s",
                "workspace-write",
                "-c",
                "approval_policy=\"never\"",
                "-c",
                "cli_auth_credentials_store=\"file\"",
                "-m",
                "gpt-6.1-sol",
            ])
            .arg("-c")
            .arg(format!(
                "chatgpt_base_url=\"https://localhost:{provider_port}/backend-api\""
            ))
            .arg("-c")
            .arg(format!(
                "openai_base_url=\"https://localhost:{provider_port}/backend-api/codex\""
            ))
            .arg("-C")
            .arg(&workspace)
            .current_dir(&workspace)
            .kill_on_drop(true);
        if resume {
            command
                .args(["resume", "--last"])
                .arg("Continue this synthetic fixture and confirm the saved value.");
        } else {
            command.arg(format!("Synthetic local fixture only. Run the shell tools the local provider requests. aws_secret_access_key={KEY}"));
        }
        let output = tokio::time::timeout(Duration::from_secs(60), command.output())
            .await
            .expect("installed Codex must finish fixture within 60 seconds")
            .unwrap();
        let stderr = String::from_utf8_lossy(&output.stderr);
        let stdout = String::from_utf8_lossy(&output.stdout);
        assert!(
            output.status.success(),
            "Codex failed (resume={resume}): {stderr}\n{stdout}"
        );
        assert_eq!(
            std::fs::read_to_string(workspace.join("restored.env"))
                .expect("real CLI must execute restored file write"),
            KEY
        );
        let rows: Vec<Value> = stdout
            .lines()
            .filter_map(|line| serde_json::from_str(line).ok())
            .collect();
        let messages: Vec<_> = rows
            .iter()
            .filter(|row| {
                row["type"] == "item.completed"
                    && row
                        .pointer("/item/type")
                        .is_some_and(|v| v == "agent_message")
            })
            .filter_map(|row| row.pointer("/item/text").and_then(Value::as_str))
            .collect();
        assert_eq!(messages,vec![format!("Completed: {KEY}")],"assistant text must restore once after fragmented deltas and terminal snapshots (resume={resume}): {stdout}");
        let commands: Vec<_> = rows
            .iter()
            .filter(|row| {
                row["type"] == "item.completed"
                    && row
                        .pointer("/item/type")
                        .is_some_and(|v| v == "command_execution")
            })
            .collect();
        assert_eq!(
            commands.len(),
            if resume { 0 } else { 2 },
            "each tool must execute exactly once: {stdout}"
        );
        assert!(
            commands
                .iter()
                .all(|row| row.pointer("/item/exit_code") == Some(&json!(0))),
            "tool execution must succeed: {stdout}"
        );
    }
    let state = provider.evidence.lock().unwrap();
    assert!(
        state.tool_output_seen,
        "real CLI must send tool output back"
    );
    assert_eq!(
        state.stages, 4,
        "two executed tools, final assistant response, and resumed conversation"
    );
    assert!(state.marker.is_some(), "provider must see a placeholder");
    assert!(
        state.lite_seen,
        "installed default client must exercise Responses Lite"
    );
    assert!(
        state.custom_output_seen,
        "installed default client must execute custom code-mode calls"
    );
    if disable_after_first {
        assert!(
            state.plaintext_after_disable,
            "Off must stop outbound masking while restoring issued references"
        );
    }
    if transition && !websockets {
        assert!(
            state.compressed > 0,
            "installed client must send real compressed HTTP requests"
        );
    }
    if websockets {
        assert!(state.transports.contains(&"websocket"));
        assert!(
            state
                .requests
                .iter()
                .any(|v| v["previous_response_id"].is_string()),
            "real CLI must actually use previous_response_id"
        );
    }
    eprintln!("Installed Codex lifecycle PASS: storage={storage_label}, transition={transition}, disable_after_first={disable_after_first}, websockets={websockets}, requests={}, transports={:?}, compressed={}, file-write + tool-result + assistant + fresh-process resume restored",state.requests.len(),state.transports,state.compressed);
    drop(state);
    drop(running);
    upstream.abort();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "requires installed Codex and native OS credential storage; local TLS provider only"]
async fn installed_codex_redact_http_lifecycle() {
    run_installed_codex(false, false, false).await;
}
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "requires installed Codex and native OS credential storage; local TLS provider only"]
async fn installed_codex_off_to_redact_websocket_lifecycle() {
    run_installed_codex(true, true, false).await;
}

/// This deterministic test runs in the ordinary suite. The installed-client
/// tests additionally verify that Codex consumes and executes these events.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn real_firewall_http_continuation_survives_mode_change_and_reconnect() {
    let root = tempfile::tempdir().unwrap();
    let storage = Arc::new(lr_api_keys::MockKeychain::new());
    let (firewall, request) = fixture(root.path(), lr_config::LlmMode::Proxy, storage.clone());
    firewall
        .config_manager
        .update(|c| c.secret_scanning.action = lr_config::SecretScanAction::Off)
        .unwrap();
    let firewall = Arc::new(firewall);
    let provider = Arc::new(Provider {
        evidence: Mutex::default(),
        firewall: firewall.clone(),
        transition: true,
        websockets: false,
        disable_after_first: false,
    });
    let (provider_port, roots, upstream) = start_provider(provider.clone()).await;
    let mut running = start_proxy(
        root.path(),
        firewall.clone(),
        &request.client_id,
        roots.clone(),
    )
    .await;
    let mut previous = None;
    let mut marker = None;
    for stage in 0..3 {
        // Each request has a new connection; the response binding must carry
        // the capability without relying on a live restorer or socket.
        let client = reqwest::Client::builder()
            .no_proxy()
            .proxy(reqwest::Proxy::all(&running.url).unwrap())
            .add_root_certificate(
                reqwest::Certificate::from_pem(running.ca_pem.as_bytes()).unwrap(),
            )
            .build()
            .unwrap();
        let content = if stage < 2 {
            format!("aws_secret_access_key={KEY}")
        } else {
            "tool completed".into()
        };
        let mut body = json!({"model":"local-test","stream":true,"input":[{"role":"user","content":content}],"tools":[{"type":"function","name":"exec_command","parameters":{"type":"object","properties":{"cmd":{"type":"string"}},"required":["cmd"]}}]});
        if stage > 0 {
            body["previous_response_id"] = json!(previous);
            body["input"] = json!([{"type":"function_call_output","call_id":format!("call_{}",stage-1),"output":content}]);
        }
        let response = client
            .post(format!(
                "https://localhost:{provider_port}/backend-api/codex/responses"
            ))
            .json(&body)
            .send()
            .await
            .unwrap();
        let status = response.status();
        let text = response.text().await.unwrap();
        assert!(
            status.is_success(),
            "stage {stage} rejected: {status} {text}"
        );
        let events: Vec<Value> = text
            .lines()
            .filter_map(|line| line.strip_prefix("data: "))
            .filter_map(|line| serde_json::from_str(line).ok())
            .collect();
        previous = events.iter().find_map(|v| {
            v.pointer("/response/id")
                .and_then(Value::as_str)
                .map(str::to_owned)
        });
        assert!(previous.is_some(), "provider ID must reach client");
        if stage == 1 {
            assert!(
                events.iter().any(|v| v
                    .pointer("/item/arguments")
                    .and_then(Value::as_str)
                    .is_some_and(|s| s.contains(KEY))),
                "file write arguments restored"
            );
            marker = provider.evidence.lock().unwrap().marker.clone();
            // Recreate the proxy + AppFirewall, retaining only persisted OS
            // storage and app configuration, as on application restart.
            let fresh = Arc::new(AppFirewall {
                config_manager: firewall.config_manager.clone(),
                firewall: firewall.firewall.clone(),
                model_approvals: firewall.model_approvals.clone(),
                metrics: firewall.metrics.clone(),
                state: firewall.state.clone(),
                secret_storage: storage.clone(),
            });
            drop(running);
            running = start_proxy(root.path(), fresh, &request.client_id, roots.clone()).await;
        }
        if stage == 2 {
            assert!(
                text.contains(KEY),
                "restored final assistant text after restart"
            );
        }
    }
    assert!(marker.is_some());
    assert!(provider.evidence.lock().unwrap().tool_output_seen);
    drop(running);
    upstream.abort();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "requires installed Codex and native OS credential storage; local TLS provider only"]
async fn installed_codex_off_to_redact_http_lifecycle() {
    run_installed_codex(true, false, false).await;
}
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "requires installed Codex and native OS credential storage; local TLS provider only"]
async fn installed_codex_redact_websocket_lifecycle() {
    run_installed_codex(false, true, false).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "requires installed Codex and native OS credential storage; local TLS provider only"]
async fn installed_codex_redact_to_off_http_lifecycle() {
    run_installed_codex(false, false, true).await;
}
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "requires installed Codex and native OS credential storage; local TLS provider only"]
async fn installed_codex_redact_to_off_websocket_lifecycle() {
    run_installed_codex(false, true, true).await;
}
