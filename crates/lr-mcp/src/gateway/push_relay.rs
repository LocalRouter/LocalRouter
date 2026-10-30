//! Gateway foundations for Push Relay (`plan/2026-10-30-PUSH_RELAY.md`).
//!
//! The relay itself (event store, ingress, egress) implements
//! [`PushRelayService`] and registers it with
//! [`super::McpGateway::set_push_relay`]. This module holds that contract, the
//! per-session context handed to it, and the capability snapshot machinery
//! that remembers what each upstream offers.

pub mod ask;
pub mod cursor;
pub mod journal;
pub mod relay;
pub mod render;
pub mod tools;
pub mod upstream;
pub mod webhook;

pub use ask::{HeldPushEventInfo, PushDecision, PushDecisionAction};
pub use relay::{
    ClientEventTypeInfo, ClientLookup, HeldChangedListener, PushConfigChange, PushConfigPersister,
    PushRelay, UpstreamSourceInfo, WatchError, WebhookSubscriptionInfo,
};
pub use upstream::{connect_relay_upstream, SubscriptionInfo};

use std::collections::HashMap;
use std::future::Future;
use std::pin::Pin;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use serde_json::Value;

use super::types::{InitializeResult, ServerCapabilities};
use crate::manager::McpServerManager;
use crate::protocol::{JsonRpcNotification, JsonRpcRequest, JsonRpcResponse, ProtocolRevision};
use crate::transport::SessionTransportSet;
use lr_config::{EventDeliveryMode, UpstreamCapabilitySnapshot, UpstreamPushSummary};
use lr_types::AppResult;

/// SEP-3415 extension id of MCP Events.
pub const EVENTS_EXTENSION_ID: &str = "io.modelcontextprotocol/events";
/// Top-level capability some clients read instead of the extension id.
pub const EVENTS_TOP_LEVEL_CAPABILITY: &str = "events";
/// Claude Code Channels experimental capability.
pub const CLAUDE_CHANNEL_CAPABILITY: &str = "claude/channel";
/// Notification a Claude Code channel server sends.
pub const CLAUDE_CHANNEL_NOTIFICATION: &str = "notifications/claude/channel";
/// Prefix of every MCP Events notification.
pub const EVENTS_NOTIFICATION_PREFIX: &str = "notifications/events/";
/// Upstream signal that its event catalogue changed.
pub const EVENTS_LIST_CHANGED_NOTIFICATION: &str = "notifications/events/list_changed";
/// Header the STDIO bridge (`localrouter --mcp-bridge`) sends on every request.
pub const STDIO_BRIDGE_HEADER: &str = "x-localrouter-bridge";
/// Value of [`STDIO_BRIDGE_HEADER`] identifying the STDIO bridge.
pub const STDIO_BRIDGE_HEADER_VALUE: &str = "stdio";

/// How long recorded snapshots are batched before one config write.
pub const SNAPSHOT_PERSIST_DEBOUNCE: Duration = Duration::from_secs(5);
/// Bound on fetching an upstream's event catalogue while recording a snapshot.
const EVENT_CATALOGUE_TIMEOUT: Duration = Duration::from_secs(5);
/// Pages of `events/list` followed while recording a snapshot.
const EVENT_CATALOGUE_MAX_PAGES: usize = 10;

/// The downstream `events/*` methods routed to the relay.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PushMethod {
    List,
    Poll,
    Stream,
    Subscribe,
    Unsubscribe,
}

impl PushMethod {
    pub fn from_method(method: &str) -> Option<Self> {
        match method {
            "events/list" => Some(Self::List),
            "events/poll" => Some(Self::Poll),
            "events/stream" => Some(Self::Stream),
            "events/subscribe" => Some(Self::Subscribe),
            "events/unsubscribe" => Some(Self::Unsubscribe),
            _ => None,
        }
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            Self::List => "events/list",
            Self::Poll => "events/poll",
            Self::Stream => "events/stream",
            Self::Subscribe => "events/subscribe",
            Self::Unsubscribe => "events/unsubscribe",
        }
    }
}

