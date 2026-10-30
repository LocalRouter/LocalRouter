//! STDIO transport for MCP
//!
//! Spawns a subprocess and communicates via stdin/stdout using JSON-RPC 2.0.
//! This is the most common transport type for MCP servers.

use crate::protocol::{JsonRpcMessage, JsonRpcNotification, JsonRpcRequest, JsonRpcResponse};
use crate::transport::rpc_stream::{self, RpcStream, StreamRegistry};
use crate::transport::Transport;
use async_trait::async_trait;
use lr_types::{AppError, AppResult};
use lr_utils::sandbox;
use parking_lot::RwLock;
use serde_json::Value;
use std::collections::HashMap;
use std::process::Stdio;
use std::sync::Arc;
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::process::{Child, ChildStdin, ChildStdout, Command};
use tokio::sync::{oneshot, Mutex};
use tokio::task::JoinHandle;

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

/// Notification callback type for STDIO transport
pub type StdioNotificationCallback = Arc<dyn Fn(JsonRpcNotification) + Send + Sync>;

/// Request callback type for STDIO transport (for server-initiated requests like sampling/elicitation)
/// Returns a future that resolves to the response
pub type StdioRequestCallback = Arc<
    dyn Fn(
            JsonRpcRequest,
        ) -> std::pin::Pin<Box<dyn std::future::Future<Output = JsonRpcResponse> + Send>>
        + Send
        + Sync,
>;

/// Largest single JSON-RPC line accepted from a child process (16 MiB).
const MAX_STDIO_LINE_BYTES: usize = 16 * 1024 * 1024;

/// STDIO transport implementation
///
/// Manages a subprocess with JSON-RPC communication over stdin/stdout.
/// Supports concurrent requests with request/response correlation.
/// Supports notification handling for server-initiated messages.
pub struct StdioTransport {
    /// Child process
    child: Arc<RwLock<Option<Child>>>,

    /// Stdin handle for sending requests
    /// Uses Mutex instead of RwLock to support concurrent writes safely
    stdin: Arc<Mutex<Option<ChildStdin>>>,

    /// Pending requests waiting for responses
    /// Maps request ID to response sender
    pending: Arc<RwLock<HashMap<String, oneshot::Sender<JsonRpcResponse>>>>,

    /// Next request ID
    next_id: Arc<RwLock<u64>>,

    /// Whether the transport is closed
    closed: Arc<RwLock<bool>>,

    /// Notification callback (optional)
    notification_callback: Arc<RwLock<Option<StdioNotificationCallback>>>,

    /// Request callback for server-initiated requests like sampling/elicitation (optional)
    request_callback: Arc<RwLock<Option<StdioRequestCallback>>>,

    /// Background reader task handle (for cancellation)
    reader_task: Arc<RwLock<Option<JoinHandle<()>>>>,

    /// Open long-lived requests (see [`Transport::open_rpc_stream`])
    streams: StreamRegistry,
}

impl StdioTransport {
    /// Spawn a new MCP server process with STDIO transport
    ///
    /// # Arguments
    /// * `command` - The command to execute (e.g., "npx")
    /// * `args` - Command arguments (e.g., ["-y", "@modelcontextprotocol/server-everything"])
    /// * `env` - Environment variables to set
    ///
    /// The process inherits LocalRouter's own working directory. Prefer
    /// [`Self::spawn_in`] for configured servers, which pins an explicit one.
    ///
    /// # Returns
    /// * The transport instance with the running process
    pub async fn spawn(
        command: String,
        args: Vec<String>,
        env: HashMap<String, String>,
    ) -> AppResult<Self> {
        Self::spawn_in(command, args, env, None).await
    }

