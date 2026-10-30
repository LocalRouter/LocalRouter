//! Per-session transport container.
//!
//! Each gateway session owns a `SessionTransportSet` that holds the transports
//! (stdio processes, SSE connections, WebSocket connections) for that session.
//! When the session ends, all transports are closed.

use std::sync::Arc;

use dashmap::DashMap;
use futures_util::stream::Stream;
use std::pin::Pin;

use crate::protocol::{JsonRpcRequest, JsonRpcResponse, StreamingChunk};
use lr_types::errors::{AppError, AppResult};

use super::Transport;

/// A set of MCP server transports owned by a single gateway session.
///
/// Provides request routing by server_id and lifecycle management (close all on drop).
pub(crate) type SessionFailures = parking_lot::RwLock<
    std::collections::HashMap<String, std::collections::HashMap<String, String>>,
>;

type FailureCallback = Arc<dyn Fn(&str, Option<&str>) + Send + Sync>;

pub struct SessionTransportSet {
    pub(crate) failures: Arc<SessionFailures>,
    failure_callback: parking_lot::RwLock<Option<FailureCallback>>,
    transports: Arc<DashMap<String, Arc<dyn Transport>>>,
    /// An interrupted response has an uncertain outcome. Never replay it;
    /// rebuild the affected connection before a later client request instead.
    reconnect_required: Arc<dashmap::DashSet<String>>,
    configurations: DashMap<String, lr_config::McpServerConfig>,

    /// Protocol revision negotiated per backend (absent = legacy).
    /// Set by the gateway after a successful `server/discover` probe.
    revisions: DashMap<String, crate::protocol::ProtocolRevision>,

    /// Client capabilities to declare in `_meta` on requests to stateless
    /// (2026-07-28) backends.
    client_capabilities: parking_lot::RwLock<Option<serde_json::Value>>,
}

impl Default for SessionTransportSet {
    fn default() -> Self {
        Self::new()
    }
}

impl SessionTransportSet {
    /// Create an empty transport set.
    pub fn new() -> Self {
        Self {
            failures: Arc::new(parking_lot::RwLock::new(std::collections::HashMap::new())),
            failure_callback: parking_lot::RwLock::new(None),
            transports: Arc::new(DashMap::new()),
            reconnect_required: Arc::new(dashmap::DashSet::new()),
            configurations: DashMap::new(),
            revisions: DashMap::new(),
            client_capabilities: parking_lot::RwLock::new(None),
        }
    }

    pub(crate) fn set_failure_callback(&self, callback: FailureCallback) {
        *self.failure_callback.write() = Some(callback);
    }

    pub(crate) fn record_failure(&self, server_id: &str, method: &str, error: &str) {
        self.failures
            .write()
            .entry(server_id.to_string())
            .or_default()
            .insert(method.to_string(), error.to_string());
        if let Some(callback) = self.failure_callback.read().clone() {
            callback(server_id, Some(error));
        }
    }

    /// Record the protocol revision negotiated with a backend.
    pub fn set_revision(&self, server_id: &str, revision: crate::protocol::ProtocolRevision) {
        self.revisions.insert(server_id.to_string(), revision);
        if let Some(transport) = self.get(server_id) {
            transport.set_protocol_revision(revision);
        }
        if revision.is_stateless() {
            self.reconnect_required.remove(server_id);
            let recovered = {
                let mut failures = self.failures.write();
                if let Some(methods) = failures.get_mut(server_id) {
                    let recovered = methods.remove("_connect").is_some();
                    if methods.is_empty() {
                        failures.remove(server_id);
                    }
                    recovered
                } else {
                    false
                }
            };
            if recovered {
                if let Some(callback) = self.failure_callback.read().clone() {
                    callback(server_id, None);
                }
            }
        }
    }

