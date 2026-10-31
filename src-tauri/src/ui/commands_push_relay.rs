//! Push Relay Tauri commands: the global default permission, per-client push
//! settings, the upstream push sources LocalRouter remembers, and the Claude
//! Code STDIO bridge setup.
//!
//! See `plan/2026-10-30-PUSH_RELAY.md` ("Configuration and UI").

use std::collections::BTreeMap;
use std::sync::Arc;

use lr_config::{
    ConfigManager, EventDeliveryMode, PushPermission, PushPermissions, PushRelayConfig,
};
use serde::{Deserialize, Serialize};
use tauri::{Emitter, State};

use crate::launcher::integrations::claude_code;
use crate::ui::commands_clients::LaunchResult;

// Every settings write below goes through `ConfigManager::update`, which emits
// `config-changed`; `main.rs` forwards that (and `clients-changed`) to the
// relay through `AppState::sync_push_relay_settings`, the one path by which
// settings reach it. No command here talks to the relay about settings.

fn emit_clients_changed(app: &tauri::AppHandle) {
    if let Err(e) = app.emit("clients-changed", ()) {
        tracing::error!("Failed to emit clients-changed event: {}", e);
    }
}

// ============================================================================
// Global settings
// ============================================================================

/// Global Push Relay settings: the default permission (Allow / Ask / Off;
/// the only global control), the Claude Channels rate and how much event
/// history each client keeps. Every other limit is internal to the relay and
/// not configurable.
#[derive(Debug, Clone, Serialize)]
pub struct PushRelayConfigView {
    pub default_permission: PushPermission,
    /// Claude Channels messages per client session per minute.
    pub channel_messages_per_minute: u32,
    /// Hours an event stays in a client's journal.
    pub history_retention_hours: u32,
    /// Events kept in each client's journal.
    pub history_max_events: u32,
}

impl From<&PushRelayConfig> for PushRelayConfigView {
    fn from(c: &PushRelayConfig) -> Self {
        Self {
            default_permission: c.default_permission,
            channel_messages_per_minute: c.channel_messages_per_minute,
            history_retention_hours: c.history_retention_hours,
            history_max_events: c.history_max_events,
        }
    }
}

/// A partial update of the global settings; `None` leaves a field as it is.
#[derive(Debug, Clone, Default)]
pub struct PushRelayConfigUpdate {
    pub default_permission: Option<PushPermission>,
    pub channel_messages_per_minute: Option<u32>,
    pub history_retention_hours: Option<u32>,
    pub history_max_events: Option<u32>,
}

/// Apply `update` to `push`, refusing (and leaving `push` unchanged) when a
/// value is out of its bounds.
pub fn apply_push_relay_update(
    push: &mut PushRelayConfig,
    update: &PushRelayConfigUpdate,
) -> Result<(), String> {
    let mut next = push.clone();
    if let Some(v) = update.default_permission {
        next.default_permission = v;
    }
    if let Some(v) = update.channel_messages_per_minute {
        next.channel_messages_per_minute = v;
    }
    if let Some(v) = update.history_retention_hours {
        next.history_retention_hours = v;
    }
    if let Some(v) = update.history_max_events {
        next.history_max_events = v;
    }
    next.validate()?;
    *push = next;
    Ok(())
}

/// Get the global Push Relay settings.
#[tauri::command]
pub async fn get_push_relay_config(
    config_manager: State<'_, ConfigManager>,
) -> Result<PushRelayConfigView, String> {
    Ok(PushRelayConfigView::from(&config_manager.get().push_relay))
}

/// Update the global Push Relay settings. Every argument is optional; a value
/// out of its bounds is refused and nothing is changed.
#[tauri::command]
pub async fn update_push_relay_config(
    default_permission: Option<PushPermission>,
    channel_messages_per_minute: Option<u32>,
    history_retention_hours: Option<u32>,
    history_max_events: Option<u32>,
    config_manager: State<'_, ConfigManager>,
) -> Result<PushRelayConfigView, String> {
    let update = PushRelayConfigUpdate {
        default_permission,
        channel_messages_per_minute,
        history_retention_hours,
        history_max_events,
    };
    // Check the values before writing, so a refusal changes nothing
    let mut probe = config_manager.get().push_relay;
    apply_push_relay_update(&mut probe, &update)?;
    let mut result = Ok(());
    config_manager
        .update(|cfg| result = apply_push_relay_update(&mut cfg.push_relay, &update))
        .map_err(|e| e.to_string())?;
    result?;

    let new_config = config_manager.get().push_relay.clone();
    config_manager.save().await.map_err(|e| e.to_string())?;
    Ok(PushRelayConfigView::from(&new_config))
}

// ============================================================================
// Upstream push sources
// ============================================================================

