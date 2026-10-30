//! STDIO ↔ HTTP bridge for MCP (`localrouter --mcp-bridge`).
//!
//! A full-duplex relay between an MCP client speaking newline-delimited
//! JSON-RPC on stdin/stdout and LocalRouter's Streamable HTTP gateway:
//!
//! - **client → server**: every stdin line is POSTed to the gateway. Requests
//!   run concurrently (a long `tools/call` or `events/stream` never blocks the
//!   next line); notifications and client responses (answers to server
//!   requests such as sampling or elicitation) are POSTed in order and write
//!   nothing to stdout.
//! - **POST responses** may be a JSON body or an SSE stream. Every JSON-RPC
//!   message in an SSE body is written to stdout, in order, until the stream
//!   ends; there is no total timeout, so a long-lived `events/stream` keeps
//!   delivering notifications before its final result.
//! - **server → client**: after `initialize`, the bridge captures the
//!   `Mcp-Session-Id`, sends it on every request, and holds the session's GET
//!   SSE stream (reconnecting with backoff). Every notification and
//!   server-initiated request on it is written to stdout verbatim, including
//!   methods the bridge does not know (`notifications/claude/channel`,
//!   `notifications/events/*`).
//! - **stdout** has exactly one writer task, so lines never interleave.
//! - **stdin EOF** stops the stream and in-flight requests and DELETEs the
//!   session.
//!
//! Every HTTP request carries `Authorization: Bearer <secret>` and
//! `X-LocalRouter-Bridge: stdio`, which tells the gateway the session is a
//! stdio bridge session. Toward its stdio client the bridge negotiates at most
//! MCP 2025-11-25; see [`super::negotiation`] for why and how.

use super::negotiation;
use super::sse::SseDecoder;
use crate::gateway::types::MCP_SESSION_ID_HEADER;
use futures_util::StreamExt;
use lr_api_keys::keychain_trait::{CachedKeychain, KeychainStorage};
use lr_config::{AppConfig, Client, ConfigManager, ServerConfig};
use lr_types::{AppError, AppResult};
use serde_json::{json, Value};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;
use tokio::io::{AsyncBufReadExt, AsyncRead, AsyncWrite, AsyncWriteExt, BufReader};
use tokio::sync::mpsc;
use tokio::task::JoinSet;
use tokio_util::sync::CancellationToken;
use tracing::{debug, error, info, trace, warn};

/// Header identifying bridge traffic to the gateway.
pub const BRIDGE_HEADER: &str = "x-localrouter-bridge";
/// Value of [`BRIDGE_HEADER`] for the stdio bridge.
pub const BRIDGE_HEADER_VALUE: &str = "stdio";
/// Environment variable overriding the gateway URL.
pub const URL_ENV: &str = "LOCALROUTER_URL";

/// Notifications and client responses are acknowledged with 202 at once; a
/// gateway that takes longer than this is not going to answer.
const QUIET_POST_TIMEOUT: Duration = Duration::from_secs(30);
const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
const DELETE_TIMEOUT: Duration = Duration::from_secs(3);
/// Time allowed to flush lines already queued for stdout at shutdown.
const DRAIN_TIMEOUT: Duration = Duration::from_secs(2);
const STREAM_BACKOFF_MIN: Duration = Duration::from_millis(250);
const STREAM_BACKOFF_MAX: Duration = Duration::from_secs(30);
/// A stream that stayed up this long resets the reconnect backoff.
const STREAM_HEALTHY_AFTER: Duration = Duration::from_secs(10);
/// Lines queued for stdout before producers wait on the writer.
const STDOUT_QUEUE: usize = 1024;

/// STDIO ↔ HTTP bridge to the LocalRouter MCP gateway.
pub struct StdioBridge {
    client_id: String,
    client_secret: String,
    server_url: String,
    http_client: reqwest::Client,
    stream_backoff_min: Duration,
}

