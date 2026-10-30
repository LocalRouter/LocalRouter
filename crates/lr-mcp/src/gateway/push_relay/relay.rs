//! The Push Relay service (`plan/2026-10-30-PUSH_RELAY.md`).
//!
//! Upstream occurrences (ingress) are normalised, checked against each
//! consuming client's permissions, held for approval when the client's
//! permission is Ask, and appended to that client's journal. Every delivery
//! mode (egress) reads the journal from its own position.

use std::collections::{HashMap, HashSet, VecDeque};
use std::future::Future;
use std::hash::{Hash, Hasher};
use std::pin::Pin;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Weak};
use std::time::{Duration, Instant};

use async_trait::async_trait;
use dashmap::{DashMap, DashSet};
use futures_util::stream::BoxStream;
use parking_lot::{Mutex, RwLock};
use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};
use tokio::sync::{mpsc, Notify};
use tokio_util::sync::CancellationToken;

use super::ask::{AskQueues, HeldEvent, HeldPushEventInfo, PushDecision, PushDecisionAction};
use super::cursor::{canonical_json, CursorError, CursorKey, CursorPosition, CursorScope};
use super::journal::{ClientJournal, IngressMode, JournalLimits, Occurrence, ReadScope};
use super::render;
use super::upstream::{
    ConnectionScope, SubscriptionInfo, SubscriptionKey, SubscriptionSpec, UpstreamHub,
};
use super::webhook;
use super::{
    PushAdvertisement, PushRelayService, PushRelaySessionContext, UpstreamPushNotification,
    CLAUDE_CHANNEL_CAPABILITY, CLAUDE_CHANNEL_NOTIFICATION, EVENTS_EXTENSION_ID,
    EVENTS_LIST_CHANGED_NOTIFICATION, EVENTS_NOTIFICATION_PREFIX, EVENTS_TOP_LEVEL_CAPABILITY,
};
use crate::gateway::firewall::{InterceptCategory, PushApprovalKind, PushEventApprovalDetails};
use crate::gateway::McpGateway;
use crate::protocol::{JsonRpcError, JsonRpcNotification, JsonRpcRequest, JsonRpcResponse};
use crate::transport::SUBSCRIPTION_ID_META_KEY;
use lr_config::{EventDeliveryMode, PushFallbackTools, PushPermission, PushRelayConfig};
use lr_types::AppResult;

/// SEP-3415 `NotFound`.
pub const ERROR_NOT_FOUND: i32 = -32023;
/// SEP-3415 `ResourceExhausted`.
pub const ERROR_RESOURCE_EXHAUSTED: i32 = -32025;
/// SEP-3415 `Unsupported`.
pub const ERROR_UNSUPPORTED: i32 = -32026;

/// Synthetic event type of an upstream Claude Code channel.
pub const CHANNEL_EVENT_NAME: &str = "channel.message";

/// How long a poll keeps its upstream subscription open after the last read.
const POLL_LEASE: Duration = Duration::from_secs(10 * 60);
/// How often expired demand, sessions and leases are swept.
const SWEEP_INTERVAL: Duration = Duration::from_secs(10);
/// `nextPollMs` LocalRouter suggests to pollers.
const DOWNSTREAM_NEXT_POLL_MS: u64 = 1_000;
/// Default and largest `EventPoll` page.
const TOOL_POLL_DEFAULT: usize = 20;
const TOOL_POLL_MAX: usize = 100;
/// Recent event ids a channel pump remembers to skip duplicates.
const CHANNEL_DEDUPE_WINDOW: usize = 512;

/// Looks up a client's current configuration.
pub type ClientLookup = Arc<dyn Fn(&str) -> Option<lr_config::Client> + Send + Sync>;

/// A change Push Relay writes back to a client's configuration.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PushConfigChange {
    /// "Allow / Deny this event type permanently".
    EventPermission {
        server_id: String,
        event_name: String,
        permission: PushPermission,
    },
    /// "Always allow" a webhook callback URL.
    WebhookUrl { url: String },
}

/// Persists [`PushConfigChange`]s for a client (wired to the config file).
pub type PushConfigPersister = Arc<
    dyn Fn(String, PushConfigChange) -> Pin<Box<dyn Future<Output = Result<(), String>> + Send>>
        + Send
        + Sync,
>;

/// One event type a client may receive.
#[derive(Debug, Clone)]
pub struct CatalogueEntry {
    pub qualified_name: String,
    pub server_id: String,
    pub server_slug: String,
    pub server_name: String,
    pub upstream_name: String,
    /// The upstream's `events/list` entry (empty for channel events).
    pub descriptor: Value,
    /// Push/poll modes the upstream offers (empty for channel events).
    pub modes: Vec<EventDeliveryMode>,
    pub is_channel: bool,
    pub permission: PushPermission,
}

/// An upstream server as the UI shows it.
#[derive(Debug, Clone, Serialize)]
pub struct UpstreamSourceInfo {
    pub server_id: String,
    pub server_name: String,
    pub enabled: bool,
    /// `stdio`, `http` or `websocket`.
    pub transport: String,
    /// `shared` (one relay-owned connection) or `per_session` (stdio).
    pub connection: String,
    /// A snapshot exists and still matches the server's configuration.
    pub snapshot_current: bool,
    pub protocol_version: Option<String>,
    pub observed_at: Option<chrono::DateTime<chrono::Utc>>,
    pub channels: bool,
    pub resource_subscribe: bool,
    pub event_types: Vec<UpstreamEventTypeInfo>,
}

/// One upstream event type as the UI shows it.
#[derive(Debug, Clone, Serialize)]
pub struct UpstreamEventTypeInfo {
    pub name: String,
    pub description: Option<String>,
    /// Modes the upstream offers (`push`, `poll`, `webhook`).
    pub modes: Vec<String>,
    /// The mode LocalRouter consumes (`push` preferred, else `poll`).
    pub ingress: Option<String>,
    pub supported: bool,
    pub unsupported_reason: Option<String>,
}

/// One event type of one client, with its effective permission.
#[derive(Debug, Clone, Serialize)]
pub struct ClientEventTypeInfo {
    pub qualified_name: String,
    pub server_id: String,
    pub event_name: String,
    pub permission: PushPermission,
    /// A session-scoped decision overrides the configured permission.
    pub session_override: Option<PushPermission>,
}

/// A client's webhook subscription as the UI shows it.
#[derive(Debug, Clone, Serialize)]
pub struct WebhookSubscriptionInfo {
    pub id: String,
    pub client_id: String,
    pub event: String,
    pub url: String,
    /// `awaiting_approval`, `active`, `suspended`, `denied`.
    pub state: String,
    pub expires_at: chrono::DateTime<chrono::Utc>,
    pub delivered: u64,
    pub last_error: Option<String>,
}

/// Which fallback tools a session sees.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ToolsView {
    pub gate: bool,
    pub list: bool,
    pub poll: bool,
    pub wait: bool,
}

/// What the `_push_relay` virtual server knows about a session.
#[derive(Debug, Clone, Default)]
pub struct PushToolsState {
    pub client_id: String,
    pub session_key: Option<String>,
    pub fallback: PushFallbackTools,
    pub channels_launched: bool,
    pub is_stdio_bridge: bool,
    pub declares_events: bool,
    pub visible_servers: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum HolderKind {
    Stream,
    Poll,
    Channel,
    Webhook,
    Watch,
    Tools,
}

impl HolderKind {
    fn egress(&self) -> &'static str {
        match self {
            Self::Stream => "stream",
            Self::Poll => "poll",
            Self::Channel => "channel",
            Self::Webhook => "webhook",
            Self::Watch | Self::Tools => "tools",
        }
    }
}

/// One consumer's demand for upstream subscriptions.
struct Holder {
    client_id: String,
    kind: HolderKind,
    keys: HashMap<SubscriptionKey, SubscriptionSpec>,
    expires: Option<Instant>,
}

struct SessionInfo {
    ctx: PushRelaySessionContext,
    /// Names this session called an `events/*` method for: their channel
    /// copies stop.
    claimed: HashSet<String>,
    streams: usize,
    channel: Option<CancellationToken>,
    signature: Option<u64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum WebhookState {
    AwaitingApproval,
    Active,
    Suspended,
    Denied,
}

impl WebhookState {
    fn as_str(&self) -> &'static str {
        match self {
            Self::AwaitingApproval => "awaiting_approval",
            Self::Active => "active",
            Self::Suspended => "suspended",
            Self::Denied => "denied",
        }
    }
}

struct WebhookSubscription {
    id: String,
    client_id: String,
    client_name: String,
    entry: CatalogueEntry,
    arguments: Map<String, Value>,
    url: String,
    secret: Vec<u8>,
    expires_at: Mutex<chrono::DateTime<chrono::Utc>>,
    position: AtomicU64,
    state: Mutex<WebhookState>,
    delivered: AtomicU64,
    last_error: Mutex<Option<String>>,
    cancel: CancellationToken,
}

struct AskState {
    queues: AskQueues,
    workers: HashSet<(String, String)>,
}

/// Parameters shared by `events/poll`, `events/stream` and `events/subscribe`.
struct EventParams {
    name: String,
    arguments: Map<String, Value>,
    cursor: Option<String>,
    max_events: Option<u64>,
}

fn parse_event_params(params: Option<&Value>) -> Result<EventParams, String> {
    let params = params
        .and_then(Value::as_object)
        .ok_or("params are required")?;
    let name = params
        .get("name")
        .and_then(Value::as_str)
        .ok_or("name is required")?
        .to_string();
    let arguments = match params.get("arguments") {
        None | Some(Value::Null) => Map::new(),
        Some(Value::Object(map)) => map.clone(),
        Some(_) => return Err("arguments must be an object".into()),
    };
    let cursor = match params.get("cursor") {
        None | Some(Value::Null) => None,
        Some(Value::String(s)) => Some(s.clone()),
        Some(_) => return Err("cursor must be a string or null".into()),
    };
    let max_events = match params.get("maxEvents") {
        None | Some(Value::Null) => None,
        Some(value) => Some(
            value
                .as_u64()
                .filter(|n| *n > 0)
                .ok_or("maxEvents must be a positive integer")?,
        ),
    };
    Ok(EventParams {
        name,
        arguments,
        cursor,
        max_events,
    })
}

fn error_response(id: Value, code: i32, message: &str, data: Option<Value>) -> JsonRpcResponse {
    JsonRpcResponse::error(id, JsonRpcError::custom(code, message, data))
}

fn invalid_params(id: Value, message: impl Into<String>) -> JsonRpcResponse {
    JsonRpcResponse::error(id, JsonRpcError::invalid_params(message))
}

fn not_found(id: Value) -> JsonRpcResponse {
    error_response(
        id,
        ERROR_NOT_FOUND,
        "NotFound",
        Some(json!({"kind": "event"})),
    )
}

fn notification(method: &str, params: Value) -> JsonRpcNotification {
    JsonRpcNotification {
        jsonrpc: "2.0".to_string(),
        method: method.to_string(),
        params: Some(params),
    }
}

fn tag_subscription(mut params: Value, id: &Value) -> Value {
    if let Some(obj) = params.as_object_mut() {
        obj.insert("_meta".to_string(), json!({ SUBSCRIPTION_ID_META_KEY: id }));
    }
    params
}

fn session_stream_key(session_key: &str, id: &Value) -> String {
    format!(
        "{session_key}\u{1f}{}",
        crate::transport::rpc_stream::id_key(id)
    )
}

/// The Push Relay. Register with [`McpGateway::set_push_relay`] and its
/// fallback tools with [`McpGateway::register_virtual_server`]
/// ([`Self::tools_server`]).
pub struct PushRelay {
    weak: Weak<PushRelay>,
    gateway: McpGateway,
    config: RwLock<PushRelayConfig>,
    limits: Arc<RwLock<lr_config::PushRelayLimits>>,
    clients: ClientLookup,
    persister: RwLock<Option<PushConfigPersister>>,
    cursor_key: CursorKey,
    journals: DashMap<String, Arc<Mutex<ClientJournal>>>,
    sessions: DashMap<String, SessionInfo>,
    holders: Mutex<HashMap<String, Holder>>,
    hub: UpstreamHub,
    ask: Mutex<AskState>,
    ask_wakers: DashMap<(String, String), Arc<Notify>>,
    overrides: DashMap<(String, String, String), PushPermission>,
    streams: DashMap<String, CancellationToken>,
    webhooks: DashMap<String, Arc<WebhookSubscription>>,
    approved_urls: DashSet<(String, String)>,
    local_base_url: RwLock<Option<String>>,
    background: AtomicBool,
    http: reqwest::Client,
}

