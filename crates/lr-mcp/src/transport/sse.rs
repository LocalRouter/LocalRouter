//! SSE (Server-Sent Events) transport for MCP
//!
//! Implements Streamable HTTP transport per MCP spec 2025-06-18:
//! - POST endpoint: Send client→server requests
//! - GET endpoint: Persistent SSE stream for server→client responses and notifications
//!
//! This provides bidirectional communication using HTTP + SSE.

use crate::protocol::{
    JsonRpcMessage, JsonRpcNotification, JsonRpcRequest, JsonRpcResponse, StreamingChunk,
};
use crate::transport::Transport;
use async_trait::async_trait;
use futures_util::{Stream, StreamExt};
use lr_types::{AppError, AppResult};
use once_cell::sync::Lazy;
use parking_lot::RwLock;
use reqwest::Client;
use serde_json::Value;
use std::collections::HashMap;
use std::pin::Pin;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::oneshot;
use tokio::task::JoinHandle;

/// Cancel the background stream when connection setup fails or is cancelled.
struct ConnectingStream(Option<JoinHandle<()>>);

impl Drop for ConnectingStream {
    fn drop(&mut self) {
        if let Some(task) = self.0.take() {
            task.abort();
        }
    }
}

/// Global shared HTTP client with connection pooling
///
/// This client is shared across all SSE transports to reuse connections.
/// Configuration:
/// - 10 idle connections per host
/// - 60 second idle timeout
/// - 30 second request timeout
static HTTP_CLIENT: Lazy<Client> = Lazy::new(|| {
    Client::builder()
        .pool_max_idle_per_host(10)
        .pool_idle_timeout(Duration::from_secs(60))
        .timeout(Duration::from_secs(30))
        // Never follow redirects: every request carries the server's
        // configured credentials (bearer tokens, custom auth headers), and a
        // redirect could carry them to another host or downgrade to http.
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .expect("Failed to create global HTTP client")
});

/// Long-lived GET/subscription streams must not inherit the request deadline.
static STREAM_CLIENT: Lazy<Client> = Lazy::new(|| {
    Client::builder()
        .connect_timeout(Duration::from_secs(10))
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .expect("MCP stream HTTP client")
});

/// Resolve the `endpoint` event of the legacy SSE transport against the
/// configured server URL, accepting it only when it stays on the same origin.
///
/// Every later POST attaches the server's credentials, so an `endpoint`
/// naming another scheme, host or port would let a malicious or compromised
/// server redirect those credentials elsewhere and use LocalRouter as an
/// HTTP client against arbitrary (including internal) hosts.
pub(crate) fn resolve_endpoint(base_url: &str, endpoint: &str) -> Option<String> {
    let base = reqwest::Url::parse(base_url).ok()?;
    let resolved = base.join(endpoint).ok()?;
    let same_origin = resolved.scheme() == base.scheme()
        && resolved.host_str() == base.host_str()
        && resolved.port_or_known_default() == base.port_or_known_default();
    if same_origin {
        Some(resolved.to_string())
    } else {
        None
    }
}

/// Notification callback type for SSE transport
pub type SseNotificationCallback = Arc<dyn Fn(JsonRpcNotification) + Send + Sync>;

/// Produces fresh auth headers (e.g. `Authorization`) after the server
/// rejects a request with 401 — OAuth access tokens expire while a
/// long-lived transport keeps running.
pub type AuthRefresher = Arc<
    dyn Fn()
            -> Pin<Box<dyn std::future::Future<Output = AppResult<HashMap<String, String>>> + Send>>
        + Send
        + Sync,
>;

/// Request headers shared with the background stream task, so refreshed
/// auth is used for stream reconnects too.
type SharedHeaders = Arc<RwLock<HashMap<String, String>>>;

/// SSE transport implementation
///
/// Implements Streamable HTTP per MCP spec:
/// - POST requests to send client→server messages
/// - Persistent GET SSE stream for server→client responses and notifications
///
/// The persistent SSE connection is established on connect() and maintained
/// for the lifetime of the transport.
pub struct SseTransport {
    /// Base URL of the MCP server (used for SSE connection)
    url: String,

    /// Message endpoint URL for POST requests (received from "endpoint" SSE event)
    /// If None, falls back to using `url` for POST requests
    message_endpoint: Arc<RwLock<Option<String>>>,

    /// Legacy Streamable HTTP session assigned by initialize (absent for modern MCP).
    session_id: Arc<RwLock<Option<String>>>,

    /// HTTP client for sending requests
    client: Client,

    /// Custom headers to include in requests
    headers: SharedHeaders,

    /// Refreshes auth headers after a 401
    auth_refresher: Arc<RwLock<Option<AuthRefresher>>>,

    /// Pending requests waiting for responses
    /// Maps request ID to response sender
    pending: Arc<RwLock<HashMap<String, oneshot::Sender<JsonRpcResponse>>>>,

    /// Next request ID
    next_id: Arc<RwLock<u64>>,

    /// Whether the transport is closed
    closed: Arc<RwLock<bool>>,

    /// Whether the SSE stream is connected and ready
    stream_ready: Arc<RwLock<bool>>,

    /// Notification callback
    notification_callback: Arc<RwLock<Option<SseNotificationCallback>>>,

    /// Request callback for server-initiated requests (sampling, elicitation, etc.)
    request_callback: Arc<RwLock<Option<crate::transport::RequestCallback>>>,

    /// Background task handle for SSE stream reader
    #[allow(dead_code)]
    stream_task: Arc<RwLock<Option<JoinHandle<()>>>>,
}

/// Derive the standard MCP HTTP headers for an outgoing POST (SEP-2243).
///
/// `MCP-Protocol-Version` reflects the revision declared in the request's
/// `_meta` (injected by the session transport set for stateless backends),
/// defaulting to the legacy version. `Mcp-Method`/`Mcp-Name` mirror the
/// JSON-RPC body so 2026-07-28 servers and load balancers can route on the
/// operation without inspecting it; legacy servers ignore them.
fn mcp_request_headers(request: &JsonRpcRequest) -> Vec<(&'static str, String)> {
    let version = request
        .params
        .as_ref()
        .and_then(|p| p.get("_meta"))
        .and_then(|m| m.get(crate::protocol::meta_keys::PROTOCOL_VERSION))
        .and_then(|v| v.as_str())
        .unwrap_or(crate::protocol::MCP_PROTOCOL_VERSION)
        .to_string();

    let mut headers = vec![
        ("MCP-Protocol-Version", version),
        ("Mcp-Method", request.method.clone()),
    ];

    if let Some(name) = request
        .params
        .as_ref()
        .and_then(|p| p.get("name").or_else(|| p.get("uri")))
        .and_then(|v| v.as_str())
    {
        headers.push(("Mcp-Name", name.to_string()));
    }

    headers
}

