//! Push Relay configuration: global settings and limits, per-client push
//! permissions, and the upstream capabilities remembered on each MCP server.
//!
//! See `plan/2026-10-30-PUSH_RELAY.md`.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap};
use std::time::Duration;

use crate::{McpAuthConfig, McpServerConfig, McpTransportConfig};

fn default_true() -> bool {
    true
}

pub(crate) fn is_false(value: &bool) -> bool {
    !*value
}

/// Block / Ask / Allow for pushed events.
#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum PushPermission {
    /// Relayed to the client and recorded in Monitor.
    #[default]
    Allow,
    /// Each occurrence is held until the user approves, edits or denies it.
    Ask,
    /// Never relayed; the event type is hidden from the client.
    Block,
}

/// Global Push Relay settings (`push_relay` in the config).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PushRelayConfig {
    /// Master switch. When off, no push is advertised or relayed to anyone.
    #[serde(default = "default_true")]
    pub enabled: bool,

    /// Permission applied when neither the client nor its server/event
    /// overrides say otherwise.
    #[serde(default)]
    pub default_permission: PushPermission,

    /// Whether clients may subscribe by webhook (`events/subscribe`), with
    /// LocalRouter signing and POSTing each event.
    #[serde(default = "default_true")]
    pub webhook_egress_enabled: bool,

    /// Accept `http://` callback URLs that point at a loopback address.
    #[serde(default = "default_true")]
    pub allow_loopback_http: bool,

    /// Bounds on memory, waiting and retries.
    #[serde(default, skip_serializing_if = "PushRelayLimits::is_default")]
    pub limits: PushRelayLimits,
}

impl Default for PushRelayConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            default_permission: PushPermission::Allow,
            webhook_egress_enabled: true,
            allow_loopback_http: true,
            limits: PushRelayLimits::default(),
        }
    }
}

impl PushRelayConfig {
    pub fn is_default(&self) -> bool {
        self == &Self::default()
    }
}

const MIB: u64 = 1024 * 1024;

/// Every limit from the plan's "Limits" table. Durations are stored as whole
/// seconds (`_secs`) or milliseconds (`_ms`); use the accessor methods for a
/// [`Duration`].
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default)]
pub struct PushRelayLimits {
    /// Occurrences held per client for polling and replay.
    pub journal_max_events: usize,
    /// Same, by total size of event data per client (first limit wins).
    pub journal_max_bytes: u64,
    /// Age after which journal entries are dropped.
    pub journal_retention_secs: u64,
    /// Largest accepted `data` of one occurrence; larger events are dropped.
    pub max_event_bytes: u64,
    /// A held (Ask) event is denied after this long without a decision.
    pub ask_timeout_secs: u64,
    /// Held events per client awaiting a decision; further Ask events are denied.
    pub ask_hold_cap: usize,
    /// How long `initialize` / `server/discover` waits for a server with no
    /// (or a stale) capability snapshot.
    pub first_discovery_wait_ms: u64,
    /// Lower clamp on an upstream's `nextPollMs`.
    pub upstream_poll_min_interval_ms: u64,
    /// Upper clamp on an upstream's `nextPollMs`.
    pub upstream_poll_max_interval_ms: u64,
    /// First delay before reconnecting a dropped upstream `events/stream`.
    pub upstream_reconnect_backoff_min_ms: u64,
    /// Largest reconnect delay (the backoff is jittered between the bounds).
    pub upstream_reconnect_backoff_max_ms: u64,
    /// An upstream stream with no heartbeat for this long is treated as dead.
    pub upstream_heartbeat_timeout_secs: u64,
    /// Interval of `notifications/events/heartbeat` sent to clients.
    pub downstream_heartbeat_secs: u64,
    /// Concurrent `events/stream` requests per client session.
    pub max_streams_per_session: usize,
    /// Ceiling on events per `events/poll` / `EventPoll` page.
    pub poll_max_events: usize,
    /// Page size of `EventPoll` when the caller gives no limit.
    pub poll_default_events: usize,
    /// Ceiling on `EventPoll`'s `wait_seconds`.
    pub poll_max_wait_secs: u64,
    /// Lifetime of one `/events/watch` connection.
    pub watch_max_lifetime_secs: u64,
    /// Rendered `content` per channel message; longer content is truncated.
    pub channel_max_content_bytes: usize,
    /// Channel messages per minute per session before coalescing.
    pub channel_rate_per_minute: u32,
    /// Timeout of one webhook delivery attempt.
    pub webhook_timeout_secs: u64,
    /// Delay before each retry of a failed webhook delivery. The first attempt
    /// plus one retry per entry is the total number of attempts; after the
    /// last the subscription is suspended until refreshed.
    pub webhook_retry_delays_secs: Vec<u64>,
    /// Active webhook subscriptions per client.
    pub webhook_max_subscriptions: usize,
    /// Webhook subscription TTL when the client asks for none.
    pub webhook_lease_default_secs: u64,
    /// Shortest webhook subscription TTL accepted.
    pub webhook_lease_min_secs: u64,
    /// Longest webhook subscription TTL accepted.
    pub webhook_lease_max_secs: u64,
}