    /// The protocol revision negotiated with a backend (legacy when unset).
    pub fn revision(&self, server_id: &str) -> crate::protocol::ProtocolRevision {
        self.revisions
            .get(server_id)
            .map(|e| *e.value())
            .unwrap_or_default()
    }

    /// Set the client capabilities declared to stateless backends in `_meta`.
    pub fn set_client_capabilities(&self, capabilities: serde_json::Value) {
        *self.client_capabilities.write() = Some(capabilities);
    }

    /// Inject the reserved 2026-07-28 `_meta` keys into a request bound for a
    /// stateless backend. Existing `_meta` values are preserved.
    fn prepare_request_for(&self, server_id: &str, mut request: JsonRpcRequest) -> JsonRpcRequest {
        if !self.revision(server_id).is_stateless() {
            return request;
        }

        use crate::protocol::meta_keys;
        let params = request.params.get_or_insert_with(|| serde_json::json!({}));
        if let Some(obj) = params.as_object_mut() {
            let meta = obj.entry("_meta").or_insert_with(|| serde_json::json!({}));
            if let Some(meta_obj) = meta.as_object_mut() {
                meta_obj
                    .entry(meta_keys::PROTOCOL_VERSION.to_string())
                    .or_insert_with(|| {
                        serde_json::json!(crate::protocol::MCP_PROTOCOL_VERSION_STATELESS)
                    });
                meta_obj
                    .entry(meta_keys::CLIENT_INFO.to_string())
                    .or_insert_with(|| {
                        serde_json::json!({
                            "name": "LocalRouter MCP Gateway",
                            "version": env!("CARGO_PKG_VERSION"),
                        })
                    });
                let capabilities = self
                    .client_capabilities
                    .read()
                    .clone()
                    .unwrap_or_else(|| serde_json::json!({}));
                meta_obj
                    .entry(meta_keys::CLIENT_CAPABILITIES.to_string())
                    .or_insert(capabilities);
            }
        }
        request
    }

    /// Insert a transport for a server.
    pub fn insert(&self, server_id: String, transport: Arc<dyn Transport>) {
        self.reconnect_required.remove(&server_id);
        self.transports.insert(server_id, transport);
    }

    pub(crate) fn insert_configured(
        &self,
        server_id: String,
        transport: Arc<dyn Transport>,
        config: lr_config::McpServerConfig,
    ) {
        self.configurations.insert(server_id.clone(), config);
        self.insert(server_id, transport);
    }

    pub(crate) fn configuration_matches(
        &self,
        server_id: &str,
        config: &lr_config::McpServerConfig,
    ) -> bool {
        self.configurations
            .get(server_id)
            .is_none_or(|saved| saved.value() == config)
    }

    pub async fn needs_reconnect(&self, server_id: &str) -> bool {
        if self.reconnect_required.contains(server_id) {
            return true;
        }
        match self.get(server_id) {
            Some(transport) => !transport.is_healthy().await,
            None => true,
        }
    }

    pub(crate) fn mark_connecting(&self, server_id: &str) {
        self.reconnect_required.insert(server_id.to_string());
    }

    /// Check if a server has an active transport.
    pub fn is_running(&self, server_id: &str) -> bool {
        self.transports.contains_key(server_id)
    }

    /// Get all running server IDs.
    pub fn running_server_ids(&self) -> Vec<String> {
        self.transports.iter().map(|e| e.key().clone()).collect()
    }

    /// Get a transport by server_id (for setting callbacks etc.).
    pub fn get(&self, server_id: &str) -> Option<Arc<dyn Transport>> {
        self.transports.get(server_id).map(|e| e.value().clone())
    }