impl StdioBridge {
    /// Create a bridge from the LocalRouter config.
    ///
    /// * `client_id` - client to authenticate as (auto-detected if `None`)
    /// * `config_path` - config file (default location if `None`)
    /// * `url` - gateway URL override (`--url`); falls back to
    ///   `LOCALROUTER_URL`, then to the server host/port in the config
    pub async fn new(
        client_id: Option<String>,
        config_path: Option<PathBuf>,
        url: Option<String>,
    ) -> AppResult<Self> {
        let config_manager = if let Some(path) = config_path {
            ConfigManager::load_from_path(path).await?
        } else {
            ConfigManager::load().await?
        };
        let config = config_manager.get();

        let (client_id, client_secret) = resolve_client_secret(client_id, &config).await?;

        // An env-supplied secret may name a client absent from this config
        // (e.g. `--url` pointing at another instance); only validate known ones.
        if let Ok(client) = find_client_by_id(&client_id, &config) {
            if !client.mcp_permissions.has_any_access() {
                return Err(AppError::Config(format!(
                    "Client '{}' has no MCP servers configured. Set 'mcp_permissions' in config.yaml",
                    client_id
                )));
            }
            info!(
                "Bridge initialized for client '{}' with {} MCP servers (global: {:?})",
                client_id,
                client
                    .mcp_permissions
                    .allowed_server_ids(&config.mcp_servers)
                    .len(),
                client.mcp_permissions.global
            );
        }

        let server_url = resolve_server_url(url, std::env::var(URL_ENV).ok(), &config.server);
        Self::with_endpoint(client_id, client_secret, server_url)
    }

    /// Create a bridge for an explicit gateway URL and credentials.
    pub fn with_endpoint(
        client_id: String,
        client_secret: String,
        server_url: String,
    ) -> AppResult<Self> {
        let mut builder = reqwest::Client::builder().connect_timeout(CONNECT_TIMEOUT);
        // A system proxy must never see loopback traffic (and probing for one
        // is slow on macOS).
        if is_loopback_url(&server_url) {
            builder = builder.no_proxy();
        }
        let http_client = builder
            .build()
            .map_err(|e| AppError::Mcp(format!("Failed to build HTTP client: {e}")))?;
        Ok(Self {
            client_id,
            client_secret,
            server_url,
            http_client,
            stream_backoff_min: STREAM_BACKOFF_MIN,
        })
    }

    /// The gateway URL this bridge talks to.
    pub fn server_url(&self) -> &str {
        &self.server_url
    }

    #[cfg(test)]
    fn with_stream_backoff(mut self, min: Duration) -> Self {
        self.stream_backoff_min = min;
        self
    }

    /// Run on the process's stdin/stdout until stdin closes.
    pub async fn run(self) -> AppResult<()> {
        self.run_with_io(tokio::io::stdin(), tokio::io::stdout())
            .await
    }

