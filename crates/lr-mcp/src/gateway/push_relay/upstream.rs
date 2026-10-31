//! Upstream ingress: demand-driven subscriptions to upstream event sources.
//!
//! One subscription exists per (connection, server, event, canonical
//! arguments) while any client consumes it, however many clients that is.
//! HTTP and WebSocket upstreams are reached over a relay-owned connection
//! shared by every client ([`RelayConnection`]). A stdio upstream runs one
//! process per downstream session, and the events of that process belong to
//! that session's client, so its subscriptions use the session's own
//! connection.
//!
//! Ingress preference is push (`events/stream`, U1) then poll (`events/poll`,
//! U2). A dropped stream reconnects with jittered exponential backoff,
//! resuming from the last upstream cursor; a stream with no message within
//! the heartbeat timeout is treated as dead.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use futures_util::StreamExt;
use serde::Serialize;
use serde_json::{json, Map, Value};
use tokio_util::sync::CancellationToken;

use super::journal::IngressMode;
use super::limits::PushRelayLimits;
use crate::gateway::types::InitializeResult;
use crate::gateway::McpGateway;
use crate::manager::McpServerManager;
use crate::protocol::{JsonRpcNotification, JsonRpcRequest, JsonRpcResponse};
use crate::transport::{RpcStream, RpcStreamMessage, Transport};
use lr_config::EventDeliveryMode;
use lr_types::{AppError, AppResult};

/// Timeout of the relay-owned connection's handshake.
const HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(30);
/// Events requested per upstream `events/poll`.
const UPSTREAM_POLL_BATCH: u64 = 100;
/// Upstream event ids remembered per subscription, so events replayed after
/// a reconnect are not relayed twice.
const REPLAY_WINDOW: usize = 1024;

/// Whose connection a subscription runs on.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize)]
#[serde(rename_all = "snake_case", tag = "kind", content = "session_key")]
pub enum ConnectionScope {
    /// The relay-owned connection to the server, shared by every client.
    Shared,
    /// One downstream session's own connection (stdio upstreams).
    Session(String),
}

/// One upstream subscription.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct SubscriptionKey {
    pub scope: ConnectionScope,
    pub server_id: String,
    /// The upstream's own event name.
    pub event: String,
    /// Canonical JSON of the arguments.
    pub args_key: String,
}

/// What to open for a key.
#[derive(Debug, Clone)]
pub struct SubscriptionSpec {
    pub arguments: Map<String, Value>,
    /// Modes the upstream offers for the event (push and/or poll).
    pub modes: Vec<EventDeliveryMode>,
}

/// Live state of one subscription, for the UI.
#[derive(Debug, Clone, Default, Serialize)]
pub struct SubscriptionStatus {
    /// `push` or `poll` once running.
    pub ingress_mode: Option<String>,
    pub connected: bool,
    pub events_received: u64,
    pub last_event_at: Option<chrono::DateTime<chrono::Utc>>,
    pub last_error: Option<String>,
    pub reconnects: u64,
}

/// One active subscription, for the UI.
#[derive(Debug, Clone, Serialize)]
pub struct SubscriptionInfo {
    pub server_id: String,
    pub event: String,
    pub arguments: Map<String, Value>,
    pub scope: ConnectionScope,
    pub status: SubscriptionStatus,
}

/// Receives each upstream occurrence object (`{eventId, name, timestamp,
/// data, cursor?}`).
pub type IngestFn = Arc<dyn Fn(&SubscriptionKey, Value, IngressMode) + Send + Sync>;

/// Receives `notifications/claude/channel` from a relay-owned connection.
pub type ChannelFn = Arc<dyn Fn(&str, JsonRpcNotification) + Send + Sync>;

/// A connection to one upstream that requests and streams can use.
#[async_trait]
pub trait UpstreamPort: Send + Sync {
    async fn request(&self, method: &str, params: Value) -> AppResult<JsonRpcResponse>;
    async fn open_stream(&self, method: &str, params: Value, buffer: usize)
        -> AppResult<RpcStream>;
    async fn supports_streams(&self) -> bool;
    /// Drop a connection that failed so the next use reconnects.
    async fn reset(&self) {}
}

fn request_id() -> Value {
    json!(format!("_lr_push_{}", uuid::Uuid::new_v4()))
}