    /// Send a JSON-RPC request to a specific server.
    pub async fn send_request(
        &self,
        server_id: &str,
        request: JsonRpcRequest,
    ) -> AppResult<JsonRpcResponse> {
        let transport = self
            .transports
            .get(server_id)
            .map(|e| e.value().clone())
            .ok_or_else(|| AppError::Mcp(format!("Server not running: {}", server_id)))?;

        let request = self.prepare_request_for(server_id, request);
        let method = request.method.clone();
        let result = transport.send_request(request).await;
        // A late response from a replaced connection must not mark the new
        // connection broken (or clear its failures). Concurrent session-expiry
        // responses otherwise trigger a cascade of unnecessary reconnects.
        let current = self.transports.get(server_id);
        if !current
            .as_ref()
            .is_some_and(|current| Arc::ptr_eq(current.value(), &transport))
        {
            return result;
        }
        // Keep the read guard until diagnostic publication finishes so replacing
        // this transport cannot race a late error's reconnect marker.
        let _current_transport = current;
        match &result {
            Err(_) if method != "server/discover" => {
                self.reconnect_required.insert(server_id.to_string());
                self.record_failure(server_id, &method, &format!("MCP connection failed during {method}; check authentication and server availability"));
            }
            Ok(response) if response.error.is_none() && !response.is_tool_error() => {
                if method == "initialize" {
                    self.reconnect_required.remove(server_id);
                }
                let recovered = {
                    let mut failures = self.failures.write();
                    if let Some(methods) = failures.get_mut(server_id) {
                        let mut recovered = methods.remove(&method).is_some();
                        if method == "initialize" {
                            recovered |= methods.remove("_connect").is_some();
                        }
                        if methods.is_empty() {
                            failures.remove(server_id);
                        }
                        recovered
                    } else {
                        false
                    }
                };
                if recovered {
                    if let Some(callback) = self.failure_callback.read().clone() {
                        callback(server_id, None);
                    }
                }
            }
            Ok(response)
                if matches!(
                    method.as_str(),
                    "initialize" | "tools/list" | "resources/list" | "prompts/list"
                ) && response.error.as_ref().is_some_and(|e| e.code != -32601) =>
            {
                self.record_failure(
                    server_id,
                    &method,
                    &format!("MCP {method} rejected; check authentication and permissions"),
                );
            }
            _ => {}
        }
        result
    }

    /// Send a streaming request to a specific server.
    pub async fn stream_request(
        &self,
        server_id: &str,
        request: JsonRpcRequest,
    ) -> AppResult<Pin<Box<dyn Stream<Item = AppResult<StreamingChunk>> + Send>>> {
        let transport = self
            .transports
            .get(server_id)
            .map(|e| e.value().clone())
            .ok_or_else(|| AppError::Mcp(format!("Server not running: {}", server_id)))?;

        use futures_util::StreamExt;
        let request = self.prepare_request_for(server_id, request);
        let method = request.method.clone();
        let stream = match transport.stream_request(request).await {
            Ok(stream) => stream,
            Err(error) => {
                let current = self.transports.get(server_id);
                if current
                    .as_ref()
                    .is_some_and(|current| Arc::ptr_eq(current.value(), &transport))
                {
                    self.reconnect_required.insert(server_id.to_string());
                    self.record_failure(server_id, &method, "MCP streaming request failed; check authentication and server availability");
                }
                return Err(error);
            }
        };
        let current_transports = self.transports.clone();
        let reconnect = self.reconnect_required.clone();
        let failures = self.failures.clone();
        let callback = self.failure_callback.read().clone();
        let id = server_id.to_string();
        Ok(Box::pin(stream.inspect(move |chunk| {
            let current = current_transports.get(&id);
            if chunk.is_err()
                && current
                    .as_ref()
                    .is_some_and(|current| Arc::ptr_eq(current.value(), &transport))
            {
                let error = "MCP response stream interrupted; the operation outcome is unknown";
                reconnect.insert(id.clone());
                failures
                    .write()
                    .entry(id.clone())
                    .or_default()
                    .insert(method.clone(), error.into());
                if let Some(callback) = &callback {
                    callback(&id, Some(error));
                }
            }
        })))
    }