/// One event type an upstream offers.
#[derive(Debug, Clone, Serialize)]
pub struct PushEventTypeInfo {
    /// The upstream's own (unqualified) event name.
    pub name: String,
    pub description: Option<String>,
    /// Delivery modes the upstream offers for it.
    pub modes: Vec<EventDeliveryMode>,
    /// Push or poll is offered, so LocalRouter can consume it.
    pub supported: bool,
}

/// The remembered push capabilities of one MCP server.
#[derive(Debug, Clone, Serialize)]
pub struct PushSourceInfo {
    pub server_id: String,
    pub server_name: String,
    pub enabled: bool,
    /// "none" (never observed), "stale" (observed under a different server
    /// configuration), or "current".
    pub snapshot_status: String,
    pub protocol_version: Option<String>,
    /// When the remembered capabilities last changed (RFC 3339).
    pub observed_at: Option<String>,
    /// The server declared `experimental["claude/channel"]`.
    pub channels: bool,
    /// The server declared `resources.subscribe`.
    pub resource_subscribe: bool,
    /// Event types from `events/list`, with webhook-only ones marked
    /// unsupported.
    pub events: Vec<PushEventTypeInfo>,
    /// LocalRouter can consume push from this server (events with push/poll,
    /// or Channels).
    pub has_consumable_push: bool,
    /// Enabled clients that may receive at least one of this server's push
    /// sources: MCP access to the server and a permission other than Off
    /// for one of its supported event types or its channel messages. Zero
    /// when the global default is Off and no override enables any of them.
    pub receiving_clients: usize,
}

fn event_descriptions(catalogue: &[serde_json::Value]) -> BTreeMap<String, String> {
    catalogue
        .iter()
        .filter_map(|e| {
            let name = e.get("name")?.as_str()?;
            let desc = e
                .get("description")
                .or_else(|| e.get("title"))
                .and_then(|d| d.as_str())?;
            Some((name.to_string(), desc.to_string()))
        })
        .collect()
}

/// Describe a server from its configuration (and its remembered snapshot).
pub fn push_source_info(server: &lr_config::McpServerConfig) -> PushSourceInfo {
    let snapshot = server.capability_snapshot.as_ref();
    let current = server.current_capability_snapshot().is_some();
    let snapshot_status = match (snapshot, current) {
        (None, _) => "none",
        (Some(_), false) => "stale",
        (Some(_), true) => "current",
    };
    let (events, channels, resource_subscribe, has_push) = match snapshot {
        Some(snap) => {
            let descriptions = event_descriptions(&snap.event_catalogue);
            let events = snap
                .push
                .events_modes
                .iter()
                .map(|(name, modes)| PushEventTypeInfo {
                    name: name.clone(),
                    description: descriptions.get(name).cloned(),
                    modes: modes.clone(),
                    supported: modes
                        .iter()
                        .any(|m| matches!(m, EventDeliveryMode::Push | EventDeliveryMode::Poll)),
                })
                .collect();
            (
                events,
                snap.push.channels,
                snap.push.resource_subscribe,
                snap.push.has_consumable_push(false),
            )
        }
        None => (Vec::new(), false, false, false),
    };
    PushSourceInfo {
        server_id: server.id.clone(),
        server_name: server.name.clone(),
        enabled: server.enabled,
        snapshot_status: snapshot_status.to_string(),
        protocol_version: snapshot.map(|s| s.protocol_version.clone()),
        observed_at: snapshot.map(|s| s.observed_at.to_rfc3339()),
        channels,
        resource_subscribe,
        events,
        has_consumable_push: has_push && current,
        receiving_clients: 0,
    }
}

/// Count the clients that may receive any of `source`'s push sources (see
/// [`PushSourceInfo::receiving_clients`]).
pub fn receiving_clients(config: &lr_config::AppConfig, source: &PushSourceInfo) -> usize {
    let mut names: Vec<&str> = source
        .events
        .iter()
        .filter(|e| e.supported)
        .map(|e| e.name.as_str())
        .collect();
    if source.channels {
        names.push(lr_mcp::gateway::push_relay::relay::CHANNEL_EVENT_NAME);
    }
    if names.is_empty() || !source.enabled {
        return 0;
    }
    config
        .clients
        .iter()
        .filter(|c| {
            c.enabled
                && !c.security.privacy.restricted()
                && c.mcp_permissions
                    .resolve_server(&source.server_id)
                    .is_enabled()
                && names.iter().any(|name| {
                    c.push_permission(&config.push_relay, &source.server_id, name)
                        != PushPermission::Off
                })
        })
        .count()
}

/// Every configured MCP server with its remembered push capabilities. The
/// running server manager's copy is preferred, since snapshot writes to the
/// config file are batched.
fn collect_push_sources(
    config: &lr_config::AppConfig,
    state: &lr_server::state::AppState,
) -> Vec<PushSourceInfo> {
    config
        .mcp_servers
        .iter()
        .map(|server| {
            let mut server = server.clone();
            if let Some(live) = state.mcp_server_manager.get_config(&server.id) {
                if live.capability_snapshot.is_some() && live.same_connection(&server) {
                    server.capability_snapshot = live.capability_snapshot;
                }
            }
            let mut info = push_source_info(&server);
            info.receiving_clients = receiving_clients(config, &info);
            info
        })
        .collect()
}