impl PushRelay {
    pub fn new(gateway: McpGateway, config: PushRelayConfig, clients: ClientLookup) -> Arc<Self> {
        let limits = Arc::new(RwLock::new(config.limits.clone()));
        Arc::new_cyclic(|weak: &Weak<PushRelay>| {
            let ingest_weak = weak.clone();
            let channel_weak = weak.clone();
            let hub = UpstreamHub::new(
                gateway.clone(),
                limits.clone(),
                Arc::new(move |key, event, mode| {
                    if let Some(relay) = ingest_weak.upgrade() {
                        relay.ingest(key, event, mode);
                    }
                }),
                Arc::new(move |server_id, notification| {
                    if let Some(relay) = channel_weak.upgrade() {
                        relay.ingest_shared_channel(server_id, notification);
                    }
                }),
            );
            Self {
                weak: weak.clone(),
                http: webhook::http_client(config.limits.webhook_timeout()),
                gateway,
                config: RwLock::new(config),
                limits,
                clients,
                persister: RwLock::new(None),
                cursor_key: CursorKey::new(),
                journals: DashMap::new(),
                sessions: DashMap::new(),
                holders: Mutex::new(HashMap::new()),
                hub,
                ask: Mutex::new(AskState {
                    queues: AskQueues::default(),
                    workers: HashSet::new(),
                }),
                ask_wakers: DashMap::new(),
                overrides: DashMap::new(),
                streams: DashMap::new(),
                webhooks: DashMap::new(),
                approved_urls: DashSet::new(),
                local_base_url: RwLock::new(None),
                background: AtomicBool::new(false),
            }
        })
    }

    fn this(&self) -> Arc<Self> {
        self.weak
            .upgrade()
            .expect("PushRelay is alive while in use")
    }

    /// The `_push_relay` virtual server providing the fallback tools.
    pub fn tools_server(&self) -> Arc<super::tools::PushRelayTools> {
        Arc::new(super::tools::PushRelayTools::new(self.weak.clone()))
    }

    /// Install the callback that writes permanent decisions to the config.
    pub fn set_persister(&self, persister: PushConfigPersister) {
        *self.persister.write() = Some(persister);
    }

    /// Base URL of LocalRouter's HTTP server (`http://127.0.0.1:<port>`),
    /// used in the `EventWait` commands.
    pub fn set_local_base_url(&self, url: impl Into<String>) {
        *self.local_base_url.write() = Some(url.into());
    }

    /// Apply changed Push Relay settings (also forwarded to the gateway).
    pub fn update_config(&self, config: &PushRelayConfig) {
        *self.config.write() = config.clone();
        *self.limits.write() = config.limits.clone();
        self.gateway.update_push_relay_config(config);
        self.refresh();
    }

    /// Re-evaluate permissions after client settings changed: list changes
    /// are announced, channel demand recomputed, blocked streams end.
    pub fn notify_clients_changed(&self) {
        self.refresh();
    }

    fn refresh(&self) {
        if tokio::runtime::Handle::try_current().is_err() {
            return;
        }
        let contexts: Vec<PushRelaySessionContext> =
            self.sessions.iter().map(|s| s.ctx.clone()).collect();
        for ctx in contexts {
            if ctx.is_stdio_bridge {
                self.ensure_channel(&ctx);
            }
        }
        // Held events whose type is no longer Ask follow the new permission
        let held = self.ask.lock().queues.list();
        for (_, event) in held.into_iter().filter(|(_, e)| e.decision.is_none()) {
            let Some(client) = self.client(&event.client_id) else {
                continue;
            };
            let action = match self.permission(
                &client,
                &event.occurrence.server_id,
                &event.occurrence.upstream_name,
            ) {
                PushPermission::Allow => PushDecisionAction::Allow,
                PushPermission::Block => PushDecisionAction::Deny,
                PushPermission::Ask => continue,
            };
            let _ = self.decide(&event.hold_id, PushDecision::new(action));
        }
        self.check_list_changed();
        self.reconcile();
    }

    pub fn config(&self) -> PushRelayConfig {
        self.config.read().clone()
    }

    fn ensure_background(&self) {
        if self.background.swap(true, Ordering::SeqCst) {
            return;
        }
        let weak = self.weak.clone();
        tokio::spawn(async move {
            let mut ticker = tokio::time::interval(SWEEP_INTERVAL);
            ticker.tick().await;
            loop {
                ticker.tick().await;
                let Some(relay) = weak.upgrade() else {
                    return;
                };
                relay.sweep();
            }
        });
    }

    // ---------------------------------------------------------------
    // Permissions and catalogue
    // ---------------------------------------------------------------

    fn client(&self, client_id: &str) -> Option<lr_config::Client> {
        (self.clients)(client_id)
    }

    fn server_visible(&self, client: &lr_config::Client, server_id: &str) -> bool {
        self.gateway
            .server_manager
            .get_config(server_id)
            .is_some_and(|config| {
                !client
                    .mcp_permissions
                    .allowed_server_ids(std::iter::once(&config))
                    .is_empty()
            })
    }

    /// Servers a client may see under its MCP permissions.
    pub fn visible_servers(&self, client: &lr_config::Client) -> Vec<String> {
        client
            .mcp_permissions
            .allowed_server_ids(&self.gateway.server_manager.list_configs())
    }

    /// Effective permission of one upstream event type for one client. Never
    /// wider than the client's MCP access; restricted privacy profiles and a
    /// disabled relay block everything.
    pub fn permission(
        &self,
        client: &lr_config::Client,
        server_id: &str,
        event_name: &str,
    ) -> PushPermission {
        let config = self.config.read();
        if !config.enabled
            || !client.enabled
            || client.security.privacy.restricted()
            || !self.server_visible(client, server_id)
        {
            return PushPermission::Block;
        }
        if let Some(permission) = self.overrides.get(&(
            client.id.clone(),
            server_id.to_string(),
            event_name.to_string(),
        )) {
            return *permission;
        }
        let permission = client.push_permission(&config, server_id, event_name);
        if permission == PushPermission::Allow
            && self
                .gateway
                .firewall_manager
                .should_intercept(&client.id, InterceptCategory::PushEvent)
        {
            PushPermission::Ask
        } else {
            permission
        }
    }

    /// Every event type of `visible` servers this client may receive.
    pub fn catalogue(&self, client: &lr_config::Client, visible: &[String]) -> Vec<CatalogueEntry> {
        if !self.config.read().enabled || client.security.privacy.restricted() {
            return Vec::new();
        }
        let mut entries = Vec::new();
        for source in self.gateway.remembered_push_sources(visible, false) {
            let slug = crate::gateway::types::slugify(&source.server_name);
            for descriptor in &source.snapshot.event_catalogue {
                let Some(name) = descriptor.get("name").and_then(Value::as_str) else {
                    continue;
                };
                let modes: Vec<EventDeliveryMode> = super::delivery_modes(descriptor)
                    .into_iter()
                    .filter(|m| matches!(m, EventDeliveryMode::Push | EventDeliveryMode::Poll))
                    .collect();
                if modes.is_empty() {
                    continue;
                }
                let permission = self.permission(client, &source.server_id, name);
                if permission == PushPermission::Block {
                    continue;
                }
                entries.push(CatalogueEntry {
                    qualified_name: format!("{slug}__{name}"),
                    server_id: source.server_id.clone(),
                    server_slug: slug.clone(),
                    server_name: source.server_name.clone(),
                    upstream_name: name.to_string(),
                    descriptor: descriptor.clone(),
                    modes,
                    is_channel: false,
                    permission,
                });
            }
            if source.snapshot.push.channels {
                let permission = self.permission(client, &source.server_id, CHANNEL_EVENT_NAME);
                if permission != PushPermission::Block {
                    entries.push(CatalogueEntry {
                        qualified_name: format!("{slug}__{CHANNEL_EVENT_NAME}"),
                        server_id: source.server_id.clone(),
                        server_slug: slug.clone(),
                        server_name: source.server_name.clone(),
                        upstream_name: CHANNEL_EVENT_NAME.to_string(),
                        descriptor: Value::Null,
                        modes: Vec::new(),
                        is_channel: true,
                        permission,
                    });
                }
            }
        }
        entries.sort_by(|a, b| a.qualified_name.cmp(&b.qualified_name));
        entries
    }

    fn session_catalogue(&self, ctx: &PushRelaySessionContext) -> Vec<CatalogueEntry> {
        match self.client(&ctx.client_id) {
            Some(client) => self.catalogue(&client, &ctx.visible_servers),
            None => Vec::new(),
        }
    }

    /// The `events/list` descriptor of an entry for this relay's delivery modes.
    fn descriptor(&self, entry: &CatalogueEntry) -> Value {
        let mut delivery = vec!["poll", "push"];
        if self.config.read().webhook_egress_enabled {
            delivery.push("webhook");
        }
        let upstream_description = entry
            .descriptor
            .get("description")
            .and_then(Value::as_str)
            .unwrap_or(if entry.is_channel {
                "A message the server sent as a Claude Code channel notification."
            } else {
                ""
            });
        let mut descriptor = json!({
            "name": entry.qualified_name,
            "description": format!("[{}] {}", entry.server_name, upstream_description).trim_end().to_string(),
            "delivery": delivery,
            "inputSchema": entry
                .descriptor
                .get("inputSchema")
                .cloned()
                .unwrap_or_else(|| json!({"type": "object"})),
        });
        if entry.is_channel {
            descriptor["payloadSchema"] = json!({
                "type": "object",
                "properties": {
                    "content": {"type": "string"},
                    "meta": {"type": "object", "additionalProperties": {"type": "string"}}
                },
                "required": ["content"]
            });
        } else if let Some(schema) = entry.descriptor.get("payloadSchema") {
            descriptor["payloadSchema"] = schema.clone();
        }
        descriptor
    }

    // ---------------------------------------------------------------
    // Sessions, demand and upstream subscriptions
    // ---------------------------------------------------------------

    /// Learn (or refresh) a downstream session.
    pub fn observe(&self, ctx: &PushRelaySessionContext) {
        if tokio::runtime::Handle::try_current().is_err() {
            return;
        }
        self.ensure_background();
        self.sessions
            .entry(ctx.session_key.clone())
            .and_modify(|s| s.ctx = ctx.clone())
            .or_insert_with(|| SessionInfo {
                ctx: ctx.clone(),
                claimed: HashSet::new(),
                streams: 0,
                channel: None,
                signature: None,
            });
        if ctx.is_stdio_bridge {
            self.ensure_channel(ctx);
        }
    }

    fn claim(&self, session_key: &str, name: &str) {
        if let Some(mut session) = self.sessions.get_mut(session_key) {
            session.claimed.insert(name.to_string());
        }
    }

    fn subscription_for(
        &self,
        client_id: &str,
        session_key: Option<&str>,
        entry: &CatalogueEntry,
        arguments: &Map<String, Value>,
    ) -> Option<(SubscriptionKey, SubscriptionSpec)> {
        if entry.is_channel {
            return None;
        }
        let config = self.gateway.server_manager.get_config(&entry.server_id)?;
        let scope = if matches!(
            config.transport_config,
            lr_config::McpTransportConfig::Stdio { .. }
        ) {
            let session_key = match session_key {
                Some(key) => key.to_string(),
                None => {
                    self.gateway
                        .get_sessions_for_client(client_id)
                        .into_iter()
                        .next()?
                        .0
                }
            };
            ConnectionScope::Session(session_key)
        } else {
            ConnectionScope::Shared
        };
        Some((
            SubscriptionKey {
                scope,
                server_id: entry.server_id.clone(),
                event: entry.upstream_name.clone(),
                args_key: canonical_json(&Value::Object(arguments.clone())),
            },
            SubscriptionSpec {
                arguments: arguments.clone(),
                modes: entry.modes.clone(),
            },
        ))
    }