    /// Close and remove a single server's transport.
    pub async fn close_server(&self, server_id: &str) {
        let transport = self
            .transports
            .remove(server_id)
            .map(|(_, transport)| transport);
        self.failures.write().remove(server_id);
        self.reconnect_required.remove(server_id);
        self.revisions.remove(server_id);
        self.configurations.remove(server_id);
        if let Some(transport) = transport {
            if let Err(e) = transport.close().await {
                tracing::warn!("Error closing transport for server {}: {}", server_id, e);
            }
        }
    }

    /// Close all transports in this set.
    pub async fn close_all(&self) {
        self.failures.write().clear();
        let entries: Vec<(String, Arc<dyn Transport>)> = self
            .transports
            .iter()
            .map(|e| (e.key().clone(), e.value().clone()))
            .collect();

        self.transports.clear();

        for (server_id, transport) in entries {
            if let Err(e) = transport.close().await {
                tracing::warn!("Error closing transport for server {}: {}", server_id, e);
            }
        }
    }
}

impl Drop for SessionTransportSet {
    fn drop(&mut self) {
        // If there are still transports when dropped, spawn a cleanup task.
        // This is a safety net — callers should call close_all() explicitly.
        if !self.transports.is_empty() {
            let entries: Vec<(String, Arc<dyn Transport>)> = self
                .transports
                .iter()
                .map(|e| (e.key().clone(), e.value().clone()))
                .collect();
            self.transports.clear();

            // Use Handle::try_current to avoid panic during runtime shutdown
            if let Ok(handle) = tokio::runtime::Handle::try_current() {
                handle.spawn(async move {
                    for (server_id, transport) in entries {
                        if let Err(e) = transport.close().await {
                            tracing::warn!(
                                "Error closing orphaned transport for server {}: {}",
                                server_id,
                                e
                            );
                        }
                    }
                });
            }
            // If runtime is gone, transports are cleaned up by OS process exit
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use async_trait::async_trait;
    use std::sync::atomic::{AtomicBool, Ordering};

    struct MockTransport {
        closed: AtomicBool,
    }

    #[async_trait]
    impl Transport for MockTransport {
        async fn send_request(&self, _request: JsonRpcRequest) -> AppResult<JsonRpcResponse> {
            Ok(JsonRpcResponse {
                jsonrpc: "2.0".to_string(),
                id: serde_json::Value::Null,
                result: Some(serde_json::json!({})),
                error: None,
            })
        }

        async fn is_healthy(&self) -> bool {
            !self.closed.load(Ordering::Relaxed)
        }

        async fn close(&self) -> AppResult<()> {
            self.closed.store(true, Ordering::Relaxed);
            Ok(())
        }
    }

    #[tokio::test]
    async fn test_insert_and_is_running() {
        let set = SessionTransportSet::new();
        assert!(!set.is_running("server-1"));

        set.insert(
            "server-1".to_string(),
            Arc::new(MockTransport {
                closed: AtomicBool::new(false),
            }),
        );
        assert!(set.is_running("server-1"));
        assert!(!set.is_running("server-2"));
    }

    #[tokio::test]
    async fn test_running_server_ids() {
        let set = SessionTransportSet::new();
        set.insert(
            "a".to_string(),
            Arc::new(MockTransport {
                closed: AtomicBool::new(false),
            }),
        );
        set.insert(
            "b".to_string(),
            Arc::new(MockTransport {
                closed: AtomicBool::new(false),
            }),
        );

        let mut ids = set.running_server_ids();
        ids.sort();
        assert_eq!(ids, vec!["a", "b"]);
    }

    #[tokio::test]
    async fn test_send_request() {
        let set = SessionTransportSet::new();
        set.insert(
            "s1".to_string(),
            Arc::new(MockTransport {
                closed: AtomicBool::new(false),
            }),
        );

        let req = JsonRpcRequest::new(Some(serde_json::json!(1)), "test".to_string(), None);
        let resp = set.send_request("s1", req).await;
        assert!(resp.is_ok());

        // Unknown server should error
        let req2 = JsonRpcRequest::new(Some(serde_json::json!(2)), "test".to_string(), None);
        let resp2 = set.send_request("unknown", req2).await;
        assert!(resp2.is_err());
    }

    struct CapturingTransport {
        last: parking_lot::Mutex<Option<JsonRpcRequest>>,
    }

    #[async_trait]
    impl Transport for CapturingTransport {
        async fn send_request(&self, request: JsonRpcRequest) -> AppResult<JsonRpcResponse> {
            *self.last.lock() = Some(request);
            Ok(JsonRpcResponse {
                jsonrpc: "2.0".to_string(),
                id: serde_json::Value::Null,
                result: Some(serde_json::json!({})),
                error: None,
            })
        }

        async fn is_healthy(&self) -> bool {
            true
        }

        async fn close(&self) -> AppResult<()> {
            Ok(())
        }
    }

    #[tokio::test]
    async fn test_stateless_backend_gets_meta_injected() {
        let transport = Arc::new(CapturingTransport {
            last: parking_lot::Mutex::new(None),
        });
        let set = SessionTransportSet::new();
        set.insert("s1".to_string(), transport.clone());
        set.set_revision("s1", crate::protocol::ProtocolRevision::V2026_07_28);
        set.set_client_capabilities(serde_json::json!({ "elicitation": {} }));

        let req = JsonRpcRequest::new(
            Some(serde_json::json!(1)),
            "tools/list".to_string(),
            Some(serde_json::json!({})),
        );
        set.send_request("s1", req).await.unwrap();

        let sent = transport.last.lock().clone().unwrap();
        let meta = &sent.params.unwrap()["_meta"];
        assert_eq!(
            meta["io.modelcontextprotocol/protocolVersion"],
            "2026-07-28"
        );
        assert_eq!(
            meta["io.modelcontextprotocol/clientInfo"]["name"],
            "LocalRouter MCP Gateway"
        );
        assert!(meta["io.modelcontextprotocol/clientCapabilities"]["elicitation"].is_object());
    }

    #[tokio::test]
    async fn test_legacy_backend_request_unchanged() {
        let transport = Arc::new(CapturingTransport {
            last: parking_lot::Mutex::new(None),
        });
        let set = SessionTransportSet::new();
        set.insert("s1".to_string(), transport.clone());
        // No revision recorded: legacy backend

        let req = JsonRpcRequest::new(
            Some(serde_json::json!(1)),
            "tools/list".to_string(),
            Some(serde_json::json!({})),
        );
        set.send_request("s1", req).await.unwrap();

        let sent = transport.last.lock().clone().unwrap();
        // Legacy backends must receive byte-identical requests: no _meta
        assert!(sent.params.unwrap().get("_meta").is_none());
    }

    #[tokio::test]
    async fn test_close_server() {
        let transport = Arc::new(MockTransport {
            closed: AtomicBool::new(false),
        });
        let set = SessionTransportSet::new();
        set.insert("s1".to_string(), transport.clone());

        assert!(set.is_running("s1"));
        set.close_server("s1").await;
        assert!(!set.is_running("s1"));
        assert!(transport.closed.load(Ordering::Relaxed));
    }

    #[tokio::test]
    async fn test_close_all() {
        let t1 = Arc::new(MockTransport {
            closed: AtomicBool::new(false),
        });
        let t2 = Arc::new(MockTransport {
            closed: AtomicBool::new(false),
        });
        let set = SessionTransportSet::new();
        set.insert("s1".to_string(), t1.clone());
        set.insert("s2".to_string(), t2.clone());

        set.close_all().await;
        assert!(!set.is_running("s1"));
        assert!(!set.is_running("s2"));
        assert!(t1.closed.load(Ordering::Relaxed));
        assert!(t2.closed.load(Ordering::Relaxed));
    }
}