impl Default for PushRelayLimits {
    fn default() -> Self {
        Self {
            journal_max_events: 10_000,
            journal_max_bytes: 64 * MIB,
            journal_retention_secs: 24 * 60 * 60,
            max_event_bytes: MIB,
            ask_timeout_secs: 30 * 60,
            ask_hold_cap: 1_000,
            first_discovery_wait_ms: 10_000,
            upstream_poll_min_interval_ms: 1_000,
            upstream_poll_max_interval_ms: 5 * 60 * 1_000,
            upstream_reconnect_backoff_min_ms: 1_000,
            upstream_reconnect_backoff_max_ms: 60_000,
            upstream_heartbeat_timeout_secs: 90,
            downstream_heartbeat_secs: 30,
            max_streams_per_session: 64,
            poll_max_events: 500,
            poll_default_events: 50,
            poll_max_wait_secs: 50,
            watch_max_lifetime_secs: 24 * 60 * 60,
            channel_max_content_bytes: 16 * 1024,
            channel_rate_per_minute: 60,
            webhook_timeout_secs: 10,
            webhook_retry_delays_secs: vec![5, 30, 120, 600, 1_800],
            webhook_max_subscriptions: 100,
            webhook_lease_default_secs: 60 * 60,
            webhook_lease_min_secs: 60,
            webhook_lease_max_secs: 24 * 60 * 60,
        }
    }
}

impl PushRelayLimits {
    pub fn is_default(&self) -> bool {
        self == &Self::default()
    }

    pub fn journal_retention(&self) -> Duration {
        Duration::from_secs(self.journal_retention_secs)
    }

    pub fn ask_timeout(&self) -> Duration {
        Duration::from_secs(self.ask_timeout_secs)
    }

    pub fn first_discovery_wait(&self) -> Duration {
        Duration::from_millis(self.first_discovery_wait_ms)
    }

    pub fn upstream_poll_min_interval(&self) -> Duration {
        Duration::from_millis(self.upstream_poll_min_interval_ms)
    }

    pub fn upstream_poll_max_interval(&self) -> Duration {
        Duration::from_millis(self.upstream_poll_max_interval_ms)
    }

    pub fn upstream_reconnect_backoff_min(&self) -> Duration {
        Duration::from_millis(self.upstream_reconnect_backoff_min_ms)
    }

    pub fn upstream_reconnect_backoff_max(&self) -> Duration {
        Duration::from_millis(self.upstream_reconnect_backoff_max_ms)
    }

    pub fn upstream_heartbeat_timeout(&self) -> Duration {
        Duration::from_secs(self.upstream_heartbeat_timeout_secs)
    }

    pub fn downstream_heartbeat(&self) -> Duration {
        Duration::from_secs(self.downstream_heartbeat_secs)
    }

    pub fn poll_max_wait(&self) -> Duration {
        Duration::from_secs(self.poll_max_wait_secs)
    }

    pub fn watch_max_lifetime(&self) -> Duration {
        Duration::from_secs(self.watch_max_lifetime_secs)
    }

    pub fn webhook_timeout(&self) -> Duration {
        Duration::from_secs(self.webhook_timeout_secs)
    }

    /// Total delivery attempts per webhook event (first try plus retries).
    pub fn webhook_attempts(&self) -> usize {
        self.webhook_retry_delays_secs.len() + 1
    }