    fn set_holder(
        &self,
        id: String,
        client_id: &str,
        kind: HolderKind,
        keys: HashMap<SubscriptionKey, SubscriptionSpec>,
        lease: Option<Duration>,
    ) {
        self.holders.lock().insert(
            id,
            Holder {
                client_id: client_id.to_string(),
                kind,
                keys,
                expires: lease.map(|l| Instant::now() + l),
            },
        );
        self.reconcile();
    }

    fn remove_holder(&self, id: &str) {
        if self.holders.lock().remove(id).is_some() {
            self.reconcile();
        }
    }

    fn reconcile(&self) {
        if tokio::runtime::Handle::try_current().is_err() {
            return;
        }
        let desired: HashMap<SubscriptionKey, SubscriptionSpec> = {
            let now = Instant::now();
            let holders = self.holders.lock();
            holders
                .values()
                .filter(|h| h.expires.is_none_or(|e| e > now))
                .flat_map(|h| h.keys.iter().map(|(k, s)| (k.clone(), s.clone())))
                .collect()
        };
        self.hub.sync(desired);
    }

    /// Clients consuming an upstream subscription.
    fn consumers(&self, key: &SubscriptionKey) -> Vec<String> {
        let now = Instant::now();
        let holders = self.holders.lock();
        let mut clients: Vec<String> = holders
            .values()
            .filter(|h| h.expires.is_none_or(|e| e > now) && h.keys.contains_key(key))
            .map(|h| h.client_id.clone())
            .collect();
        clients.sort();
        clients.dedup();
        clients
    }

    fn egress_modes(&self, client_id: &str) -> Vec<String> {
        let now = Instant::now();
        let mut modes: Vec<String> = self
            .holders
            .lock()
            .values()
            .filter(|h| h.client_id == client_id && h.expires.is_none_or(|e| e > now))
            .map(|h| h.kind.egress().to_string())
            .collect();
        modes.sort();
        modes.dedup();
        modes
    }

    /// Every open upstream subscription (for the UI).
    pub fn active_subscriptions(&self) -> Vec<SubscriptionInfo> {
        self.hub.subscriptions()
    }

    fn sweep(&self) {
        let now = Instant::now();
        self.holders
            .lock()
            .retain(|_, h| h.expires.is_none_or(|e| e > now));
        let gone: Vec<String> = self
            .sessions
            .iter()
            .filter(|s| self.gateway.get_session(s.key()).is_none())
            .map(|s| s.key().clone())
            .collect();
        for session_key in gone {
            if let Some((_, session)) = self.sessions.remove(&session_key) {
                if let Some(cancel) = session.channel {
                    cancel.cancel();
                }
            }
            self.holders
                .lock()
                .remove(&format!("channel:{session_key}"));
            let prefix = format!("{session_key}\u{1f}");
            self.streams.retain(|key, cancel| {
                let ours = key.starts_with(&prefix);
                if ours {
                    cancel.cancel();
                }
                !ours
            });
        }
        let utc_now = chrono::Utc::now();
        let expired: Vec<String> = self
            .webhooks
            .iter()
            .filter(|w| *w.expires_at.lock() <= utc_now)
            .map(|w| w.key().clone())
            .collect();
        for id in expired {
            self.end_webhook(&id);
        }
        let limits = JournalLimits::from_config(&self.limits.read());
        for journal in self.journals.iter() {
            journal.lock().prune(&limits);
        }
        self.check_list_changed();
        self.reconcile();
    }

    fn signature(&self, ctx: &PushRelaySessionContext) -> u64 {
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        for entry in self.session_catalogue(ctx) {
            entry.qualified_name.hash(&mut hasher);
            entry.permission.hash(&mut hasher);
        }
        let view = self.tools_view_for_ctx(ctx);
        (view.list, view.poll, view.wait).hash(&mut hasher);
        hasher.finish()
    }

    /// Tell sessions whose event catalogue or fallback tools changed.
    fn check_list_changed(&self) {
        let contexts: Vec<(String, PushRelaySessionContext, Option<u64>)> = self
            .sessions
            .iter()
            .map(|s| (s.key().clone(), s.ctx.clone(), s.signature))
            .collect();
        for (session_key, ctx, old) in contexts {
            let new = self.signature(&ctx);
            if let Some(mut session) = self.sessions.get_mut(&session_key) {
                session.signature = Some(new);
            }
            if old.is_some_and(|old| old != new) {
                self.gateway.invalidate_tools_cache(&ctx.client_id);
                for method in [
                    EVENTS_LIST_CHANGED_NOTIFICATION,
                    "notifications/tools/list_changed",
                ] {
                    self.gateway.send_session_notification(
                        &session_key,
                        JsonRpcNotification {
                            jsonrpc: "2.0".to_string(),
                            method: method.to_string(),
                            params: None,
                        },
                    );
                }
            }
        }
    }

    // ---------------------------------------------------------------
    // Ingress → permissions → Ask → journal
    // ---------------------------------------------------------------

    fn journal(&self, client_id: &str) -> Arc<Mutex<ClientJournal>> {
        self.journals
            .entry(client_id.to_string())
            .or_insert_with(|| Arc::new(Mutex::new(ClientJournal::new())))
            .clone()
    }

    fn occurrence(
        &self,
        server_id: &str,
        upstream_name: &str,
        event: &Value,
        mode: IngressMode,
        args_key: String,
    ) -> Option<Arc<Occurrence>> {
        let config = self.gateway.server_manager.get_config(server_id)?;
        let slug = crate::gateway::types::slugify(&config.name);
        let upstream_id = event
            .get("eventId")
            .and_then(Value::as_str)
            .map(str::to_string)
            .unwrap_or_else(|| uuid::Uuid::new_v4().to_string());
        let timestamp = event
            .get("timestamp")
            .and_then(Value::as_str)
            .and_then(|t| chrono::DateTime::parse_from_rfc3339(t).ok())
            .map(|t| t.with_timezone(&chrono::Utc))
            .unwrap_or_else(chrono::Utc::now);
        Some(Arc::new(Occurrence {
            event_id: format!("{slug}__{upstream_id}"),
            name: format!("{slug}__{upstream_name}"),
            server_id: server_id.to_string(),
            server_slug: slug,
            upstream_name: upstream_name.to_string(),
            timestamp,
            data: event.get("data").cloned().unwrap_or(Value::Null),
            upstream_cursor: event
                .get("cursor")
                .and_then(Value::as_str)
                .map(str::to_string),
            ingress: mode,
            args_key,
        }))
    }

    fn ingest(&self, key: &SubscriptionKey, event: Value, mode: IngressMode) {
        let Some(occurrence) = self.occurrence(
            &key.server_id,
            &key.event,
            &event,
            mode,
            key.args_key.clone(),
        ) else {
            return;
        };
        for client_id in self.consumers(key) {
            self.admit(&client_id, occurrence.clone());
        }
    }

    fn channel_occurrence(
        &self,
        server_id: &str,
        notification: &JsonRpcNotification,
    ) -> Option<Arc<Occurrence>> {
        let params = notification.params.clone().unwrap_or(Value::Null);
        let data = json!({
            "content": params.get("content").cloned().unwrap_or_else(|| json!("")),
            "meta": params.get("meta").cloned().unwrap_or_else(|| json!({})),
        });
        self.occurrence(
            server_id,
            CHANNEL_EVENT_NAME,
            &json!({ "data": data }),
            IngressMode::Channel,
            String::new(),
        )
    }

    /// A channel notification on a relay-owned connection: every client
    /// consuming anything from that server receives it.
    fn ingest_shared_channel(&self, server_id: &str, notification: JsonRpcNotification) {
        let Some(occurrence) = self.channel_occurrence(server_id, &notification) else {
            return;
        };
        let clients: HashSet<String> = self
            .holders
            .lock()
            .values()
            .filter(|h| h.keys.keys().any(|k| k.server_id == server_id))
            .map(|h| h.client_id.clone())
            .collect();
        for client_id in clients {
            self.admit(&client_id, occurrence.clone());
        }
    }

    /// Route one occurrence to one client: drop, hold for Ask, or deliver.
    pub fn admit(&self, client_id: &str, occurrence: Arc<Occurrence>) {
        let Some(client) = self.client(client_id) else {
            return;
        };
        let max_bytes = self.limits.read().max_event_bytes;
        if Occurrence::data_bytes(&occurrence.data) as u64 > max_bytes {
            self.monitor(
                &client,
                &occurrence,
                "too_large",
                None,
                lr_monitor::EventStatus::Error,
                None,
                Some(format!("Event data exceeds {max_bytes} bytes")),
            );
            return;
        }
        match self.permission(&client, &occurrence.server_id, &occurrence.upstream_name) {
            PushPermission::Block => {
                self.monitor(
                    &client,
                    &occurrence,
                    "blocked",
                    None,
                    lr_monitor::EventStatus::Complete,
                    None,
                    None,
                );
            }
            PushPermission::Allow => {
                let data = occurrence.data.clone();
                self.append(&client.id, occurrence.clone(), data.clone());
                self.monitor(
                    &client,
                    &occurrence,
                    "allowed",
                    Some(&data),
                    lr_monitor::EventStatus::Complete,
                    None,
                    None,
                );
            }
            PushPermission::Ask => self.hold(&client, occurrence),
        }
    }

    fn append(&self, client_id: &str, occurrence: Arc<Occurrence>, data: Value) {
        let limits = JournalLimits::from_config(&self.limits.read());
        self.journal(client_id)
            .lock()
            .append(occurrence, data, &limits);
    }

    #[allow(clippy::too_many_arguments)]
    fn monitor(
        &self,
        client: &lr_config::Client,
        occurrence: &Occurrence,
        decision: &str,
        delivered: Option<&Value>,
        status: lr_monitor::EventStatus,
        held_ms: Option<u64>,
        error: Option<String>,
    ) -> String {
        let keep_payload = !client.security.privacy.redact_payloads();
        self.gateway.emit_monitor_event(
            lr_monitor::MonitorEventType::McpPushEvent,
            Some(client.id.clone()),
            Some(client.name.clone()),
            None,
            lr_monitor::MonitorEventData::McpPushEvent {
                server_id: occurrence.server_id.clone(),
                server_name: Some(occurrence.server_slug.clone()),
                event_name: occurrence.name.clone(),
                event_id: occurrence.event_id.clone(),
                ingress_mode: occurrence.ingress.as_str().to_string(),
                egress_modes: self.egress_modes(&client.id),
                decision: decision.to_string(),
                original_payload: keep_payload.then(|| occurrence.data.clone()),
                delivered_payload: delivered.filter(|_| keep_payload).cloned(),
                held_ms,
                error,
            },
            status,
            held_ms,
        )
    }

    fn update_monitor(
        &self,
        monitor_id: &str,
        decision: &str,
        delivered: Option<Value>,
        status: lr_monitor::EventStatus,
        held_ms: u64,
        egress: Vec<String>,
    ) {
        if monitor_id.is_empty() {
            return;
        }
        let decision = decision.to_string();
        self.gateway.update_monitor_event(
            monitor_id,
            Box::new(move |event| {
                if let lr_monitor::MonitorEventData::McpPushEvent {
                    decision: d,
                    delivered_payload,
                    held_ms: h,
                    egress_modes,
                    original_payload,
                    ..
                } = &mut event.data
                {
                    *d = decision;
                    if original_payload.is_some() {
                        *delivered_payload = delivered;
                    }
                    *h = Some(held_ms);
                    *egress_modes = egress;
                }
                event.status = status;
                event.duration_ms = Some(held_ms);
            }),
        );
    }