/// What the relay knows about the downstream session a call belongs to.
#[derive(Debug, Clone)]
pub struct PushRelaySessionContext {
    pub client_id: String,
    pub session_key: String,
    pub client_name: String,
    /// The session is served through the STDIO bridge (`X-LocalRouter-Bridge:
    /// stdio`), the only transport that can carry Claude Code Channels.
    pub is_stdio_bridge: bool,
    /// Protocol revision the client speaks.
    pub protocol_revision: ProtocolRevision,
    /// The client's raw capabilities (including `extensions`), when known.
    pub client_capabilities: Option<Value>,
    /// Servers the client may see under its MCP permissions (enabled only).
    pub visible_servers: Vec<String>,
    /// Visible servers whose connection started for this session.
    pub running_servers: Vec<String>,
    /// Broadcast key that delivers a notification to this session only (see
    /// [`super::McpGateway::send_session_notification`]).
    pub notification_key: String,
}

impl PushRelaySessionContext {
    /// The client declared MCP Events support (either capability location).
    pub fn client_declares_events(&self) -> bool {
        self.client_capabilities.as_ref().is_some_and(|caps| {
            caps.get("extensions")
                .and_then(|e| e.get(EVENTS_EXTENSION_ID))
                .is_some()
                || caps.get(EVENTS_TOP_LEVEL_CAPABILITY).is_some()
        })
    }
}

/// Capabilities the relay asks the gateway to advertise to one session.
/// Upstream `experimental` capabilities and the events extension are never
/// forwarded on their own; this is the only way they reach a client.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct PushAdvertisement {
    /// Added to `capabilities.extensions` (e.g. the events extension).
    pub extensions: serde_json::Map<String, Value>,
    /// Added at the top level of `capabilities` (e.g. `events`).
    pub capabilities: serde_json::Map<String, Value>,
    /// Added to `capabilities.experimental` (e.g. `claude/channel`).
    pub experimental: serde_json::Map<String, Value>,
}

impl PushAdvertisement {
    pub fn is_empty(&self) -> bool {
        self.extensions.is_empty() && self.capabilities.is_empty() && self.experimental.is_empty()
    }

    /// Merge into a serialised capabilities object.
    pub fn apply_to(&self, capabilities: &mut Value) {
        let Some(caps) = capabilities.as_object_mut() else {
            return;
        };
        for (key, value) in &self.capabilities {
            caps.insert(key.clone(), value.clone());
        }
        for (field, entries) in [
            ("extensions", &self.extensions),
            ("experimental", &self.experimental),
        ] {
            if entries.is_empty() {
                continue;
            }
            let target = caps
                .entry(field)
                .or_insert_with(|| Value::Object(Default::default()));
            if !target.is_object() {
                *target = Value::Object(Default::default());
            }
            if let Some(target) = target.as_object_mut() {
                for (key, value) in entries {
                    target.insert(key.clone(), value.clone());
                }
            }
        }
    }
}

/// A `subscriptions/listen` opt-in key the relay accepts (SEP-2575 has no
/// key for events, so the relay names its own), and the notifications it
/// lets through: every method equal to or starting with `method_prefix`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ListenOptIn {
    pub opt_in: String,
    pub method_prefix: String,
}

impl ListenOptIn {
    pub fn matches(&self, method: &str) -> bool {
        method.starts_with(&self.method_prefix)
    }
}

/// A push notification an upstream sent on a session's connection:
/// `notifications/claude/channel`, or an `notifications/events/*` not
/// correlated with an open [`crate::transport::RpcStream`].
#[derive(Debug, Clone)]
pub struct UpstreamPushNotification {
    pub server_id: String,
    pub session_key: String,
    pub client_id: String,
    pub notification: JsonRpcNotification,
}

/// Per-request channels for the notifications of a downstream
/// `events/stream`, registered by a transport that answers the request with
/// its own response stream (the HTTP route's SSE body). Without a sink the
/// relay falls back to [`super::McpGateway::send_session_notification`].
#[derive(Default)]
pub struct PushStreamSinks {
    sinks: dashmap::DashMap<String, tokio::sync::mpsc::Sender<JsonRpcNotification>>,
}

/// Removes its sink when dropped.
pub struct PushStreamSinkGuard {
    sinks: Arc<PushStreamSinks>,
    key: String,
}

impl Drop for PushStreamSinkGuard {
    fn drop(&mut self) {
        self.sinks.sinks.remove(&self.key);
    }
}

impl PushStreamSinks {
    fn key(session_key: &str, request_id: &Value) -> String {
        format!(
            "{session_key}\u{1f}{}",
            crate::transport::rpc_stream::id_key(request_id)
        )
    }