/// Create and initialise a relay-owned connection to `server_id`: the
/// transport, the legacy `initialize` handshake and
/// `notifications/initialized`. Notifications the upstream sends on it go to
/// `on_notification`.
pub async fn connect_relay_upstream(
    manager: &McpServerManager,
    server_id: &str,
    on_notification: crate::transport::NotificationCallback,
) -> AppResult<(Arc<dyn Transport>, InitializeResult)> {
    let transport = manager.create_transport(server_id).await?;
    transport.set_notification_callback(on_notification);
    let initialize = JsonRpcRequest::new(
        Some(request_id()),
        "initialize".to_string(),
        Some(json!({
            "protocolVersion": crate::protocol::MCP_PROTOCOL_VERSION,
            "capabilities": {"extensions": {crate::protocol::EVENTS_EXTENSION_ID: {}}},
            "clientInfo": {
                "name": "LocalRouter Push Relay",
                "version": env!("CARGO_PKG_VERSION")
            }
        })),
    );
    let response =
        match tokio::time::timeout(HANDSHAKE_TIMEOUT, transport.send_request(initialize)).await {
            Ok(Ok(response)) => response,
            Ok(Err(e)) => {
                let _ = transport.close().await;
                return Err(e);
            }
            Err(_) => {
                let _ = transport.close().await;
                return Err(AppError::Mcp(format!(
                    "Push Relay connection to {server_id} timed out during initialize"
                )));
            }
        };
    let result = response
        .result
        .filter(|_| response.error.is_none())
        .and_then(|value| serde_json::from_value::<InitializeResult>(value).ok());
    let Some(result) = result else {
        let _ = transport.close().await;
        return Err(AppError::Mcp(format!(
            "Push Relay connection to {server_id}: initialize failed"
        )));
    };
    let initialized = JsonRpcRequest::new(
        None,
        "notifications/initialized".to_string(),
        Some(json!({})),
    );
    let _ = tokio::time::timeout(HANDSHAKE_TIMEOUT, transport.send_request(initialized)).await;
    Ok((transport, result))
}

/// A relay-owned connection to one upstream, connected on first use and
/// re-connected after [`UpstreamPort::reset`].
pub struct RelayConnection {
    manager: Arc<McpServerManager>,
    server_id: String,
    on_channel: ChannelFn,
    transport: tokio::sync::Mutex<Option<Arc<dyn Transport>>>,
}

impl RelayConnection {
    pub fn new(manager: Arc<McpServerManager>, server_id: String, on_channel: ChannelFn) -> Self {
        Self {
            manager,
            server_id,
            on_channel,
            transport: tokio::sync::Mutex::new(None),
        }
    }

    async fn transport(&self) -> AppResult<Arc<dyn Transport>> {
        let mut slot = self.transport.lock().await;
        if let Some(transport) = slot.as_ref() {
            return Ok(transport.clone());
        }
        let server_id = self.server_id.clone();
        let on_channel = self.on_channel.clone();
        let callback: crate::transport::NotificationCallback =
            Arc::new(move |notification: JsonRpcNotification| {
                if notification.method == super::CLAUDE_CHANNEL_NOTIFICATION {
                    on_channel(&server_id, notification);
                }
            });
        let (transport, _) =
            connect_relay_upstream(&self.manager, &self.server_id, callback).await?;
        *slot = Some(transport.clone());
        Ok(transport)
    }

    pub async fn close(&self) {
        if let Some(transport) = self.transport.lock().await.take() {
            let _ = transport.close().await;
        }
    }
}

#[async_trait]
impl UpstreamPort for RelayConnection {
    async fn request(&self, method: &str, params: Value) -> AppResult<JsonRpcResponse> {
        let transport = self.transport().await?;
        let mut request = JsonRpcRequest::new(Some(request_id()), method.to_string(), Some(params));
        crate::protocol::declare_events_on_request(&mut request);
        transport.send_request(request).await
    }

    async fn open_stream(
        &self,
        method: &str,
        params: Value,
        buffer: usize,
    ) -> AppResult<RpcStream> {
        let transport = self.transport().await?;
        let mut request = JsonRpcRequest::new(Some(request_id()), method.to_string(), Some(params));
        crate::protocol::declare_events_on_request(&mut request);
        transport.open_rpc_stream(request, buffer).await
    }

    async fn supports_streams(&self) -> bool {
        match self.transport().await {
            Ok(transport) => transport.supports_rpc_streams(),
            Err(_) => true,
        }
    }