    fn hold(&self, client: &lr_config::Client, occurrence: Arc<Occurrence>) {
        let cap = self.limits.read().ask_hold_cap;
        if self.ask.lock().queues.count_for_client(&client.id) >= cap {
            self.monitor(
                client,
                &occurrence,
                "hold_cap",
                None,
                lr_monitor::EventStatus::Error,
                None,
                Some(format!("{cap} events already await a decision")),
            );
            return;
        }
        let monitor_id = self.monitor(
            client,
            &occurrence,
            "pending",
            None,
            lr_monitor::EventStatus::Pending,
            None,
            None,
        );
        let held = HeldEvent {
            hold_id: uuid::Uuid::new_v4().to_string(),
            client_id: client.id.clone(),
            client_name: client.name.clone(),
            occurrence: occurrence.clone(),
            held_at: Instant::now(),
            held_at_utc: chrono::Utc::now(),
            monitor_id: monitor_id.clone(),
            decision: None,
        };
        let queue = (client.id.clone(), occurrence.name.clone());
        let start_worker = {
            let mut ask = self.ask.lock();
            if ask.queues.push(held, cap).is_err() {
                drop(ask);
                self.update_monitor(
                    &monitor_id,
                    "hold_cap",
                    None,
                    lr_monitor::EventStatus::Error,
                    0,
                    Vec::new(),
                );
                return;
            }
            ask.workers.insert(queue.clone())
        };
        if start_worker {
            let relay = self.this();
            tokio::spawn(async move { relay.ask_worker(queue.0, queue.1).await });
        }
    }

    fn approval_details(&self, held: &HeldEvent) -> PushEventApprovalDetails {
        let (pending_for_client, pending_of_type) = {
            let ask = self.ask.lock();
            (
                ask.queues.count_for_client(&held.client_id),
                ask.queues
                    .count_for_type(&held.client_id, &held.occurrence.name),
            )
        };
        let entry = super::journal::JournalEntry::preview(held.occurrence.clone());
        PushEventApprovalDetails {
            kind: PushApprovalKind::Event,
            hold_id: held.hold_id.clone(),
            server_id: held.occurrence.server_id.clone(),
            server_name: held.occurrence.server_slug.clone(),
            event_name: held.occurrence.upstream_name.clone(),
            qualified_name: held.occurrence.name.clone(),
            event_id: held.occurrence.event_id.clone(),
            ingress_mode: held.occurrence.ingress.as_str().to_string(),
            data: held.occurrence.data.clone(),
            rendered_content: render::channel_content(
                &entry,
                self.limits.read().channel_max_content_bytes,
            ),
            webhook_url: None,
            pending_for_client,
            pending_of_type,
        }
    }

    async fn ask_worker(self: Arc<Self>, client_id: String, name: String) {
        let queue = (client_id.clone(), name.clone());
        let waker = self
            .ask_wakers
            .entry(queue.clone())
            .or_insert_with(|| Arc::new(Notify::new()))
            .clone();
        loop {
            let head = {
                let mut ask = self.ask.lock();
                match ask.queues.front(&client_id, &name) {
                    Some(head) => head,
                    None => {
                        ask.workers.remove(&queue);
                        self.ask_wakers.remove(&queue);
                        break;
                    }
                }
            };
            let decision: Option<PushDecision> = match head.decision.clone() {
                Some(decision) => Some(decision),
                None => {
                    let remaining = self
                        .limits
                        .read()
                        .ask_timeout()
                        .saturating_sub(head.held_at.elapsed());
                    if remaining.is_zero() {
                        None
                    } else {
                        let details = self.approval_details(&head);
                        let approval = self.gateway.firewall_manager.request_push_event_approval(
                            head.client_id.clone(),
                            head.client_name.clone(),
                            details,
                            remaining.as_secs() + 60,
                        );
                        tokio::select! {
                            response = approval => Some(match response {
                                Ok(response) => PushDecision {
                                    action: PushDecisionAction::from(&response.action),
                                    edited_data: response.edited_arguments,
                                },
                                Err(_) => PushDecision::new(PushDecisionAction::Deny),
                            }),
                            _ = tokio::time::sleep(remaining) => None,
                            _ = waker.notified() => continue,
                        }
                    }
                }
            };
            let popped = self.ask.lock().queues.pop(&client_id, &name, &head.hold_id);
            if let Some(held) = popped {
                self.resolve(held, decision).await;
            }
        }
    }

    /// Apply a decision (`None` = timed out) to a held event.
    async fn resolve(&self, held: HeldEvent, decision: Option<PushDecision>) {
        let held_ms = held.held_at.elapsed().as_millis() as u64;
        let occurrence = held.occurrence.clone();
        let Some(decision) = decision else {
            self.update_monitor(
                &held.monitor_id,
                "timeout",
                None,
                lr_monitor::EventStatus::Error,
                held_ms,
                self.egress_modes(&held.client_id),
            );
            return;
        };
        if let Some(permission) = decision.action.permission() {
            self.overrides.insert(
                (
                    held.client_id.clone(),
                    occurrence.server_id.clone(),
                    occurrence.upstream_name.clone(),
                ),
                permission,
            );
            if decision.action.is_permanent() {
                self.persist(
                    &held.client_id,
                    PushConfigChange::EventPermission {
                        server_id: occurrence.server_id.clone(),
                        event_name: occurrence.upstream_name.clone(),
                        permission,
                    },
                )
                .await;
            }
            // The rest of the queue follows the type-wide decision
            let follow = PushDecision::new(if decision.action.allows() {
                PushDecisionAction::Allow
            } else {
                PushDecisionAction::Deny
            });
            self.ask
                .lock()
                .queues
                .decide_all(&held.client_id, &occurrence.name, &follow);
            self.check_list_changed();
        }
        if decision.action.allows() {
            let data = decision
                .edited_data
                .clone()
                .unwrap_or_else(|| occurrence.data.clone());
            let label = if data != occurrence.data {
                "asked_edited"
            } else {
                "asked_allowed"
            };
            self.append(&held.client_id, occurrence.clone(), data.clone());
            self.update_monitor(
                &held.monitor_id,
                label,
                Some(data),
                lr_monitor::EventStatus::Complete,
                held_ms,
                self.egress_modes(&held.client_id),
            );
        } else {
            self.update_monitor(
                &held.monitor_id,
                "denied",
                None,
                lr_monitor::EventStatus::Complete,
                held_ms,
                self.egress_modes(&held.client_id),
            );
        }
    }

    async fn persist(&self, client_id: &str, change: PushConfigChange) {
        let persister = self.persister.read().clone();
        if let Some(persister) = persister {
            if let Err(e) = persister(client_id.to_string(), change).await {
                tracing::warn!("Push Relay could not save a decision: {}", e);
            }
        }
    }

    /// Held events awaiting a decision (all clients, or one).
    pub fn pending_events(&self, client_id: Option<&str>) -> Vec<HeldPushEventInfo> {
        let timeout = self.limits.read().ask_timeout_secs;
        let held = self.ask.lock().queues.list();
        let mut out: Vec<HeldPushEventInfo> = held
            .into_iter()
            .filter(|(_, e)| client_id.is_none_or(|id| id == e.client_id))
            .map(|(position, e)| HeldPushEventInfo {
                firewall_request_id: self
                    .gateway
                    .firewall_manager
                    .push_request_for_hold(&e.hold_id),
                hold_id: e.hold_id.clone(),
                client_id: e.client_id.clone(),
                client_name: e.client_name.clone(),
                server_id: e.occurrence.server_id.clone(),
                server_name: e.occurrence.server_slug.clone(),
                event_name: e.occurrence.upstream_name.clone(),
                qualified_name: e.occurrence.name.clone(),
                event_id: e.occurrence.event_id.clone(),
                ingress_mode: e.occurrence.ingress.as_str().to_string(),
                data: e.occurrence.data.clone(),
                held_at: e.held_at_utc,
                held_secs: e.held_at.elapsed().as_secs(),
                timeout_secs: timeout,
                position,
                decided: e.decision.is_some(),
            })
            .collect();
        out.sort_by_key(|e| e.held_at);
        out
    }

    /// Decide one held event (or a webhook URL approval, `webhook:<id>`).
    pub fn decide(&self, hold_id: &str, decision: PushDecision) -> Result<(), String> {
        let firewall = &self.gateway.firewall_manager;
        if let Some(request_id) = firewall.push_request_for_hold(hold_id) {
            return firewall
                .submit_response(
                    &request_id,
                    decision.action.firewall_action(),
                    decision.edited_data,
                )
                .map_err(|e| e.to_string());
        }
        let (queue, is_head) = {
            let mut ask = self.ask.lock();
            let Some(queue) = ask.queues.decide(hold_id, decision) else {
                return Err(format!("No held event {hold_id}"));
            };
            let is_head = ask
                .queues
                .front(&queue.0, &queue.1)
                .is_some_and(|head| head.hold_id == hold_id);
            (queue, is_head)
        };
        // Only the head is being asked about; later events wait their turn
        if is_head {
            if let Some(waker) = self.ask_wakers.get(&queue) {
                waker.notify_one();
            }
        }
        Ok(())
    }

    /// Decide every held event of one type for one client ("allow all
    /// pending of this type"). Returns how many were decided.
    pub fn decide_all(
        &self,
        client_id: &str,
        qualified_name: &str,
        decision: PushDecision,
    ) -> usize {
        let ids: Vec<String> = self
            .ask
            .lock()
            .queues
            .list()
            .into_iter()
            .filter(|(_, e)| e.client_id == client_id && e.occurrence.name == qualified_name)
            .map(|(_, e)| e.hold_id)
            .collect();
        ids.iter()
            .filter(|id| self.decide(id, decision.clone()).is_ok())
            .count()
    }

    /// Every server with remembered capabilities, with its ingress modes
    /// (webhook-only event types marked unsupported).
    pub fn upstream_sources(&self) -> Vec<UpstreamSourceInfo> {
        self.gateway
            .server_manager
            .list_configs()
            .into_iter()
            .filter_map(|config| {
                let snapshot = config.capability_snapshot.clone()?;
                let current = snapshot.is_current_for(&config);
                let (transport, connection) = match &config.transport_config {
                    lr_config::McpTransportConfig::Stdio { .. } => ("stdio", "per_session"),
                    lr_config::McpTransportConfig::WebSocket { .. } => ("websocket", "shared"),
                    _ => ("http", "shared"),
                };
                let mut event_types: Vec<UpstreamEventTypeInfo> = snapshot
                    .event_catalogue
                    .iter()
                    .filter_map(|entry| {
                        let name = entry.get("name")?.as_str()?.to_string();
                        let modes = super::delivery_modes(entry);
                        let ingress = if modes.contains(&EventDeliveryMode::Push) {
                            Some("push")
                        } else if modes.contains(&EventDeliveryMode::Poll) {
                            Some("poll")
                        } else {
                            None
                        };
                        Some(UpstreamEventTypeInfo {
                            name,
                            description: entry
                                .get("description")
                                .and_then(Value::as_str)
                                .map(str::to_string),
                            modes: modes
                                .iter()
                                .map(|m| {
                                    match m {
                                        EventDeliveryMode::Push => "push",
                                        EventDeliveryMode::Poll => "poll",
                                        EventDeliveryMode::Webhook => "webhook",
                                    }
                                    .to_string()
                                })
                                .collect(),
                            ingress: ingress.map(str::to_string),
                            supported: ingress.is_some(),
                            unsupported_reason: ingress
                                .is_none()
                                .then(|| "webhook-only — not supported".to_string()),
                        })
                    })
                    .collect();
                if snapshot.push.channels {
                    event_types.push(UpstreamEventTypeInfo {
                        name: CHANNEL_EVENT_NAME.to_string(),
                        description: Some("Claude Code channel messages".to_string()),
                        modes: vec!["channel".to_string()],
                        ingress: Some("channel".to_string()),
                        supported: true,
                        unsupported_reason: None,
                    });
                }
                Some(UpstreamSourceInfo {
                    server_id: config.id.clone(),
                    server_name: config.name.clone(),
                    enabled: config.enabled,
                    transport: transport.to_string(),
                    connection: connection.to_string(),
                    snapshot_current: current,
                    protocol_version: Some(snapshot.protocol_version.clone()),
                    observed_at: Some(snapshot.observed_at),
                    channels: snapshot.push.channels,
                    resource_subscribe: snapshot.push.resource_subscribe,
                    event_types,
                })
            })
            .collect()
    }