/// List upstream push sources from the remembered capability snapshots.
#[tauri::command]
pub async fn list_push_sources(
    config_manager: State<'_, ConfigManager>,
    state: State<'_, Arc<lr_server::state::AppState>>,
) -> Result<Vec<PushSourceInfo>, String> {
    Ok(collect_push_sources(&config_manager.get(), &state))
}

// ============================================================================
// Per-client settings
// ============================================================================

/// A server in a client's push tree, with the client's MCP access to it.
#[derive(Debug, Clone, Serialize)]
pub struct ClientPushServerAccess {
    pub server_id: String,
    /// The client's MCP permissions allow this server (Allow or Ask). Push
    /// never widens MCP access, so a server without access delivers nothing.
    pub mcp_access: bool,
    /// Effective push permission for the server as a whole.
    pub effective: PushPermission,
}

/// A client's Push Relay settings and what they resolve to.
#[derive(Debug, Clone, Serialize)]
pub struct ClientPushSettings {
    pub client_id: String,
    pub permissions: PushPermissions,
    pub webhook_urls: Vec<String>,
    /// The global default the client inherits.
    pub global_default: PushPermission,
    /// The client-level effective permission (client override or default).
    pub effective_client: PushPermission,
    /// Any push can reach this client: the global default is not Off, or
    /// one of its overrides enables something.
    pub receives_push: bool,
    pub servers: Vec<ClientPushServerAccess>,
}

fn client_push_settings(
    config: &lr_config::AppConfig,
    client: &lr_config::Client,
) -> ClientPushSettings {
    let default = config.push_relay.default_permission;
    ClientPushSettings {
        client_id: client.id.clone(),
        permissions: client.push_permissions.clone(),
        webhook_urls: client.push_webhook_urls.clone(),
        global_default: default,
        effective_client: client.push_permissions.global.unwrap_or(default),
        receives_push: client.may_receive_push(&config.push_relay),
        servers: config
            .mcp_servers
            .iter()
            .map(|s| ClientPushServerAccess {
                server_id: s.id.clone(),
                mcp_access: s.enabled && client.mcp_permissions.resolve_server(&s.id).is_enabled(),
                effective: client.push_permissions.effective_for_server(default, &s.id),
            })
            .collect(),
    }
}

fn find_client<'a>(
    config: &'a lr_config::AppConfig,
    client_id: &str,
) -> Result<&'a lr_config::Client, String> {
    config
        .clients
        .iter()
        .find(|c| c.id == client_id)
        .ok_or_else(|| format!("Client not found: {}", client_id))
}

/// Update one client in the config, failing when it does not exist.
async fn update_client<F>(
    config_manager: &ConfigManager,
    client_id: &str,
    f: F,
) -> Result<(), String>
where
    F: FnOnce(&mut lr_config::Client),
{
    let mut found = false;
    config_manager
        .update(|cfg| {
            if let Some(client) = cfg.clients.iter_mut().find(|c| c.id == client_id) {
                f(client);
                found = true;
            }
        })
        .map_err(|e| e.to_string())?;
    if !found {
        return Err(format!("Client not found: {}", client_id));
    }
    config_manager.save().await.map_err(|e| e.to_string())
}

/// Get a client's Push Relay settings.
#[tauri::command]
pub async fn get_client_push_settings(
    client_id: String,
    config_manager: State<'_, ConfigManager>,
) -> Result<ClientPushSettings, String> {
    let config = config_manager.get();
    Ok(client_push_settings(
        &config,
        find_client(&config, &client_id)?,
    ))
}

/// Apply one change to a push permission tree. `level` is "global",
/// "server" (needs `server_id`) or "event" (needs `server_id` and
/// `event_name`); `state: None` clears the override (inherit).
pub fn apply_push_permission(
    permissions: &mut PushPermissions,
    level: &str,
    server_id: Option<&str>,
    event_name: Option<&str>,
    state: Option<PushPermission>,
) -> Result<(), String> {
    match level {
        "global" => permissions.global = state,
        "server" => {
            let server = server_id.ok_or("server_id is required for a server permission")?;
            match state {
                Some(s) => {
                    permissions.servers.insert(server.to_string(), s);
                }
                None => {
                    permissions.servers.remove(server);
                }
            }
        }
        "event" => {
            let server = server_id.ok_or("server_id is required for an event permission")?;
            let event = event_name.ok_or("event_name is required for an event permission")?;
            let key = PushPermissions::event_key(server, event);
            match state {
                Some(s) => {
                    permissions.events.insert(key, s);
                }
                None => {
                    permissions.events.remove(&key);
                }
            }
        }
        other => return Err(format!("Unknown push permission level: {other}")),
    }
    Ok(())
}