    /// Clamp a requested webhook lease to the configured bounds.
    pub fn webhook_lease(&self, requested_secs: Option<u64>) -> Duration {
        let secs = requested_secs
            .unwrap_or(self.webhook_lease_default_secs)
            .clamp(self.webhook_lease_min_secs, self.webhook_lease_max_secs);
        Duration::from_secs(secs)
    }

    /// Clamp an upstream's `nextPollMs` to the configured bounds.
    pub fn clamp_upstream_poll_interval(&self, next_poll_ms: u64) -> Duration {
        Duration::from_millis(next_poll_ms.clamp(
            self.upstream_poll_min_interval_ms,
            self.upstream_poll_max_interval_ms,
        ))
    }
}

/// Per-client push permission tree. Every level is optional and an absent
/// entry inherits from the level above it:
/// event type → server → client → global default.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct PushPermissions {
    /// Client-wide override (`None` = inherit the global default).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub global: Option<PushPermission>,
    /// Per upstream server (key: server id).
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub servers: HashMap<String, PushPermission>,
    /// Per event type (key: `server_id__event_name`, see [`Self::event_key`]).
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub events: HashMap<String, PushPermission>,
}

impl PushPermissions {
    pub fn is_empty(&self) -> bool {
        self.global.is_none() && self.servers.is_empty() && self.events.is_empty()
    }

    /// Key of an event type in [`Self::events`]: the server id and the
    /// upstream's own (unqualified) event name, joined like tool names.
    pub fn event_key(server_id: &str, event_name: &str) -> String {
        format!("{server_id}__{event_name}")
    }

    /// The effective permission for one event type: the most specific entry
    /// wins (event → server → client → `global_default`).
    pub fn effective(
        &self,
        global_default: PushPermission,
        server_id: &str,
        event_name: &str,
    ) -> PushPermission {
        self.events
            .get(&Self::event_key(server_id, event_name))
            .or_else(|| self.servers.get(server_id))
            .or(self.global.as_ref())
            .copied()
            .unwrap_or(global_default)
    }

    /// The effective permission for a server as a whole, ignoring per-event
    /// overrides (server → client → `global_default`).
    pub fn effective_for_server(
        &self,
        global_default: PushPermission,
        server_id: &str,
    ) -> PushPermission {
        self.servers
            .get(server_id)
            .or(self.global.as_ref())
            .copied()
            .unwrap_or(global_default)
    }
}

/// When the fallback polling tools (`EventList`, `EventPoll`, `EventWait`)
/// are shown to a client.
#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum PushFallbackTools {
    /// Shown unless the client has a native mechanism (see the plan).
    #[default]
    Auto,
    /// Shown whenever any push source is available.
    Always,
    /// Never shown.
    Never,
}

impl PushFallbackTools {
    pub fn is_auto(&self) -> bool {
        matches!(self, Self::Auto)
    }
}

/// Delivery mode an upstream offers for one event type (SEP-3415).
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[serde(rename_all = "snake_case")]
pub enum EventDeliveryMode {
    /// `events/stream`
    Push,
    /// `events/poll`
    Poll,
    /// `events/subscribe` (never consumed upstream by LocalRouter)
    Webhook,
}

/// Summary of the push an upstream offers, derived from its capabilities and
/// event catalogue.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct UpstreamPushSummary {
    /// Delivery modes per upstream event name. Event types offering only
    /// `webhook` are listed, so they can be shown as unsupported.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub events_modes: BTreeMap<String, Vec<EventDeliveryMode>>,
    /// The server declared `experimental["claude/channel"]`.
    #[serde(default, skip_serializing_if = "is_false")]
    pub channels: bool,
    /// The server declared `resources.subscribe`.
    #[serde(default, skip_serializing_if = "is_false")]
    pub resource_subscribe: bool,
}

impl UpstreamPushSummary {
    /// Whether any push source LocalRouter can consume exists: an event type
    /// with push or poll, or Claude Channels. Resource updates count only when
    /// the caller enables them.
    pub fn has_consumable_push(&self, include_resource_updates: bool) -> bool {
        self.channels
            || (include_resource_updates && self.resource_subscribe)
            || self.events_modes.values().any(|modes| {
                modes
                    .iter()
                    .any(|m| matches!(m, EventDeliveryMode::Push | EventDeliveryMode::Poll))
            })
    }
}