    /// A client's event types with their effective permissions, for the
    /// permission tree (Blocked types included).
    pub fn client_event_types(&self, client_id: &str) -> Vec<ClientEventTypeInfo> {
        let Some(client) = self.client(client_id) else {
            return Vec::new();
        };
        let visible = self.visible_servers(&client);
        let mut out = Vec::new();
        for source in self.gateway.remembered_push_sources(&visible, false) {
            let slug = crate::gateway::types::slugify(&source.server_name);
            let mut names: Vec<String> = source
                .snapshot
                .event_catalogue
                .iter()
                .filter(|e| {
                    super::delivery_modes(e)
                        .iter()
                        .any(|m| matches!(m, EventDeliveryMode::Push | EventDeliveryMode::Poll))
                })
                .filter_map(|e| e.get("name")?.as_str().map(str::to_string))
                .collect();
            if source.snapshot.push.channels {
                names.push(CHANNEL_EVENT_NAME.to_string());
            }
            for name in names {
                out.push(ClientEventTypeInfo {
                    qualified_name: format!("{slug}__{name}"),
                    server_id: source.server_id.clone(),
                    permission: self.permission(&client, &source.server_id, &name),
                    session_override: self
                        .overrides
                        .get(&(client.id.clone(), source.server_id.clone(), name.clone()))
                        .map(|p| *p),
                    event_name: name,
                });
            }
        }
        out
    }

    /// Clear a session-scoped decision (the configured permission applies again).
    pub fn clear_session_override(&self, client_id: &str, server_id: &str, event_name: &str) {
        self.overrides.remove(&(
            client_id.to_string(),
            server_id.to_string(),
            event_name.to_string(),
        ));
        self.refresh();
    }

    // ---------------------------------------------------------------
    // Reading the journal
    // ---------------------------------------------------------------

    /// Resolve a presented cursor to a journal position. `None` starts at
    /// "now"; a foreign cursor resets to now with truncation.
    fn position(
        &self,
        client_id: &str,
        scope: &CursorScope,
        cursor: Option<&str>,
    ) -> Result<(u64, bool), CursorError> {
        let now = self.journal(client_id).lock().now();
        match cursor {
            None => Ok((now, false)),
            Some(cursor) => match self.cursor_key.decode(client_id, scope, cursor)? {
                CursorPosition::At(seq) if seq <= now => Ok((seq, false)),
                CursorPosition::At(_) | CursorPosition::Reset => Ok((now, true)),
            },
        }
    }

    fn read_scope(names: &[String], arguments: &Map<String, Value>) -> ReadScope {
        ReadScope {
            names: Some(names.iter().cloned().collect()),
            args_key: Some(canonical_json(&Value::Object(arguments.clone()))),
        }
    }

    async fn wait_for_entries(&self, client_id: &str, after: u64, timeout: Duration) {
        let mut changed = self.journal(client_id).lock().subscribe();
        if self.journal(client_id).lock().now() > after {
            return;
        }
        let _ = tokio::time::timeout(timeout, changed.changed()).await;
    }

    // ---------------------------------------------------------------
    // Downstream events/*
    // ---------------------------------------------------------------

    async fn handle_poll(
        &self,
        ctx: &PushRelaySessionContext,
        request: JsonRpcRequest,
    ) -> JsonRpcResponse {
        let id = request.id.clone().unwrap_or(Value::Null);
        let params = match parse_event_params(request.params.as_ref()) {
            Ok(p) => p,
            Err(e) => return invalid_params(id, e),
        };
        let Some(entry) = self
            .session_catalogue(ctx)
            .into_iter()
            .find(|e| e.qualified_name == params.name)
        else {
            return not_found(id);
        };
        self.claim(&ctx.session_key, &entry.qualified_name);
        self.hold_poll_demand(
            &ctx.client_id,
            Some(&ctx.session_key),
            HolderKind::Poll,
            std::slice::from_ref(&entry),
            &params.arguments,
        );
        let names = vec![entry.qualified_name.clone()];
        let scope = CursorScope::new(&names, &params.arguments);
        let (position, reset) =
            match self.position(&ctx.client_id, &scope, params.cursor.as_deref()) {
                Ok(p) => p,
                Err(e) => return invalid_params(id, e.message()),
            };
        let limits = self.limits.read().clone();
        let limit = params
            .max_events
            .map(|n| n as usize)
            .unwrap_or(limits.poll_default_events)
            .min(limits.poll_max_events.max(1));
        let batch = if params.cursor.is_none() {
            None
        } else {
            Some(self.journal(&ctx.client_id).lock().read(
                position,
                &Self::read_scope(&names, &params.arguments),
                limit,
            ))
        };
        let (events, next, truncated, has_more) = match batch {
            None => (Vec::new(), position, reset, false),
            Some(batch) => (
                batch.entries.iter().map(|e| e.wire()).collect(),
                batch.position,
                reset || batch.truncated,
                batch.has_more,
            ),
        };
        JsonRpcResponse::success(
            id,
            json!({
                "resultType": "complete",
                "events": events,
                "cursor": self.cursor_key.issue(&ctx.client_id, &scope, next),
                "truncated": truncated,
                "hasMore": has_more,
                "nextPollMs": DOWNSTREAM_NEXT_POLL_MS,
            }),
        )
    }

    fn hold_poll_demand(
        &self,
        client_id: &str,
        session_key: Option<&str>,
        kind: HolderKind,
        entries: &[CatalogueEntry],
        arguments: &Map<String, Value>,
    ) {
        let keys: HashMap<SubscriptionKey, SubscriptionSpec> = entries
            .iter()
            .filter_map(|e| self.subscription_for(client_id, session_key, e, arguments))
            .collect();
        let mut names: Vec<&str> = entries.iter().map(|e| e.qualified_name.as_str()).collect();
        names.sort_unstable();
        let id = format!(
            "{}:{client_id}:{}:{}:{}",
            kind.egress(),
            session_key.unwrap_or_default(),
            names.join(","),
            canonical_json(&Value::Object(arguments.clone()))
        );
        self.set_holder(id, client_id, kind, keys, Some(POLL_LEASE));
    }

    async fn send_stream_notification(
        &self,
        ctx: &PushRelaySessionContext,
        sink: &Option<mpsc::Sender<JsonRpcNotification>>,
        id: &Value,
        method: &str,
        params: Value,
    ) -> bool {
        let message = notification(method, tag_subscription(params, id));
        match sink {
            Some(sink) => sink.send(message).await.is_ok(),
            None => {
                self.gateway
                    .send_session_notification(&ctx.session_key, message);
                self.gateway.get_session(&ctx.session_key).is_some()
            }
        }
    }

    async fn handle_stream(
        &self,
        ctx: &PushRelaySessionContext,
        request: JsonRpcRequest,
    ) -> JsonRpcResponse {
        let id = request.id.clone().unwrap_or(Value::Null);
        let params = match parse_event_params(request.params.as_ref()) {
            Ok(p) if p.max_events.is_some() => {
                return invalid_params(id, "maxEvents is only valid for events/poll")
            }
            Ok(p) => p,
            Err(e) => return invalid_params(id, e),
        };
        let Some(entry) = self
            .session_catalogue(ctx)
            .into_iter()
            .find(|e| e.qualified_name == params.name)
        else {
            return not_found(id);
        };
        let max_streams = self.limits.read().max_streams_per_session;
        {
            let Some(mut session) = self.sessions.get_mut(&ctx.session_key) else {
                return not_found(id);
            };
            if session.streams >= max_streams {
                return error_response(
                    id,
                    ERROR_RESOURCE_EXHAUSTED,
                    "ResourceExhausted",
                    Some(json!({"limit": "subscriptions", "max": max_streams})),
                );
            }
            session.streams += 1;
            session.claimed.insert(entry.qualified_name.clone());
        }
        let names = vec![entry.qualified_name.clone()];
        let scope = CursorScope::new(&names, &params.arguments);
        let result = match self.position(&ctx.client_id, &scope, params.cursor.as_deref()) {
            Err(e) => invalid_params(id.clone(), e.message()),
            Ok((position, reset)) => {
                self.run_stream(ctx, &id, &entry, &params.arguments, &scope, position, reset)
                    .await
            }
        };
        if let Some(mut session) = self.sessions.get_mut(&ctx.session_key) {
            session.streams = session.streams.saturating_sub(1);
        }
        result
    }