/// Set (or clear) a client's push permission at the client, server or event
/// level.
#[tauri::command]
pub async fn set_client_push_permission(
    client_id: String,
    level: String,
    server_id: Option<String>,
    event_name: Option<String>,
    state: Option<PushPermission>,
    config_manager: State<'_, ConfigManager>,
    app: tauri::AppHandle,
) -> Result<(), String> {
    // Validate before touching the config.
    let mut probe = PushPermissions::default();
    apply_push_permission(
        &mut probe,
        &level,
        server_id.as_deref(),
        event_name.as_deref(),
        state,
    )?;
    let mut result = Ok(());
    update_client(&config_manager, &client_id, |client| {
        result = apply_push_permission(
            &mut client.push_permissions,
            &level,
            server_id.as_deref(),
            event_name.as_deref(),
            state,
        );
    })
    .await?;
    result?;
    emit_clients_changed(&app);
    Ok(())
}

/// Revoke an approved webhook callback URL. The next delivery to it asks
/// again.
#[tauri::command]
pub async fn remove_client_push_webhook_url(
    client_id: String,
    url: String,
    config_manager: State<'_, ConfigManager>,
    app: tauri::AppHandle,
) -> Result<(), String> {
    update_client(&config_manager, &client_id, |client| {
        client.push_webhook_urls.retain(|u| u != &url);
    })
    .await?;
    emit_clients_changed(&app);
    Ok(())
}

// ============================================================================
// Held events (Ask)
// ============================================================================

fn relay(
    state: &lr_server::state::AppState,
) -> Result<&Arc<lr_mcp::gateway::push_relay::PushRelay>, String> {
    state
        .push_relay
        .as_ref()
        .ok_or_else(|| "Push Relay is not running".to_string())
}

/// One held (Ask) event awaiting a decision for one client.
#[derive(Debug, Clone, Serialize)]
pub struct PendingPushApproval {
    /// The held event's id (`hold_id`); pass it to `submit_push_approval`.
    pub request_id: String,
    pub client_id: String,
    pub client_name: String,
    pub server_id: String,
    /// Slugified server name, as used in qualified event names.
    pub server_name: String,
    /// The upstream's own event name.
    pub event_name: String,
    /// `<server slug>__<event>`.
    pub qualified_name: String,
    pub event_id: String,
    /// `push`, `poll` or `channel`.
    pub ingress: String,
    /// When the event was held (RFC 3339).
    pub received_at: String,
    /// The event's `data`, as it would be delivered.
    pub data: serde_json::Value,
    /// Position in its (client, event type) queue; 0 is being asked now.
    pub position: usize,
    /// Other held events of the same type for the same client.
    pub pending_of_type: usize,
    /// Already decided, waiting for the events ahead of it.
    pub decided: bool,
    /// The approval popup asking about it, while one is open.
    pub firewall_request_id: Option<String>,
}

/// Convert the relay's held events into the UI's shape.
pub fn pending_push_approvals(
    held: Vec<lr_mcp::gateway::push_relay::HeldPushEventInfo>,
) -> Vec<PendingPushApproval> {
    let mut per_type: std::collections::HashMap<(String, String), usize> = Default::default();
    for e in &held {
        *per_type
            .entry((e.client_id.clone(), e.qualified_name.clone()))
            .or_default() += 1;
    }
    held.into_iter()
        .map(|e| {
            let of_type = per_type
                .get(&(e.client_id.clone(), e.qualified_name.clone()))
                .copied()
                .unwrap_or(1);
            PendingPushApproval {
                request_id: e.hold_id,
                client_id: e.client_id,
                client_name: e.client_name,
                server_id: e.server_id,
                server_name: e.server_name,
                event_name: e.event_name,
                qualified_name: e.qualified_name,
                event_id: e.event_id,
                ingress: e.ingress_mode,
                received_at: e.held_at.to_rfc3339(),
                data: e.data,
                position: e.position,
                pending_of_type: of_type.saturating_sub(1),
                decided: e.decided,
                firewall_request_id: e.firewall_request_id,
            }
        })
        .collect()
}

/// Held events awaiting a decision, oldest first (all clients, or one).
#[tauri::command]
pub async fn list_pending_push_approvals(
    client_id: Option<String>,
    state: State<'_, Arc<lr_server::state::AppState>>,
) -> Result<Vec<PendingPushApproval>, String> {
    let relay = relay(&state)?;
    Ok(pending_push_approvals(
        relay.pending_events(client_id.as_deref()),
    ))
}

/// An answer to a held event. The `*_type_*` actions apply to every later
/// event of the type for that client; `*_always` are written to its config.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PushApprovalAction {
    Allow,
    AllowEdited,
    Deny,
    AllowAllPendingOfType,
    AllowTypeSession,
    DenyTypeSession,
    AllowTypeAlways,
    DenyTypeAlways,
}