    /// Run on arbitrary byte streams until `input` reaches EOF (or `output`
    /// closes). Logs go to tracing (stderr); `output` carries only JSON-RPC.
    pub async fn run_with_io<R, W>(self, input: R, output: W) -> AppResult<()>
    where
        R: AsyncRead + Unpin + Send + 'static,
        W: AsyncWrite + Unpin + Send + 'static,
    {
        info!(
            "Bridge started for client '{}' -> {}",
            self.client_id, self.server_url
        );
        let shutdown = CancellationToken::new();
        let (out_tx, out_rx) = mpsc::channel::<String>(STDOUT_QUEUE);
        let writer = tokio::spawn(write_stdout(output, out_rx, shutdown.clone()));

        let ctx = Arc::new(Ctx {
            http: self.http_client,
            url: self.server_url,
            secret: self.client_secret,
            out: out_tx,
            session: parking_lot::RwLock::new(SessionState::default()),
            pending_init: parking_lot::Mutex::new(None),
            shutdown: shutdown.clone(),
        });

        let mut requests: JoinSet<()> = JoinSet::new();
        let mut stream_task: Option<tokio::task::JoinHandle<()>> = None;
        let mut reader = BufReader::new(input);
        let mut line = String::new();

        let result = loop {
            while requests.try_join_next().is_some() {}
            line.clear();
            let read = tokio::select! {
                _ = shutdown.cancelled() => break Ok(()),
                read = reader.read_line(&mut line) => read,
            };
            match read {
                Ok(0) => {
                    debug!("EOF on stdin, shutting down");
                    break Ok(());
                }
                Ok(_) => {}
                Err(e) => {
                    error!("Failed to read from stdin: {e}");
                    break Err(e.into());
                }
            }
            let text = line.trim();
            if text.is_empty() {
                continue;
            }
            trace!("stdin: {text}");
            let message: Value = match serde_json::from_str(text) {
                Ok(v) => v,
                Err(e) => {
                    warn!("Unparseable stdin line: {e}");
                    ctx.emit_error(Value::Null, -32700, format!("Parse error: {e}"))
                        .await;
                    continue;
                }
            };

            match classify(&message) {
                Kind::Request { id, method } => {
                    let initialized = ctx.session.read().initialized;
                    if let Some(reply) = negotiation::intercept(&message, &id, &method, initialized)
                    {
                        debug!("Declined {method} locally (bridge negotiates <= 2025-11-25)");
                        ctx.emit(reply).await;
                        continue;
                    }
                    if method == "initialize" {
                        let (forward, negotiated) = negotiation::prepare_initialize(message);
                        *ctx.pending_init.lock() = Some((id.clone(), negotiated.clone()));
                        {
                            // A re-initialize starts a new gateway session.
                            let mut s = ctx.session.write();
                            s.session_id = None;
                            s.protocol_version = None;
                        }
                        if let Some(task) = stream_task.take() {
                            task.abort();
                        }
                        // Inline: later lines need the session id.
                        ctx.clone().forward_request(id, forward).await;
                        {
                            let mut s = ctx.session.write();
                            s.protocol_version = Some(negotiated);
                            s.initialized = true;
                        }
                        stream_task = Some(tokio::spawn(
                            ctx.clone().run_server_stream(self.stream_backoff_min),
                        ));
                    } else {
                        requests.spawn(ctx.clone().forward_request(id, message));
                    }
                }
                Kind::Notification | Kind::Response => {
                    ctx.post_quiet(&message).await;
                }
                Kind::Invalid { id } => {
                    ctx.emit_error(id, -32600, "Invalid Request".to_string())
                        .await;
                }
            }
        };

        // Shutdown: stop producers, end the gateway session, drain stdout.
        shutdown.cancel();
        requests.shutdown().await;
        if let Some(task) = stream_task.take() {
            task.abort();
            let _ = task.await;
        }
        ctx.delete_session().await;
        drop(ctx);
        if tokio::time::timeout(DRAIN_TIMEOUT, writer).await.is_err() {
            warn!("Timed out flushing stdout at shutdown");
        }
        info!("Bridge stopped");
        result
    }
}

/// The single stdout writer: one JSON-RPC message per line, flushed each time.
async fn write_stdout<W: AsyncWrite + Unpin>(
    mut output: W,
    mut rx: mpsc::Receiver<String>,
    shutdown: CancellationToken,
) {
    while let Some(line) = rx.recv().await {
        let written = async {
            output.write_all(line.as_bytes()).await?;
            output.write_all(b"\n").await?;
            output.flush().await
        }
        .await;
        if let Err(e) = written {
            warn!("stdout closed ({e}); shutting down");
            shutdown.cancel();
            return;
        }
    }
    let _ = output.shutdown().await;
}

enum Kind {
    Request { id: Value, method: String },
    Notification,
    Response,
    Invalid { id: Value },
}

fn classify(message: &Value) -> Kind {
    let id = message.get("id").cloned().unwrap_or(Value::Null);
    if !message.is_object() {
        return Kind::Invalid { id: Value::Null };
    }
    match message.get("method") {
        Some(Value::String(method)) => {
            if id.is_null() {
                Kind::Notification
            } else {
                Kind::Request {
                    id,
                    method: method.clone(),
                }
            }
        }
        Some(_) => Kind::Invalid { id },
        None if message.get("result").is_some() || message.get("error").is_some() => Kind::Response,
        None => Kind::Invalid { id },
    }
}

#[derive(Default)]
struct SessionState {
    session_id: Option<String>,
    /// Version negotiated with the stdio client (sent as MCP-Protocol-Version).
    protocol_version: Option<String>,
    initialized: bool,
}

/// State shared by the reader, request tasks and the server stream.
struct Ctx {
    http: reqwest::Client,
    url: String,
    secret: String,
    out: mpsc::Sender<String>,
    session: parking_lot::RwLock<SessionState>,
    /// `initialize` awaiting its result: (request id, negotiated version).
    pending_init: parking_lot::Mutex<Option<(Value, String)>>,
    shutdown: CancellationToken,
}

impl Ctx {
    fn with_headers(&self, builder: reqwest::RequestBuilder) -> reqwest::RequestBuilder {
        let session = self.session.read();
        let mut builder = builder
            .bearer_auth(&self.secret)
            .header(BRIDGE_HEADER, BRIDGE_HEADER_VALUE);
        if let Some(id) = &session.session_id {
            builder = builder.header(MCP_SESSION_ID_HEADER, id);
        }
        if let Some(version) = &session.protocol_version {
            builder = builder.header("mcp-protocol-version", version);
        }
        builder
    }