    #[allow(clippy::too_many_arguments)]
    async fn run_stream(
        &self,
        ctx: &PushRelaySessionContext,
        id: &Value,
        entry: &CatalogueEntry,
        arguments: &Map<String, Value>,
        scope: &CursorScope,
        mut position: u64,
        reset: bool,
    ) -> JsonRpcResponse {
        let stream_key = session_stream_key(&ctx.session_key, id);
        let cancel = CancellationToken::new();
        self.streams.insert(stream_key.clone(), cancel.clone());
        let holder_id = format!("stream:{stream_key}");
        let keys = self
            .subscription_for(&ctx.client_id, Some(&ctx.session_key), entry, arguments)
            .into_iter()
            .collect();
        self.set_holder(
            holder_id.clone(),
            &ctx.client_id,
            HolderKind::Stream,
            keys,
            None,
        );
        let sink = self.gateway.push_stream_sinks.sink(&ctx.session_key, id);
        let names = vec![entry.qualified_name.clone()];
        let read_scope = Self::read_scope(&names, arguments);
        let journal = self.journal(&ctx.client_id);
        let mut changed = journal.lock().subscribe();
        let mut heartbeat = tokio::time::interval(self.limits.read().downstream_heartbeat());
        heartbeat.tick().await;
        let mut first = true;
        let mut pending_truncation = reset;
        let mut ended = "cancelled";
        'outer: loop {
            changed.borrow_and_update();
            let batch = journal.lock().read(position, &read_scope, 100);
            if first || batch.truncated || pending_truncation {
                let start = if first { position } else { batch.start };
                if !self
                    .send_stream_notification(
                        ctx,
                        &sink,
                        id,
                        "notifications/events/active",
                        json!({
                            "cursor": self.cursor_key.issue(&ctx.client_id, scope, start),
                            "truncated": pending_truncation || batch.truncated,
                        }),
                    )
                    .await
                {
                    break;
                }
                first = false;
                pending_truncation = false;
            }
            for entry in &batch.entries {
                let mut params = entry.wire();
                params["cursor"] = json!(self.cursor_key.issue(&ctx.client_id, scope, entry.seq));
                if !self
                    .send_stream_notification(ctx, &sink, id, "notifications/events/event", params)
                    .await
                {
                    break 'outer;
                }
            }
            position = batch.position;
            if batch.has_more {
                continue;
            }
            tokio::select! {
                biased;
                _ = cancel.cancelled() => break,
                result = changed.changed() => if result.is_err() { break },
                _ = heartbeat.tick() => {
                    let still_permitted = self
                        .session_catalogue(ctx)
                        .iter()
                        .any(|e| e.qualified_name == entry.qualified_name);
                    if !still_permitted {
                        let _ = self
                            .send_stream_notification(
                                ctx,
                                &sink,
                                id,
                                "notifications/events/terminated",
                                json!({"reason": "blocked"}),
                            )
                            .await;
                        ended = "blocked";
                        break;
                    }
                    if !self
                        .send_stream_notification(
                            ctx,
                            &sink,
                            id,
                            "notifications/events/heartbeat",
                            json!({"cursor": self.cursor_key.issue(&ctx.client_id, scope, position)}),
                        )
                        .await
                    {
                        break;
                    }
                }
                _ = async {
                    match &sink {
                        Some(sink) => sink.closed().await,
                        None => std::future::pending().await,
                    }
                } => break,
            }
        }
        self.streams.remove(&stream_key);
        self.remove_holder(&holder_id);
        tracing::debug!(
            "Push Relay stream {} for {} ended ({})",
            entry.qualified_name,
            ctx.client_id,
            ended
        );
        JsonRpcResponse::success(
            id.clone(),
            json!({
                "resultType": "complete",
                "_meta": { SUBSCRIPTION_ID_META_KEY: id },
            }),
        )
    }

    fn handle_list(
        &self,
        ctx: &PushRelaySessionContext,
        request: &JsonRpcRequest,
    ) -> JsonRpcResponse {
        let id = request.id.clone().unwrap_or(Value::Null);
        let events: Vec<Value> = self
            .session_catalogue(ctx)
            .iter()
            .map(|e| self.descriptor(e))
            .collect();
        JsonRpcResponse::success(id, json!({"resultType": "complete", "events": events}))
    }

    // ---------------------------------------------------------------
    // Webhook egress (D3)
    // ---------------------------------------------------------------

    fn webhook_id(client_id: &str, url: &str, name: &str, args_key: &str) -> String {
        use sha2::{Digest, Sha256};
        let digest = Sha256::digest(format!("{client_id}\n{url}\n{name}\n{args_key}").as_bytes());
        let hex: String = digest[..12].iter().map(|b| format!("{b:02x}")).collect();
        format!("whsub_{hex}")
    }

    async fn handle_subscribe(
        &self,
        ctx: &PushRelaySessionContext,
        request: JsonRpcRequest,
    ) -> JsonRpcResponse {
        let id = request.id.clone().unwrap_or(Value::Null);
        let config = self.config.read().clone();
        if !config.webhook_egress_enabled {
            return error_response(
                id,
                ERROR_UNSUPPORTED,
                "Unsupported",
                Some(json!({"feature": "deliveryMode", "deliveryMode": "webhook"})),
            );
        }
        let params = match parse_event_params(request.params.as_ref()) {
            Ok(p) => p,
            Err(e) => return invalid_params(id, e),
        };
        let raw = request.params.clone().unwrap_or(Value::Null);
        let delivery = raw.get("delivery").cloned().unwrap_or(Value::Null);
        match delivery.get("mode").and_then(Value::as_str) {
            None | Some("webhook") => {}
            Some(_) => {
                return error_response(
                    id,
                    ERROR_UNSUPPORTED,
                    "Only webhook delivery uses events/subscribe",
                    Some(json!({"feature": "deliveryMode"})),
                )
            }
        }
        let Some(url) = delivery.get("url").and_then(Value::as_str) else {
            return invalid_params(id, "delivery.url is required");
        };
        let Some(secret) = delivery.get("secret").and_then(Value::as_str) else {
            return invalid_params(id, "delivery.secret is required");
        };
        if let Err(e) = webhook::validate_callback_url(url, config.allow_loopback_http) {
            return invalid_params(id, e.0);
        }
        let secret = match webhook::decode_secret(secret) {
            Ok(s) => s,
            Err(e) => return invalid_params(id, e.0),
        };
        let Some(entry) = self
            .session_catalogue(ctx)
            .into_iter()
            .find(|e| e.qualified_name == params.name)
        else {
            return not_found(id);
        };
        let Some(client) = self.client(&ctx.client_id) else {
            return not_found(id);
        };
        self.claim(&ctx.session_key, &entry.qualified_name);
        let args_key = canonical_json(&Value::Object(params.arguments.clone()));
        let sub_id = Self::webhook_id(&ctx.client_id, url, &entry.qualified_name, &args_key);
        let ttl_secs = raw.get("ttlMs").and_then(Value::as_u64).map(|ms| ms / 1000);
        let lease = config.limits.webhook_lease(ttl_secs);
        let expires_at = chrono::Utc::now()
            + chrono::Duration::from_std(lease).unwrap_or_else(|_| chrono::Duration::hours(1));
        let names = vec![entry.qualified_name.clone()];
        let scope = CursorScope::new(&names, &params.arguments);

        if let Some(existing) = self.webhooks.get(&sub_id).map(|w| w.clone()) {
            *existing.expires_at.lock() = expires_at;
            let resume = *existing.state.lock() == WebhookState::Suspended;
            if resume {
                *existing.state.lock() = WebhookState::Active;
                self.spawn_webhook_worker(existing.clone());
            }
            let position = existing.position.load(Ordering::SeqCst);
            return JsonRpcResponse::success(
                id,
                json!({
                    "id": sub_id,
                    "refreshBefore": expires_at.to_rfc3339_opts(chrono::SecondsFormat::Secs, true),
                    "cursor": self.cursor_key.issue(&ctx.client_id, &scope, position),
                    "truncated": false,
                }),
            );
        }
        let active = self
            .webhooks
            .iter()
            .filter(|w| w.client_id == ctx.client_id)
            .count();
        if active >= config.limits.webhook_max_subscriptions {
            return error_response(
                id,
                ERROR_RESOURCE_EXHAUSTED,
                "Too many event subscriptions",
                Some(
                    json!({"limit": "subscriptions", "max": config.limits.webhook_max_subscriptions}),
                ),
            );
        }
        let (position, reset) =
            match self.position(&ctx.client_id, &scope, params.cursor.as_deref()) {
                Ok(p) => p,
                Err(e) => return invalid_params(id, e.message()),
            };
        if let Err(reason) = webhook::verify_endpoint(&self.http, url, &sub_id, &secret).await {
            return invalid_params(id, format!("Callback verification failed: {reason}"));
        }
        let approved = client.push_webhook_urls.iter().any(|u| u == url)
            || self
                .approved_urls
                .contains(&(ctx.client_id.clone(), url.to_string()));
        let subscription = Arc::new(WebhookSubscription {
            id: sub_id.clone(),
            client_id: ctx.client_id.clone(),
            client_name: client.name.clone(),
            entry: entry.clone(),
            arguments: params.arguments.clone(),
            url: url.to_string(),
            secret,
            expires_at: Mutex::new(expires_at),
            position: AtomicU64::new(position),
            state: Mutex::new(if approved {
                WebhookState::Active
            } else {
                WebhookState::AwaitingApproval
            }),
            delivered: AtomicU64::new(0),
            last_error: Mutex::new(None),
            cancel: CancellationToken::new(),
        });
        self.webhooks.insert(sub_id.clone(), subscription.clone());
        let keys = self
            .subscription_for(
                &ctx.client_id,
                Some(&ctx.session_key),
                &entry,
                &params.arguments,
            )
            .into_iter()
            .collect();
        self.set_holder(
            format!("webhook:{sub_id}"),
            &ctx.client_id,
            HolderKind::Webhook,
            keys,
            None,
        );
        self.spawn_webhook_worker(subscription);
        JsonRpcResponse::success(
            id,
            json!({
                "id": sub_id,
                "refreshBefore": expires_at.to_rfc3339_opts(chrono::SecondsFormat::Secs, true),
                "cursor": self.cursor_key.issue(&ctx.client_id, &scope, position),
                "truncated": reset,
            }),
        )
    }

    fn handle_unsubscribe(
        &self,
        ctx: &PushRelaySessionContext,
        request: &JsonRpcRequest,
    ) -> JsonRpcResponse {
        let id = request.id.clone().unwrap_or(Value::Null);
        let params = match parse_event_params(request.params.as_ref()) {
            Ok(p) => p,
            Err(e) => return invalid_params(id, e),
        };
        let Some(url) = request
            .params
            .as_ref()
            .and_then(|p| p.pointer("/delivery/url"))
            .and_then(Value::as_str)
        else {
            return invalid_params(id, "delivery.url is required");
        };
        let args_key = canonical_json(&Value::Object(params.arguments));
        let sub_id = Self::webhook_id(&ctx.client_id, url, &params.name, &args_key);
        if !self.end_webhook(&sub_id) {
            return not_found(id);
        }
        JsonRpcResponse::success(id, json!({"resultType": "complete"}))
    }

    fn end_webhook(&self, sub_id: &str) -> bool {
        let Some((_, subscription)) = self.webhooks.remove(sub_id) else {
            return false;
        };
        subscription.cancel.cancel();
        self.remove_holder(&format!("webhook:{sub_id}"));
        true
    }

    fn spawn_webhook_worker(&self, subscription: Arc<WebhookSubscription>) {
        let relay = self.this();
        tokio::spawn(async move { relay.webhook_worker(subscription).await });
    }

    async fn approve_webhook_url(&self, subscription: &WebhookSubscription) -> bool {
        let details = PushEventApprovalDetails {
            kind: PushApprovalKind::WebhookUrl,
            hold_id: format!("webhook:{}", subscription.id),
            server_id: subscription.entry.server_id.clone(),
            server_name: subscription.entry.server_slug.clone(),
            event_name: subscription.entry.upstream_name.clone(),
            qualified_name: subscription.entry.qualified_name.clone(),
            event_id: String::new(),
            ingress_mode: String::new(),
            data: json!({"url": subscription.url}),
            rendered_content: format!(
                "{} wants {} delivered to {}",
                subscription.client_name, subscription.entry.qualified_name, subscription.url
            ),
            webhook_url: Some(subscription.url.clone()),
            pending_for_client: 0,
            pending_of_type: 0,
        };
        let timeout = self.limits.read().ask_timeout();
        let approval = self.gateway.firewall_manager.request_push_event_approval(
            subscription.client_id.clone(),
            subscription.client_name.clone(),
            details,
            timeout.as_secs(),
        );
        let decision = tokio::select! {
            _ = subscription.cancel.cancelled() => return false,
            response = approval => response
                .map(|r| PushDecisionAction::from(&r.action))
                .unwrap_or(PushDecisionAction::Deny),
        };
        if !decision.allows() {
            return false;
        }
        self.approved_urls
            .insert((subscription.client_id.clone(), subscription.url.clone()));
        if decision.is_permanent() {
            self.persist(
                &subscription.client_id,
                PushConfigChange::WebhookUrl {
                    url: subscription.url.clone(),
                },
            )
            .await;
        }
        true
    }

    async fn webhook_worker(&self, subscription: Arc<WebhookSubscription>) {
        if *subscription.state.lock() == WebhookState::AwaitingApproval {
            if !self.approve_webhook_url(&subscription).await {
                *subscription.state.lock() = WebhookState::Denied;
                return;
            }
            *subscription.state.lock() = WebhookState::Active;
        }
        let names = vec![subscription.entry.qualified_name.clone()];
        let read_scope = Self::read_scope(&names, &subscription.arguments);
        let cursor_scope = CursorScope::new(&names, &subscription.arguments);
        let journal = self.journal(&subscription.client_id);
        let mut changed = journal.lock().subscribe();
        loop {
            changed.borrow_and_update();
            let position = subscription.position.load(Ordering::SeqCst);
            let batch = journal.lock().read(position, &read_scope, 50);
            for entry in &batch.entries {
                let mut body = entry.wire();
                body["cursor"] =
                    json!(self
                        .cursor_key
                        .issue(&subscription.client_id, &cursor_scope, entry.seq));
                let delays = self.limits.read().webhook_retry_delays_secs.clone();
                let mut attempt = 0usize;
                loop {
                    let result = webhook::deliver_event(
                        &self.http,
                        &subscription.url,
                        &subscription.id,
                        &subscription.secret,
                        &entry.occurrence.event_id,
                        &body,
                    )
                    .await;
                    match result {
                        Ok(()) => {
                            subscription.delivered.fetch_add(1, Ordering::SeqCst);
                            *subscription.last_error.lock() = None;
                            break;
                        }
                        Err(error) => {
                            *subscription.last_error.lock() = Some(error);
                            let Some(delay) = delays.get(attempt) else {
                                *subscription.state.lock() = WebhookState::Suspended;
                                return;
                            };
                            attempt += 1;
                            tokio::select! {
                                _ = subscription.cancel.cancelled() => return,
                                _ = tokio::time::sleep(Duration::from_secs(*delay)) => {}
                            }
                        }
                    }
                }
                subscription.position.store(entry.seq, Ordering::SeqCst);
            }
            subscription
                .position
                .fetch_max(batch.position, Ordering::SeqCst);
            if batch.has_more {
                continue;
            }
            tokio::select! {
                _ = subscription.cancel.cancelled() => return,
                result = changed.changed() => if result.is_err() { return },
            }
        }
    }

    /// Webhook subscriptions (all clients, or one).
    pub fn webhook_subscriptions(&self, client_id: Option<&str>) -> Vec<WebhookSubscriptionInfo> {
        self.webhooks
            .iter()
            .filter(|w| client_id.is_none_or(|id| id == w.client_id))
            .map(|w| WebhookSubscriptionInfo {
                id: w.id.clone(),
                client_id: w.client_id.clone(),
                event: w.entry.qualified_name.clone(),
                url: w.url.clone(),
                state: w.state.lock().as_str().to_string(),
                expires_at: *w.expires_at.lock(),
                delivered: w.delivered.load(Ordering::SeqCst),
                last_error: w.last_error.lock().clone(),
            })
            .collect()
    }

    // ---------------------------------------------------------------
    // Claude Channels egress (D4)
    // ---------------------------------------------------------------

    /// Start (or stop) channel delivery for a STDIO bridge session.
    fn ensure_channel(&self, ctx: &PushRelaySessionContext) {
        let catalogue = self.session_catalogue(ctx);
        let holder_id = format!("channel:{}", ctx.session_key);
        if catalogue.is_empty() {
            if let Some(mut session) = self.sessions.get_mut(&ctx.session_key) {
                if let Some(cancel) = session.channel.take() {
                    cancel.cancel();
                }
            }
            self.remove_holder(&holder_id);
            return;
        }
        let keys: HashMap<SubscriptionKey, SubscriptionSpec> = catalogue
            .iter()
            .filter_map(|e| {
                self.subscription_for(&ctx.client_id, Some(&ctx.session_key), e, &Map::new())
            })
            .collect();
        let unchanged = self
            .holders
            .lock()
            .get(&holder_id)
            .is_some_and(|h| h.keys.keys().collect::<HashSet<_>>() == keys.keys().collect());
        if !unchanged {
            self.set_holder(holder_id, &ctx.client_id, HolderKind::Channel, keys, None);
        }
        let start = {
            let Some(mut session) = self.sessions.get_mut(&ctx.session_key) else {
                return;
            };
            if session.channel.is_some() {
                None
            } else {
                let cancel = CancellationToken::new();
                session.channel = Some(cancel.clone());
                Some(cancel)
            }
        };
        if let Some(cancel) = start {
            let relay = self.this();
            let ctx = ctx.clone();
            tokio::spawn(async move { relay.channel_pump(ctx, cancel).await });
        }
    }

    fn send_channel(&self, session_key: &str, content: String, meta: Value) {
        self.gateway.send_session_notification(
            session_key,
            notification(
                CLAUDE_CHANNEL_NOTIFICATION,
                json!({ "content": content, "meta": meta }),
            ),
        );
    }

    async fn channel_pump(&self, ctx: PushRelaySessionContext, cancel: CancellationToken) {
        let journal = self.journal(&ctx.client_id);
        let mut changed = journal.lock().subscribe();
        let mut position = journal.lock().now();
        let mut sent: VecDeque<Instant> = VecDeque::new();
        let mut coalesced = 0usize;
        let mut recent: VecDeque<String> = VecDeque::new();
        loop {
            changed.borrow_and_update();
            let batch = journal.lock().read(position, &ReadScope::default(), 100);
            let (rate, max_content) = {
                let limits = self.limits.read();
                (
                    limits.channel_rate_per_minute.max(1) as usize,
                    limits.channel_max_content_bytes.max(64),
                )
            };
            for entry in &batch.entries {
                let claimed = self
                    .sessions
                    .get(&ctx.session_key)
                    .is_some_and(|s| s.claimed.contains(&entry.occurrence.name));
                if claimed || recent.contains(&entry.occurrence.event_id) {
                    continue;
                }
                recent.push_back(entry.occurrence.event_id.clone());
                if recent.len() > CHANNEL_DEDUPE_WINDOW {
                    recent.pop_front();
                }
                let now = Instant::now();
                while sent
                    .front()
                    .is_some_and(|t| now.duration_since(*t) > Duration::from_secs(60))
                {
                    sent.pop_front();
                }
                if sent.len() >= rate {
                    coalesced += 1;
                    continue;
                }
                sent.push_back(now);
                self.send_channel(
                    &ctx.session_key,
                    render::channel_content(entry, max_content),
                    json!({
                        "source": entry.occurrence.server_slug,
                        "event": entry.occurrence.name,
                        "event_id": entry.occurrence.event_id,
                    }),
                );
            }
            position = batch.position;
            if batch.has_more {
                continue;
            }
            if coalesced > 0 {
                let now = Instant::now();
                while sent
                    .front()
                    .is_some_and(|t| now.duration_since(*t) > Duration::from_secs(60))
                {
                    sent.pop_front();
                }
                if sent.len() < rate {
                    sent.push_back(now);
                    self.send_channel(
                        &ctx.session_key,
                        render::coalesced_content(coalesced),
                        json!({"source": "localrouter", "event": "events.coalesced"}),
                    );
                    coalesced = 0;
                }
            }
            let wait = if coalesced > 0 {
                Duration::from_secs(1)
            } else {
                Duration::from_secs(3600)
            };
            tokio::select! {
                _ = cancel.cancelled() => return,
                result = changed.changed() => if result.is_err() { return },
                _ = tokio::time::sleep(wait) => {}
            }
        }
    }

    // ---------------------------------------------------------------
    // Fallback tools (D5) and /events/watch
    // ---------------------------------------------------------------

    fn tools_view_for_ctx(&self, ctx: &PushRelaySessionContext) -> ToolsView {
        let Some(client) = self.client(&ctx.client_id) else {
            return ToolsView::default();
        };
        self.tools_view(&PushToolsState {
            client_id: ctx.client_id.clone(),
            session_key: Some(ctx.session_key.clone()),
            fallback: client.push_fallback_tools,
            channels_launched: client.claude_channels_launched,
            is_stdio_bridge: ctx.is_stdio_bridge,
            declares_events: ctx.client_declares_events(),
            visible_servers: ctx.visible_servers.clone(),
        })
    }

    /// Which fallback tools a session sees ("When the fallback tools are shown").
    pub fn tools_view(&self, state: &PushToolsState) -> ToolsView {
        let gate = self
            .client(&state.client_id)
            .is_some_and(|client| !self.catalogue(&client, &state.visible_servers).is_empty());
        if !gate {
            return ToolsView::default();
        }
        let all = ToolsView {
            gate,
            list: true,
            poll: true,
            wait: true,
        };
        match state.fallback {
            PushFallbackTools::Never => ToolsView {
                gate,
                ..Default::default()
            },
            PushFallbackTools::Always => all,
            PushFallbackTools::Auto if state.declares_events => ToolsView {
                gate,
                ..Default::default()
            },
            PushFallbackTools::Auto if state.is_stdio_bridge && state.channels_launched => {
                ToolsView { wait: false, ..all }
            }
            PushFallbackTools::Auto => all,
        }
    }

    /// The `<events>` instructions for a session, or `None` when the gate fails.
    pub fn tools_instructions(&self, state: &PushToolsState) -> Option<String> {
        let view = self.tools_view(state);
        if !view.gate {
            return None;
        }
        let client = self.client(&state.client_id)?;
        let names = self
            .catalogue(&client, &state.visible_servers)
            .into_iter()
            .map(|e| e.qualified_name)
            .collect();
        Some(render::instructions(&render::InstructionsInput {
            show_poll: view.poll,
            show_wait: view.wait,
            channels: state.is_stdio_bridge,
            event_names: names,
        }))
    }

    /// `EventList`.
    pub fn tool_list(&self, state: &PushToolsState) -> String {
        let Some(client) = self.client(&state.client_id) else {
            return "No events are available.".into();
        };
        let catalogue = self.catalogue(&client, &state.visible_servers);
        if catalogue.is_empty() {
            return "No events are available.".into();
        }
        let native = if self.config.read().webhook_egress_enabled {
            "events/stream, events/poll, events/subscribe (webhook)"
        } else {
            "events/stream, events/poll"
        };
        let mut out = format!("{} event type(s):\n", catalogue.len());
        for entry in &catalogue {
            let description = entry
                .descriptor
                .get("description")
                .and_then(Value::as_str)
                .unwrap_or(if entry.is_channel {
                    "Claude Code channel messages from this server"
                } else {
                    ""
                });
            out.push_str(&format!(
                "\n- {} (server: {}){}{}",
                entry.qualified_name,
                entry.server_name,
                if description.is_empty() { "" } else { " — " },
                description
            ));
            if let Some(schema) = entry
                .descriptor
                .get("inputSchema")
                .and_then(|s| s.get("properties"))
                .filter(|p| p.as_object().is_some_and(|o| !o.is_empty()))
            {
                out.push_str(&format!("\n  filter: {}", schema));
            }
            if entry.permission == PushPermission::Ask {
                out.push_str("\n  (each event is approved by the user before delivery)");
            }
        }
        out.push_str(&format!(
            "\n\nRead with {}. Native MCP Events methods also available: {native}.",
            render::EVENT_POLL_TOOL
        ));
        out
    }

    fn tool_scope(
        &self,
        state: &PushToolsState,
        arguments: &Value,
    ) -> Result<(Vec<CatalogueEntry>, Map<String, Value>), String> {
        let client = self
            .client(&state.client_id)
            .ok_or("No events are available.")?;
        let catalogue = self.catalogue(&client, &state.visible_servers);
        let requested: Option<Vec<String>> = match arguments.get("events") {
            None | Some(Value::Null) => None,
            Some(Value::String(s)) => Some(vec![s.clone()]),
            Some(Value::Array(items)) => Some(
                items
                    .iter()
                    .map(|v| v.as_str().map(str::to_string).ok_or("events must be names"))
                    .collect::<Result<_, _>>()?,
            ),
            Some(_) => return Err("events must be an array of event names".into()),
        };
        let entries: Vec<CatalogueEntry> = match requested {
            None => catalogue,
            Some(names) => {
                let mut out = Vec::new();
                for name in names {
                    let entry = catalogue
                        .iter()
                        .find(|e| e.qualified_name == name)
                        .ok_or_else(|| {
                            format!(
                                "Unknown event type: {name}. Call {} to see them.",
                                render::EVENT_LIST_TOOL
                            )
                        })?;
                    out.push(entry.clone());
                }
                out
            }
        };
        if entries.is_empty() {
            return Err("No events are available.".into());
        }
        let filter = match arguments.get("filter") {
            None | Some(Value::Null) => Map::new(),
            Some(Value::Object(map)) => map.clone(),
            Some(_) => return Err("filter must be an object".into()),
        };
        Ok((entries, filter))
    }

    /// `EventPoll`.
    pub async fn tool_poll(
        &self,
        state: &PushToolsState,
        arguments: &Value,
    ) -> Result<String, String> {
        let (entries, filter) = self.tool_scope(state, arguments)?;
        let limits = self.limits.read().clone();
        let max = TOOL_POLL_MAX.min(limits.poll_max_events.max(1));
        let limit = arguments
            .get("limit")
            .and_then(Value::as_u64)
            .map(|n| n as usize)
            .unwrap_or(TOOL_POLL_DEFAULT)
            .clamp(1, max);
        let wait = Duration::from_secs(
            arguments
                .get("wait_seconds")
                .and_then(Value::as_u64)
                .unwrap_or(0)
                .min(limits.poll_max_wait_secs),
        );
        let cursor = arguments.get("cursor").and_then(Value::as_str);
        self.hold_poll_demand(
            &state.client_id,
            state.session_key.as_deref(),
            HolderKind::Tools,
            &entries,
            &filter,
        );
        let names: Vec<String> = entries.iter().map(|e| e.qualified_name.clone()).collect();
        let scope = CursorScope::new(&names, &filter);
        let (position, reset) = self
            .position(&state.client_id, &scope, cursor)
            .map_err(|e| e.message().to_string())?;
        let read_scope = Self::read_scope(&names, &filter);
        let journal = self.journal(&state.client_id);
        let mut batch = journal.lock().read(position, &read_scope, limit);
        if batch.entries.is_empty() && !wait.is_zero() {
            let deadline = Instant::now() + wait;
            while batch.entries.is_empty() {
                let remaining = deadline.saturating_duration_since(Instant::now());
                if remaining.is_zero() {
                    break;
                }
                self.wait_for_entries(&state.client_id, batch.position, remaining)
                    .await;
                batch = journal.lock().read(position, &read_scope, limit);
            }
        }
        let view = self.tools_view(state);
        let mut out = String::new();
        if reset || batch.truncated {
            out.push_str("(some earlier events were dropped before they were read)\n\n");
        }
        if batch.entries.is_empty() {
            out.push_str(if cursor.is_none() {
                "No events yet; reading starts now.\n"
            } else {
                "No new events.\n"
            });
        }
        for entry in &batch.entries {
            out.push_str(&render::event_block(entry));
            out.push('\n');
        }
        out.push_str(&format!(
            "\nnext_cursor: {}\n",
            self.cursor_key
                .issue(&state.client_id, &scope, batch.position)
        ));
        if !batch.has_more {
            out.push_str("(no more events)\n");
        }
        out.push('\n');
        out.push_str(&render::poll_trailer(view.wait));
        Ok(out)
    }

    fn watch_token(
        &self,
        client_id: &str,
        names: &[String],
        filter: &Map<String, Value>,
    ) -> String {
        use base64::Engine;
        let expires =
            chrono::Utc::now().timestamp() + self.limits.read().watch_max_lifetime_secs as i64;
        let payload = json!({"c": client_id, "n": names, "f": filter, "e": expires}).to_string();
        let payload = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(payload);
        let signature = self.cursor_key.sign("watch", &payload);
        format!("{payload}.{signature}")
    }

    /// `EventWait`: a ready-to-run watcher command against `/events/watch`.
    pub fn tool_wait(&self, state: &PushToolsState, arguments: &Value) -> Result<String, String> {
        let (entries, filter) = self.tool_scope(state, arguments)?;
        let base = self.local_base_url.read().clone().ok_or(
            "LocalRouter's server address is not known yet; use EventPoll with wait_seconds.",
        )?;
        let names: Vec<String> = entries.iter().map(|e| e.qualified_name.clone()).collect();
        let scope = CursorScope::new(&names, &filter);
        let cursor = match arguments.get("cursor").and_then(Value::as_str) {
            Some(cursor) => {
                self.position(&state.client_id, &scope, Some(cursor))
                    .map_err(|e| e.message().to_string())?;
                cursor.to_string()
            }
            None => {
                let now = self.journal(&state.client_id).lock().now();
                self.cursor_key.issue(&state.client_id, &scope, now)
            }
        };
        self.hold_poll_demand(
            &state.client_id,
            state.session_key.as_deref(),
            HolderKind::Tools,
            &entries,
            &filter,
        );
        let token = self.watch_token(&state.client_id, &names, &filter);
        let url = format!(
            "{}/events/watch?token={}&cursor={}",
            base.trim_end_matches('/'),
            urlencoding::encode(&token),
            urlencoding::encode(&cursor)
        );
        let scope_text = if names.len() == 1 {
            names[0].clone()
        } else {
            format!("{} event types", names.len())
        };
        Ok(render::wait_commands(
            &url,
            &scope_text,
            self.limits.read().watch_max_lifetime_secs,
            &cursor,
        ))
    }

    /// Open `/events/watch`: verify the token minted by `EventWait`, then
    /// stream one JSON object per event (with its cursor) until the
    /// lifetime ends. The stream holds the upstream subscriptions open.
    pub fn open_watch(
        &self,
        token: &str,
        cursor: Option<&str>,
    ) -> Result<BoxStream<'static, Value>, WatchError> {
        use base64::Engine;
        let (payload_b64, signature) = token.split_once('.').ok_or(WatchError::Unauthorized)?;
        if !self.cursor_key.verify("watch", payload_b64, signature) {
            return Err(WatchError::Unauthorized);
        }
        let payload: Value = base64::engine::general_purpose::URL_SAFE_NO_PAD
            .decode(payload_b64)
            .ok()
            .and_then(|b| serde_json::from_slice(&b).ok())
            .ok_or(WatchError::Unauthorized)?;
        if payload.get("e").and_then(Value::as_i64).unwrap_or(0) < chrono::Utc::now().timestamp() {
            return Err(WatchError::Expired);
        }
        let client_id = payload
            .get("c")
            .and_then(Value::as_str)
            .ok_or(WatchError::Unauthorized)?
            .to_string();
        let names: Vec<String> = payload
            .get("n")
            .and_then(Value::as_array)
            .map(|a| {
                a.iter()
                    .filter_map(|v| v.as_str().map(str::to_string))
                    .collect()
            })
            .unwrap_or_default();
        let filter = payload
            .get("f")
            .and_then(Value::as_object)
            .cloned()
            .unwrap_or_default();
        let client = self.client(&client_id).ok_or(WatchError::Forbidden)?;
        let visible = self.visible_servers(&client);
        let entries: Vec<CatalogueEntry> = self
            .catalogue(&client, &visible)
            .into_iter()
            .filter(|e| names.contains(&e.qualified_name))
            .collect();
        if entries.is_empty() {
            return Err(WatchError::Forbidden);
        }
        let scope = CursorScope::new(&names, &filter);
        let (position, reset) = self
            .position(&client_id, &scope, cursor)
            .map_err(|e| WatchError::BadCursor(e.message().to_string()))?;
        let holder_id = format!("watch:{}", uuid::Uuid::new_v4());
        let keys = entries
            .iter()
            .filter_map(|e| self.subscription_for(&client_id, None, e, &filter))
            .collect();
        self.set_holder(holder_id.clone(), &client_id, HolderKind::Watch, keys, None);
        let relay = self.this();
        let lifetime = self.limits.read().watch_max_lifetime();
        let read_scope = Self::read_scope(&names, &filter);
        let stream = async_stream::stream! {
            let _guard = HolderGuard { relay: relay.clone(), id: holder_id };
            let deadline = tokio::time::Instant::now() + lifetime;
            let journal = relay.journal(&client_id);
            let mut changed = journal.lock().subscribe();
            let mut position = position;
            if reset {
                yield json!({"type": "gap", "truncated": true,
                    "cursor": relay.cursor_key.issue(&client_id, &scope, position)});
            }
            loop {
                changed.borrow_and_update();
                let batch = journal.lock().read(position, &read_scope, 100);
                if batch.truncated {
                    yield json!({"type": "gap", "truncated": true,
                        "cursor": relay.cursor_key.issue(&client_id, &scope, batch.start)});
                }
                for entry in &batch.entries {
                    let mut value = entry.wire();
                    value["cursor"] = json!(relay.cursor_key.issue(&client_id, &scope, entry.seq));
                    yield value;
                }
                position = batch.position;
                if batch.has_more {
                    continue;
                }
                tokio::select! {
                    _ = tokio::time::sleep_until(deadline) => break,
                    result = changed.changed() => if result.is_err() { break },
                }
            }
        };
        Ok(Box::pin(stream))
    }

    /// Dispatch a fallback tool call.
    pub async fn handle_tool(
        &self,
        state: &PushToolsState,
        tool_name: &str,
        arguments: &Value,
    ) -> Result<String, String> {
        match tool_name {
            render::EVENT_LIST_TOOL => Ok(self.tool_list(state)),
            render::EVENT_POLL_TOOL => self.tool_poll(state, arguments).await,
            render::EVENT_WAIT_TOOL => self.tool_wait(state, arguments),
            other => Err(format!("Unknown tool {other}")),
        }
    }
}