/// The relay decision an action maps to, and whether it applies to every
/// held event of the type rather than just this one.
pub fn push_decision_for(
    action: PushApprovalAction,
    edited_data: Option<serde_json::Value>,
) -> Result<(lr_mcp::gateway::push_relay::PushDecision, bool), String> {
    use lr_mcp::gateway::push_relay::{PushDecision, PushDecisionAction as A};
    let (decision_action, all_of_type) = match action {
        PushApprovalAction::Allow => (A::Allow, false),
        PushApprovalAction::AllowEdited => {
            let data = edited_data.ok_or("allow_edited needs editedData")?;
            return Ok((
                PushDecision {
                    action: A::Allow,
                    edited_data: Some(data),
                },
                false,
            ));
        }
        PushApprovalAction::Deny => (A::Deny, false),
        PushApprovalAction::AllowAllPendingOfType => (A::Allow, true),
        PushApprovalAction::AllowTypeSession => (A::AllowSession, false),
        PushApprovalAction::DenyTypeSession => (A::DenySession, false),
        PushApprovalAction::AllowTypeAlways => (A::AllowAlways, false),
        PushApprovalAction::DenyTypeAlways => (A::DenyAlways, false),
    };
    Ok((PushDecision::new(decision_action), all_of_type))
}

/// Answer one held event (`request_id` from `list_pending_push_approvals`).
/// Returns how many held events the answer decided.
#[tauri::command]
pub async fn submit_push_approval(
    request_id: String,
    action: PushApprovalAction,
    edited_data: Option<serde_json::Value>,
    state: State<'_, Arc<lr_server::state::AppState>>,
) -> Result<usize, String> {
    let relay = relay(&state)?;
    let (decision, all_of_type) = push_decision_for(action, edited_data)?;
    if !all_of_type {
        relay.decide(&request_id, decision)?;
        return Ok(1);
    }
    let held = relay
        .pending_events(None)
        .into_iter()
        .find(|e| e.hold_id == request_id)
        .ok_or_else(|| format!("No held event {request_id}"))?;
    Ok(relay.decide_all(&held.client_id, &held.qualified_name, decision))
}

// ============================================================================
// Testing pushes (Try it out → MCP → Push)
// ============================================================================

/// What `send_push_test_event` sent.
#[derive(Debug, Clone, Serialize)]
pub struct PushTestEventResult {
    /// Namespaced event id, `localrouter__test-…`.
    pub event_id: String,
    /// `localrouter__test.event`.
    pub name: String,
    /// Clients the event was admitted to, each under its own permission
    /// (Allow delivers, Ask holds it for approval, Off drops it).
    pub client_ids: Vec<String>,
}

/// Send one event from the built-in test source (`localrouter__test.event`)
/// through the real Push Relay pipeline: permissions and Ask, Monitor, the
/// journal and every delivery mode. It reaches only sessions that opted into
/// push testing (Try it out's MCP connection, or a STDIO bridge started with
/// `LOCALROUTER_PUSH_TEST=1`), of `client_id` when given. `data` replaces the
/// default data `{message, sent_at}`.
#[tauri::command]
pub async fn send_push_test_event(
    client_id: Option<String>,
    message: Option<String>,
    data: Option<serde_json::Value>,
    state: State<'_, Arc<lr_server::state::AppState>>,
) -> Result<PushTestEventResult, String> {
    let relay = relay(&state)?;
    let message = message.filter(|m| !m.trim().is_empty());
    let sent = relay.send_test_event(client_id.as_deref(), message, data)?;
    Ok(PushTestEventResult {
        event_id: sent.event_id,
        name: sent.name,
        client_ids: sent.client_ids,
    })
}

/// A Claude Code channel message as a STDIO bridge session receives it.
#[derive(Debug, Clone, Serialize)]
pub struct PushChannelPreview {
    /// `notifications/claude/channel` `content`.
    pub content: String,
    /// `notifications/claude/channel` `meta` (`source`, `event`, `event_id`).
    pub meta: BTreeMap<String, String>,
}

/// Render the channel message the relay would send a STDIO bridge session for
/// an occurrence in SEP-3415 wire form (`eventId`, `name`, `timestamp`,
/// `data`), using the relay's own renderer and limits.
#[tauri::command]
pub async fn preview_push_channel_message(
    event: serde_json::Value,
    state: State<'_, Arc<lr_server::state::AppState>>,
) -> Result<PushChannelPreview, String> {
    let relay = relay(&state)?;
    let (content, meta) = relay.preview_channel_message(&event)?;
    let meta = meta
        .as_object()
        .map(|m| {
            m.iter()
                .map(|(k, v)| (k.clone(), v.as_str().unwrap_or_default().to_string()))
                .collect()
        })
        .unwrap_or_default();
    Ok(PushChannelPreview { content, meta })
}

// ============================================================================
// Claude Code STDIO bridge
// ============================================================================