/// What an upstream MCP server offered the last time it initialised.
/// Machine-recorded: never shown as a user edit and never part of the
/// server's identity (see [`McpServerConfig::same_connection`]).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct UpstreamCapabilitySnapshot {
    /// [`fingerprint_for`] of the server configuration this was observed
    /// with. A different fingerprint invalidates the snapshot.
    pub fingerprint: String,
    /// Protocol version the upstream negotiated.
    pub protocol_version: String,
    /// Raw capabilities, including `experimental`, `extensions` and unknown
    /// top-level fields such as `events`.
    pub capabilities: serde_json::Value,
    /// Derived push summary.
    #[serde(default)]
    pub push: UpstreamPushSummary,
    /// Raw `events/list` result entries (names, descriptions, schemas,
    /// delivery modes). Empty when the server has no events capability.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub event_catalogue: Vec<serde_json::Value>,
    /// When this content was last recorded.
    pub observed_at: DateTime<Utc>,
}

impl UpstreamCapabilitySnapshot {
    /// Same observed content, ignoring when it was observed.
    pub fn same_content(&self, other: &Self) -> bool {
        self.fingerprint == other.fingerprint
            && self.protocol_version == other.protocol_version
            && self.capabilities == other.capabilities
            && self.push == other.push
            && self.event_catalogue == other.event_catalogue
    }

    /// Whether this snapshot still describes `server`.
    pub fn is_current_for(&self, server: &McpServerConfig) -> bool {
        self.fingerprint == fingerprint_for(server)
    }
}