    /// Register a sink for one request; it lives until the guard is dropped.
    pub fn register(
        self: &Arc<Self>,
        session_key: &str,
        request_id: &Value,
        buffer: usize,
    ) -> (
        tokio::sync::mpsc::Receiver<JsonRpcNotification>,
        PushStreamSinkGuard,
    ) {
        let (tx, rx) = tokio::sync::mpsc::channel(buffer.max(1));
        let key = Self::key(session_key, request_id);
        self.sinks.insert(key.clone(), tx);
        (
            rx,
            PushStreamSinkGuard {
                sinks: self.clone(),
                key,
            },
        )
    }

    /// The sink registered for a request, if any.
    pub fn sink(
        &self,
        session_key: &str,
        request_id: &Value,
    ) -> Option<tokio::sync::mpsc::Sender<JsonRpcNotification>> {
        self.sinks
            .get(&Self::key(session_key, request_id))
            .map(|s| s.clone())
    }
}

/// An upstream whose remembered capabilities include push.
#[derive(Debug, Clone)]
pub struct RememberedPushSource {
    pub server_id: String,
    pub server_name: String,
    pub snapshot: UpstreamCapabilitySnapshot,
}

/// The relay as seen by the gateway. With no service registered the gateway
/// behaves as before: `events/*` are method-not-found, nothing extra is
/// advertised, and upstream push notifications are forwarded raw.
#[async_trait]
pub trait PushRelayService: Send + Sync {
    /// Capabilities to advertise on `initialize` / `server/discover`.
    fn advertisement(&self, ctx: &PushRelaySessionContext) -> PushAdvertisement;

    /// An instructions section for this session (e.g. the `<events>` block),
    /// added to the gateway instructions at priority 25.
    fn instructions(&self, _ctx: &PushRelaySessionContext) -> Option<String> {
        None
    }

    /// Extra `subscriptions/listen` opt-ins this client may use.
    fn listen_opt_ins(&self, _client_id: &str, _session_key: &str) -> Vec<ListenOptIn> {
        Vec::new()
    }

    /// An upstream push notification arrived. Return `true` when the relay
    /// took it (it is then not forwarded raw to the session's client).
    fn on_upstream_notification(&self, _notification: UpstreamPushNotification) -> bool {
        false
    }

    /// The client sent `notifications/cancelled` for one of its requests on
    /// this session (e.g. an open `events/stream`).
    fn on_client_cancelled(&self, _session_key: &str, _request_id: &Value) {}

    async fn events_list(
        &self,
        ctx: &PushRelaySessionContext,
        request: JsonRpcRequest,
    ) -> AppResult<JsonRpcResponse>;

    async fn events_poll(
        &self,
        ctx: &PushRelaySessionContext,
        request: JsonRpcRequest,
    ) -> AppResult<JsonRpcResponse>;

    /// Held for the life of the stream: the gateway awaits this without
    /// holding any session lock. Deliver notifications through
    /// [`super::McpGateway::send_session_notification`] or the downstream
    /// transport the relay manages.
    async fn events_stream(
        &self,
        ctx: &PushRelaySessionContext,
        request: JsonRpcRequest,
    ) -> AppResult<JsonRpcResponse>;

    async fn events_subscribe(
        &self,
        ctx: &PushRelaySessionContext,
        request: JsonRpcRequest,
    ) -> AppResult<JsonRpcResponse>;

    async fn events_unsubscribe(
        &self,
        ctx: &PushRelaySessionContext,
        request: JsonRpcRequest,
    ) -> AppResult<JsonRpcResponse>;
}

/// Delivery modes of one `events/list` entry. A missing `delivery` field
/// means poll, the one mode every SEP-3415 server supports.
pub(crate) fn delivery_modes(entry: &Value) -> Vec<EventDeliveryMode> {
    let Some(modes) = entry.get("delivery").and_then(Value::as_array) else {
        return vec![EventDeliveryMode::Poll];
    };
    let mut modes: Vec<EventDeliveryMode> = modes
        .iter()
        .filter_map(|m| match m.as_str()? {
            "push" => Some(EventDeliveryMode::Push),
            "poll" => Some(EventDeliveryMode::Poll),
            "webhook" => Some(EventDeliveryMode::Webhook),
            _ => None,
        })
        .collect();
    modes.sort();
    modes.dedup();
    modes
}

/// The push an upstream offers, from its capabilities and event catalogue.
pub fn derive_push_summary(
    capabilities: &ServerCapabilities,
    event_catalogue: &[Value],
) -> UpstreamPushSummary {
    let events_modes = event_catalogue
        .iter()
        .filter_map(|entry| {
            let name = entry.get("name")?.as_str()?;
            Some((name.to_string(), delivery_modes(entry)))
        })
        .collect();
    UpstreamPushSummary {
        events_modes,
        channels: capabilities.has_claude_channel(),
        resource_subscribe: capabilities.has_resource_subscribe(),
    }
}