/// What Claude Code's config holds for LocalRouter, and how to launch it.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClaudeCodeMcpSetup {
    /// "none", "http" (tools only), "stdio_bridge" or "other".
    pub transport: String,
    pub config_path: String,
    pub launch_command: String,
    /// The LocalRouter binary the bridge entry runs.
    pub binary_path: Option<String>,
}

/// Inspect `~/.claude.json` for the client's Claude Code MCP setup.
#[tauri::command]
pub async fn get_claude_code_mcp_setup(
    client_id: String,
    config_manager: State<'_, ConfigManager>,
) -> Result<ClaudeCodeMcpSetup, String> {
    let config = config_manager.get();
    find_client(&config, &client_id)?;
    let transport = match claude_code::installed_transport()? {
        claude_code::ClaudeMcpTransport::None => "none",
        claude_code::ClaudeMcpTransport::Http => "http",
        claude_code::ClaudeMcpTransport::StdioBridge => "stdio_bridge",
        claude_code::ClaudeMcpTransport::Other => "other",
    };
    Ok(ClaudeCodeMcpSetup {
        transport: transport.to_string(),
        config_path: claude_code::mcp_config_display_path(),
        launch_command: claude_code::CHANNELS_LAUNCH_COMMAND.to_string(),
        binary_path: claude_code::localrouter_binary().ok(),
    })
}