impl SseTransport {
    /// Parse SSE response and extract JSON data
    ///
    /// SSE responses have the format:
    /// ```text
    /// event: message
    /// data: {"jsonrpc":"2.0",...}
    /// ```
    ///
    /// Also handles plain JSON responses (not wrapped in SSE format).
    fn parse_sse_response(sse_text: &str) -> AppResult<String> {
        let trimmed = sse_text.trim();

        // First, try to extract from SSE format
        for line in trimmed.lines() {
            let line = line.trim();
            if line.starts_with("data:") {
                // Extract JSON after "data: "
                let json_str = line.strip_prefix("data:").unwrap_or("").trim();
                if !json_str.is_empty() {
                    return Ok(json_str.to_string());
                }
            }
        }

        // Fallback: Check if it's plain JSON (starts with { or [)
        if trimmed.starts_with('{') || trimmed.starts_with('[') {
            // Validate it's actually JSON
            if serde_json::from_str::<Value>(trimmed).is_ok() {
                return Ok(trimmed.to_string());
            }
        }

        Err(AppError::Mcp(
            "No valid JSON found in response (expected SSE data: field or plain JSON)".to_string(),
        ))
    }

    /// Read a POST response incrementally; SSE may remain open after the result.
    async fn read_inline_response(
        &self,
        response: reqwest::Response,
    ) -> AppResult<Option<JsonRpcResponse>> {
        let is_sse = response
            .headers()
            .get(reqwest::header::CONTENT_TYPE)
            .and_then(|v| v.to_str().ok())
            .is_some_and(|v| v.starts_with("text/event-stream"));
        if !is_sse {
            let text = response
                .text()
                .await
                .map_err(|e| AppError::Mcp(format!("Failed to read MCP response: {e}")))?;
            if text.trim().is_empty() {
                return Ok(None);
            }
            // Legacy servers sometimes send SSE framing without an SSE content type.
            let json = Self::parse_sse_response(&text)?;
            return serde_json::from_str(&json)
                .map(Some)
                .map_err(|e| AppError::Mcp(format!("Invalid MCP JSON response: {e}")));
        }
        let mut stream = response.bytes_stream();
        let mut buffer = Vec::new();
        while let Some(chunk) = stream.next().await {
            buffer.extend_from_slice(
                &chunk.map_err(|e| AppError::Mcp(format!("MCP response stream failed: {e}")))?,
            );
            if buffer.len() > 16 * 1024 * 1024 {
                return Err(AppError::Mcp("MCP SSE event exceeds 16 MiB".into()));
            }
            loop {
                let boundary = buffer
                    .windows(2)
                    .position(|w| w == b"\n\n")
                    .map(|i| (i, 2))
                    .or_else(|| {
                        buffer
                            .windows(4)
                            .position(|w| w == b"\r\n\r\n")
                            .map(|i| (i, 4))
                    });
                let Some((end, delimiter_len)) = boundary else {
                    break;
                };
                let event = String::from_utf8(buffer.drain(..end + delimiter_len).collect())
                    .map_err(|e| AppError::Mcp(format!("Invalid SSE UTF-8: {e}")))?;
                let data = event
                    .lines()
                    .filter_map(|line| line.strip_prefix("data:").map(str::trim_start))
                    .collect::<Vec<_>>()
                    .join("\n");
                if data.is_empty() {
                    continue;
                }
                match serde_json::from_str::<crate::protocol::JsonRpcMessage>(&data)
                    .map_err(|e| AppError::Mcp(format!("Invalid MCP SSE message: {e}")))?
                {
                    crate::protocol::JsonRpcMessage::Response(response) => {
                        return Ok(Some(response))
                    }
                    crate::protocol::JsonRpcMessage::Notification(notification) => {
                        if let Some(callback) = self.notification_callback.read().clone() {
                            callback(notification);
                        }
                    }
                    crate::protocol::JsonRpcMessage::Request(request) => {
                        let callback = self.request_callback.read().clone();
                        if let Some(callback) = callback {
                            let response = callback(request).await;
                            let mut post = self
                                .client
                                .post(&self.url)
                                .json(&response)
                                .header("Accept", "application/json, text/event-stream");
                            for (key, value) in self.headers.read().clone() {
                                post = post.header(key, value);
                            }
                            if let Some(id) = self.session_id.read().clone() {
                                post = post.header("Mcp-Session-Id", id);
                            }
                            post.send()
                                .await
                                .map_err(|e| {
                                    AppError::Mcp(format!("Failed to respond to MCP request: {e}"))
                                })?
                                .error_for_status()
                                .map_err(|e| {
                                    AppError::Mcp(format!("MCP response rejected: {e}"))
                                })?;
                        }
                    }
                }
            }
        }
        Ok(None)
    }

    /// Parse SSE event and extract event type and data
    ///
    /// SSE events have the format:
    /// ```text
    /// event: endpoint
    /// data: /messages
    /// ```
    ///
    /// Returns (event_type, data) where either can be None if not present.
    fn parse_sse_event(sse_text: &str) -> (Option<String>, Option<String>) {
        let mut event_type = None;
        let mut data = None;

        for line in sse_text.lines() {
            let line = line.trim();
            if line.starts_with("event:") {
                event_type = Some(line.strip_prefix("event:").unwrap_or("").trim().to_string());
            } else if line.starts_with("data:") {
                data = Some(line.strip_prefix("data:").unwrap_or("").trim().to_string());
            }
        }

        (event_type, data)
    }

    /// Create a new SSE transport
    ///
    /// # Arguments
    /// * `url` - Base URL of the MCP server
    /// * `headers` - Custom headers to include in requests
    ///
    /// # Returns
    /// * The transport instance
    ///
    /// # Errors
    /// * Returns an error if the HTTP client cannot be created
    /// * Returns an error if the server is not reachable or returns an error status
    pub async fn connect(url: String, headers: HashMap<String, String>) -> AppResult<Self> {
        tracing::info!("Connecting to MCP SSE server: {}", url);

        // Use shared HTTP client with connection pooling
        let client = HTTP_CLIENT.clone();

        // Do NOT send initialize during transport connect.
        // The gateway broadcasts the real initialize (with actual client capabilities
        // and current protocol version) after all transports are connected.
        // This aligns SSE with how stdio/websocket transports work.

        let pending = Arc::new(RwLock::new(HashMap::new()));
        let closed = Arc::new(RwLock::new(false));
        let stream_ready = Arc::new(RwLock::new(false));
        let notification_callback = Arc::new(RwLock::new(None));
        let request_callback: Arc<RwLock<Option<crate::transport::RequestCallback>>> =
            Arc::new(RwLock::new(None));
        let message_endpoint = Arc::new(RwLock::new(None));
        let session_id = Arc::new(RwLock::new(None));
        let stream_session_id = session_id.clone();
        let headers: SharedHeaders = Arc::new(RwLock::new(headers));

        // Start persistent SSE stream in background
        let stream_url = url.clone();
        let stream_headers = headers.clone();
        let stream_pending = pending.clone();
        let stream_closed = closed.clone();
        let stream_ready_clone = stream_ready.clone();
        let stream_callback = notification_callback.clone();
        let stream_request_callback = request_callback.clone();
        let stream_client = STREAM_CLIENT.clone();
        let stream_message_endpoint = message_endpoint.clone();

        let mut stream_task = ConnectingStream(Some(tokio::spawn(async move {
            Self::sse_stream_task(
                stream_url,
                stream_headers,
                stream_client,
                stream_pending,
                stream_closed,
                stream_ready_clone,
                stream_callback,
                stream_request_callback,
                stream_message_endpoint,
                stream_session_id,
            )
            .await;
        })));

        // Wait for stream to be ready (with timeout)
        // This prevents race conditions where requests are sent before stream connects
        let ready_timeout = tokio::time::timeout(Duration::from_secs(10), async {
            loop {
                if *stream_ready.read() {
                    break;
                }
                if *closed.read() {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(50)).await;
            }
        })
        .await;

        // Check if the stream became ready
        let is_ready = *stream_ready.read();
        let is_closed = *closed.read();

        if is_closed && !is_ready {
            // Stream task gave up (exhausted reconnect attempts or permanent error)
            return Err(AppError::Mcp(format!(
                "Failed to connect to SSE server at {}: stream closed before becoming ready",
                url
            )));
        }

        if ready_timeout.is_err() && !is_ready {
            // Close the stream task since we're giving up
            *closed.write() = true;
            return Err(AppError::Mcp(format!(
                "Failed to connect to SSE server at {}: stream did not become ready within timeout",
                url
            )));
        }

        let transport = Self {
            url,
            message_endpoint,
            session_id,
            client,
            headers,
            auth_refresher: Arc::new(RwLock::new(None)),
            pending,
            next_id: Arc::new(RwLock::new(1)),
            closed,
            stream_ready,
            notification_callback,
            request_callback,
            stream_task: Arc::new(RwLock::new(stream_task.0.take())),
        };

        tracing::info!("MCP SSE transport connected successfully with persistent stream");

        Ok(transport)
    }