    fn post(&self, body: &Value) -> reqwest::RequestBuilder {
        self.with_headers(self.http.post(&self.url))
            .header(reqwest::header::CONTENT_TYPE, "application/json")
            .header(
                reqwest::header::ACCEPT,
                "application/json, text/event-stream",
            )
            .json(body)
    }

    /// Queue one message for stdout, applying the `initialize` rewrite.
    async fn emit(&self, mut message: Value) {
        {
            let mut pending = self.pending_init.lock();
            let matches = pending
                .as_ref()
                .is_some_and(|(id, _)| message.get("id") == Some(id))
                && message.get("method").is_none();
            if matches {
                let (_, negotiated) = pending.take().unwrap_or_default();
                negotiation::rewrite_initialize_result(&mut message, &negotiated);
            }
        }
        match serde_json::to_string(&message) {
            Ok(line) => {
                if self.out.send(line).await.is_err() {
                    debug!("stdout writer gone; dropping message");
                }
            }
            Err(e) => error!("Failed to serialize message for stdout: {e}"),
        }
    }

    async fn emit_error(&self, id: Value, code: i32, message: String) {
        self.emit(json!({
            "jsonrpc": "2.0",
            "id": id,
            "error": { "code": code, "message": message }
        }))
        .await;
    }

    /// Emit a JSON-RPC message (or each message of a batch array).
    async fn emit_body(&self, value: Value) {
        match value {
            Value::Array(items) => {
                for item in items {
                    self.emit(item).await;
                }
            }
            other => self.emit(other).await,
        }
    }

    fn connect_error(&self, e: &reqwest::Error) -> String {
        if e.is_connect() {
            format!(
                "Could not connect to LocalRouter at {}. Is the app running?",
                self.url
            )
        } else {
            format!("HTTP request to LocalRouter failed: {e}")
        }
    }

    /// POST a client request and relay its answer to stdout.
    async fn forward_request(self: Arc<Self>, id: Value, body: Value) {
        let response = match self.post(&body).send().await {
            Ok(r) => r,
            Err(e) => {
                warn!("Request {id} failed: {e}");
                let message = self.connect_error(&e);
                self.emit_error(id, -32603, message).await;
                return;
            }
        };

        if let Some(session_id) = response
            .headers()
            .get(MCP_SESSION_ID_HEADER)
            .and_then(|v| v.to_str().ok())
        {
            let mut s = self.session.write();
            if s.session_id.as_deref() != Some(session_id) {
                debug!("Gateway session: {session_id}");
                s.session_id = Some(session_id.to_string());
            }
        }

        let status = response.status();
        if status == reqwest::StatusCode::ACCEPTED || status == reqwest::StatusCode::NO_CONTENT {
            // The answer comes on the server stream.
            debug!("Request {id} accepted; response will arrive on the stream");
            return;
        }

        let is_sse = response
            .headers()
            .get(reqwest::header::CONTENT_TYPE)
            .and_then(|v| v.to_str().ok())
            .is_some_and(|v| v.starts_with("text/event-stream"));

        if status.is_success() && is_sse {
            let answered = self.relay_sse(response, Some(&id)).await;
            if !answered && !self.shutdown.is_cancelled() {
                self.emit_error(
                    id,
                    -32603,
                    "LocalRouter closed the response stream before answering".into(),
                )
                .await;
            }
            return;
        }

        let bytes = match response.bytes().await {
            Ok(b) => b,
            Err(e) => {
                let message = self.connect_error(&e);
                self.emit_error(id, -32603, message).await;
                return;
            }
        };

        if status.is_success() {
            if bytes.iter().all(u8::is_ascii_whitespace) {
                return;
            }
            match serde_json::from_slice::<Value>(&bytes) {
                Ok(value) => self.emit_body(value).await,
                Err(e) => {
                    self.emit_error(
                        id,
                        -32603,
                        format!("Invalid response from LocalRouter: {e}"),
                    )
                    .await
                }
            }
            return;
        }

        // Non-2xx: forward a JSON-RPC error body as is, else explain the status.
        if let Ok(mut value) = serde_json::from_slice::<Value>(&bytes) {
            if value.get("jsonrpc").is_some()
                && (value.get("error").is_some() || value.get("result").is_some())
            {
                if value.get("id").is_none_or(Value::is_null) {
                    value["id"] = id;
                }
                self.emit(value).await;
                return;
            }
        }
        let detail = String::from_utf8_lossy(&bytes);
        let message = match status.as_u16() {
            401 => "Invalid client credentials. Check LOCALROUTER_CLIENT_SECRET or run the LocalRouter app once to store credentials.".to_string(),
            403 => "This client is not allowed to access MCP servers. Check its MCP permissions in LocalRouter.".to_string(),
            404 if self.session.read().session_id.is_some() => {
                "LocalRouter no longer knows this MCP session (was the app restarted?). Restart the MCP client to reconnect.".to_string()
            }
            404 => format!("MCP endpoint not found at {}. Check --url / LOCALROUTER_URL.", self.url),
            _ => format!("LocalRouter returned HTTP {status}: {}", detail.trim()),
        };
        self.emit_error(id, -32603, message).await;
    }