    /// Spawn a new MCP server process in an explicit working directory.
    ///
    /// `cwd` of `None` inherits LocalRouter's working directory, which on a GUI
    /// launch is `/` — callers with a config should pass
    /// `McpTransportConfig::resolve_stdio_cwd()` instead.
    pub async fn spawn_in(
        command: String,
        args: Vec<String>,
        env: HashMap<String, String>,
        cwd: Option<std::path::PathBuf>,
    ) -> AppResult<Self> {
        tracing::debug!(
            "Spawning MCP STDIO process: {} {:?} (cwd: {:?})",
            command,
            args,
            cwd
        );

        // Fail with a directed message rather than the opaque OS error the
        // spawn would otherwise produce for a missing/!dir working directory.
        //
        // Skipped when proxying to the host: inside a Flatpak sandbox the path
        // is resolved on the *host*, and a directory outside our exported
        // filesystem is invisible here even though the host process can enter
        // it. Let the host report the failure instead of rejecting it wrongly.
        if let Some(dir) = &cwd {
            if !sandbox::current().needs_host_proxy() && !dir.is_dir() {
                return Err(AppError::Mcp(format!(
                    "Working directory for MCP process '{}' is not an existing directory: {}",
                    command,
                    dir.display()
                )));
            }
        }

        // Assemble the final environment before handing it to the sandbox
        // helper: under Flatpak these become `--env=` flags on
        // `flatpak-spawn`, so they have to be complete by then.
        let mut env = env;
        if let Some(dir) = &cwd {
            // The child inherits our environment, so a stale inherited `PWD`
            // would disagree with the directory we just set — shells and
            // scripts read `$PWD` rather than calling getcwd(). An explicitly
            // configured PWD still wins, as with every other env var.
            if !env.contains_key("PWD") {
                if let Some(dir_str) = dir.to_str() {
                    env.insert("PWD".to_string(), dir_str.to_string());
                }
            }
        }

        // MCP servers (`npx`, `uvx`, …) live on the host. Inside a Flatpak
        // sandbox they are not on our filesystem at all, so this rewrites the
        // invocation to proxy through `flatpak-spawn --host`. Outside a
        // sandbox it is a pass-through.
        let invocation = sandbox::host_invocation(&command, env, cwd.as_deref());

        // Spawn the child process
        let mut cmd = Command::new(&invocation.program);
        cmd.args(&invocation.leading_args)
            .args(&args)
            .envs(invocation.envs.clone())
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null()) // Discard stderr to prevent pipe buffer deadlock
            .kill_on_drop(true);
        // Under Flatpak the working directory travels as `--directory=`; the
        // host path is meaningless to the `flatpak-spawn` process itself.
        if !sandbox::current().needs_host_proxy() {
            if let Some(dir) = &cwd {
                cmd.current_dir(dir);
            }
        }
        let mut child = cmd.spawn().map_err(|e| {
            AppError::Mcp(format!("Failed to spawn MCP process '{}': {}", command, e))
        })?;

