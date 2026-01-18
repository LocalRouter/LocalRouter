//! STDIO transport for MCP
//!
//! Spawns a subprocess and communicates via stdin/stdout using JSON-RPC 2.0.
//! This is the most common transport type for MCP servers.

use crate::mcp::protocol::{JsonRpcRequest, JsonRpcResponse};
use crate::mcp::transport::Transport;
use crate::utils::errors::{AppError, AppResult};
use async_trait::async_trait;
use parking_lot::RwLock;
use serde_json::Value;
use std::collections::HashMap;
use std::process::Stdio;
use std::sync::Arc;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::{Child, ChildStdin, ChildStdout, Command};
use tokio::sync::{oneshot, Mutex};

/// STDIO transport implementation
///
/// Manages a subprocess with JSON-RPC communication over stdin/stdout.
/// Supports concurrent requests with request/response correlation.
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
}

impl StdioTransport {
    /// Spawn a new MCP server process with STDIO transport
    ///
    /// # Arguments
    /// * `command` - The command to execute (e.g., "npx")
    /// * `args` - Command arguments (e.g., ["-y", "@modelcontextprotocol/server-everything"])
    /// * `env` - Environment variables to set
    ///
    /// # Returns
    /// * The transport instance with the running process
    pub async fn spawn(
        command: String,
        args: Vec<String>,
        env: HashMap<String, String>,
    ) -> AppResult<Self> {
        tracing::info!("Spawning MCP STDIO process: {} {:?}", command, args);

        // Spawn the child process
        let mut child = Command::new(&command)
            .args(&args)
            .envs(env)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true)
            .spawn()
            .map_err(|e| {
                AppError::Mcp(format!("Failed to spawn MCP process '{}': {}", command, e))
            })?;

        // Take stdin and stdout handles
        let stdin = child.stdin.take().ok_or_else(|| {
            AppError::Mcp("Failed to capture stdin of MCP process".to_string())
        })?;

        let stdout = child.stdout.take().ok_or_else(|| {
            AppError::Mcp("Failed to capture stdout of MCP process".to_string())
        })?;

        // Create transport instance
        let transport = Self {
            child: Arc::new(RwLock::new(Some(child))),
            stdin: Arc::new(Mutex::new(Some(stdin))),
            pending: Arc::new(RwLock::new(HashMap::new())),
            next_id: Arc::new(RwLock::new(1)),
            closed: Arc::new(RwLock::new(false)),
        };

        // Start reading stdout in background
        transport.start_stdout_reader(stdout);

        tracing::info!("MCP STDIO process spawned successfully");

        Ok(transport)
    }

    /// Start background task to read stdout and dispatch responses
    fn start_stdout_reader(&self, stdout: ChildStdout) {
        let pending = self.pending.clone();
        let closed = self.closed.clone();

        tokio::spawn(async move {
            let mut reader = BufReader::new(stdout);
            let mut line = String::new();

            loop {
                line.clear();

                match reader.read_line(&mut line).await {
                    Ok(0) => {
                        // EOF - process terminated
                        tracing::info!("MCP STDIO process stdout closed");
                        *closed.write() = true;
                        break;
                    }
                    Ok(_) => {
                        // Parse JSON-RPC response
                        let trimmed = line.trim();
                        if trimmed.is_empty() {
                            continue;
                        }

                        match serde_json::from_str::<JsonRpcResponse>(trimmed) {
                            Ok(response) => {
                                // Extract ID and find pending sender
                                let id_str = response.id.to_string();

                                if let Some(sender) = pending.write().remove(&id_str) {
                                    // Send response to waiting caller
                                    if sender.send(response).is_err() {
                                        tracing::warn!(
                                            "Failed to send response for request ID: {}",
                                            id_str
                                        );
                                    }
                                } else {
                                    tracing::warn!(
                                        "Received response for unknown request ID: {}",
                                        id_str
                                    );
                                }
                            }
                            Err(e) => {
                                tracing::error!("Failed to parse JSON-RPC response: {}\nLine: {}", e, trimmed);
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
        });
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

    /// Kill the child process
    pub async fn kill(&self) -> AppResult<()> {
        tracing::info!("Killing MCP STDIO process");

        *self.closed.write() = true;

        // Take child out of lock temporarily
        let child_process = {
            let mut child = self.child.write();
            child.take()
        }; // Lock is dropped here

        if let Some(mut process) = child_process {
            process.kill().await.map_err(|e| {
                AppError::Mcp(format!("Failed to kill MCP process: {}", e))
            })?;
        }

        Ok(())
    }
}

#[async_trait]
impl Transport for StdioTransport {
    async fn send_request(&self, mut request: JsonRpcRequest) -> AppResult<JsonRpcResponse> {
        if *self.closed.read() {
            return Err(AppError::Mcp("Transport is closed".to_string()));
        }

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
        let response = tokio::time::timeout(std::time::Duration::from_secs(30), rx)
            .await
            .map_err(|_| {
                self.pending.write().remove(&request_id);
                AppError::Mcp(format!("Request timeout for ID: {}", request_id))
            })?
            .map_err(|_| {
                AppError::Mcp(format!("Response channel closed for ID: {}", request_id))
            })?;

        Ok(response)
    }

    async fn is_healthy(&self) -> bool {
        self.is_alive()
    }

    async fn close(&self) -> AppResult<()> {
        self.kill().await
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
            vec!["-y".to_string(), "@modelcontextprotocol/server-everything".to_string()],
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

    #[tokio::test]
    async fn test_request_id_generation() {
        let transport = StdioTransport {
            child: Arc::new(RwLock::new(None)),
            stdin: Arc::new(Mutex::new(None)),
            pending: Arc::new(RwLock::new(HashMap::new())),
            next_id: Arc::new(RwLock::new(1)),
            closed: Arc::new(RwLock::new(false)),
        };

        assert_eq!(transport.next_request_id(), 1);
        assert_eq!(transport.next_request_id(), 2);
        assert_eq!(transport.next_request_id(), 3);
    }

    #[test]
    fn test_json_rpc_serialization() {
        let request = JsonRpcRequest::with_id(1, "test_method".to_string(), Some(json!({"key": "value"})));
        let json = serde_json::to_string(&request).unwrap();
        assert!(json.contains("\"jsonrpc\":\"2.0\""));
        assert!(json.contains("\"method\":\"test_method\""));
    }
}