    /// Relay every JSON-RPC message of an SSE body to stdout until it ends.
    /// Returns whether a response for `answer_id` was seen.
    async fn relay_sse(&self, response: reqwest::Response, answer_id: Option<&Value>) -> bool {
        let mut decoder = SseDecoder::new();
        let mut body = response.bytes_stream();
        let mut answered = false;
        loop {
            let chunk = tokio::select! {
                _ = self.shutdown.cancelled() => return answered,
                chunk = body.next() => chunk,
            };
            let chunk = match chunk {
                Some(Ok(c)) => c,
                Some(Err(e)) => {
                    debug!("SSE body ended with error: {e}");
                    return answered;
                }
                None => return answered,
            };
            let events = match decoder.feed(&chunk) {
                Ok(events) => events,
                Err(e) => {
                    warn!("Dropping SSE stream: {e}");
                    return answered;
                }
            };
            for event in events {
                // `endpoint` belongs to the legacy HTTP+SSE transport.
                if !matches!(event.event.as_deref(), None | Some("message")) {
                    continue;
                }
                match serde_json::from_str::<Value>(&event.data) {
                    Ok(message) => {
                        if let Some(answer_id) = answer_id {
                            if message.get("method").is_none()
                                && message.get("id") == Some(answer_id)
                            {
                                answered = true;
                            }
                        }
                        self.emit_body(message).await;
                    }
                    Err(e) => debug!("Skipping non-JSON SSE event: {e}"),
                }
            }
        }
    }

    /// POST a notification or client response; nothing is written to stdout.
    async fn post_quiet(&self, body: &Value) {
        let what = body
            .get("method")
            .and_then(Value::as_str)
            .map(str::to_string)
            .unwrap_or_else(|| format!("response {}", body.get("id").unwrap_or(&Value::Null)));
        match tokio::time::timeout(QUIET_POST_TIMEOUT, self.post(body).send()).await {
            Ok(Ok(r)) if r.status().is_success() => trace!("{what}: HTTP {}", r.status()),
            Ok(Ok(r)) => warn!("{what}: LocalRouter answered HTTP {}", r.status()),
            Ok(Err(e)) => warn!("{what}: {}", self.connect_error(&e)),
            Err(_) => warn!("{what}: no answer from LocalRouter within {QUIET_POST_TIMEOUT:?}"),
        }
    }

    /// Hold the session's GET SSE stream, reconnecting with backoff, and relay
    /// every server notification and request to stdout.
    async fn run_server_stream(self: Arc<Self>, backoff_min: Duration) {
        let mut backoff = backoff_min;
        loop {
            if self.shutdown.is_cancelled() {
                return;
            }
            let request = self
                .with_headers(self.http.get(&self.url))
                .header(reqwest::header::ACCEPT, "text/event-stream");
            let sent = tokio::select! {
                _ = self.shutdown.cancelled() => return,
                sent = request.send() => sent,
            };
            match sent {
                Ok(response)
                    if response.status().is_success()
                        && response
                            .headers()
                            .get(reqwest::header::CONTENT_TYPE)
                            .and_then(|v| v.to_str().ok())
                            .is_some_and(|v| v.starts_with("text/event-stream")) =>
                {
                    debug!("Server stream connected");
                    let started = tokio::time::Instant::now();
                    self.relay_sse(response, None).await;
                    if started.elapsed() >= STREAM_HEALTHY_AFTER {
                        backoff = backoff_min;
                    }
                    debug!("Server stream ended; reconnecting");
                }
                Ok(response) if response.status() == reqwest::StatusCode::METHOD_NOT_ALLOWED => {
                    warn!("LocalRouter offers no server stream (HTTP 405); server notifications will not reach this client");
                    return;
                }
                Ok(response) => warn!("Server stream refused: HTTP {}", response.status()),
                Err(e) => debug!("Server stream: {}", self.connect_error(&e)),
            }
            tokio::select! {
                _ = self.shutdown.cancelled() => return,
                _ = tokio::time::sleep(backoff) => {}
            }
            backoff = (backoff * 2).min(STREAM_BACKOFF_MAX);
        }
    }