/// Stable identity of an upstream server: transport, command + args or URL,
/// and the authentication method. Environment values, header values and
/// secret references are never part of the input, so rotating a secret does
/// not invalidate the remembered capabilities.
pub fn fingerprint_for(server: &McpServerConfig) -> String {
    use sha2::{Digest, Sha256};

    let mut identity = serde_json::Map::new();
    let transport = match &server.transport_config {
        McpTransportConfig::Stdio { command, args, .. } => serde_json::json!({
            "type": "stdio",
            "command": command,
            "args": args,
        }),
        McpTransportConfig::HttpSse { url, .. } | McpTransportConfig::Sse { url, .. } => {
            serde_json::json!({ "type": "http", "url": url })
        }
        McpTransportConfig::WebSocket { url, .. } => {
            serde_json::json!({ "type": "websocket", "url": url })
        }
    };
    identity.insert("transport".into(), transport);
    let auth = match &server.auth_config {
        None | Some(McpAuthConfig::None) => serde_json::json!({ "method": "none" }),
        Some(McpAuthConfig::BearerToken { .. }) => serde_json::json!({ "method": "bearer_token" }),
        Some(McpAuthConfig::CustomHeaders { header_refs }) => {
            let mut names: Vec<&String> = header_refs.keys().collect();
            names.sort();
            serde_json::json!({ "method": "custom_headers", "headers": names })
        }
        Some(McpAuthConfig::OAuth {
            client_id,
            auth_url,
            token_url,
            scopes,
            ..
        }) => serde_json::json!({
            "method": "oauth",
            "client_id": client_id,
            "auth_url": auth_url,
            "token_url": token_url,
            "scopes": scopes,
        }),
        Some(McpAuthConfig::OAuthBrowser {
            client_id,
            auth_url,
            token_url,
            scopes,
            ..
        }) => serde_json::json!({
            "method": "oauth_browser",
            "client_id": client_id,
            "auth_url": auth_url,
            "token_url": token_url,
            "scopes": scopes,
        }),
        Some(McpAuthConfig::EnvVars { env_refs }) => {
            let mut names: Vec<&String> = env_refs.keys().collect();
            names.sort();
            serde_json::json!({ "method": "env_vars", "variables": names })
        }
    };
    identity.insert("auth".into(), auth);

    // Built in a fixed order, so the encoding is stable.
    let encoded = serde_json::Value::Object(identity).to_string();
    let digest = Sha256::digest(encoded.as_bytes());
    let mut hex = String::with_capacity(digest.len() * 2);
    for byte in digest {
        hex.push_str(&format!("{byte:02x}"));
    }
    format!("sha256:{hex}")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{AppConfig, Client, McpTransportType};

    fn stdio_server(command: &str) -> McpServerConfig {
        McpServerConfig::new(
            "Test".to_string(),
            McpTransportType::Stdio,
            McpTransportConfig::Stdio {
                command: command.to_string(),
                args: vec![],
                env: HashMap::new(),
                cwd: None,
            },
        )
    }

    fn snapshot_for(server: &McpServerConfig) -> UpstreamCapabilitySnapshot {
        let mut events_modes = BTreeMap::new();
        events_modes.insert(
            "message.received".to_string(),
            vec![EventDeliveryMode::Push, EventDeliveryMode::Poll],
        );
        UpstreamCapabilitySnapshot {
            fingerprint: fingerprint_for(server),
            protocol_version: "2025-11-25".to_string(),
            capabilities: serde_json::json!({
                "tools": {},
                "events": {},
                "experimental": { "claude/channel": {} },
                "extensions": { "io.modelcontextprotocol/events": { "listChanged": true } }
            }),
            push: UpstreamPushSummary {
                events_modes,
                channels: true,
                resource_subscribe: false,
            },
            event_catalogue: vec![serde_json::json!({
                "name": "message.received",
                "delivery": ["push", "poll"]
            })],
            observed_at: Utc::now(),
        }
    }

    #[test]
    fn old_config_without_push_relay_loads_with_defaults() {
        let yaml = r#"
version: 1
clients:
  - id: c1
    name: Client
    created_at: "2026-01-01T00:00:00Z"
    strategy_id: s1
mcp_servers:
  - id: m1
    name: Server
    transport: stdio
    transport_config:
      type: stdio
      command: echo
    created_at: "2026-01-01T00:00:00Z"
"#;
        let config: AppConfig = serde_yaml::from_str(yaml).unwrap();
        assert_eq!(config.push_relay, PushRelayConfig::default());
        assert!(config.push_relay.enabled);
        assert_eq!(config.push_relay.default_permission, PushPermission::Allow);
        let client = &config.clients[0];
        assert!(client.push_permissions.is_empty());
        assert_eq!(client.push_fallback_tools, PushFallbackTools::Auto);
        assert!(client.push_webhook_urls.is_empty());
        assert!(!client.claude_channels_launched);
        assert!(config.mcp_servers[0].capability_snapshot.is_none());
    }

    #[test]
    fn defaults_are_not_serialized() {
        let mut config = AppConfig::default();
        config
            .clients
            .push(Client::new_with_strategy("c".into(), "s".into()));
        config.mcp_servers.push(stdio_server("echo"));
        let yaml = serde_yaml::to_string(&config).unwrap();
        for key in [
            "push_relay",
            "push_permissions",
            "push_fallback_tools",
            "push_webhook_urls",
            "claude_channels_launched",
            "capability_snapshot",
        ] {
            assert!(!yaml.contains(key), "{key} serialized at default:\n{yaml}");
        }
    }

    #[test]
    fn non_default_settings_round_trip() {
        let mut config = AppConfig::default();
        config.push_relay.default_permission = PushPermission::Ask;
        config.push_relay.webhook_egress_enabled = false;
        config.push_relay.limits.first_discovery_wait_ms = 2_500;
        config.push_relay.limits.webhook_retry_delays_secs = vec![1, 2];

        let mut client = Client::new_with_strategy("c".into(), "s".into());
        client.push_permissions.global = Some(PushPermission::Block);
        client
            .push_permissions
            .servers
            .insert("m1".into(), PushPermission::Allow);
        client.push_permissions.events.insert(
            PushPermissions::event_key("m1", "message.received"),
            PushPermission::Ask,
        );
        client.push_fallback_tools = PushFallbackTools::Never;
        client.push_webhook_urls = vec!["http://127.0.0.1:9000/hook".into()];
        client.claude_channels_launched = true;
        config.clients.push(client);

        let mut server = stdio_server("echo");
        server.capability_snapshot = Some(snapshot_for(&server));
        config.mcp_servers.push(server);

        let yaml = serde_yaml::to_string(&config).unwrap();
        assert!(yaml.contains("default_permission: ask"));
        assert!(yaml.contains("push_fallback_tools: never"));
        let parsed: AppConfig = serde_yaml::from_str(&yaml).unwrap();
        assert_eq!(parsed.push_relay, config.push_relay);
        assert_eq!(parsed.clients[0], config.clients[0]);
        assert_eq!(parsed.mcp_servers[0], config.mcp_servers[0]);
    }

    #[test]
    fn partial_limits_keep_other_defaults() {
        let limits: PushRelayLimits =
            serde_yaml::from_str("ask_timeout_secs: 60\npoll_max_events: 10\n").unwrap();
        assert_eq!(limits.ask_timeout(), Duration::from_secs(60));
        assert_eq!(limits.poll_max_events, 10);
        assert_eq!(limits.first_discovery_wait(), Duration::from_secs(10));
        assert_eq!(limits.journal_max_bytes, 64 * MIB);
        assert_eq!(limits.webhook_attempts(), 6);
    }

    #[test]
    fn limit_helpers_clamp() {
        let limits = PushRelayLimits::default();
        assert_eq!(limits.webhook_lease(None), Duration::from_secs(3600));
        assert_eq!(limits.webhook_lease(Some(1)), Duration::from_secs(60));
        assert_eq!(
            limits.webhook_lease(Some(10_000_000)),
            Duration::from_secs(86_400)
        );
        assert_eq!(
            limits.clamp_upstream_poll_interval(10),
            Duration::from_secs(1)
        );
        assert_eq!(
            limits.clamp_upstream_poll_interval(u64::MAX),
            Duration::from_secs(300)
        );
    }

    #[test]
    fn most_specific_permission_wins() {
        let mut perms = PushPermissions::default();
        let default = PushPermission::Allow;
        assert_eq!(perms.effective(default, "m1", "e"), PushPermission::Allow);
        assert_eq!(
            perms.effective(PushPermission::Ask, "m1", "e"),
            PushPermission::Ask
        );

        perms.global = Some(PushPermission::Block);
        assert_eq!(perms.effective(default, "m1", "e"), PushPermission::Block);

        perms.servers.insert("m1".into(), PushPermission::Ask);
        assert_eq!(perms.effective(default, "m1", "e"), PushPermission::Ask);
        assert_eq!(perms.effective(default, "m2", "e"), PushPermission::Block);
        assert_eq!(
            perms.effective_for_server(default, "m1"),
            PushPermission::Ask
        );

        perms
            .events
            .insert(PushPermissions::event_key("m1", "e"), PushPermission::Allow);
        assert_eq!(perms.effective(default, "m1", "e"), PushPermission::Allow);
        assert_eq!(perms.effective(default, "m1", "other"), PushPermission::Ask);
    }

    #[test]
    fn disabled_relay_blocks_everything() {
        let mut client = Client::new_with_strategy("c".into(), "s".into());
        client.push_permissions.global = Some(PushPermission::Allow);
        let mut global = PushRelayConfig::default();
        assert_eq!(
            client.push_permission(&global, "m", "e"),
            PushPermission::Allow
        );
        global.enabled = false;
        assert_eq!(
            client.push_permission(&global, "m", "e"),
            PushPermission::Block
        );
    }

    #[test]
    fn fingerprint_ignores_secrets_and_tracks_identity() {
        let base = stdio_server("npx -y server");
        let fp = fingerprint_for(&base);
        assert!(fp.starts_with("sha256:"));
        assert_eq!(fp, fingerprint_for(&base.clone()));

        // Env values, name, enabled flag and the snapshot itself are not identity
        let mut same = base.clone();
        same.name = "Renamed".into();
        same.enabled = false;
        if let McpTransportConfig::Stdio { env, .. } = &mut same.transport_config {
            env.insert("API_KEY".into(), "secret".into());
        }
        same.auth_config = Some(McpAuthConfig::EnvVars {
            env_refs: HashMap::from([("TOKEN".into(), "ref-1".into())]),
        });
        let mut rotated = same.clone();
        rotated.auth_config = Some(McpAuthConfig::EnvVars {
            env_refs: HashMap::from([("TOKEN".into(), "ref-2".into())]),
        });
        assert_eq!(fingerprint_for(&same), fingerprint_for(&rotated));
        same.capability_snapshot = Some(snapshot_for(&base));
        assert_eq!(fingerprint_for(&same), fingerprint_for(&rotated));

        // Command, transport and auth method are identity
        assert_ne!(fp, fingerprint_for(&stdio_server("npx -y other")));
        let mut with_bearer = base.clone();
        with_bearer.auth_config = Some(McpAuthConfig::BearerToken {
            token_ref: "t".into(),
        });
        assert_ne!(fp, fingerprint_for(&with_bearer));
        let http = McpServerConfig::new(
            "Test".into(),
            McpTransportType::HttpSse,
            McpTransportConfig::HttpSse {
                url: "https://example.com/mcp".into(),
                headers: HashMap::from([("Authorization".into(), "Bearer x".into())]),
            },
        );
        let mut http_other_headers = http.clone();
        if let McpTransportConfig::HttpSse { headers, .. } =
            &mut http_other_headers.transport_config
        {
            headers.insert("Authorization".into(), "Bearer y".into());
        }
        assert_eq!(fingerprint_for(&http), fingerprint_for(&http_other_headers));
        assert_ne!(fp, fingerprint_for(&http));
    }

    #[test]
    fn snapshot_invalidated_by_identity_change() {
        let mut server = stdio_server("npx -y server");
        server.capability_snapshot = Some(snapshot_for(&server));
        assert!(server.current_capability_snapshot().is_some());

        let mut changed = server.clone();
        changed.transport_config = McpTransportConfig::Stdio {
            command: "npx -y server --flag".into(),
            args: vec![],
            env: HashMap::new(),
            cwd: None,
        };
        assert!(changed.capability_snapshot.is_some());
        assert!(changed.current_capability_snapshot().is_none());
    }

    #[test]
    fn snapshot_is_not_part_of_connection_identity() {
        let server = stdio_server("echo");
        let mut with_snapshot = server.clone();
        with_snapshot.capability_snapshot = Some(snapshot_for(&server));
        assert_ne!(server, with_snapshot);
        assert!(server.same_connection(&with_snapshot));
        let mut renamed = server.clone();
        renamed.name = "Other".into();
        assert!(!server.same_connection(&renamed));
    }

    #[test]
    fn same_content_ignores_observation_time() {
        let server = stdio_server("echo");
        let a = snapshot_for(&server);
        let mut b = a.clone();
        b.observed_at = a.observed_at + chrono::Duration::hours(1);
        assert!(a.same_content(&b));
        b.push.channels = false;
        assert!(!a.same_content(&b));
    }

    #[test]
    fn push_summary_counts_only_consumable_modes() {
        let mut summary = UpstreamPushSummary::default();
        assert!(!summary.has_consumable_push(true));
        summary
            .events_modes
            .insert("e".into(), vec![EventDeliveryMode::Webhook]);
        assert!(!summary.has_consumable_push(true));
        summary.resource_subscribe = true;
        assert!(!summary.has_consumable_push(false));
        assert!(summary.has_consumable_push(true));
        summary
            .events_modes
            .insert("f".into(), vec![EventDeliveryMode::Poll]);
        assert!(summary.has_consumable_push(false));
    }

    #[tokio::test]
    async fn recording_snapshots_writes_only_on_change() {
        let dir = tempfile::tempdir().unwrap();
        let mut config = AppConfig::default();
        let server = stdio_server("echo");
        let id = server.id.clone();
        config.mcp_servers.push(server.clone());
        let manager = crate::ConfigManager::new(config, dir.path().join("settings.yaml"));

        let snapshot = snapshot_for(&server);
        let mut batch = HashMap::from([(id.clone(), snapshot.clone())]);
        batch.insert("missing".into(), snapshot.clone());
        assert!(manager.record_capability_snapshots(&batch).unwrap());
        assert_eq!(
            manager.get().mcp_servers[0].capability_snapshot,
            Some(snapshot.clone())
        );

        // A later observation of the same content is not a change
        let mut later = snapshot.clone();
        later.observed_at = snapshot.observed_at + chrono::Duration::minutes(5);
        let batch = HashMap::from([(id.clone(), later)]);
        assert!(!manager.record_capability_snapshots(&batch).unwrap());
        assert_eq!(
            manager.get().mcp_servers[0]
                .capability_snapshot
                .as_ref()
                .unwrap()
                .observed_at,
            snapshot.observed_at
        );

        let mut changed = snapshot.clone();
        changed.protocol_version = "2026-07-28".into();
        let batch = HashMap::from([(id, changed.clone())]);
        assert!(manager.record_capability_snapshots(&batch).unwrap());
        assert_eq!(
            manager.get().mcp_servers[0].capability_snapshot,
            Some(changed)
        );
    }
}