/// Entries and the next-page cursor of one `events/list` result.
pub fn event_catalogue_page(result: &Value) -> (Vec<Value>, Option<String>) {
    let entries = result
        .get("events")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let next = result
        .get("nextCursor")
        .and_then(Value::as_str)
        .map(str::to_string);
    (entries, next)
}

/// Build a snapshot of what `server_id` offered on this connection,
/// fetching its event catalogue when it declares events.
pub(crate) async fn observe_upstream(
    manager: &McpServerManager,
    transports: &SessionTransportSet,
    server_id: &str,
    init: &InitializeResult,
) -> Option<UpstreamCapabilitySnapshot> {
    let config = manager.get_config(server_id)?;
    let capabilities = serde_json::to_value(&init.capabilities).ok()?;
    let mut catalogue = Vec::new();
    if init.capabilities.has_events() {
        let mut cursor: Option<String> = None;
        for page in 0..EVENT_CATALOGUE_MAX_PAGES {
            let params = match &cursor {
                Some(cursor) => serde_json::json!({ "cursor": cursor }),
                None => serde_json::json!({}),
            };
            let request = JsonRpcRequest::new(
                Some(serde_json::json!(format!("_lr_events_list_{page}"))),
                "events/list".to_string(),
                Some(params),
            );
            let response = tokio::time::timeout(
                EVENT_CATALOGUE_TIMEOUT,
                transports.send_request(server_id, request),
            )
            .await;
            let Ok(Ok(response)) = response else {
                tracing::debug!("Could not read the event catalogue of {}", server_id);
                break;
            };
            let Some(result) = response.result.filter(|_| response.error.is_none()) else {
                break;
            };
            let (entries, next) = event_catalogue_page(&result);
            catalogue.extend(entries);
            match next {
                Some(next) if Some(&next) != cursor.as_ref() => cursor = Some(next),
                _ => break,
            }
        }
    }
    Some(UpstreamCapabilitySnapshot {
        fingerprint: lr_config::fingerprint_for(&config),
        protocol_version: init.protocol_version.clone(),
        push: derive_push_summary(&init.capabilities, &catalogue),
        capabilities,
        event_catalogue: catalogue,
        observed_at: chrono::Utc::now(),
    })
}

/// Writes recorded snapshots to the config file. Receives every snapshot
/// whose content changed since the last call, keyed by server id.
pub type CapabilitySnapshotPersister = Arc<
    dyn Fn(HashMap<String, UpstreamCapabilitySnapshot>) -> Pin<Box<dyn Future<Output = ()> + Send>>
        + Send
        + Sync,
>;

/// Remembers upstream capabilities: updates the in-memory server config at
/// once and persists changes in debounced batches.
pub(crate) struct SnapshotRecorder {
    pending: parking_lot::Mutex<HashMap<String, UpstreamCapabilitySnapshot>>,
    flush_scheduled: AtomicBool,
    persister: parking_lot::RwLock<Option<CapabilitySnapshotPersister>>,
    debounce: parking_lot::RwLock<Duration>,
}

impl SnapshotRecorder {
    pub(crate) fn new() -> Self {
        Self {
            pending: parking_lot::Mutex::new(HashMap::new()),
            flush_scheduled: AtomicBool::new(false),
            persister: parking_lot::RwLock::new(None),
            debounce: parking_lot::RwLock::new(SNAPSHOT_PERSIST_DEBOUNCE),
        }
    }

    pub(crate) fn set_persister(&self, persister: CapabilitySnapshotPersister) {
        *self.persister.write() = Some(persister);
    }

    pub(crate) fn set_debounce(&self, debounce: Duration) {
        *self.debounce.write() = debounce;
    }