/// Why `/events/watch` refused a request.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WatchError {
    Unauthorized,
    Expired,
    Forbidden,
    BadCursor(String),
}

impl WatchError {
    pub fn message(&self) -> String {
        match self {
            Self::Unauthorized => "Invalid watch token".into(),
            Self::Expired => "Watch token expired; call EventWait again".into(),
            Self::Forbidden => "These events are no longer available to this client".into(),
            Self::BadCursor(m) => m.clone(),
        }
    }
}

/// Removes a demand holder when the consumer goes away.
struct HolderGuard {
    relay: Arc<PushRelay>,
    id: String,
}

impl Drop for HolderGuard {
    fn drop(&mut self) {
        self.relay.remove_holder(&self.id);
    }
}

#[async_trait]
impl PushRelayService for PushRelay {
    fn advertisement(&self, ctx: &PushRelaySessionContext) -> PushAdvertisement {
        self.observe(ctx);
        let mut ad = PushAdvertisement::default();
        if self.session_catalogue(ctx).is_empty() {
            return ad;
        }
        ad.extensions.insert(
            EVENTS_EXTENSION_ID.to_string(),
            json!({"listChanged": true}),
        );
        ad.capabilities
            .insert(EVENTS_TOP_LEVEL_CAPABILITY.to_string(), json!({}));
        if ctx.is_stdio_bridge {
            ad.experimental
                .insert(CLAUDE_CHANNEL_CAPABILITY.to_string(), json!({}));
        }
        ad
    }

