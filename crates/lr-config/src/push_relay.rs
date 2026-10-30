//! Push Relay configuration: the global switch and default permission, per-client push
//! permissions, and the upstream capabilities remembered on each MCP server.
//!
//! See `plan/2026-10-30-PUSH_RELAY.md`.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap};

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
///
/// Only the master switch and the global default permission are
/// configurable. Every bound (journal size, poll page size, webhook retries,
/// ...) is an internal constant of the relay. Unknown keys are ignored, so a
/// config written with the older `limits`, `webhook_egress_enabled`,
/// `allow_loopback_http` or `ask_timeout_secs` keys still loads.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PushRelayConfig {
    /// Master switch. When off, no push is advertised or relayed to anyone.
    #[serde(default = "default_true")]
    pub enabled: bool,

    /// Permission applied when neither the client nor its server/event
    /// overrides say otherwise.
    #[serde(default)]
    pub default_permission: PushPermission,
}

impl Default for PushRelayConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            default_permission: PushPermission::Allow,
        }
    }
}

impl PushRelayConfig {
    pub fn is_default(&self) -> bool {
        self == &Self::default()
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

/// A client's push settings, as handed to the gateway with every request.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ClientPushSettings {
    pub permissions: PushPermissions,
    pub webhook_urls: Vec<String>,
}

impl ClientPushSettings {
    pub fn from_client(client: &crate::Client) -> Self {
        Self {
            permissions: client.push_permissions.clone(),
            webhook_urls: client.push_webhook_urls.clone(),
        }
    }

    /// Copy these settings onto a client.
    pub fn apply_to(&self, client: &mut crate::Client) {
        client.push_permissions = self.permissions.clone();
        client.push_webhook_urls = self.webhook_urls.clone();
    }
}

impl crate::Client {
    /// Record a per-event push permission (`None` removes the override).
    pub fn set_push_event_permission(
        &mut self,
        server_id: &str,
        event_name: &str,
        permission: Option<PushPermission>,
    ) {
        let key = PushPermissions::event_key(server_id, event_name);
        match permission {
            Some(permission) => {
                self.push_permissions.events.insert(key, permission);
            }
            None => {
                self.push_permissions.events.remove(&key);
            }
        }
    }

    /// Remember an approved webhook callback URL. Returns whether it was new.
    pub fn approve_push_webhook_url(&mut self, url: &str) -> bool {
        if self.push_webhook_urls.iter().any(|u| u == url) {
            return false;
        }
        self.push_webhook_urls.push(url.to_string());
        true
    }
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
        assert!(client.push_webhook_urls.is_empty());
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
            "push_webhook_urls",
            "capability_snapshot",
        ] {
            assert!(!yaml.contains(key), "{key} serialized at default:\n{yaml}");
        }
    }

    #[test]
    fn non_default_settings_round_trip() {
        let mut config = AppConfig::default();
        config.push_relay.default_permission = PushPermission::Ask;
        config.push_relay.enabled = false;

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
        client.push_webhook_urls = vec!["http://127.0.0.1:9000/hook".into()];
        config.clients.push(client);

        let mut server = stdio_server("echo");
        server.capability_snapshot = Some(snapshot_for(&server));
        config.mcp_servers.push(server);

        let yaml = serde_yaml::to_string(&config).unwrap();
        assert!(yaml.contains("default_permission: ask"));
        assert!(yaml.contains("enabled: false"));
        let parsed: AppConfig = serde_yaml::from_str(&yaml).unwrap();
        assert_eq!(parsed.push_relay, config.push_relay);
        assert_eq!(parsed.clients[0], config.clients[0]);
        assert_eq!(parsed.mcp_servers[0], config.mcp_servers[0]);
    }

    #[test]
    fn config_with_removed_settings_still_loads() {
        let yaml = r#"
version: 1
push_relay:
  enabled: false
  default_permission: ask
  webhook_egress_enabled: false
  allow_loopback_http: false
  ask_timeout_secs: 60
  limits:
    journal_max_events: 5
    ask_timeout_secs: 60
    ask_hold_cap: 2
    webhook_retry_delays_secs: [1, 2]
clients:
  - id: c1
    name: Client
    created_at: "2026-01-01T00:00:00Z"
    strategy_id: s1
    push_fallback_tools: never
    claude_channels_launched: true
    push_webhook_urls: ["http://127.0.0.1:9000/hook"]
"#;
        let config: AppConfig = serde_yaml::from_str(yaml).unwrap();
        assert!(!config.push_relay.enabled);
        assert_eq!(config.push_relay.default_permission, PushPermission::Ask);
        assert_eq!(
            config.clients[0].push_webhook_urls,
            vec!["http://127.0.0.1:9000/hook".to_string()]
        );
        // Saving drops the removed keys
        let saved = serde_yaml::to_string(&config).unwrap();
        for key in [
            "limits",
            "webhook_egress_enabled",
            "allow_loopback_http",
            "ask_timeout_secs",
            "push_fallback_tools",
            "claude_channels_launched",
        ] {
            assert!(!saved.contains(key), "{key} still saved:\n{saved}");
        }
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