    /// End the gateway session (Streamable HTTP `DELETE`).
    async fn delete_session(&self) {
        if self.session.read().session_id.is_none() {
            return;
        }
        let request = self.with_headers(self.http.delete(&self.url));
        match tokio::time::timeout(DELETE_TIMEOUT, request.send()).await {
            Ok(Ok(r)) => debug!("Session DELETE: HTTP {}", r.status()),
            Ok(Err(e)) => debug!("Session DELETE failed: {e}"),
            Err(_) => debug!("Session DELETE timed out"),
        }
    }
}

/// Gateway URL: `--url`, then `LOCALROUTER_URL`, then the configured server
/// address (the same host/port the app listens on; debug builds default to
/// 33625, release to 3625).
pub fn resolve_server_url(
    cli: Option<String>,
    env: Option<String>,
    server: &ServerConfig,
) -> String {
    if let Some(url) = cli.or(env).map(|u| u.trim().to_string()) {
        if !url.is_empty() {
            return url;
        }
    }
    let host = match server.host.trim() {
        "" | "0.0.0.0" => "127.0.0.1".to_string(),
        "::" | "[::]" => "[::1]".to_string(),
        h if h.contains(':') && !h.starts_with('[') => format!("[{h}]"),
        h => h.to_string(),
    };
    format!("http://{host}:{}/mcp", server.port)
}

fn is_loopback_url(url: &str) -> bool {
    let Some(host) = reqwest::Url::parse(url)
        .ok()
        .and_then(|u| u.host_str().map(str::to_string))
    else {
        return false;
    };
    let bare = host.trim_start_matches('[').trim_end_matches(']');
    bare.eq_ignore_ascii_case("localhost")
        || bare
            .parse::<std::net::IpAddr>()
            .is_ok_and(|ip| ip.is_loopback())
}

/// Resolve client ID and secret
///
/// Resolution order:
/// 1. LOCALROUTER_CLIENT_SECRET env var (if set, use provided client_id or "env")
/// 2. Load from config + keychain (find client, load secret)
///
/// # Arguments
/// * `client_id` - Optional explicit client ID
/// * `config` - Application configuration
///
/// # Returns
/// Tuple of (client_id, client_secret)
///
/// # Errors
/// Returns error if:
/// - Client not found in config
/// - Client is disabled
/// - Client secret not found in keychain
async fn resolve_client_secret(
    client_id: Option<String>,
    config: &AppConfig,
) -> AppResult<(String, String)> {
    // Try environment variable first
    if let Ok(secret) = std::env::var("LOCALROUTER_CLIENT_SECRET") {
        let id = client_id.unwrap_or_else(|| {
            warn!("Using LOCALROUTER_CLIENT_SECRET without explicit client ID");
            "env".to_string()
        });

        warn!("Using client secret from LOCALROUTER_CLIENT_SECRET environment variable - this is less secure than keychain storage");
        return Ok((id, secret));
    }

    // Load from config + keychain
    let client = if let Some(id) = client_id.as_ref() {
        // Explicit client ID provided
        find_client_by_id(id, config)?
    } else {
        // Auto-detect first enabled client with MCP servers
        find_first_enabled_client(config)?
    };

    // Load secret from keychain
    let keychain = CachedKeychain::auto()?;
    let secret = keychain
        .get("LocalRouter-Clients", &client.id)?
        .ok_or_else(|| {
            AppError::Config(format!(
                "Client secret not found for '{}'. Run LocalRouter GUI once to create credentials, or set LOCALROUTER_CLIENT_SECRET env var",
                client.id
            ))
        })?;

    info!("Using client '{}' from config", client.id);
    Ok((client.id.clone(), secret))
}