    fn listen_opt_ins(&self, client_id: &str, session_key: &str) -> Vec<super::ListenOptIn> {
        let gate = self
            .sessions
            .get(session_key)
            .map(|s| s.ctx.clone())
            .map(|ctx| !self.session_catalogue(&ctx).is_empty())
            .unwrap_or_else(|| {
                self.client(client_id).is_some_and(|client| {
                    let visible = self.visible_servers(&client);
                    !self.catalogue(&client, &visible).is_empty()
                })
            });
        if !gate {
            return Vec::new();
        }
        vec![
            super::ListenOptIn {
                opt_in: "events".to_string(),
                method_prefix: EVENTS_NOTIFICATION_PREFIX.to_string(),
            },
            super::ListenOptIn {
                opt_in: "claudeChannel".to_string(),
                method_prefix: CLAUDE_CHANNEL_NOTIFICATION.to_string(),
            },
        ]
    }

    fn on_upstream_notification(&self, n: UpstreamPushNotification) -> bool {
        if !self.config.read().enabled {
            return false;
        }
        if n.notification.method == CLAUDE_CHANNEL_NOTIFICATION {
            if let Some(occurrence) = self.channel_occurrence(&n.server_id, &n.notification) {
                self.admit(&n.client_id, occurrence);
            }
        }
        // Uncorrelated `notifications/events/*` belong to no stream the relay
        // opened; nothing is forwarded raw.
        true
    }

    fn on_client_cancelled(&self, session_key: &str, request_id: &Value) {
        if let Some(cancel) = self
            .streams
            .get(&session_stream_key(session_key, request_id))
        {
            cancel.cancel();
        }
    }

    async fn events_list(
        &self,
        ctx: &PushRelaySessionContext,
        request: JsonRpcRequest,
    ) -> AppResult<JsonRpcResponse> {
        self.observe(ctx);
        Ok(self.handle_list(ctx, &request))
    }

    async fn events_poll(
        &self,
        ctx: &PushRelaySessionContext,
        request: JsonRpcRequest,
    ) -> AppResult<JsonRpcResponse> {
        self.observe(ctx);
        Ok(self.handle_poll(ctx, request).await)
    }

    async fn events_stream(
        &self,
        ctx: &PushRelaySessionContext,
        request: JsonRpcRequest,
    ) -> AppResult<JsonRpcResponse> {
        self.observe(ctx);
        Ok(self.handle_stream(ctx, request).await)
    }

    async fn events_subscribe(
        &self,
        ctx: &PushRelaySessionContext,
        request: JsonRpcRequest,
    ) -> AppResult<JsonRpcResponse> {
        self.observe(ctx);
        Ok(self.handle_subscribe(ctx, request).await)
    }

    async fn events_unsubscribe(
        &self,
        ctx: &PushRelaySessionContext,
        request: JsonRpcRequest,
    ) -> AppResult<JsonRpcResponse> {
        self.observe(ctx);
        Ok(self.handle_unsubscribe(ctx, &request))
    }
}

/// Serde helper so the UI can send decisions as JSON.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PushDecisionRequest {
    pub hold_id: String,
    #[serde(flatten)]
    pub decision: PushDecision,
}