        // Take stdin and stdout handles
        let stdin = child
            .stdin
            .take()
            .ok_or_else(|| AppError::Mcp("Failed to capture stdin of MCP process".to_string()))?;

        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| AppError::Mcp("Failed to capture stdout of MCP process".to_string()))?;

        // Create transport instance
        let pending = Arc::new(RwLock::new(HashMap::new()));
        let closed = Arc::new(RwLock::new(false));
        let notification_callback = Arc::new(RwLock::new(None));
        let request_callback = Arc::new(RwLock::new(None));
        let stdin = Arc::new(Mutex::new(Some(stdin)));
        let streams: StreamRegistry = Arc::new(RwLock::new(HashMap::new()));

        // Start reading stdout in background
        let reader_task = Self::start_stdout_reader(
            stdout,
            pending.clone(),
            closed.clone(),
            notification_callback.clone(),
            request_callback.clone(),
            stdin.clone(),
            streams.clone(),
        );

        let transport = Self {
            child: Arc::new(RwLock::new(Some(child))),
            stdin,
            pending,
            next_id: Arc::new(RwLock::new(1)),
            closed,
            notification_callback,
            request_callback,
            reader_task: Arc::new(RwLock::new(Some(reader_task))),
            streams,
        };

        tracing::debug!("MCP STDIO process spawned successfully");

        Ok(transport)
    }

    /// Set a notification callback
    ///
    /// # Arguments
    /// * `callback` - The callback to invoke when notifications are received
    pub fn set_notification_callback(&self, callback: StdioNotificationCallback) {
        *self.notification_callback.write() = Some(callback);
    }

    /// Set a request callback for server-initiated requests (sampling, elicitation, etc.)
    ///
    /// # Arguments
    /// * `callback` - The callback to invoke when requests are received from the server
    pub fn set_request_callback(&self, callback: StdioRequestCallback) {
        *self.request_callback.write() = Some(callback);
    }

    /// Start background task to read stdout and dispatch responses/notifications/requests
    ///
    /// Returns a JoinHandle that can be used to cancel the task.
    fn start_stdout_reader(
        stdout: ChildStdout,
        pending: Arc<RwLock<HashMap<String, oneshot::Sender<JsonRpcResponse>>>>,
        closed: Arc<RwLock<bool>>,
        notification_callback: Arc<RwLock<Option<StdioNotificationCallback>>>,
        request_callback: Arc<RwLock<Option<StdioRequestCallback>>>,
        stdin: Arc<Mutex<Option<ChildStdin>>>,
        streams: StreamRegistry,
    ) -> JoinHandle<()> {
        tokio::spawn(async move {
            let mut reader = BufReader::new(stdout);
            let mut line = String::new();
            tracing::debug!("STDIO stdout reader task started");

            loop {
                line.clear();

                // Bound a single JSON-RPC line: the child process decides when
                // to send a newline, so read through a `take` so one endless
                // line cannot grow memory without bound.
                let read = (&mut reader)
                    .take(MAX_STDIO_LINE_BYTES as u64 + 1)
                    .read_line(&mut line)
                    .await;
                match read {
                    Ok(n) if n > MAX_STDIO_LINE_BYTES => {
                        tracing::error!(
                            "MCP STDIO message exceeds {} MiB; closing transport",
                            MAX_STDIO_LINE_BYTES / (1024 * 1024)
                        );
                        *closed.write() = true;
                        break;
                    }
                    Ok(0) => {
                        // EOF - process terminated
                        tracing::debug!("MCP STDIO process stdout closed (EOF)");
                        *closed.write() = true;
                        break;
                    }
                    Ok(n) => {
                        tracing::debug!("STDIO stdout received {} bytes", n);
                        // Parse JSON-RPC message (could be response, notification, or request)
                        let trimmed = line.trim();
                        if trimmed.is_empty() {
                            continue;
                        }

                        match serde_json::from_str::<JsonRpcMessage>(trimmed) {
                            Ok(JsonRpcMessage::Response(response)) => {
                                // Handle response using normalized ID
                                let id_str = normalize_response_id(&response.id);
                                let sender = pending.write().remove(&id_str);
                                if let Some(sender) = sender {
                                    // Send response to waiting caller
                                    if sender.send(response).is_err() {
                                        tracing::warn!(
                                            "Failed to send response for request ID: {} (receiver dropped)",
                                            id_str
                                        );
                                    }
                                } else if let Some(stream) =
                                    rpc_stream::take_stream_for_response(&streams, &response)
                                {
                                    stream.finish(response).await;
                                } else {
                                    tracing::warn!(
                                        "Received response for unknown request ID: {}",
                                        id_str
                                    );
                                }
                            }
                            Ok(JsonRpcMessage::Notification(notification)) => {
                                // Handle notification
                                tracing::debug!("Received notification: {}", notification.method);
                                if let Some(stream) =
                                    rpc_stream::stream_for_notification(&streams, &notification)
                                {
                                    // Waiting here is the backpressure: a full
                                    // consumer buffer stops reading the pipe.
                                    if !stream.notify(notification).await {
                                        streams.write().retain(|_, s| !s.is_closed());
                                    }
                                } else if let Some(callback) = notification_callback.read().clone()
                                {
                                    callback(notification);
                                }
                            }
                            Ok(JsonRpcMessage::Request(request)) => {
                                // Handle server-initiated request (sampling, elicitation, roots/list)
                                tracing::info!(
                                    "Received request from server: method={}, id={:?}",
                                    request.method,
                                    request.id
                                );

                                // Get the callback if registered
                                let callback = request_callback.read().clone();
                                if let Some(callback) = callback {
                                    let stdin_clone = stdin.clone();
                                    let request_id = request.id.clone();

                                    // Spawn a task to handle the request asynchronously
                                    tokio::spawn(async move {
                                        // Call the handler and get the response
                                        let response = callback(request).await;

                                        // Send response back to the server via stdin
                                        let mut json = match serde_json::to_string(&response) {
                                            Ok(j) => j,
                                            Err(e) => {
                                                tracing::error!(
                                                    "Failed to serialize response for request {:?}: {}",
                                                    request_id,
                                                    e
                                                );
                                                return;
                                            }
                                        };
                                        json.push('\n');

                                        let mut stdin_guard = stdin_clone.lock().await;
                                        if let Some(stdin) = stdin_guard.as_mut() {
                                            if let Err(e) = stdin.write_all(json.as_bytes()).await {
                                                tracing::error!(
                                                    "Failed to write response to stdin for request {:?}: {}",
                                                    request_id,
                                                    e
                                                );
                                                return;
                                            }
                                            if let Err(e) = stdin.flush().await {
                                                tracing::error!(
                                                    "Failed to flush stdin for request {:?}: {}",
                                                    request_id,
                                                    e
                                                );
                                            }
                                            tracing::debug!(
                                                "Sent response for server request {:?}",
                                                request_id
                                            );
                                        } else {
                                            tracing::error!(
                                                "Stdin not available for sending response to request {:?}",
                                                request_id
                                            );
                                        }
                                    });
                                } else {
                                    tracing::warn!(
                                        "No request callback registered, ignoring server request: {}",
                                        request.method
                                    );
                                }
                            }
                            Err(e) => {
                                tracing::error!(
                                    "Failed to parse JSON-RPC message: {}\nLine: {}",
                                    e,
                                    trimmed
                                );
                            }
                        }
                    }
                    Err(e) => {
                        tracing::error!("Error reading from MCP process stdout: {}", e);
                        *closed.write() = true;
                        break;
                    }
                }
            }

            // Clean up pending requests on shutdown
            let mut pending = pending.write();
            for (id, _sender) in pending.drain() {
                tracing::warn!("Request ID {} terminated without response", id);
            }
            // Dropping the sinks ends each open stream with an error
            streams.write().clear();
        })
    }

    /// Generate the next request ID
    fn next_request_id(&self) -> u64 {
        let mut next_id = self.next_id.write();
        let id = *next_id;
        *next_id += 1;
        id
    }

    /// Check if the process is still running
    pub fn is_alive(&self) -> bool {
        if *self.closed.read() {
            return false;
        }

        let mut child = self.child.write();
        if let Some(ref mut process) = *child {
            // Check if process has exited
            match process.try_wait() {
                Ok(Some(_status)) => {
                    // Process has exited
                    false
                }
                Ok(None) => {
                    // Process is still running
                    true
                }
                Err(e) => {
                    tracing::error!("Error checking process status: {}", e);
                    false
                }
            }
        } else {
            false
        }
    }

    /// Kill the child process and cancel the reader task
    pub async fn kill(&self) -> AppResult<()> {
        tracing::info!("Killing MCP STDIO process");

        *self.closed.write() = true;

        // Abort the reader task first
        if let Some(task) = self.reader_task.write().take() {
            task.abort();
            tracing::debug!("STDIO reader task aborted");
        }

        // Take child out of lock temporarily
        let child_process = {
            let mut child = self.child.write();
            child.take()
        }; // Lock is dropped here

        if let Some(mut process) = child_process {
            process
                .kill()
                .await
                .map_err(|e| AppError::Mcp(format!("Failed to kill MCP process: {}", e)))?;
        }

        Ok(())
    }
}