    /// Background task that maintains persistent SSE stream
    ///
    /// Reads from GET SSE endpoint and dispatches:
    /// - Endpoint events → message_endpoint for POST URL
    /// - Responses → pending request handlers
    /// - Notifications → notification callback
    ///
    /// Uses exponential backoff for reconnection with a maximum of 10 attempts.
    async fn sse_stream_task(
        url: String,
        headers: SharedHeaders,
        client: Client,
        pending: Arc<RwLock<HashMap<String, oneshot::Sender<JsonRpcResponse>>>>,
        closed: Arc<RwLock<bool>>,
        stream_ready: Arc<RwLock<bool>>,
        notification_callback: Arc<RwLock<Option<SseNotificationCallback>>>,
        request_callback: Arc<RwLock<Option<crate::transport::RequestCallback>>>,
        message_endpoint: Arc<RwLock<Option<String>>>,
        session_id: Arc<RwLock<Option<String>>>,
    ) {
        tracing::info!("Starting persistent SSE stream task for: {}", url);

        const MAX_RECONNECT_ATTEMPTS: u32 = 10;
        const BASE_DELAY_SECS: u64 = 1;
        const MAX_DELAY_SECS: u64 = 60;

        let mut reconnect_attempts = 0u32;
        let mut utf8_buffer = Vec::new(); // Buffer for incomplete UTF-8 sequences

        loop {
            // Check if closed
            if *closed.read() {
                tracing::info!("SSE stream task shutting down");
                break;
            }

            // Check reconnection limit
            if reconnect_attempts >= MAX_RECONNECT_ATTEMPTS {
                tracing::error!(
                    "SSE stream exceeded maximum reconnection attempts ({}), giving up",
                    MAX_RECONNECT_ATTEMPTS
                );
                break;
            }

            // Connect to GET SSE endpoint
            let mut request = client.get(&url);

            // Add headers
            for (key, value) in headers.read().clone() {
                request = request.header(key, value);
            }
            request = request.header("Accept", "text/event-stream");
            if let Some(id) = session_id.read().clone() {
                request = request.header("Mcp-Session-Id", id);
            }

            // Send request and get streaming response
            let response = match request.send().await {
                Ok(resp) => resp,
                Err(e) => {
                    reconnect_attempts += 1;
                    let delay = std::cmp::min(
                        BASE_DELAY_SECS * 2u64.saturating_pow(reconnect_attempts - 1),
                        MAX_DELAY_SECS,
                    );
                    tracing::warn!(
                        "Failed to connect to SSE stream (attempt {}/{}): {}. Retrying in {}s",
                        reconnect_attempts,
                        MAX_RECONNECT_ATTEMPTS,
                        e,
                        delay
                    );
                    tokio::time::sleep(Duration::from_secs(delay)).await;
                    continue;
                }
            };

            // Check status
            if !response.status().is_success() {
                let status = response.status();

                // 405 Method Not Allowed means the server doesn't support GET SSE streams
                // This is a permanent failure - the server only supports POST (inline responses)
                // Mark as ready anyway so POST requests work, then exit the SSE stream task
                if status == reqwest::StatusCode::METHOD_NOT_ALLOWED
                    || status == reqwest::StatusCode::BAD_REQUEST
                {
                    tracing::info!(
                        "Server at {} doesn't support GET SSE stream (405). Transport will use inline responses only. Server-initiated notifications won't be received.",
                        url
                    );
                    *stream_ready.write() = true;
                    break;
                }

                // 404 Not Found also means no SSE endpoint exists - don't retry
                if status == reqwest::StatusCode::NOT_FOUND {
                    tracing::info!(
                        "SSE endpoint not found at {} (404). Transport will use inline responses only.",
                        url
                    );
                    *stream_ready.write() = true;
                    break;
                }

                reconnect_attempts += 1;
                let delay = std::cmp::min(
                    BASE_DELAY_SECS * 2u64.saturating_pow(reconnect_attempts - 1),
                    MAX_DELAY_SECS,
                );
                tracing::warn!(
                    "SSE stream returned error status: {} (attempt {}/{}). Retrying in {}s",
                    status,
                    reconnect_attempts,
                    MAX_RECONNECT_ATTEMPTS,
                    delay
                );
                tokio::time::sleep(Duration::from_secs(delay)).await;
                continue;
            }

            // Reset reconnection counter on successful connection
            reconnect_attempts = 0;
            *stream_ready.write() = true;
            tracing::info!("Connected to persistent SSE stream");

            // Read SSE events
            let mut stream = response.bytes_stream();
            let mut buffer = String::new();
            // The server controls how much it sends before a blank line; cap
            // the pending-event buffer so a misbehaving server cannot grow
            // memory without bound (the inline POST path has the same cap).
            const MAX_PENDING_EVENT_BYTES: usize = 16 * 1024 * 1024;

            while let Some(chunk_result) = stream.next().await {
                // Check if closed
                if *closed.read() {
                    break;
                }

                match chunk_result {
                    Ok(chunk) => {
                        if buffer.len() + utf8_buffer.len() + chunk.len() > MAX_PENDING_EVENT_BYTES
                        {
                            tracing::error!(
                                "MCP SSE event from {} exceeds {} MiB without a terminator; dropping stream",
                                url,
                                MAX_PENDING_EVENT_BYTES / (1024 * 1024)
                            );
                            break;
                        }
                        // Proper UTF-8 handling: append chunk to byte buffer and decode
                        utf8_buffer.extend_from_slice(&chunk);

                        // Try to decode as much valid UTF-8 as possible
                        match String::from_utf8(utf8_buffer.clone()) {
                            Ok(text) => {
                                buffer.push_str(&text);
                                utf8_buffer.clear();
                            }
                            Err(e) => {
                                // Partial UTF-8 sequence at the end
                                let valid_up_to = e.utf8_error().valid_up_to();
                                if valid_up_to > 0 {
                                    // Decode the valid portion
                                    let valid_text =
                                        String::from_utf8_lossy(&utf8_buffer[..valid_up_to]);
                                    buffer.push_str(&valid_text);
                                    // Keep the incomplete sequence for next chunk
                                    utf8_buffer = utf8_buffer[valid_up_to..].to_vec();
                                }
                                // If valid_up_to is 0, we have an invalid sequence at the start
                                // Skip one byte and try again
                                if valid_up_to == 0 && !utf8_buffer.is_empty() {
                                    tracing::warn!("Invalid UTF-8 byte in SSE stream, skipping");
                                    utf8_buffer.remove(0);
                                }
                            }
                        }

                        // Process complete SSE events (separated by \n\n)
                        while let Some(event_end) = buffer.find("\n\n") {
                            let event_text = buffer[..event_end].to_string();
                            buffer = buffer[event_end + 2..].to_string();

                            // Parse SSE event type and data
                            let (event_type, event_data) = Self::parse_sse_event(&event_text);

                            // Handle "endpoint" event (MCP SSE transport spec)
                            if event_type.as_deref() == Some("endpoint") {
                                if let Some(endpoint_path) = event_data {
                                    // Resolve against the configured URL; only a
                                    // same-origin endpoint may receive our POSTs.
                                    match resolve_endpoint(&url, &endpoint_path) {
                                        Some(endpoint_url) => {
                                            tracing::info!(
                                                "Received MCP endpoint event: {} -> {}",
                                                endpoint_path,
                                                endpoint_url
                                            );
                                            *message_endpoint.write() = Some(endpoint_url);
                                        }
                                        None => {
                                            tracing::warn!(
                                                "Ignoring MCP endpoint event {:?}: not on the same origin as {}",
                                                endpoint_path,
                                                url
                                            );
                                        }
                                    }
                                }
                                continue;
                            }

                            // Parse SSE data as JSON
                            if let Ok(json_str) = Self::parse_sse_response(&event_text) {
                                // Try to parse as JSON-RPC message
                                if let Ok(message) =
                                    serde_json::from_str::<JsonRpcMessage>(&json_str)
                                {
                                    match message {
                                        JsonRpcMessage::Response(response) => {
                                            // Find pending request using normalized ID
                                            let id_str = Self::normalize_response_id(&response.id);
                                            let pending_keys: Vec<String> =
                                                pending.read().keys().cloned().collect();
                                            tracing::info!(
                                                "SSE transport received response: id={}, pending_keys={:?}",
                                                id_str,
                                                pending_keys
                                            );
                                            if let Some(sender) = pending.write().remove(&id_str) {
                                                if sender.send(response).is_err() {
                                                    tracing::warn!("Failed to send response to pending request: {}", id_str);
                                                }
                                            } else {
                                                tracing::warn!(
                                                    "Received response for unknown request ID: {} (pending_keys={:?})",
                                                    id_str,
                                                    pending_keys
                                                );
                                            }
                                        }
                                        JsonRpcMessage::Notification(notification) => {
                                            // Invoke notification callback
                                            if let Some(callback) =
                                                notification_callback.read().as_ref()
                                            {
                                                callback(notification);
                                            }
                                        }
                                        JsonRpcMessage::Request(request) => {
                                            // Handle server→client request (sampling, elicitation, etc.)
                                            tracing::info!(
                                                "Received server→client request: method={}, id={:?}",
                                                request.method,
                                                request.id
                                            );

                                            let callback = request_callback.read().clone();
                                            if let Some(callback) = callback {
                                                // Determine POST URL for sending response back
                                                let post_url = message_endpoint
                                                    .read()
                                                    .clone()
                                                    .unwrap_or_else(|| url.clone());
                                                let response_client = client.clone();
                                                let response_headers = headers.read().clone();
                                                let response_session = session_id.read().clone();

                                                tokio::spawn(async move {
                                                    let request_id = request.id.clone();
                                                    let response = callback(request).await;

                                                    // POST response back to the server
                                                    let mut req_builder = response_client
                                                        .post(&post_url)
                                                        .json(&response)
                                                        .header(
                                                            "Accept",
                                                            "application/json, text/event-stream",
                                                        );
                                                    for (key, value) in &response_headers {
                                                        req_builder =
                                                            req_builder.header(key, value);
                                                    }

                                                    if let Some(id) = response_session {
                                                        req_builder = req_builder
                                                            .header("Mcp-Session-Id", id);
                                                    }
                                                    match req_builder.send().await {
                                                        Ok(resp) => {
                                                            if !resp.status().is_success() {
                                                                tracing::warn!(
                                                                    "Failed to POST server→client response (status={}): request_id={:?}",
                                                                    resp.status(),
                                                                    request_id
                                                                );
                                                            } else {
                                                                tracing::debug!(
                                                                    "Sent server→client response: request_id={:?}",
                                                                    request_id
                                                                );
                                                            }
                                                        }
                                                        Err(e) => {
                                                            tracing::error!(
                                                                "Failed to POST server→client response: request_id={:?}, error={}",
                                                                request_id,
                                                                e
                                                            );
                                                        }
                                                    }
                                                });
                                            } else {
                                                tracing::debug!(
                                                    "Received server→client request but no callback set: method={}",
                                                    request.method
                                                );
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }
                    Err(e) => {
                        tracing::warn!("Error reading SSE stream: {}", e);
                        *stream_ready.write() = false;
                        break;
                    }
                }
            }

            // Connection lost - mark not ready and reconnect after delay
            *stream_ready.write() = false;
            reconnect_attempts += 1;
            let delay = std::cmp::min(
                BASE_DELAY_SECS * 2u64.saturating_pow(reconnect_attempts - 1),
                MAX_DELAY_SECS,
            );
            tracing::info!(
                "SSE stream connection lost, reconnecting in {}s (attempt {}/{})",
                delay,
                reconnect_attempts,
                MAX_RECONNECT_ATTEMPTS
            );
            tokio::time::sleep(Duration::from_secs(delay)).await;
        }

        tracing::info!("SSE stream task terminated");
    }

    /// Normalize response ID for pending map lookup
    ///
    /// Handles the case where server returns `id: null` by converting to a special key.
    /// For other values, converts to string representation.
    fn normalize_response_id(id: &Value) -> String {
        match id {
            Value::Null => "__null_id__".to_string(),
            Value::Number(n) => n.to_string(),
            Value::String(s) => format!("\"{}\"", s),
            _ => id.to_string(),
        }
    }

    /// Set a notification callback
    ///
    /// # Arguments
    /// * `callback` - The callback to invoke when notifications are received
    ///
    /// Note: SSE notifications require persistent streaming (not yet implemented)
    pub fn set_notification_callback(&self, callback: SseNotificationCallback) {
        *self.notification_callback.write() = Some(callback);
    }

    /// Set a request callback for server-initiated requests (sampling, elicitation, etc.)
    ///
    /// # Arguments
    /// * `callback` - The callback to invoke when requests are received from the server
    pub fn set_request_callback(&self, callback: crate::transport::RequestCallback) {
        *self.request_callback.write() = Some(callback);
    }

    /// Generate the next request ID
    fn next_request_id(&self) -> u64 {
        let mut next_id = self.next_id.write();
        let id = *next_id;
        *next_id += 1;
        id
    }

    /// Install the callback used to refresh auth headers after a 401.
    pub fn set_auth_refresher(&self, refresher: AuthRefresher) {
        *self.auth_refresher.write() = Some(refresher);
    }

    /// Run the auth refresher, merging the returned headers. Returns whether
    /// the request should be retried.
    async fn refresh_auth(&self) -> bool {
        let Some(refresher) = self.auth_refresher.read().clone() else {
            return false;
        };
        match refresher().await {
            Ok(new_headers) => {
                tracing::info!("SSE transport: refreshed auth after 401 for {}", self.url);
                self.headers.write().extend(new_headers);
                true
            }
            Err(e) => {
                tracing::warn!("SSE transport: auth refresh for {} failed: {}", self.url, e);
                false
            }
        }
    }

    /// Check if the transport is healthy
    pub fn is_healthy(&self) -> bool {
        !*self.closed.read()
    }

    /// Close the transport
    pub async fn disconnect(&self) -> AppResult<()> {
        tracing::info!("Disconnecting MCP SSE transport");
        *self.closed.write() = true;

        // Stop the SSE stream task
        if let Some(task) = self.stream_task.write().take() {
            task.abort();
            tracing::debug!("SSE stream task aborted");
        }

        Ok(())
    }
}

impl Drop for SseTransport {
    fn drop(&mut self) {
        *self.closed.write() = true;
        if let Some(task) = self.stream_task.write().take() {
            task.abort();
        }
    }
}

#[async_trait]
impl Transport for SseTransport {
    async fn send_request(&self, mut request: JsonRpcRequest) -> AppResult<JsonRpcResponse> {
        if *self.closed.read() {
            return Err(AppError::Mcp("Transport is closed".to_string()));
        }

        // Check if this is a notification (no ID, starts with "notifications/")
        // Notifications are fire-and-forget - no response expected
        let is_notification = request.id.is_none() && request.method.starts_with("notifications/");

        if is_notification {
            // For notifications: send without waiting for response
            tracing::debug!("SSE sending notification: {}", request.method);

            // Determine POST URL: use message_endpoint if available, otherwise fall back to base url
            let post_url = self
                .message_endpoint
                .read()
                .clone()
                .unwrap_or_else(|| self.url.clone());

            // Build POST request for notification (no ID added)
            let mut req_builder = self.client.post(&post_url).json(&request);
            if let Some(id) = self.session_id.read().clone() {
                req_builder = req_builder.header("Mcp-Session-Id", id);
            }
            req_builder = req_builder.header("Accept", "application/json, text/event-stream");
            for (key, value) in mcp_request_headers(&request) {
                req_builder = req_builder.header(key, value);
            }
            for (key, value) in self.headers.read().clone() {
                req_builder = req_builder.header(key, value);
            }

            // Send POST request (fire and forget)
            let post_response = req_builder
                .send()
                .await
                .map_err(|e| AppError::Mcp(format!("Failed to send notification: {}", e)))?;

            // Check status but don't wait for response body
            if !post_response.status().is_success() {
                tracing::warn!(
                    "SSE notification POST returned non-success status: {}",
                    post_response.status()
                );
            }

            // Return empty success response for notifications
            return Ok(JsonRpcResponse {
                jsonrpc: "2.0".to_string(),
                id: Value::Null,
                result: Some(Value::Null),
                error: None,
            });
        }

        // For regular requests: assign ID and wait for response

        // Store the original request ID to restore in response
        let original_request_id = request.id.clone();

        // Generate unique internal request ID for tracking pending requests
        let request_id = {
            let id = self.next_request_id();
            request.id = Some(Value::Number(id.into()));
            id.to_string()
        };

        tracing::info!(
            "SSE transport send_request: method={}, internal_id={}, original_id={:?}",
            request.method,
            request_id,
            original_request_id
        );

        // Create oneshot channel for response
        let (tx, rx) = oneshot::channel();

        // Register pending request
        self.pending.write().insert(request_id.clone(), tx);
        let _pending_guard = super::PendingRequestGuard::new(&self.pending, request_id.clone());

        // Determine POST URL: use message_endpoint if available, otherwise fall back to base url
        let post_url = self
            .message_endpoint
            .read()
            .clone()
            .unwrap_or_else(|| self.url.clone());

        // Send the POST; on 401 refresh auth once and retry (expired OAuth token)
        let mut auth_refreshed = false;
        let post_response = loop {
            let mut req_builder = self.client.post(&post_url).json(&request);
            if let Some(id) = self.session_id.read().clone() {
                req_builder = req_builder.header("Mcp-Session-Id", id);
            }

            // Add Accept header for content negotiation
            req_builder = req_builder.header("Accept", "application/json, text/event-stream");

            // Add standard MCP request headers (SEP-2243)
            for (key, value) in mcp_request_headers(&request) {
                req_builder = req_builder.header(key, value);
            }

            // Add custom headers
            let headers = self.headers.read().clone();
            tracing::debug!(
                "SSE POST request: url={}, method={}, header_names={:?}",
                post_url,
                request.method,
                headers.keys().collect::<Vec<_>>()
            );
            for (key, value) in headers {
                req_builder = req_builder.header(key, value);
            }

            let response = req_builder.send().await.map_err(|e| {
                // Remove from pending on error
                self.pending.write().remove(&request_id);
                AppError::Mcp(format!("Failed to send request: {}", e))
            })?;

            if response.status() == reqwest::StatusCode::UNAUTHORIZED
                && !auth_refreshed
                && self.refresh_auth().await
            {
                auth_refreshed = true;
                continue;
            }
            break response;
        };

        // Check POST status (should be 202 Accepted or 200 OK)
        if !post_response.status().is_success() {
            self.pending.write().remove(&request_id);
            let status = post_response.status();
            let headers = post_response.headers().clone();
            let body = post_response.text().await.unwrap_or_default();
            tracing::error!(
                "SSE POST request failed: status={}, url={}, method={}, header_names={:?}, body={}",
                status,
                post_url,
                request.method,
                headers.keys().collect::<Vec<_>>(),
                body
            );
            if let Ok(mut response) = serde_json::from_str::<JsonRpcResponse>(&body) {
                if response.error.is_some() {
                    response.id = original_request_id.clone().unwrap_or(Value::Null);
                    return Ok(response);
                }
            }
            return Err(AppError::Mcp(format!(
                "Server returned error status: {} - {}",
                status,
                if body.is_empty() {
                    "no body".to_string()
                } else {
                    body
                }
            )));
        }

        if request.method == "initialize" {
            if let Some(id) = post_response.headers().get("Mcp-Session-Id") {
                *self.session_id.write() = Some(
                    id.to_str()
                        .map_err(|e| AppError::Mcp(format!("Invalid MCP session ID: {e}")))?
                        .to_string(),
                );
            }
        }

        if let Some(mut response) = self.read_inline_response(post_response).await? {
            response.id = original_request_id.clone().unwrap_or(Value::Null);
            return Ok(response);
        }

        // No inline response - wait for response from SSE stream (with timeout)
        tracing::info!(
            "SSE transport waiting for SSE stream response (internal_id={}, timeout=30s)",
            request_id
        );

        let mut response = tokio::time::timeout(Duration::from_secs(30), rx)
            .await
            .map_err(|_| {
                self.pending.write().remove(&request_id);
                tracing::error!(
                    "SSE transport timeout waiting for response (internal_id={})",
                    request_id
                );
                AppError::Mcp("Request timeout waiting for response".to_string())
            })?
            .map_err(|e| {
                tracing::error!(
                    "SSE transport response channel error (internal_id={}): {}",
                    request_id,
                    e
                );
                AppError::Mcp("Response channel closed".to_string())
            })?;

        // Restore original request ID in response
        response.id = original_request_id.unwrap_or(Value::Null);
        tracing::info!(
            "SSE transport received response via SSE stream (internal_id={}, restored_id={:?})",
            request_id,
            response.id
        );
        Ok(response)
    }

    async fn stream_request(
        &self,
        mut request: JsonRpcRequest,
    ) -> AppResult<Pin<Box<dyn Stream<Item = AppResult<StreamingChunk>> + Send>>> {
        if *self.closed.read() {
            return Err(AppError::Mcp("Transport is closed".to_string()));
        }

        // Generate unique request ID
        let request_id = {
            let id = self.next_request_id();
            request.id = Some(Value::Number(id.into()));
            id
        };

        // Add streaming parameter to request
        if let Some(params) = request.params.as_mut() {
            if let Some(obj) = params.as_object_mut() {
                obj.insert("stream".to_string(), serde_json::json!(true));
            }
        } else {
            request.params = Some(serde_json::json!({"stream": true}));
        }

        // Build POST request
        let mut req_builder = self.client.post(&self.url).json(&request);
        if let Some(id) = self.session_id.read().clone() {
            req_builder = req_builder.header("Mcp-Session-Id", id);
        }

        // Add standard MCP request headers (SEP-2243)
        for (key, value) in mcp_request_headers(&request) {
            req_builder = req_builder.header(key, value);
        }

        // Add headers
        for (key, value) in self.headers.read().clone() {
            req_builder = req_builder.header(key, value);
        }
        req_builder = req_builder.header("Accept", "text/event-stream");

        // Send POST request
        let response = req_builder
            .send()
            .await
            .map_err(|e| AppError::Mcp(format!("Failed to send streaming request: {}", e)))?;

        // Check status
        if !response.status().is_success() {
            return Err(AppError::Mcp(format!(
                "Server returned error status: {}",
                response.status()
            )));
        }

        // Create async stream from response
        let mut byte_stream = response.bytes_stream();
        let mut buffer = String::new();
        let mut utf8_buffer = Vec::new(); // Buffer for incomplete UTF-8 sequences
        let mut chunk_index = 0u32;

        let stream = async_stream::stream! {
            loop {
                match byte_stream.next().await {
                    Some(Ok(chunk)) => {
                        // Proper UTF-8 handling: append chunk to byte buffer and decode
                        utf8_buffer.extend_from_slice(&chunk);

                        // Try to decode as much valid UTF-8 as possible
                        match String::from_utf8(utf8_buffer.clone()) {
                            Ok(text) => {
                                buffer.push_str(&text);
                                utf8_buffer.clear();
                            }
                            Err(e) => {
                                // Partial UTF-8 sequence at the end
                                let valid_up_to = e.utf8_error().valid_up_to();
                                if valid_up_to > 0 {
                                    let valid_text = String::from_utf8_lossy(&utf8_buffer[..valid_up_to]);
                                    buffer.push_str(&valid_text);
                                    utf8_buffer = utf8_buffer[valid_up_to..].to_vec();
                                }
                                if valid_up_to == 0 && !utf8_buffer.is_empty() {
                                    tracing::warn!("Invalid UTF-8 byte in streaming response, skipping");
                                    utf8_buffer.remove(0);
                                }
                            }
                        }

                        // Process complete SSE events
                        while let Some(event_end) = buffer.find("\n\n") {
                            let event_text = buffer[..event_end].to_string();
                            buffer = buffer[event_end + 2..].to_string();

                            // Parse SSE event
                            if let Ok(json_str) = Self::parse_sse_response(&event_text) {
                                // Try to parse as StreamingChunk
                                if let Ok(chunk) = serde_json::from_str::<StreamingChunk>(&json_str) {
                                    let is_final = chunk.is_final;
                                    yield Ok(chunk);

                                    if is_final {
                                        return;
                                    }
                                    chunk_index += 1;
                                } else {
                                    // Fallback: wrap as chunk
                                    let data: Value = serde_json::from_str(&json_str).unwrap_or(serde_json::json!(null));
                                    yield Ok(StreamingChunk::new(
                                        Value::Number(request_id.into()),
                                        chunk_index,
                                        false,
                                        data,
                                    ));
                                    chunk_index += 1;
                                }
                            }
                        }
                    }
                    Some(Err(e)) => {
                        yield Err(AppError::Mcp(format!("Stream error: {}", e)));
                        return;
                    }
                    None => {
                        // Stream ended
                        return;
                    }
                }
            }
        };

        Ok(Box::pin(stream))
    }

    fn set_protocol_revision(&self, revision: crate::protocol::ProtocolRevision) {
        if !revision.is_stateless() {
            return;
        }
        // Modern MCP uses an explicitly filtered POST stream instead of GET.
        if let Some(task) = self.stream_task.write().take() {
            task.abort();
        }
        *self.session_id.write() = None;
        let url = self.url.clone();
        let headers = self.headers.read().clone();
        let callback = self.notification_callback.clone();
        let closed = self.closed.clone();
        let next_id = self.next_id.clone();
        let task = tokio::spawn(async move {
            loop {
                if *closed.read() {
                    return;
                }
                let id = {
                    let mut id = next_id.write();
                    let result = *id;
                    *id += 1;
                    result
                };
                let request = JsonRpcRequest::with_id(
                    id,
                    "subscriptions/listen".into(),
                    Some(serde_json::json!({
                        "notifications": { "toolsListChanged": true, "promptsListChanged": true, "resourcesListChanged": true },
                        "_meta": {
                            crate::protocol::meta_keys::PROTOCOL_VERSION: crate::protocol::MCP_PROTOCOL_VERSION_STATELESS,
                            crate::protocol::meta_keys::CLIENT_CAPABILITIES: {},
                            crate::protocol::meta_keys::CLIENT_INFO: { "name": "LocalRouter MCP Gateway", "version": env!("CARGO_PKG_VERSION") }
                        }
                    })),
                );
                let mut post = STREAM_CLIENT
                    .post(&url)
                    .json(&request)
                    .header("Accept", "application/json, text/event-stream");
                for (key, value) in &headers {
                    post = post.header(key, value);
                }
                for (key, value) in mcp_request_headers(&request) {
                    post = post.header(key, value);
                }
                let response = match post.send().await {
                    Ok(response) if response.status().is_success() => response,
                    Ok(response) if response.status().is_client_error() => return, // unsupported or auth rejected
                    _ => {
                        tokio::time::sleep(Duration::from_secs(2)).await;
                        continue;
                    }
                };
                if !response
                    .headers()
                    .get(reqwest::header::CONTENT_TYPE)
                    .and_then(|v| v.to_str().ok())
                    .is_some_and(|v| v.starts_with("text/event-stream"))
                {
                    return;
                }
                let mut stream = response.bytes_stream();
                let mut buffer = Vec::new();
                while let Some(Ok(chunk)) = stream.next().await {
                    buffer.extend_from_slice(&chunk);
                    if buffer.len() > 16 * 1024 * 1024 {
                        return;
                    }
                    loop {
                        let boundary = buffer
                            .windows(2)
                            .position(|w| w == b"\n\n")
                            .map(|i| (i, 2))
                            .or_else(|| {
                                buffer
                                    .windows(4)
                                    .position(|w| w == b"\r\n\r\n")
                                    .map(|i| (i, 4))
                            });
                        let Some((end, size)) = boundary else {
                            break;
                        };
                        let event = String::from_utf8_lossy(
                            &buffer.drain(..end + size).collect::<Vec<_>>(),
                        )
                        .to_string();
                        let data = event
                            .lines()
                            .filter_map(|line| line.strip_prefix("data:").map(str::trim_start))
                            .collect::<Vec<_>>()
                            .join("\n");
                        if let Ok(crate::protocol::JsonRpcMessage::Notification(notification)) =
                            serde_json::from_str(&data)
                        {
                            if let Some(callback) = callback.read().clone() {
                                callback(notification);
                            }
                        }
                    }
                }
                tokio::time::sleep(Duration::from_secs(2)).await;
            }
        });
        *self.stream_task.write() = Some(task);
    }

    fn supports_streaming(&self) -> bool {
        true
    }

    async fn is_healthy(&self) -> bool {
        self.is_healthy()
    }

    async fn close(&self) -> AppResult<()> {
        self.disconnect().await
    }

    fn set_notification_callback(&self, callback: super::NotificationCallback) {
        *self.notification_callback.write() = Some(callback);
    }

    fn set_request_callback(&self, callback: super::RequestCallback) {
        *self.request_callback.write() = Some(callback);
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn endpoint_event_must_stay_on_the_servers_origin() {
        let base = "https://mcp.example.com:8443/sse";
        assert_eq!(
            super::resolve_endpoint(base, "/messages?sessionId=abc").as_deref(),
            Some("https://mcp.example.com:8443/messages?sessionId=abc")
        );
        assert_eq!(
            super::resolve_endpoint(base, "messages").as_deref(),
            Some("https://mcp.example.com:8443/messages")
        );
        assert_eq!(
            super::resolve_endpoint(base, "https://mcp.example.com:8443/other").as_deref(),
            Some("https://mcp.example.com:8443/other")
        );
        // Different host, scheme or port: credentials must not go there.
        assert!(super::resolve_endpoint(base, "https://evil.example.com/messages").is_none());
        assert!(super::resolve_endpoint(base, "http://mcp.example.com:8443/messages").is_none());
        assert!(super::resolve_endpoint(base, "https://mcp.example.com/messages").is_none());
        assert!(super::resolve_endpoint(base, "http://127.0.0.1:9/admin").is_none());
        assert!(super::resolve_endpoint(base, "//evil.example.com/messages").is_none());
        assert!(super::resolve_endpoint("not a url", "/messages").is_none());
    }

    use super::*;
    use serde_json::json;

    /// Server that accepts only `Bearer fresh`, like an OAuth resource
    /// server after the access token expired.
    async fn spawn_token_checking_server() -> (String, Arc<std::sync::atomic::AtomicUsize>) {
        use axum::{http::HeaderMap, response::IntoResponse, routing::post, Json, Router};
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}/mcp", listener.local_addr().unwrap());
        let rejected = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let rejected_count = rejected.clone();
        let app = Router::new().route(
            "/mcp",
            post(
                move |headers: HeaderMap, Json(request): Json<JsonRpcRequest>| {
                    let rejected_count = rejected_count.clone();
                    async move {
                        let auth = headers
                            .get("Authorization")
                            .and_then(|v| v.to_str().ok())
                            .unwrap_or_default();
                        if auth != "Bearer fresh" {
                            rejected_count.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                            return axum::http::StatusCode::UNAUTHORIZED.into_response();
                        }
                        Json(json!({"jsonrpc": "2.0", "id": request.id, "result": {"tools": []}}))
                            .into_response()
                    }
                },
            ),
        );
        tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });
        (url, rejected)
    }

    #[tokio::test]
    async fn unauthorized_request_refreshes_auth_and_retries_once() {
        let (url, rejected) = spawn_token_checking_server().await;
        let headers = HashMap::from([("Authorization".to_string(), "Bearer stale".to_string())]);
        let transport = SseTransport::connect(url, headers).await.unwrap();
        let refreshes = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let refresh_count = refreshes.clone();
        transport.set_auth_refresher(Arc::new(move || {
            refresh_count.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            Box::pin(async {
                Ok(HashMap::from([(
                    "Authorization".to_string(),
                    "Bearer fresh".to_string(),
                )]))
            })
        }));

        let response = transport
            .send_request(JsonRpcRequest::with_id(1, "tools/list".into(), None))
            .await
            .unwrap();
        assert!(response.error.is_none());
        assert_eq!(refreshes.load(std::sync::atomic::Ordering::SeqCst), 1);
        assert_eq!(rejected.load(std::sync::atomic::Ordering::SeqCst), 1);

        // The refreshed header sticks: no further refresh or rejection
        transport
            .send_request(JsonRpcRequest::with_id(2, "tools/list".into(), None))
            .await
            .unwrap();
        assert_eq!(refreshes.load(std::sync::atomic::Ordering::SeqCst), 1);
        assert_eq!(rejected.load(std::sync::atomic::Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn unauthorized_without_refresher_fails() {
        let (url, _rejected) = spawn_token_checking_server().await;
        let headers = HashMap::from([("Authorization".to_string(), "Bearer stale".to_string())]);
        let transport = SseTransport::connect(url, headers).await.unwrap();
        let err = transport
            .send_request(JsonRpcRequest::with_id(1, "tools/list".into(), None))
            .await
            .unwrap_err();
        assert!(err.to_string().contains("401"), "{err}");
    }

    #[tokio::test]
    async fn failed_refresh_does_not_loop() {
        let (url, rejected) = spawn_token_checking_server().await;
        let transport = SseTransport::connect(url, HashMap::new()).await.unwrap();
        transport.set_auth_refresher(Arc::new(|| {
            Box::pin(async {
                Ok(HashMap::from([(
                    "Authorization".to_string(),
                    "Bearer still-bad".to_string(),
                )]))
            })
        }));
        assert!(transport
            .send_request(JsonRpcRequest::with_id(1, "tools/list".into(), None))
            .await
            .is_err());
        // Original attempt + exactly one retry
        assert_eq!(rejected.load(std::sync::atomic::Ordering::SeqCst), 2);
    }

    #[tokio::test]
    async fn legacy_http_preserves_session_and_query_and_reads_open_sse() {
        use axum::{
            extract::Query, http::HeaderMap, response::IntoResponse, routing::post, Json, Router,
        };
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!(
            "http://{}/v2/mcp?toolsets=ddsql",
            listener.local_addr().unwrap()
        );
        let app = Router::new().route("/v2/mcp", post(
            |Query(query): Query<HashMap<String, String>>, headers: HeaderMap, Json(request): Json<JsonRpcRequest>| async move {
                assert_eq!(query.get("toolsets").map(String::as_str), Some("ddsql"));
                if request.method == "initialize" {
                    return ([("Mcp-Session-Id", "session-123")], Json(serde_json::json!({
                        "jsonrpc":"2.0", "id":request.id, "result": {"protocolVersion":"2025-11-25"}
                    }))).into_response();
                }
                assert_eq!(headers.get("Mcp-Session-Id").unwrap(), "session-123");
                if request.method == "notifications/initialized" {
                    return axum::http::StatusCode::ACCEPTED.into_response();
                }
                let stream = async_stream::stream! {
                    yield Ok::<_, std::convert::Infallible>("event: message\r\ndata: {\"jsonrpc\":\"2.0\",\"method\":\"notifications/progress\",\"params\":{}}\r\n\r\n".to_string());
                    yield Ok(format!("event: message\r\ndata: {{\"jsonrpc\":\"2.0\",\"id\":{},\"result\":{{\"content\":[]}}}}\r\n\r\n", request.id.unwrap()));
                    std::future::pending::<()>().await;
                };
                ([("Content-Type", "text/event-stream")], axum::body::Body::from_stream(stream)).into_response()
            }
        ));
        let task = tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });
        let transport = SseTransport::connect(url, HashMap::new()).await.unwrap();
        let notifications = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let received = notifications.clone();
        transport.set_notification_callback(Arc::new(move |_| {
            received.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        }));
        transport
            .send_request(JsonRpcRequest::with_id(
                10,
                "initialize".into(),
                Some(json!({})),
            ))
            .await
            .unwrap();
        transport
            .send_request(JsonRpcRequest {
                jsonrpc: "2.0".into(),
                id: None,
                method: "notifications/initialized".into(),
                params: None,
            })
            .await
            .unwrap();
        let result = tokio::time::timeout(
            Duration::from_secs(2),
            transport.send_request(JsonRpcRequest::with_id(
                42,
                "tools/call".into(),
                Some(json!({"name":"test"})),
            )),
        )
        .await
        .expect("return final result without waiting for stream closure")
        .unwrap();
        assert_eq!(result.id, json!(42));
        assert_eq!(notifications.load(std::sync::atomic::Ordering::SeqCst), 1);
        transport.disconnect().await.unwrap();
        task.abort();
    }

    #[tokio::test]
    async fn modern_http_preserves_errors_and_receives_post_subscriptions() {
        use axum::{
            response::IntoResponse,
            routing::{get, post},
            Json, Router,
        };
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}/mcp", listener.local_addr().unwrap());
        let app = Router::new().route("/mcp", get(|| async { axum::http::StatusCode::BAD_REQUEST }).merge(post(
            |Json(request): Json<JsonRpcRequest>| async move {
                if request.method == "subscriptions/listen" {
                    assert_eq!(request.params.as_ref().unwrap()["notifications"]["toolsListChanged"], true);
                    let stream = async_stream::stream! {
                        yield Ok::<_, std::convert::Infallible>("data: {\"jsonrpc\":\"2.0\",\"method\":\"notifications/subscriptions/acknowledged\",\"params\":{}}\n\n");
                        yield Ok("data: {\"jsonrpc\":\"2.0\",\"method\":\"notifications/tools/list_changed\",\"params\":{}}\n\n");
                        std::future::pending::<()>().await;
                    };
                    return ([("Content-Type", "text/event-stream")], axum::body::Body::from_stream(stream)).into_response();
                }
                (axum::http::StatusCode::BAD_REQUEST, Json(json!({
                    "jsonrpc":"2.0", "id":request.id,
                    "error":{"code":-32022,"message":"Unsupported protocol version","data":{"supported":["2026-07-28"]}}
                }))).into_response()
            }
        )));
        let task = tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });
        let transport = SseTransport::connect(url, HashMap::new()).await.unwrap();
        let response = transport
            .send_request(JsonRpcRequest::with_id(
                42,
                "server/discover".into(),
                Some(json!({})),
            ))
            .await
            .unwrap();
        assert_eq!(response.id, json!(42));
        assert_eq!(response.error.unwrap().code, -32022);
        let received = Arc::new(tokio::sync::Notify::new());
        let signal = received.clone();
        transport.set_notification_callback(Arc::new(move |notification| {
            if notification.method == "notifications/tools/list_changed" {
                signal.notify_one();
            }
        }));
        transport.set_protocol_revision(crate::protocol::ProtocolRevision::V2026_07_28);
        tokio::time::timeout(Duration::from_secs(2), received.notified())
            .await
            .unwrap();
        transport.disconnect().await.unwrap();
        task.abort();
    }

    #[tokio::test]
    async fn test_request_id_generation() {
        let transport = SseTransport {
            url: "http://localhost:3000".to_string(),
            message_endpoint: Arc::new(RwLock::new(None)),
            session_id: Arc::new(RwLock::new(None)),
            client: Client::new(),
            headers: Arc::new(RwLock::new(HashMap::new())),
            auth_refresher: Arc::new(RwLock::new(None)),
            pending: Arc::new(RwLock::new(HashMap::new())),
            next_id: Arc::new(RwLock::new(1)),
            closed: Arc::new(RwLock::new(false)),
            stream_ready: Arc::new(RwLock::new(false)),
            notification_callback: Arc::new(RwLock::new(None)),
            request_callback: Arc::new(RwLock::new(None)),
            stream_task: Arc::new(RwLock::new(None)),
        };

        assert_eq!(transport.next_request_id(), 1);
        assert_eq!(transport.next_request_id(), 2);
        assert_eq!(transport.next_request_id(), 3);
    }

    #[test]
    fn test_parse_sse_response_plain_json() {
        // Test plain JSON (not wrapped in SSE format)
        let plain_json = r#"{"jsonrpc":"2.0","id":1,"result":{}}"#;
        let result = SseTransport::parse_sse_response(plain_json);
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), plain_json);
    }