    async fn reset(&self) {
        self.close().await;
    }
}

/// One downstream session's own connection to an upstream.
pub struct SessionPort {
    pub gateway: McpGateway,
    pub session_key: String,
    pub server_id: String,
}

#[async_trait]
impl UpstreamPort for SessionPort {
    async fn request(&self, method: &str, params: Value) -> AppResult<JsonRpcResponse> {
        self.gateway
            .upstream_request(&self.session_key, &self.server_id, method, Some(params))
            .await
    }

    async fn open_stream(
        &self,
        method: &str,
        params: Value,
        buffer: usize,
    ) -> AppResult<RpcStream> {
        self.gateway
            .open_upstream_stream(
                &self.session_key,
                &self.server_id,
                method,
                Some(params),
                buffer,
            )
            .await
    }

    async fn supports_streams(&self) -> bool {
        self.gateway
            .server_manager
            .get_config(&self.server_id)
            .is_some_and(|c| {
                !matches!(
                    c.transport_config,
                    lr_config::McpTransportConfig::WebSocket { .. }
                )
            })
    }
}

/// The next reconnect delay: doubled, capped, jittered into its upper half.
pub fn next_backoff(current: Duration, limits: &PushRelayLimits) -> Duration {
    let min = limits.upstream_reconnect_backoff_min();
    let max = limits.upstream_reconnect_backoff_max().max(min);
    current.saturating_mul(2).clamp(min, max)
}

fn jittered(delay: Duration) -> Duration {
    let fraction = (uuid::Uuid::new_v4().as_u128() % 1000) as u32;
    delay / 2 + delay / 2 * fraction / 1000
}

/// Stream helper result: `Ok` when cancelled, `Err` when it should reconnect.
type RunResult = Result<(), String>;

struct Runner {
    key: SubscriptionKey,
    spec: SubscriptionSpec,
    port: Arc<dyn UpstreamPort>,
    limits: Arc<parking_lot::RwLock<PushRelayLimits>>,
    ingest: IngestFn,
    status: Arc<parking_lot::Mutex<SubscriptionStatus>>,
    cancel: CancellationToken,
    cursor: Option<String>,
    backoff: Duration,
    seen: std::collections::VecDeque<String>,
}

impl Runner {
    fn params(&self, extra: &[(&str, Value)]) -> Value {
        let mut params = json!({
            "name": self.key.event,
            "arguments": self.spec.arguments,
            "cursor": self.cursor,
        });
        for (key, value) in extra {
            params[*key] = value.clone();
        }
        params
    }

    fn deliver(&mut self, mut event: Value, mode: IngressMode) {
        if let Some(cursor) = event.get("cursor").and_then(Value::as_str) {
            self.cursor = Some(cursor.to_string());
        }
        if let Some(obj) = event.as_object_mut() {
            obj.remove("_meta");
        }
        if let Some(id) = event.get("eventId").and_then(Value::as_str) {
            if self.seen.iter().any(|seen| seen == id) {
                return;
            }
            self.seen.push_back(id.to_string());
            if self.seen.len() > REPLAY_WINDOW {
                self.seen.pop_front();
            }
        }
        {
            let mut status = self.status.lock();
            status.events_received += 1;
            status.last_event_at = Some(chrono::Utc::now());
        }
        (self.ingest)(&self.key, event, mode);
    }

    fn mark_connected(&mut self, mode: IngressMode) {
        self.backoff = self.limits.read().upstream_reconnect_backoff_min();
        let mut status = self.status.lock();
        status.connected = true;
        status.ingress_mode = Some(mode.as_str().to_string());
        status.last_error = None;
    }

    async fn run(mut self) {
        self.backoff = self.limits.read().upstream_reconnect_backoff_min();
        loop {
            if self.cancel.is_cancelled() {
                return;
            }
            let push = self.spec.modes.contains(&EventDeliveryMode::Push)
                && self.port.supports_streams().await;
            let poll = self.spec.modes.contains(&EventDeliveryMode::Poll);
            let result = if push {
                self.stream_once().await
            } else if poll {
                self.poll_loop().await
            } else {
                return;
            };
            let Err(error) = result else {
                return;
            };
            tracing::debug!(
                "Push Relay subscription {}/{} reconnecting: {}",
                self.key.server_id,
                self.key.event,
                error
            );
            {
                let mut status = self.status.lock();
                status.connected = false;
                status.last_error = Some(error);
                status.reconnects += 1;
            }
            self.port.reset().await;
            let delay = jittered(self.backoff);
            self.backoff = next_backoff(self.backoff, &self.limits.read());
            tokio::select! {
                _ = self.cancel.cancelled() => return,
                _ = tokio::time::sleep(delay) => {}
            }
        }
    }