/// Find client by ID in config
fn find_client_by_id<'a>(client_id: &str, config: &'a AppConfig) -> AppResult<&'a Client> {
    let client = config
        .clients
        .iter()
        .find(|c| c.id == client_id)
        .ok_or_else(|| {
            AppError::Config(format!(
                "Client '{}' not found in config.yaml. Available clients: {}",
                client_id,
                config
                    .clients
                    .iter()
                    .map(|c| c.id.as_str())
                    .collect::<Vec<_>>()
                    .join(", ")
            ))
        })?;

    if !client.credentials_active() {
        return Err(AppError::Config(format!(
            "Client '{}' is disabled. Enable it in config.yaml",
            client_id
        )));
    }

    Ok(client)
}

/// Find first enabled client with MCP servers
fn find_first_enabled_client(config: &AppConfig) -> AppResult<&Client> {
    config
        .clients
        .iter()
        .find(|c| c.credentials_active() && c.mcp_permissions.has_any_access())
        .ok_or_else(|| {
            AppError::Config(
                "No enabled clients with MCP servers found. Configure a client in config.yaml"
                    .to_string(),
            )
        })
}

#[cfg(test)]
mod duplex_tests;

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;
    use lr_config::{
        FirewallRules, McpPermissions, McpServerAccess, ModelPermissions, PermissionState,
        SkillsPermissions,
    };

    fn test_config() -> AppConfig {
        let mut config = AppConfig::default();

        // Create mcp_permissions with filesystem server enabled
        let mut test_mcp_permissions = McpPermissions::default();
        test_mcp_permissions
            .servers
            .insert("filesystem".to_string(), PermissionState::Allow);

        config.clients = vec![
            Client {
                id: "test_client".to_string(),
                name: "Test Client".to_string(),
                enabled: true,
                security: Default::default(),
                allowed_llm_providers: vec![],
                mcp_server_access: McpServerAccess::Specific(vec!["filesystem".to_string()]),
                context_management_enabled: None,
                created_at: Utc::now(),
                last_used: None,
                strategy_id: "default".to_string(),
                roots: None,
                mcp_sampling_enabled: false,
                mcp_sampling_requires_approval: true,
                mcp_sampling_max_tokens: None,
                mcp_sampling_rate_limit: None,
                firewall: FirewallRules::default(),
                skills_access: lr_config::SkillsAccess::None,
                marketplace_enabled: false,
                mcp_permissions: test_mcp_permissions,
                skills_permissions: SkillsPermissions::default(),
                coding_agents_permissions: lr_config::CodingAgentsPermissions::default(),
                coding_agent_permission: PermissionState::Off,
                coding_agent_type: None,
                model_permissions: ModelPermissions::default(),
                marketplace_permission: PermissionState::Off,
                client_mode: lr_config::ClientMode::default(),
                llm_mode: lr_config::LlmMode::default(),
                mcp_mode: lr_config::McpMode::default(),
                template_id: None,
                sync_config: false,
                reverse_proxy: None,
                guardrails_enabled: None,
                guardrails: lr_config::ClientGuardrailsConfig::default(),
                prompt_compression: lr_config::ClientPromptCompressionConfig::default(),
                json_repair: lr_config::ClientJsonRepairConfig::default(),
                secret_scanning: lr_config::ClientSecretScanningConfig::default(),
                mcp_sampling_permission: PermissionState::Ask,
                mcp_elicitation_permission: PermissionState::Ask,
                catalog_compression_enabled: None,
                client_tools_indexing: None,
                memory_enabled: None,
                memory_folder: None,
                push_permissions: Default::default(),
                push_fallback_tools: Default::default(),
                push_webhook_urls: Vec::new(),
                claude_channels_launched: false,
            },
            Client {
                id: "disabled_client".to_string(),
                name: "Disabled Client".to_string(),
                enabled: false,
                security: Default::default(),
                allowed_llm_providers: vec![],
                mcp_server_access: McpServerAccess::Specific(vec!["web".to_string()]),
                context_management_enabled: None,
                created_at: Utc::now(),
                last_used: None,
                strategy_id: "default".to_string(),
                roots: None,
                mcp_sampling_enabled: false,
                mcp_sampling_requires_approval: true,
                mcp_sampling_max_tokens: None,
                mcp_sampling_rate_limit: None,
                firewall: FirewallRules::default(),
                skills_access: lr_config::SkillsAccess::None,
                marketplace_enabled: false,
                mcp_permissions: McpPermissions::default(),
                skills_permissions: SkillsPermissions::default(),
                coding_agents_permissions: lr_config::CodingAgentsPermissions::default(),
                coding_agent_permission: PermissionState::Off,
                coding_agent_type: None,
                model_permissions: ModelPermissions::default(),
                marketplace_permission: PermissionState::Off,
                client_mode: lr_config::ClientMode::default(),
                llm_mode: lr_config::LlmMode::default(),
                mcp_mode: lr_config::McpMode::default(),
                template_id: None,
                sync_config: false,
                reverse_proxy: None,
                guardrails_enabled: None,
                guardrails: lr_config::ClientGuardrailsConfig::default(),
                prompt_compression: lr_config::ClientPromptCompressionConfig::default(),
                json_repair: lr_config::ClientJsonRepairConfig::default(),
                secret_scanning: lr_config::ClientSecretScanningConfig::default(),
                mcp_sampling_permission: PermissionState::Ask,
                mcp_elicitation_permission: PermissionState::Ask,
                catalog_compression_enabled: None,
                client_tools_indexing: None,
                memory_enabled: None,
                memory_folder: None,
                push_permissions: Default::default(),
                push_fallback_tools: Default::default(),
                push_webhook_urls: Vec::new(),
                claude_channels_launched: false,
            },
            Client {
                id: "no_mcp_client".to_string(),
                name: "No MCP Client".to_string(),
                enabled: true,
                security: Default::default(),
                allowed_llm_providers: vec![],
                mcp_server_access: McpServerAccess::None,
                context_management_enabled: None,
                created_at: Utc::now(),
                last_used: None,
                strategy_id: "default".to_string(),
                roots: None,
                mcp_sampling_enabled: false,
                mcp_sampling_requires_approval: true,
                mcp_sampling_max_tokens: None,
                mcp_sampling_rate_limit: None,
                firewall: FirewallRules::default(),
                skills_access: lr_config::SkillsAccess::None,
                marketplace_enabled: false,
                mcp_permissions: McpPermissions::default(),
                skills_permissions: SkillsPermissions::default(),
                coding_agents_permissions: lr_config::CodingAgentsPermissions::default(),
                coding_agent_permission: PermissionState::Off,
                coding_agent_type: None,
                model_permissions: ModelPermissions::default(),
                marketplace_permission: PermissionState::Off,
                client_mode: lr_config::ClientMode::default(),
                llm_mode: lr_config::LlmMode::default(),
                mcp_mode: lr_config::McpMode::default(),
                template_id: None,
                sync_config: false,
                reverse_proxy: None,
                guardrails_enabled: None,
                guardrails: lr_config::ClientGuardrailsConfig::default(),
                prompt_compression: lr_config::ClientPromptCompressionConfig::default(),
                json_repair: lr_config::ClientJsonRepairConfig::default(),
                secret_scanning: lr_config::ClientSecretScanningConfig::default(),
                mcp_sampling_permission: PermissionState::Ask,
                mcp_elicitation_permission: PermissionState::Ask,
                catalog_compression_enabled: None,
                client_tools_indexing: None,
                memory_enabled: None,
                memory_folder: None,
                push_permissions: Default::default(),
                push_fallback_tools: Default::default(),
                push_webhook_urls: Vec::new(),
                claude_channels_launched: false,
            },
        ];
        config
    }

    #[test]
    fn test_find_client_by_id() {
        let config = test_config();

        // Find existing enabled client
        let client = find_client_by_id("test_client", &config).unwrap();
        assert_eq!(client.id, "test_client");

        // Disabled client should error
        let result = find_client_by_id("disabled_client", &config);
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("disabled"));

        // Non-existent client should error
        let result = find_client_by_id("nonexistent", &config);
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("not found"));
    }

    #[test]
    fn test_find_first_enabled_client() {
        let config = test_config();

        // Should find test_client (first enabled with MCP servers)
        let client = find_first_enabled_client(&config).unwrap();
        assert_eq!(client.id, "test_client");

        // Empty config should error
        let empty_config = AppConfig {
            clients: vec![],
            ..AppConfig::default()
        };
        let result = find_first_enabled_client(&empty_config);
        assert!(result.is_err());
    }
}