    #[test]
    fn test_parse_sse_response_sse_format() {
        // Test SSE format
        let sse_text = "event: message\ndata: {\"jsonrpc\":\"2.0\",\"id\":1,\"result\":{}}\n\n";
        let result = SseTransport::parse_sse_response(sse_text);
        assert!(result.is_ok());
        assert!(result.unwrap().contains("jsonrpc"));
    }

    #[test]
    fn test_normalize_response_id() {
        // Test null ID
        assert_eq!(
            SseTransport::normalize_response_id(&Value::Null),
            "__null_id__"
        );

        // Test numeric ID
        assert_eq!(SseTransport::normalize_response_id(&json!(42)), "42");

        // Test string ID
        assert_eq!(
            SseTransport::normalize_response_id(&json!("abc")),
            "\"abc\""
        );
    }

    #[test]
    fn test_json_rpc_request_serialization() {
        let request = JsonRpcRequest {
            jsonrpc: "2.0".to_string(),
            id: Some(json!(1)),
            method: "test_method".to_string(),
            params: Some(json!({"key": "value"})),
        };

        let json = serde_json::to_string(&request).unwrap();
        assert!(json.contains("\"jsonrpc\":\"2.0\""));
        assert!(json.contains("\"method\":\"test_method\""));
    }
}