    async fn stream_once(&mut self) -> RunResult {
        let mut stream = self
            .port
            .open_stream(
                "events/stream",
                self.params(&[]),
                crate::transport::rpc_stream::DEFAULT_RPC_STREAM_BUFFER,
            )
            .await
            .map_err(|e| e.to_string())?;
        loop {
            let heartbeat = self.limits.read().upstream_heartbeat_timeout();
            let next = tokio::select! {
                _ = self.cancel.cancelled() => return Ok(()),
                next = tokio::time::timeout(heartbeat, stream.next()) => next,
            };
            let message = match next {
                Err(_) => return Err("no heartbeat from upstream stream".into()),
                Ok(None) => return Err("upstream stream ended".into()),
                Ok(Some(Err(e))) => return Err(e.to_string()),
                Ok(Some(Ok(message))) => message,
            };
            match message {
                RpcStreamMessage::Notification(notification) => {
                    let params = notification.params.unwrap_or(Value::Null);
                    match notification.method.as_str() {
                        "notifications/events/event" => {
                            self.mark_connected(IngressMode::Push);
                            self.deliver(params, IngressMode::Push);
                        }
                        "notifications/events/active" | "notifications/events/heartbeat" => {
                            self.mark_connected(IngressMode::Push);
                            if let Some(cursor) = params.get("cursor").and_then(Value::as_str) {
                                self.cursor = Some(cursor.to_string());
                            }
                        }
                        "notifications/events/terminated" => {
                            return Err("upstream terminated the stream".into())
                        }
                        "notifications/events/error" => {
                            tracing::debug!(
                                "Upstream {} reported an events error: {}",
                                self.key.server_id,
                                params
                            );
                        }
                        _ => {}
                    }
                }
                RpcStreamMessage::Result(response) => {
                    return Err(match response.error {
                        Some(error) => format!("upstream stream error: {}", error.message),
                        None => "upstream stream completed".into(),
                    });
                }
            }
        }
    }

    async fn poll_loop(&mut self) -> RunResult {
        loop {
            let params = self.params(&[("maxEvents", json!(UPSTREAM_POLL_BATCH))]);
            let response = tokio::select! {
                _ = self.cancel.cancelled() => return Ok(()),
                response = self.port.request("events/poll", params) => response,
            }
            .map_err(|e| e.to_string())?;
            if let Some(error) = response.error {
                return Err(format!("upstream poll error: {}", error.message));
            }
            let result = response.result.unwrap_or(Value::Null);
            self.mark_connected(IngressMode::Poll);
            if let Some(events) = result.get("events").and_then(Value::as_array) {
                for event in events.clone() {
                    self.deliver(event, IngressMode::Poll);
                }
            }
            if let Some(cursor) = result.get("cursor").and_then(Value::as_str) {
                self.cursor = Some(cursor.to_string());
            }
            if result.get("hasMore").and_then(Value::as_bool) == Some(true) {
                continue;
            }
            let delay = self.limits.read().clamp_upstream_poll_interval(
                result
                    .get("nextPollMs")
                    .and_then(Value::as_u64)
                    .unwrap_or(1_000),
            );
            tokio::select! {
                _ = self.cancel.cancelled() => return Ok(()),
                _ = tokio::time::sleep(delay) => {}
            }
        }
    }
}

struct ActiveSubscription {
    spec: SubscriptionSpec,
    cancel: CancellationToken,
    status: Arc<parking_lot::Mutex<SubscriptionStatus>>,
}

/// Opens, shares and closes upstream subscriptions to match demand.
pub struct UpstreamHub {
    gateway: McpGateway,
    limits: Arc<parking_lot::RwLock<PushRelayLimits>>,
    ingest: IngestFn,
    on_channel: ChannelFn,
    subscriptions: parking_lot::Mutex<HashMap<SubscriptionKey, ActiveSubscription>>,
    connections: parking_lot::Mutex<HashMap<String, Arc<RelayConnection>>>,
}