/// Write one JSON-RPC message as a line on the child's stdin.
async fn write_message(
    stdin: &Mutex<Option<ChildStdin>>,
    message: &JsonRpcRequest,
) -> AppResult<()> {
    let mut json = serde_json::to_string(message)
        .map_err(|e| AppError::Mcp(format!("Failed to serialize request: {}", e)))?;
    json.push('\n');
    let mut guard = stdin.lock().await;
    let stdin = guard
        .as_mut()
        .ok_or_else(|| AppError::Mcp("Stdin not available".to_string()))?;
    stdin
        .write_all(json.as_bytes())
        .await
        .map_err(|e| AppError::Mcp(format!("Failed to write to stdin: {}", e)))?;
    stdin
        .flush()
        .await
        .map_err(|e| AppError::Mcp(format!("Failed to flush stdin: {}", e)))
}

impl Drop for StdioTransport {
    fn drop(&mut self) {
        // The child is spawned with `kill_on_drop(true)`, so the underlying
        // process is killed when the last `Child` clone is dropped. However,
        // the background reader task captures its own clones of the shared
        // `Arc<RwLock<...>>` state, so if any of those keep the inherent
        // `Child` alive past this Drop the process would linger. We
        // defensively `start_kill()` here too.
        //
        // We also abort the reader task explicitly: it would otherwise exit
        // on its own once stdout EOFs, but aborting guarantees no lingering
        // task survives the transport.
        //
        // `try_write` (not `write`) — if some other thread happens to hold
        // the lock at drop time we'd rather skip cleanup than deadlock the
        // dropping thread. The process kill via `kill_on_drop` still applies.
        if let Some(mut guard) = self.reader_task.try_write() {
            if let Some(task) = guard.take() {
                task.abort();
            }
        }
        if let Some(mut guard) = self.child.try_write() {
            if let Some(mut child) = guard.take() {
                let _ = child.start_kill();
            }
        }
    }
}