/// The explicit "Switch to STDIO bridge" action for an existing Claude Code
/// client: rewrites LocalRouter's entry in `~/.claude.json` to the bridge.
#[tauri::command]
pub async fn switch_claude_code_to_stdio_bridge(
    client_id: String,
    config_manager: State<'_, ConfigManager>,
    client_manager: State<'_, Arc<lr_clients::ClientManager>>,
) -> Result<LaunchResult, String> {
    let config = config_manager.get();
    let client = find_client(&config, &client_id)?;
    if client.template_id.as_deref() != Some("claude-code") {
        return Err("Only Claude Code clients use the Claude Code STDIO bridge".to_string());
    }
    let secret = client_manager
        .get_secret(&client_id)
        .map_err(|e| format!("Failed to get client secret: {}", e))?
        .ok_or("Client secret not found in keychain")?;
    claude_code::switch_to_stdio_bridge(&client_id, &secret)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn view_carries_the_global_settings() {
        let view = PushRelayConfigView::from(&PushRelayConfig::default());
        let json = serde_json::to_value(&view).unwrap();
        assert_eq!(
            json,
            serde_json::json!({
                "default_permission": "allow",
                "channel_messages_per_minute": 10,
                "history_retention_hours": 24,
                "history_max_events": 10000,
            })
        );
    }

    #[test]
    fn update_applies_values_within_bounds() {
        let mut push = PushRelayConfig::default();
        apply_push_relay_update(
            &mut push,
            &PushRelayConfigUpdate {
                channel_messages_per_minute: Some(600),
                history_retention_hours: Some(1),
                history_max_events: Some(1_000_000),
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(push.channel_messages_per_minute, 600);
        assert_eq!(push.history_retention_hours, 1);
        assert_eq!(push.history_max_events, 1_000_000);
        assert_eq!(
            push.default_permission,
            PushPermission::Allow,
            "untouched fields stay"
        );
    }

    #[test]
    fn update_refuses_out_of_bounds_values_and_changes_nothing() {
        for update in [
            PushRelayConfigUpdate {
                channel_messages_per_minute: Some(0),
                ..Default::default()
            },
            PushRelayConfigUpdate {
                channel_messages_per_minute: Some(601),
                ..Default::default()
            },
            PushRelayConfigUpdate {
                history_retention_hours: Some(0),
                ..Default::default()
            },
            PushRelayConfigUpdate {
                history_retention_hours: Some(721),
                ..Default::default()
            },
            PushRelayConfigUpdate {
                history_max_events: Some(99),
                ..Default::default()
            },
            PushRelayConfigUpdate {
                // A valid field beside an invalid one is not applied either
                default_permission: Some(PushPermission::Off),
                history_max_events: Some(1_000_001),
                ..Default::default()
            },
        ] {
            let mut push = PushRelayConfig::default();
            assert!(apply_push_relay_update(&mut push, &update).is_err());
            assert_eq!(push, PushRelayConfig::default(), "{update:?}");
        }
    }

    #[test]
    fn permission_levels_set_and_clear() {
        let mut p = PushPermissions::default();
        apply_push_permission(&mut p, "global", None, None, Some(PushPermission::Ask)).unwrap();
        apply_push_permission(
            &mut p,
            "server",
            Some("vc"),
            None,
            Some(PushPermission::Off),
        )
        .unwrap();
        apply_push_permission(
            &mut p,
            "event",
            Some("vc"),
            Some("message.received"),
            Some(PushPermission::Allow),
        )
        .unwrap();
        assert_eq!(p.global, Some(PushPermission::Ask));
        assert_eq!(p.servers["vc"], PushPermission::Off);
        assert_eq!(
            p.effective(PushPermission::Allow, "vc", "message.received"),
            PushPermission::Allow
        );
        assert_eq!(
            p.effective(PushPermission::Allow, "vc", "other"),
            PushPermission::Off
        );

        apply_push_permission(&mut p, "event", Some("vc"), Some("message.received"), None).unwrap();
        apply_push_permission(&mut p, "server", Some("vc"), None, None).unwrap();
        apply_push_permission(&mut p, "global", None, None, None).unwrap();
        assert!(p.is_empty());

        assert!(apply_push_permission(&mut p, "server", None, None, None).is_err());
        assert!(apply_push_permission(&mut p, "event", Some("vc"), None, None).is_err());
        assert!(apply_push_permission(&mut p, "tool", None, None, None).is_err());
    }

    fn server_with_snapshot(
        modes: Vec<(&str, Vec<EventDeliveryMode>)>,
        channels: bool,
    ) -> lr_config::McpServerConfig {
        let mut server = lr_config::McpServerConfig::new(
            "VocalCord".to_string(),
            lr_config::McpTransportType::Stdio,
            lr_config::McpTransportConfig::Stdio {
                command: "vocalcord".to_string(),
                args: vec![],
                env: Default::default(),
                cwd: None,
            },
        );
        let events_modes = modes
            .iter()
            .map(|(n, m)| (n.to_string(), m.clone()))
            .collect();
        let catalogue = modes
            .iter()
            .map(|(n, _)| serde_json::json!({"name": n, "description": format!("{n} happened")}))
            .collect();
        server.capability_snapshot = Some(lr_config::UpstreamCapabilitySnapshot {
            fingerprint: lr_config::fingerprint_for(&server),
            protocol_version: "2025-11-25".to_string(),
            capabilities: serde_json::json!({}),
            push: lr_config::UpstreamPushSummary {
                events_modes,
                channels,
                resource_subscribe: false,
            },
            event_catalogue: catalogue,
            observed_at: chrono::Utc::now(),
        });
        server
    }

    #[test]
    fn source_marks_webhook_only_events_unsupported() {
        let server = server_with_snapshot(
            vec![
                (
                    "message.received",
                    vec![EventDeliveryMode::Push, EventDeliveryMode::Poll],
                ),
                ("invoice.paid", vec![EventDeliveryMode::Webhook]),
            ],
            false,
        );
        let info = push_source_info(&server);
        assert_eq!(info.snapshot_status, "current");
        assert!(info.has_consumable_push);
        let paid = info
            .events
            .iter()
            .find(|e| e.name == "invoice.paid")
            .unwrap();
        assert!(!paid.supported);
        let msg = info
            .events
            .iter()
            .find(|e| e.name == "message.received")
            .unwrap();
        assert!(msg.supported);
        assert_eq!(
            msg.description.as_deref(),
            Some("message.received happened")
        );
    }

    #[test]
    fn webhook_only_server_has_no_consumable_push() {
        let server = server_with_snapshot(
            vec![("invoice.paid", vec![EventDeliveryMode::Webhook])],
            false,
        );
        assert!(!push_source_info(&server).has_consumable_push);
    }

    #[test]
    fn stale_and_missing_snapshots() {
        let mut server = server_with_snapshot(vec![], true);
        if let lr_config::McpTransportConfig::Stdio { command, .. } = &mut server.transport_config {
            *command = "changed".to_string();
        }
        let info = push_source_info(&server);
        assert_eq!(info.snapshot_status, "stale");
        assert!(!info.has_consumable_push);

        server.capability_snapshot = None;
        let info = push_source_info(&server);
        assert_eq!(info.snapshot_status, "none");
        assert!(info.observed_at.is_none());
    }

    #[test]
    fn client_settings_report_mcp_access() {
        let mut config = lr_config::AppConfig::default();
        let server = server_with_snapshot(vec![], true);
        let server_id = server.id.clone();
        config.mcp_servers.push(server);
        let mut client =
            lr_config::Client::new_with_strategy("Claude".to_string(), "s1".to_string());
        client.push_permissions.global = Some(PushPermission::Ask);
        let settings = client_push_settings(&config, &client);
        assert_eq!(settings.effective_client, PushPermission::Ask);
        assert_eq!(settings.global_default, PushPermission::Allow);
        let access = settings
            .servers
            .iter()
            .find(|s| s.server_id == server_id)
            .unwrap();
        assert_eq!(
            access.mcp_access,
            client
                .mcp_permissions
                .resolve_server(&server_id)
                .is_enabled()
        );
        assert_eq!(access.effective, PushPermission::Ask);
    }

    #[test]
    fn receiving_clients_follow_the_default_and_overrides() {
        let mut config = lr_config::AppConfig::default();
        let server = server_with_snapshot(
            vec![
                ("message.received", vec![EventDeliveryMode::Push]),
                ("invoice.paid", vec![EventDeliveryMode::Webhook]),
            ],
            false,
        );
        let server_id = server.id.clone();
        config.mcp_servers.push(server);
        let mut a = lr_config::Client::new_with_strategy("A".to_string(), "s1".to_string());
        a.mcp_permissions.global = lr_config::PermissionState::Allow;
        let mut b = a.clone();
        b.id = "b".to_string();
        config.clients = vec![a, b];
        let count = |config: &lr_config::AppConfig| {
            let info = push_source_info(&config.mcp_servers[0]);
            receiving_clients(config, &info)
        };
        assert_eq!(count(&config), 2);
        assert!(client_push_settings(&config, &config.clients[0]).receives_push);

        config.push_relay.default_permission = PushPermission::Off;
        assert_eq!(count(&config), 0);
        assert!(!client_push_settings(&config, &config.clients[0]).receives_push);

        // An override on the webhook-only type enables nothing deliverable
        config.clients[0].set_push_event_permission(
            &server_id,
            "invoice.paid",
            Some(PushPermission::Allow),
        );
        assert_eq!(count(&config), 0);

        config.clients[0].set_push_event_permission(
            &server_id,
            "message.received",
            Some(PushPermission::Ask),
        );
        assert_eq!(count(&config), 1);
        assert!(client_push_settings(&config, &config.clients[0]).receives_push);
        assert!(!client_push_settings(&config, &config.clients[1]).receives_push);
    }

    fn held(
        hold_id: &str,
        name: &str,
        position: usize,
    ) -> lr_mcp::gateway::push_relay::HeldPushEventInfo {
        lr_mcp::gateway::push_relay::HeldPushEventInfo {
            hold_id: hold_id.to_string(),
            client_id: "c1".to_string(),
            client_name: "Claude".to_string(),
            server_id: "vc-id".to_string(),
            server_name: "vocalcord".to_string(),
            event_name: name.to_string(),
            qualified_name: format!("vocalcord__{name}"),
            event_id: format!("vocalcord__{hold_id}"),
            ingress_mode: "push".to_string(),
            data: serde_json::json!({"text": "hi"}),
            held_at: chrono::DateTime::parse_from_rfc3339("2026-10-30T10:00:00Z")
                .unwrap()
                .with_timezone(&chrono::Utc),
            held_secs: 5,
            position,
            firewall_request_id: (position == 0).then(|| "fw-1".to_string()),
            decided: false,
        }
    }

    #[test]
    fn pending_approvals_count_others_of_the_same_type() {
        let out = pending_push_approvals(vec![
            held("h1", "message.received", 0),
            held("h2", "message.received", 1),
            held("h3", "reaction.added", 0),
        ]);
        assert_eq!(out[0].request_id, "h1");
        assert_eq!(out[0].pending_of_type, 1);
        assert_eq!(out[1].pending_of_type, 1);
        assert_eq!(out[2].pending_of_type, 0);
        assert_eq!(out[0].firewall_request_id.as_deref(), Some("fw-1"));
        assert!(out[0].received_at.starts_with("2026-10-30T10:00:00"));
        let json = serde_json::to_value(&out[0]).unwrap();
        assert_eq!(json["ingress"], "push");
        assert_eq!(json["qualified_name"], "vocalcord__message.received");
    }

    #[test]
    fn approval_actions_map_to_relay_decisions() {
        use lr_mcp::gateway::push_relay::PushDecisionAction as A;
        let cases = [
            (PushApprovalAction::Allow, A::Allow, false),
            (PushApprovalAction::Deny, A::Deny, false),
            (PushApprovalAction::AllowAllPendingOfType, A::Allow, true),
            (PushApprovalAction::AllowTypeSession, A::AllowSession, false),
            (PushApprovalAction::DenyTypeSession, A::DenySession, false),
            (PushApprovalAction::AllowTypeAlways, A::AllowAlways, false),
            (PushApprovalAction::DenyTypeAlways, A::DenyAlways, false),
        ];
        for (action, expected, all) in cases {
            let (decision, all_of_type) = push_decision_for(action, None).unwrap();
            assert_eq!(decision.action, expected, "{action:?}");
            assert_eq!(all_of_type, all, "{action:?}");
            assert!(decision.edited_data.is_none());
        }
        assert!(push_decision_for(PushApprovalAction::AllowEdited, None).is_err());
        let (decision, all) = push_decision_for(
            PushApprovalAction::AllowEdited,
            Some(serde_json::json!({"text": "edited"})),
        )
        .unwrap();
        assert_eq!(decision.action, A::Allow);
        assert!(!all);
        assert_eq!(decision.edited_data.unwrap()["text"], "edited");
        let parsed: PushApprovalAction =
            serde_json::from_value(serde_json::json!("allow_all_pending_of_type")).unwrap();
        assert_eq!(parsed, PushApprovalAction::AllowAllPendingOfType);
    }
}