impl UpstreamHub {
    pub fn new(
        gateway: McpGateway,
        limits: Arc<parking_lot::RwLock<PushRelayLimits>>,
        ingest: IngestFn,
        on_channel: ChannelFn,
    ) -> Self {
        Self {
            gateway,
            limits,
            ingest,
            on_channel,
            subscriptions: parking_lot::Mutex::new(HashMap::new()),
            connections: parking_lot::Mutex::new(HashMap::new()),
        }
    }

    fn port_for(&self, key: &SubscriptionKey) -> Arc<dyn UpstreamPort> {
        match &key.scope {
            ConnectionScope::Session(session_key) => Arc::new(SessionPort {
                gateway: self.gateway.clone(),
                session_key: session_key.clone(),
                server_id: key.server_id.clone(),
            }),
            ConnectionScope::Shared => self
                .connections
                .lock()
                .entry(key.server_id.clone())
                .or_insert_with(|| {
                    Arc::new(RelayConnection::new(
                        self.gateway.server_manager.clone(),
                        key.server_id.clone(),
                        self.on_channel.clone(),
                    ))
                })
                .clone(),
        }
    }

    /// Make the open subscriptions exactly `desired`: start missing ones,
    /// cancel unwanted ones, and close shared connections nothing uses.
    pub fn sync(&self, desired: HashMap<SubscriptionKey, SubscriptionSpec>) {
        let mut subscriptions = self.subscriptions.lock();
        subscriptions.retain(|key, active| {
            let keep = desired
                .get(key)
                .is_some_and(|spec| spec.modes == active.spec.modes);
            if !keep {
                active.cancel.cancel();
            }
            keep
        });
        for (key, spec) in desired {
            if subscriptions.contains_key(&key) {
                continue;
            }
            let cancel = CancellationToken::new();
            let status = Arc::new(parking_lot::Mutex::new(SubscriptionStatus::default()));
            let runner = Runner {
                key: key.clone(),
                spec: spec.clone(),
                port: self.port_for(&key),
                limits: self.limits.clone(),
                ingest: self.ingest.clone(),
                status: status.clone(),
                cancel: cancel.clone(),
                cursor: None,
                backoff: Duration::ZERO,
                seen: std::collections::VecDeque::new(),
            };
            tokio::spawn(runner.run());
            subscriptions.insert(
                key,
                ActiveSubscription {
                    spec,
                    cancel,
                    status,
                },
            );
        }
        let used: HashSet<String> = subscriptions
            .keys()
            .filter(|k| k.scope == ConnectionScope::Shared)
            .map(|k| k.server_id.clone())
            .collect();
        drop(subscriptions);
        let unused: Vec<Arc<RelayConnection>> = {
            let mut connections = self.connections.lock();
            let unused = connections
                .iter()
                .filter(|(id, _)| !used.contains(*id))
                .map(|(_, c)| c.clone())
                .collect();
            connections.retain(|id, _| used.contains(id));
            unused
        };
        for connection in unused {
            tokio::spawn(async move { connection.close().await });
        }
    }

    /// Every open subscription with its status.
    pub fn subscriptions(&self) -> Vec<SubscriptionInfo> {
        self.subscriptions
            .lock()
            .iter()
            .map(|(key, active)| SubscriptionInfo {
                server_id: key.server_id.clone(),
                event: key.event.clone(),
                arguments: active.spec.arguments.clone(),
                scope: key.scope.clone(),
                status: active.status.lock().clone(),
            })
            .collect()
    }

    /// Cancel everything (shutdown).
    pub fn shutdown(&self) {
        self.sync(HashMap::new());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn backoff_doubles_within_bounds() {
        let limits = PushRelayLimits {
            upstream_reconnect_backoff_min_ms: 1_000,
            upstream_reconnect_backoff_max_ms: 5_000,
            ..Default::default()
        };
        assert_eq!(
            next_backoff(Duration::ZERO, &limits),
            Duration::from_secs(1)
        );
        assert_eq!(
            next_backoff(Duration::from_secs(1), &limits),
            Duration::from_secs(2)
        );
        assert_eq!(
            next_backoff(Duration::from_secs(4), &limits),
            Duration::from_secs(5)
        );
        for _ in 0..50 {
            let delay = jittered(Duration::from_secs(4));
            assert!(delay >= Duration::from_secs(2) && delay <= Duration::from_secs(4));
        }
    }
}