#[async_trait]
impl Transport for StdioTransport {
    async fn send_request(&self, mut request: JsonRpcRequest) -> AppResult<JsonRpcResponse> {
        if *self.closed.read() {
            return Err(AppError::Mcp("Transport is closed".to_string()));
        }

        // Check if this is a notification (no ID, starts with "notifications/")
        // Notifications are fire-and-forget - no response expected
        let is_notification = request.id.is_none() && request.method.starts_with("notifications/");

        if is_notification {
            // For notifications: don't add ID, don't wait for response
            let mut json = serde_json::to_string(&request)
                .map_err(|e| AppError::Mcp(format!("Failed to serialize notification: {}", e)))?;
            json.push('\n');

            tracing::debug!("STDIO sending notification: {}", request.method);

            // Write to stdin
            {
                let mut stdin_guard = self.stdin.lock().await;
                let stdin = stdin_guard
                    .as_mut()
                    .ok_or_else(|| AppError::Mcp("Stdin not available".to_string()))?;

                stdin.write_all(json.as_bytes()).await.map_err(|e| {
                    AppError::Mcp(format!("Failed to write notification to stdin: {}", e))
                })?;

                stdin
                    .flush()
                    .await
                    .map_err(|e| AppError::Mcp(format!("Failed to flush stdin: {}", e)))?;
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

        // Always generate a unique request ID to avoid collisions
        // This prevents race conditions when concurrent requests might have the same ID
        let request_id = {
            let id = self.next_request_id();
            request.id = Some(Value::Number(id.into()));
            id.to_string()
        };

        // Create channel for response
        let (tx, rx) = oneshot::channel();

        // Register pending request
        self.pending.write().insert(request_id.clone(), tx);
        let _pending_guard = super::PendingRequestGuard::new(&self.pending, request_id.clone());

        // Serialize request to JSON
        let mut json = serde_json::to_string(&request).map_err(|e| {
            self.pending.write().remove(&request_id);
            AppError::Mcp(format!("Failed to serialize request: {}", e))
        })?;

        json.push('\n');

        // Write to stdin
        // Use Mutex to safely handle concurrent writes
        {
            let mut stdin_guard = self.stdin.lock().await;
            let stdin = stdin_guard.as_mut().ok_or_else(|| {
                self.pending.write().remove(&request_id);
                AppError::Mcp("Stdin not available".to_string())
            })?;

            // Write and flush while holding the lock
            // This is safe because Mutex allows holding across await points
            stdin.write_all(json.as_bytes()).await.map_err(|e| {
                self.pending.write().remove(&request_id);
                AppError::Mcp(format!("Failed to write to stdin: {}", e))
            })?;

            stdin.flush().await.map_err(|e| {
                self.pending.write().remove(&request_id);
                AppError::Mcp(format!("Failed to flush stdin: {}", e))
            })?;
        }

        // Wait for response (with timeout)
        let wait_start = std::time::Instant::now();
        let mut response = tokio::time::timeout(std::time::Duration::from_secs(30), rx)
            .await
            .map_err(|_| {
                self.pending.write().remove(&request_id);
                tracing::warn!(
                    "STDIO request timeout (30s): method={}, id={}",
                    request.method,
                    request_id
                );
                AppError::Mcp(format!("Request timeout for ID: {}", request_id))
            })?
            .map_err(|_| {
                tracing::warn!(
                    "STDIO response channel closed: method={}, id={}, elapsed={:?}",
                    request.method,
                    request_id,
                    wait_start.elapsed()
                );
                AppError::Mcp(format!("Response channel closed for ID: {}", request_id))
            })?;
        tracing::debug!(
            "STDIO response: method={}, id={}, elapsed={:?}",
            request.method,
            request_id,
            wait_start.elapsed()
        );

        // Restore original request ID in response
        response.id = original_request_id.unwrap_or(Value::Null);
        Ok(response)
    }

    async fn open_rpc_stream(
        &self,
        mut request: JsonRpcRequest,
        buffer: usize,
    ) -> AppResult<RpcStream> {
        if *self.closed.read() {
            return Err(AppError::Mcp("Transport is closed".to_string()));
        }
        let original_id = request.id.clone().unwrap_or(Value::Null);
        let wire_id = self.next_request_id();
        request.id = Some(Value::Number(wire_id.into()));
        let key = wire_id.to_string();

        let (mut stream, sink) = RpcStream::channel(original_id, buffer, None);
        self.streams.write().insert(key.clone(), sink);

        let streams = self.streams.clone();
        let stdin = self.stdin.clone();
        let closed = self.closed.clone();
        let cancel_key = key.clone();
        stream.set_cancel(Box::new(move || {
            // Already finished or never sent: nothing to cancel upstream
            if streams.write().remove(&cancel_key).is_none() || *closed.read() {
                return;
            }
            let Ok(runtime) = tokio::runtime::Handle::try_current() else {
                return;
            };
            runtime.spawn(async move {
                let cancelled = JsonRpcRequest::new(
                    None,
                    "notifications/cancelled".to_string(),
                    Some(serde_json::json!({
                        "requestId": wire_id,
                        "reason": "Stream closed by LocalRouter",
                    })),
                );
                if let Err(e) = write_message(&stdin, &cancelled).await {
                    tracing::debug!("Could not cancel MCP stream {}: {}", wire_id, e);
                }
            });
        }));

        tracing::debug!(
            "STDIO opening stream: method={}, id={}",
            request.method,
            wire_id
        );
        if let Err(e) = write_message(&self.stdin, &request).await {
            self.streams.write().remove(&key);
            return Err(e);
        }
        Ok(stream)
    }

    fn supports_rpc_streams(&self) -> bool {
        true
    }

    async fn is_healthy(&self) -> bool {
        self.is_alive()
    }

    async fn close(&self) -> AppResult<()> {
        self.kill().await
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
    use super::*;
    use serde_json::json;

    #[tokio::test]
    #[ignore] // Requires npx to be installed
    async fn test_stdio_spawn() {
        // Test with a simple echo server (if available)
        // This test is ignored by default as it requires external dependencies
        let result = StdioTransport::spawn(
            "npx".to_string(),
            vec![
                "-y".to_string(),
                "@modelcontextprotocol/server-everything".to_string(),
            ],
            HashMap::new(),
        )
        .await;

        if let Ok(transport) = result {
            assert!(transport.is_alive());
            transport.kill().await.unwrap();
            tokio::time::sleep(std::time::Duration::from_millis(100)).await;
            assert!(!transport.is_alive());
        }
    }

    /// A missing working directory must fail with a message naming the
    /// directory, not the opaque OS error the raw spawn would produce.
    #[tokio::test]
    async fn test_spawn_in_missing_cwd_reports_directory() {
        let missing = std::path::PathBuf::from("/definitely/not/a/real/directory-12345");
        let result = StdioTransport::spawn_in(
            "echo".to_string(),
            vec!["hi".to_string()],
            HashMap::new(),
            Some(missing.clone()),
        )
        .await;

        // StdioTransport isn't Debug, so match rather than unwrap_err().
        let msg = match result {
            Ok(_) => panic!("spawn should fail for a missing working directory"),
            Err(e) => e.to_string(),
        };
        assert!(
            msg.contains("Working directory"),
            "unexpected error: {}",
            msg
        );
        assert!(
            msg.contains(&missing.display().to_string()),
            "error should name the directory: {}",
            msg
        );
    }

    /// The child really is started in the requested directory: it writes its
    /// own `pwd` to a relative path, which can only land in `cwd`.
    #[cfg(unix)]
    #[tokio::test]
    async fn test_spawn_in_uses_requested_cwd() {
        // Resolve symlinks (/tmp -> /private/tmp on macOS) so the path the
        // child reports matches the one we asked for.
        let dir = std::fs::canonicalize(std::env::temp_dir())
            .unwrap()
            .join(format!("lr-mcp-cwd-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();

        let transport = StdioTransport::spawn_in(
            "sh".to_string(),
            // Keep the process alive after writing so the transport stays up.
            vec!["-c".to_string(), "pwd > where.txt; sleep 30".to_string()],
            HashMap::new(),
            Some(dir.clone()),
        )
        .await
        .expect("spawn should succeed");

        // Give the child a moment to write before reading.
        let marker = dir.join("where.txt");
        let mut recorded = None;
        for _ in 0..50 {
            if let Ok(contents) = std::fs::read_to_string(&marker) {
                if !contents.trim().is_empty() {
                    recorded = Some(contents.trim().to_string());
                    break;
                }
            }
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        }

        transport.kill().await.unwrap();
        let _ = std::fs::remove_dir_all(&dir);

        assert_eq!(
            recorded.as_deref(),
            Some(dir.to_str().unwrap()),
            "child should have run in the requested directory"
        );
    }

    /// Spawn `sh -c script` as a fake MCP server.
    #[cfg(unix)]
    async fn fake_server(script: &str) -> StdioTransport {
        StdioTransport::spawn(
            "sh".to_string(),
            vec!["-c".to_string(), script.to_string()],
            HashMap::new(),
        )
        .await
        .expect("spawn fake server")
    }

    #[cfg(unix)]
    async fn next_message(stream: &mut RpcStream) -> rpc_stream::RpcStreamMessage {
        use futures_util::StreamExt;
        tokio::time::timeout(std::time::Duration::from_secs(5), stream.next())
            .await
            .expect("stream message in time")
            .expect("stream not ended")
            .expect("stream message ok")
    }

    /// One request yields every correlated notification and then its result;
    /// uncorrelated notifications keep going to the generic callback, and an
    /// ordinary request on the same transport is answered in between.
    #[cfg(unix)]
    #[tokio::test]
    async fn rpc_stream_delivers_many_results_alongside_requests() {
        use rpc_stream::RpcStreamMessage;
        let transport = fake_server(
            r#"read stream_request
read plain_request
echo '{"jsonrpc":"2.0","method":"notifications/events/active","params":{"_meta":{"io.modelcontextprotocol/subscriptionId":1}}}'
echo '{"jsonrpc":"2.0","method":"notifications/message","params":{"level":"info"}}'
echo '{"jsonrpc":"2.0","id":2,"result":{"plain":true}}'
echo '{"jsonrpc":"2.0","method":"notifications/events/event","params":{"name":"e","_meta":{"io.modelcontextprotocol/subscriptionId":1}}}'
echo '{"jsonrpc":"2.0","method":"notifications/events/event","params":{"name":"other","_meta":{"io.modelcontextprotocol/subscriptionId":99}}}'
echo '{"jsonrpc":"2.0","id":1,"result":{"resultType":"complete"}}'
sleep 30"#,
        )
        .await;
        let generic = Arc::new(parking_lot::Mutex::new(Vec::new()));
        let seen = generic.clone();
        Transport::set_notification_callback(
            &transport,
            Arc::new(move |n: JsonRpcNotification| seen.lock().push(n.method)),
        );

        let mut stream = transport
            .open_rpc_stream(
                JsonRpcRequest::with_id(7, "events/stream".into(), Some(json!({}))),
                8,
            )
            .await
            .unwrap();
        assert_eq!(stream.request_id(), &json!(7));
        let plain = transport
            .send_request(JsonRpcRequest::with_id(8, "tools/list".into(), None))
            .await
            .unwrap();
        assert_eq!(plain.id, json!(8));
        assert_eq!(plain.result, Some(json!({"plain": true})));

        match next_message(&mut stream).await {
            RpcStreamMessage::Notification(n) => {
                assert_eq!(n.method, "notifications/events/active");
                assert_eq!(rpc_stream::subscription_id(&n), Some(&json!(7)));
            }
            other => panic!("unexpected {other:?}"),
        }
        match next_message(&mut stream).await {
            RpcStreamMessage::Notification(n) => {
                assert_eq!(n.params.unwrap()["name"], "e")
            }
            other => panic!("unexpected {other:?}"),
        }
        match next_message(&mut stream).await {
            RpcStreamMessage::Result(r) => {
                assert_eq!(r.id, json!(7));
                assert_eq!(r.result, Some(json!({"resultType": "complete"})));
            }
            other => panic!("unexpected {other:?}"),
        }
        {
            use futures_util::StreamExt;
            assert!(stream.next().await.is_none());
        }
        let generic = generic.lock().clone();
        assert!(generic.contains(&"notifications/message".to_string()));
        assert!(generic.contains(&"notifications/events/event".to_string()));
        assert_eq!(generic.len(), 2, "{generic:?}");
        transport.kill().await.unwrap();
    }

    /// Dropping a stream before its result sends `notifications/cancelled`
    /// for the wire id of the request.
    #[cfg(unix)]
    #[tokio::test]
    async fn dropping_rpc_stream_cancels_upstream() {
        let transport = fake_server(
            r#"read stream_request
echo '{"jsonrpc":"2.0","method":"notifications/events/active","params":{"_meta":{"io.modelcontextprotocol/subscriptionId":1}}}'
read cancellation
printf '{"jsonrpc":"2.0","method":"notifications/echo","params":{"line":%s}}
' "$cancellation"
sleep 30"#,
        )
        .await;
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
        Transport::set_notification_callback(
            &transport,
            Arc::new(move |n: JsonRpcNotification| {
                let _ = tx.send(n);
            }),
        );
        let mut stream = transport
            .open_rpc_stream(JsonRpcRequest::with_id(1, "events/stream".into(), None), 4)
            .await
            .unwrap();
        next_message(&mut stream).await;
        drop(stream);

        let echoed = tokio::time::timeout(std::time::Duration::from_secs(5), rx.recv())
            .await
            .unwrap()
            .unwrap();
        assert_eq!(echoed.method, "notifications/echo");
        let line = &echoed.params.unwrap()["line"];
        assert_eq!(line["method"], "notifications/cancelled");
        assert_eq!(line["params"]["requestId"], json!(1));
        transport.kill().await.unwrap();
    }

    /// A process that exits mid-stream ends the stream with an error, not a
    /// result, so the consumer knows to reconnect.
    #[cfg(unix)]
    #[tokio::test]
    async fn rpc_stream_errors_when_process_exits() {
        use futures_util::StreamExt;
        let transport = fake_server(
            r#"read stream_request
echo '{"jsonrpc":"2.0","method":"notifications/events/active","params":{"_meta":{"io.modelcontextprotocol/subscriptionId":1}}}'"#,
        )
        .await;
        let mut stream = transport
            .open_rpc_stream(JsonRpcRequest::with_id(1, "events/stream".into(), None), 4)
            .await
            .unwrap();
        next_message(&mut stream).await;
        let end = tokio::time::timeout(std::time::Duration::from_secs(5), stream.next())
            .await
            .unwrap()
            .unwrap();
        assert!(end.is_err());
    }

    #[tokio::test]
    async fn test_request_id_generation() {
        let transport = StdioTransport {
            child: Arc::new(RwLock::new(None)),
            stdin: Arc::new(Mutex::new(None)),
            pending: Arc::new(RwLock::new(HashMap::new())),
            next_id: Arc::new(RwLock::new(1)),
            closed: Arc::new(RwLock::new(false)),
            notification_callback: Arc::new(RwLock::new(None)),
            request_callback: Arc::new(RwLock::new(None)),
            reader_task: Arc::new(RwLock::new(None)),
            streams: Arc::new(RwLock::new(HashMap::new())),
        };

        assert_eq!(transport.next_request_id(), 1);
        assert_eq!(transport.next_request_id(), 2);
        assert_eq!(transport.next_request_id(), 3);
    }

    #[test]
    fn test_normalize_response_id() {
        use serde_json::json;

        // Test null ID
        assert_eq!(normalize_response_id(&Value::Null), "__null_id__");

        // Test numeric ID
        assert_eq!(normalize_response_id(&json!(42)), "42");

        // Test string ID
        assert_eq!(normalize_response_id(&json!("abc")), "\"abc\"");
    }

    #[test]
    fn test_json_rpc_serialization() {
        let request =
            JsonRpcRequest::with_id(1, "test_method".to_string(), Some(json!({"key": "value"})));
        let json = serde_json::to_string(&request).unwrap();
        assert!(json.contains("\"jsonrpc\":\"2.0\""));
        assert!(json.contains("\"method\":\"test_method\""));
    }
}