    /// Record a snapshot. Returns `false` (and writes nothing) when the
    /// server already has the same content remembered.
    pub(crate) fn record(
        self: &Arc<Self>,
        manager: &McpServerManager,
        server_id: &str,
        snapshot: UpstreamCapabilitySnapshot,
    ) -> bool {
        let Some(config) = manager.get_config(server_id) else {
            return false;
        };
        if config
            .capability_snapshot
            .as_ref()
            .is_some_and(|existing| existing.same_content(&snapshot))
        {
            return false;
        }
        manager.set_capability_snapshot(server_id, Some(snapshot.clone()));
        self.pending.lock().insert(server_id.to_string(), snapshot);
        if !self.flush_scheduled.swap(true, Ordering::SeqCst) {
            let recorder = self.clone();
            let debounce = *self.debounce.read();
            tokio::spawn(async move {
                tokio::time::sleep(debounce).await;
                recorder.flush_scheduled.store(false, Ordering::SeqCst);
                let batch = std::mem::take(&mut *recorder.pending.lock());
                let persister = recorder.persister.read().clone();
                if let (false, Some(persister)) = (batch.is_empty(), persister) {
                    persister(batch).await;
                }
            });
        }
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn capabilities(value: Value) -> ServerCapabilities {
        serde_json::from_value(value).unwrap()
    }

    #[test]
    fn summary_reads_catalogue_modes_and_capabilities() {
        let caps = capabilities(json!({
            "resources": { "subscribe": true },
            "experimental": { "claude/channel": {} },
            "events": {}
        }));
        let catalogue = vec![
            json!({"name": "a", "delivery": ["push", "poll", "push"]}),
            json!({"name": "b", "delivery": ["webhook", "future-mode"]}),
            json!({"name": "c"}),
            json!({"description": "nameless"}),
        ];
        let summary = derive_push_summary(&caps, &catalogue);
        assert!(summary.channels);
        assert!(summary.resource_subscribe);
        assert_eq!(
            summary.events_modes["a"],
            vec![EventDeliveryMode::Push, EventDeliveryMode::Poll]
        );
        assert_eq!(summary.events_modes["b"], vec![EventDeliveryMode::Webhook]);
        assert_eq!(summary.events_modes["c"], vec![EventDeliveryMode::Poll]);
        assert_eq!(summary.events_modes.len(), 3);
    }

    #[test]
    fn catalogue_page_reads_entries_and_cursor() {
        let (entries, next) =
            event_catalogue_page(&json!({"events": [{"name": "a"}], "nextCursor": "c2"}));
        assert_eq!(entries.len(), 1);
        assert_eq!(next.as_deref(), Some("c2"));
        let (entries, next) = event_catalogue_page(&json!({"resultType": "complete"}));
        assert!(entries.is_empty());
        assert!(next.is_none());
    }

    #[test]
    fn advertisement_merges_into_existing_capabilities() {
        let mut caps = json!({
            "tools": {"listChanged": true},
            "extensions": {"other/ext": {}}
        });
        let mut ad = PushAdvertisement::default();
        assert!(ad.is_empty());
        ad.extensions
            .insert(EVENTS_EXTENSION_ID.into(), json!({"listChanged": true}));
        ad.capabilities
            .insert(EVENTS_TOP_LEVEL_CAPABILITY.into(), json!({}));
        ad.experimental
            .insert(CLAUDE_CHANNEL_CAPABILITY.into(), json!({}));
        ad.apply_to(&mut caps);
        assert_eq!(caps["extensions"]["other/ext"], json!({}));
        assert_eq!(
            caps["extensions"][EVENTS_EXTENSION_ID],
            json!({"listChanged": true})
        );
        assert_eq!(caps["events"], json!({}));
        assert_eq!(caps["experimental"]["claude/channel"], json!({}));
        assert_eq!(caps["tools"], json!({"listChanged": true}));
    }

    #[test]
    fn client_events_declaration_is_read_from_either_location() {
        let mut ctx = PushRelaySessionContext {
            client_id: "c".into(),
            session_key: "s".into(),
            client_name: String::new(),
            is_stdio_bridge: false,
            protocol_revision: ProtocolRevision::default(),
            client_capabilities: None,
            visible_servers: vec![],
            running_servers: vec![],
            notification_key: String::new(),
        };
        assert!(!ctx.client_declares_events());
        ctx.client_capabilities = Some(json!({"extensions": {EVENTS_EXTENSION_ID: {}}}));
        assert!(ctx.client_declares_events());
        ctx.client_capabilities = Some(json!({"events": {}}));
        assert!(ctx.client_declares_events());
        ctx.client_capabilities = Some(json!({"sampling": {}}));
        assert!(!ctx.client_declares_events());
    }

    fn snapshot_for(
        server: &lr_config::McpServerConfig,
        version: &str,
    ) -> UpstreamCapabilitySnapshot {
        UpstreamCapabilitySnapshot {
            fingerprint: lr_config::fingerprint_for(server),
            protocol_version: version.to_string(),
            capabilities: json!({"events": {}}),
            push: UpstreamPushSummary::default(),
            event_catalogue: vec![],
            observed_at: chrono::Utc::now(),
        }
    }

    /// Changed snapshots update the in-memory config at once and are
    /// persisted in one debounced batch; unchanged content writes nothing.
    #[tokio::test]
    async fn recorder_persists_changes_once_per_batch() {
        let manager = McpServerManager::new_for_test();
        let mut servers = Vec::new();
        for name in ["a", "b"] {
            let server = lr_config::McpServerConfig::new(
                name.to_string(),
                lr_config::McpTransportType::HttpSse,
                lr_config::McpTransportConfig::HttpSse {
                    url: format!("http://127.0.0.1:9/{name}"),
                    headers: HashMap::new(),
                },
            );
            manager.add_config(server.clone());
            servers.push(server);
        }
        let batches = Arc::new(parking_lot::Mutex::new(Vec::new()));
        let seen = batches.clone();
        let recorder = Arc::new(SnapshotRecorder::new());
        recorder.set_debounce(Duration::from_millis(50));
        recorder.set_persister(Arc::new(move |batch| {
            seen.lock().push(batch);
            Box::pin(async {})
        }));

        assert!(recorder.record(&manager, &servers[0].id, snapshot_for(&servers[0], "v1")));
        assert!(recorder.record(&manager, &servers[1].id, snapshot_for(&servers[1], "v1")));
        // Visible immediately, before the write
        assert!(manager
            .get_config(&servers[0].id)
            .unwrap()
            .current_capability_snapshot()
            .is_some());
        // Same content again (only the time differs): not a change
        assert!(!recorder.record(&manager, &servers[0].id, snapshot_for(&servers[0], "v1")));
        // Unknown servers are ignored
        assert!(!recorder.record(&manager, "missing", snapshot_for(&servers[0], "v1")));

        tokio::time::sleep(Duration::from_millis(200)).await;
        {
            let batches = batches.lock();
            assert_eq!(batches.len(), 1, "one write for the burst");
            assert_eq!(batches[0].len(), 2);
        }

        assert!(!recorder.record(&manager, &servers[1].id, snapshot_for(&servers[1], "v1")));
        tokio::time::sleep(Duration::from_millis(200)).await;
        assert_eq!(batches.lock().len(), 1, "no write without a change");

        assert!(recorder.record(&manager, &servers[1].id, snapshot_for(&servers[1], "v2")));
        tokio::time::sleep(Duration::from_millis(200)).await;
        let batches = batches.lock();
        assert_eq!(batches.len(), 2);
        assert_eq!(batches[1][&servers[1].id].protocol_version, "v2");
    }

    /// A configuration copy without a snapshot does not erase a current one,
    /// but a changed identity drops it.
    #[test]
    fn manager_keeps_current_snapshot_across_config_reloads() {
        let manager = McpServerManager::new_for_test();
        let server = lr_config::McpServerConfig::new(
            "a".to_string(),
            lr_config::McpTransportType::HttpSse,
            lr_config::McpTransportConfig::HttpSse {
                url: "http://127.0.0.1:9/a".into(),
                headers: HashMap::new(),
            },
        );
        manager.add_config(server.clone());
        manager.set_capability_snapshot(&server.id, Some(snapshot_for(&server, "v1")));

        let mut renamed = server.clone();
        renamed.name = "renamed".into();
        manager.add_config(renamed);
        assert!(manager
            .get_config(&server.id)
            .unwrap()
            .capability_snapshot
            .is_some());

        let mut moved = server.clone();
        moved.transport_config = lr_config::McpTransportConfig::HttpSse {
            url: "http://127.0.0.1:9/elsewhere".into(),
            headers: HashMap::new(),
        };
        manager.add_config(moved);
        assert!(manager
            .get_config(&server.id)
            .unwrap()
            .capability_snapshot
            .is_none());
    }

    #[test]
    fn push_methods_round_trip() {
        for method in [
            "events/list",
            "events/poll",
            "events/stream",
            "events/subscribe",
            "events/unsubscribe",
        ] {
            assert_eq!(PushMethod::from_method(method).unwrap().as_str(), method);
        }
        assert!(PushMethod::from_method("events/other").is_none());
        let opt_in = ListenOptIn {
            opt_in: "events".into(),
            method_prefix: EVENTS_NOTIFICATION_PREFIX.into(),
        };
        assert!(opt_in.matches("notifications/events/event"));
        assert!(!opt_in.matches(CLAUDE_CHANNEL_NOTIFICATION));
    }
}
