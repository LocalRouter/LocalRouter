//! Built-in LocalRouter management tools, exposed through the client MCP gateway.

mod operations;

use std::any::Any;
use std::collections::BTreeSet;
use std::sync::{Arc, Weak};

use async_trait::async_trait;
use lr_config::{AdminMcpAccess, Client};
use lr_mcp::gateway::access_control::FirewallCheckResult;
use lr_mcp::gateway::virtual_server::*;
use lr_mcp::gateway::FirewallDecisionResult;
use lr_mcp::protocol::McpTool;
use lr_server::state::AppState;
use serde_json::{json, Value};

pub const ID: &str = lr_mcp::gateway::firewall::ADMIN_MCP_SERVER_ID;
const LIST: &str = "lr_skills_list";
const GET: &str = "lr_skills_get";
const STATUS: &str = "lr_status";

#[derive(Clone, Copy, PartialEq, Eq)]
enum Access {
    Configuration,
    Manage,
    Providers,
    Monitor,
    Intercept,
}

impl Access {
    fn granted(self, grant: &AdminMcpAccess) -> bool {
        match self {
            Self::Configuration => grant.configuration || grant.manage,
            Self::Manage => grant.manage,
            Self::Providers => grant.providers,
            Self::Monitor => grant.monitor,
            Self::Intercept => grant.intercept,
        }
    }
}

struct Skill {
    id: &'static str,
    description: &'static str,
    content: &'static str,
    access: Access,
}

macro_rules! skill {
    ($id:literal, $description:literal, $access:ident) => {
        Skill {
            id: $id,
            description: $description,
            content: include_str!(concat!("admin_mcp/skills/", $id, "/SKILL.md")),
            access: Access::$access,
        }
    };
}

const SKILLS: &[Skill] = &[
    skill!(
        "client-setup",
        "Create clients and configure their connection.",
        Configuration
    ),
    skill!(
        "providers",
        "Configure providers, check health, and discover models.",
        Providers
    ),
    skill!(
        "model-routing",
        "Inspect and configure strategies, profiles, and model routing.",
        Configuration
    ),
    skill!(
        "firewall",
        "Inspect and configure client firewall permissions.",
        Configuration
    ),
    skill!(
        "configuration",
        "Inspect and change application settings.",
        Configuration
    ),
    skill!(
        "monitoring",
        "Find and inspect local traffic metadata.",
        Monitor
    ),
    skill!(
        "active-intercept",
        "Hold requests for review and watch for new approvals.",
        Intercept
    ),
    skill!(
        "mcp-servers",
        "Inspect and manage upstream MCP servers.",
        Configuration
    ),
];

#[derive(Clone)]
struct AdminSession {
    client_id: String,
    loaded: BTreeSet<&'static str>,
}

impl VirtualSessionState for AdminSession {
    fn as_any(&self) -> &dyn Any {
        self
    }

    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }

    fn clone_box(&self) -> Box<dyn VirtualSessionState> {
        Box::new(self.clone())
    }
}

struct Tool {
    name: &'static str,
    skill: &'static str,
    access: Access,
    description: &'static str,
    arguments: &'static str,
    confirm: bool,
}

macro_rules! tool {
    ($name:literal, $skill:literal, $access:ident, $description:literal, $arguments:literal $(, $confirm:literal)?) => {
        Tool {
            name: $name,
            skill: $skill,
            access: Access::$access,
            description: $description,
            arguments: $arguments,
            confirm: false $(|| $confirm)?,
        }
    };
}

const TOOLS: &[Tool] = &[
    tool!("lr_clients_credentials", "client-setup", Manage, "Rotate a client credential or revoke existing tokens. Use its desktop Connection tab for the credential.", "client_id:s action:s", true),
    tool!("lr_routing_profiles_manage", "model-routing", Manage, "Create, revise, publish, roll back, bind or delete a routing profile through the routing manager.", "action:s profile_id:s? strategy_id:s? name:s? settings:o?", true),
    tool!("lr_local_models_list", "providers", Providers, "List local model library entries and download jobs. Supply provider to inspect its loaded models.", "provider:s?"),
    tool!("lr_local_models_manage", "providers", Manage, "Download, import, load, unload, pause, cancel or remove a local model.", "action:s provider:s? model:s? id:s? path:s? repo:s? revision:s? files:a?", true),
    tool!("lr_mcp_servers_check", "mcp-servers", Manage, "Run the desktop health check against a configured MCP server.", "server_id:s", true),
    tool!("lr_skill_sources_manage", "mcp-servers", Manage, "List, add, or remove a local skill source path.", "action:s path:s?", true),
    tool!("lr_installed_skills_list", "mcp-servers", Configuration, "List discovered skills and their enabled status.", ""),
    tool!("lr_installed_skills_manage", "mcp-servers", Manage, "Enable or disable an installed skill.", "name:s enabled:b", true),
    tool!("lr_accounts_manage", "configuration", Manage, "Inspect usage accounts or update usage tracking settings. Connect accounts in the desktop.", "action:s account_id:s? settings:o?", true),
    tool!("lr_monitor_metrics", "monitoring", Monitor, "Read recent metrics by client, provider, or model. Omit filters for global totals.", "minutes:i? client_id:s? provider:s? model:s?"),
    tool!("lr_monitor_events_wait", "monitoring", Monitor, "Wait briefly for newer Monitor metadata after a sequence number.", "after_sequence:u seconds:i? client_id:s?"),

    tool!("lr_clients_list", "client-setup", Configuration, "List clients. Use their IDs in other tools.", ""),
    tool!("lr_clients_get", "client-setup", Configuration, "Inspect a client and its strategy binding.", "client_id:s"),
    tool!("lr_clients_create", "client-setup", Manage, "Create a client and its default strategy. Use the desktop connection panel for its credential.", "name:s llm_mode:s? mcp_mode:s?", true),
    tool!("lr_clients_update", "client-setup", Manage, "Update a client's name, enabled state, modes, or strategy binding.", "client_id:s changes:o", true),
    tool!("lr_clients_delete", "client-setup", Manage, "Delete a client and revoke its credentials.", "client_id:s", true),
    tool!("lr_clients_connection", "client-setup", Configuration, "Get a client's gateway URLs and desktop connection setup instructions.", "client_id:s"),
    tool!("lr_provider_types_list", "providers", Providers, "List provider types and setup fields.", ""),
    tool!("lr_providers_list", "providers", Providers, "List configured provider instances.", ""),
    tool!("lr_providers_get", "providers", Providers, "Inspect a configured provider and its credential status.", "provider:s"),
    tool!("lr_providers_create", "providers", Manage, "Create a keyless provider. For API keys or OAuth use the desktop credential flow.", "provider:s provider_type:s settings:o?", true),
    tool!("lr_providers_update", "providers", Manage, "Change a configured provider's enabled state.", "provider:s enabled:b", true),
    tool!("lr_providers_delete", "providers", Manage, "Remove a provider instance and its saved credentials.", "provider:s", true),
    tool!("lr_providers_credentials", "providers", Providers, "Get the desktop path for provider sign-in, key entry, or disconnect.", "provider:s"),
    tool!("lr_providers_check", "providers", Providers, "Run the provider's existing health check. It may send an upstream request.", "provider:s", true),
    tool!("lr_providers_test_model", "providers", Providers, "Send a bounded synthetic chat request to the selected provider and model. This can incur charges.", "provider:s model:s", true),
    tool!("lr_models_list", "providers", Providers, "List cached models for one or all providers. Use lr_models_refresh to fetch live models.", "provider:s? offset:u? limit:i?"),
    tool!("lr_models_get", "providers", Providers, "Inspect a cached model's capabilities and effective pricing.", "provider:s model:s"),
    tool!("lr_models_refresh", "providers", Providers, "Refresh selected provider catalogs. Omit provider to refresh enabled instances.", "provider:s?", true),
    tool!("lr_models_configure", "providers", Manage, "Replace a provider's managed-model settings.", "provider:s settings:o", true),
    tool!("lr_routing_strategies_list", "model-routing", Configuration, "List routing strategies and their client bindings.", ""),
    tool!("lr_routing_strategies_get", "model-routing", Configuration, "Read one strategy and its client bindings.", "strategy_id:s"),
    tool!("lr_routing_strategies_manage", "model-routing", Manage, "Create, edit, or delete a routing strategy. Read it before editing.", "action:s strategy_id:s? changes:o?", true),
    tool!("lr_routing_profiles_list", "model-routing", Configuration, "List reusable routing profiles.", ""),
    tool!("lr_routing_profiles_get", "model-routing", Configuration, "Read a reusable routing profile by ID.", "profile_id:s"),
    tool!("lr_routing_preview", "model-routing", Configuration, "Preview the effective routing policy for a client without running inference.", "client_id:s"),
    tool!("lr_firewall_get", "firewall", Configuration, "Read configured firewall rules and model permissions, including inheritance defaults.", "client_id:s"),
    tool!("lr_firewall_preview", "firewall", Manage, "Preview a patch to firewall settings. The result identifies clients sharing the strategy.", "client_id:s changes:o"),
    tool!("lr_firewall_apply", "firewall", Manage, "Apply a previously previewed firewall patch against its revision.", "client_id:s changes:o revision:s", true),
    tool!("lr_config_list_sections", "configuration", Configuration, "List application settings supported by the config tools.", ""),
    tool!("lr_config_schema", "configuration", Configuration, "Inspect configurable keys and default values for a section.", "section:s"),
    tool!("lr_config_get", "configuration", Configuration, "Read one application settings section, including its revision.", "section:s"),
    tool!("lr_config_preview", "configuration", Manage, "Validate a typed section patch and show its impact.", "section:s changes:o"),
    tool!("lr_config_apply", "configuration", Manage, "Apply a previously previewed section patch against its revision.", "section:s changes:o revision:s", true),
    tool!("lr_monitor_stats", "monitoring", Monitor, "Show Monitor retention and event counts.", ""),
    tool!("lr_monitor_events_list", "monitoring", Monitor, "List recent event metadata. Use before_sequence from the previous page to continue.", "client_id:s? before_sequence:i? limit:i?"),
    tool!("lr_monitor_event_get", "monitoring", Monitor, "Read one Monitor event's metadata by ID.", "event_id:s"),
    tool!("lr_monitor_clear", "monitoring", Monitor, "Clear the in-memory Monitor history.", "", true),
    tool!("lr_intercept_get", "active-intercept", Intercept, "Read the active Monitor intercept rule.", ""),
    tool!("lr_intercept_start", "active-intercept", Intercept, "Temporarily route matching clients and categories through the approval popup.", "client_ids:a categories:a seconds:i", true),
    tool!("lr_intercept_stop", "active-intercept", Intercept, "Stop the rule this client previously started.", "rule_id:s", true),
    tool!("lr_intercept_list_pending", "active-intercept", Intercept, "List pending requests matching your active intercept rule.", ""),
    tool!("lr_intercept_get_pending", "active-intercept", Intercept, "Inspect metadata for one pending request matching your rule.", "request_id:s"),
    tool!("lr_intercept_resolve", "active-intercept", Intercept, "Allow or deny an eligible held request once, after desktop confirmation.", "request_id:s action:s", true),
    tool!("lr_mcp_servers_list", "mcp-servers", Configuration, "List configured MCP servers and their current status.", ""),
    tool!("lr_mcp_servers_get", "mcp-servers", Configuration, "Inspect a configured MCP server's safe metadata.", "server_id:s"),
    tool!("lr_mcp_servers_capabilities", "mcp-servers", Configuration, "Read remembered upstream capabilities.", "server_id:s"),
    tool!("lr_mcp_servers_manage", "mcp-servers", Manage, "Create an HTTP MCP server, rename or remove a server, or start or stop it.", "server_id:s? action:s name:s? url:s?", true),
];

fn definition(name: &str, description: &str, arguments: &str) -> McpTool {
    let mut properties = serde_json::Map::new();
    let mut required = Vec::new();
    for field in arguments.split_whitespace() {
        let (key, kind) = field.split_once(':').expect("tool argument descriptor");
        let schema = match kind.trim_end_matches('?') {
            "s" => json!({"type": "string"}),
            "i" => json!({"type": "integer", "minimum": 1}),
            "u" => json!({"type": "integer", "minimum": 0}),
            "o" => json!({"type": "object"}),
            "b" => json!({"type": "boolean"}),
            "a" => json!({"type": "array", "items": {"type": "string"}}),
            _ => unreachable!("unknown tool argument descriptor"),
        };
        properties.insert(key.to_owned(), schema);
        if !kind.ends_with('?') {
            required.push(key);
        }
    }
    McpTool {
        name: name.to_owned(),
        description: Some(description.to_owned()),
        input_schema: json!({
            "type": "object", "properties": properties, "required": required,
            "additionalProperties": false,
        }),
    }
}

fn result(data: Value) -> Value {
    json!({
        "content": [{"type": "text", "text": data.to_string()}],
        "structuredContent": data,
    })
}

pub struct AdminMcp {
    app: tauri::AppHandle,
    state: Weak<AppState>,
}

impl AdminMcp {
    pub fn new(app: tauri::AppHandle, state: &Arc<AppState>) -> Self {
        let mut notifications = state.mcp_notification_broadcast.subscribe();
        let weak = Arc::downgrade(state);
        let task_state = weak.clone();
        tauri::async_runtime::spawn(async move {
            let mut announced = std::collections::HashMap::new();
            loop {
                let (channel, notification) = match notifications.recv().await {
                    Ok(event) => event,
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => continue,
                    Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
                };
                if channel != "_firewall" {
                    continue;
                }
                let Some(state) = task_state.upgrade() else {
                    break;
                };
                let Some(relay) = state.push_relay.as_ref() else {
                    continue;
                };
                let Some(params) = notification.params.as_ref() else {
                    continue;
                };
                let Some(id) = params["request_id"].as_str() else {
                    continue;
                };
                match notification.method.as_str() {
                    "firewall/approvalRequired" => {
                        if let Some((actor, data)) =
                            state.mcp_gateway.firewall_manager.intercept_event(id)
                        {
                            if announced.len() >= 1024 {
                                announced.clear();
                            }
                            relay.publish_intercept(&actor, data);
                            announced.insert(id.to_string(), actor);
                        }
                    }
                    "firewall/approvalResolved" => {
                        if let Some(actor) = announced.remove(id) {
                            relay.publish_intercept(
                                &actor,
                                json!({"request_id": id, "status": "resolved"}),
                            );
                        }
                    }
                    _ => {}
                }
            }
        });
        Self { app, state: weak }
    }

    fn grant(&self, client_id: &str) -> Option<AdminMcpAccess> {
        self.state
            .upgrade()?
            .client_manager
            .get_client(client_id)
            .filter(|client| self.is_enabled(client))
            .map(|client| client.security.admin_mcp)
    }

    fn available(tool: &Tool, session: &AdminSession, grant: &AdminMcpAccess) -> bool {
        session.loaded.contains(tool.skill) && tool.access.granted(grant)
    }
}

#[async_trait]
impl VirtualMcpServer for AdminMcp {
    fn id(&self) -> &str {
        ID
    }
    fn display_name(&self) -> &str {
        lr_mcp::gateway::types::LOCALROUTER_MCP_NAME
    }
    fn owns_tool(&self, name: &str) -> bool {
        matches!(name, STATUS | LIST | GET) || TOOLS.iter().any(|tool| tool.name == name)
    }

    fn is_enabled(&self, client: &Client) -> bool {
        client.credentials_active() && !client.security.admin_mcp.is_default()
    }

    fn list_tools(&self, state: &dyn VirtualSessionState) -> Vec<McpTool> {
        let Some(session) = state.as_any().downcast_ref::<AdminSession>() else {
            return Vec::new();
        };
        let Some(grant) = self.grant(&session.client_id) else {
            return Vec::new();
        };
        let mut tools = vec![
            definition(STATUS, "Get LocalRouter status.", ""),
            definition(LIST, "List available LocalRouter administration skills.", ""),
            definition(GET, "Load a skill and unlock its tools in this session. For providers use providers, for routing use model-routing, for clients use client-setup, and for live monitoring use active-intercept. Refresh tools/list after loading.", "skill:s"),
        ];
        tools.extend(
            TOOLS
                .iter()
                .filter(|tool| Self::available(tool, session, &grant))
                .map(|tool| definition(tool.name, tool.description, tool.arguments)),
        );
        tools
    }

    fn check_permissions(
        &self,
        state: &dyn VirtualSessionState,
        name: &str,
        _: Option<&Value>,
        _: bool,
        _: bool,
    ) -> VirtualFirewallResult {
        if self.list_tools(state).iter().any(|tool| tool.name == name) {
            // Each write uses a dedicated desktop approval. The management
            // connection must remain usable while the Monitor holds traffic.
            VirtualFirewallResult::Handled(FirewallDecisionResult::Proceed)
        } else {
            VirtualFirewallResult::Standard(FirewallCheckResult::Deny)
        }
    }

    async fn handle_tool_call(
        &self,
        state: Box<dyn VirtualSessionState>,
        name: &str,
        args: Value,
        client_id: &str,
        client_name: &str,
    ) -> VirtualToolCallResult {
        let Some(session) = state.as_any().downcast_ref::<AdminSession>() else {
            return VirtualToolCallResult::NotHandled;
        };
        let Some(grant) = self
            .grant(client_id)
            .filter(|_| session.client_id == client_id)
        else {
            return VirtualToolCallResult::ToolError("LocalRouter access is disabled".into());
        };
        match name {
            STATUS => {
                return VirtualToolCallResult::Success(result(
                    json!({"version": self.app.package_info().version.to_string(), "gateway": "running"}),
                ))
            }
            LIST => {
                let skills: Vec<_> = SKILLS.iter().filter(|skill| skill.access.granted(&grant))
                    .map(|skill| json!({"name": skill.id, "description": skill.description, "loaded": session.loaded.contains(skill.id)})).collect();
                return VirtualToolCallResult::Success(result(json!({"skills": skills})));
            }
            GET => {
                let selected = args["skill"].as_str();
                let Some(skill) = SKILLS
                    .iter()
                    .find(|skill| Some(skill.id) == selected && skill.access.granted(&grant))
                else {
                    return VirtualToolCallResult::ToolError(
                        "Unknown or unavailable skill. Use lr_skills_list.".into(),
                    );
                };
                let id = skill.id;
                let tools: Vec<_> = TOOLS
                    .iter()
                    .filter(|tool| tool.skill == id && tool.access.granted(&grant))
                    .map(|tool| tool.name)
                    .collect();
                let response = result(json!({
                    "skill": id, "instructions": skill.content, "tools": tools,
                    "next": "Refresh tools/list to use the unlocked tools.",
                }));
                if session.loaded.contains(id) {
                    return VirtualToolCallResult::Success(response);
                }
                return VirtualToolCallResult::SuccessWithSideEffects {
                    response,
                    invalidate_cache: true,
                    send_list_changed: true,
                    state_update: Some(Box::new(move |state| {
                        if let Some(session) = state.as_any_mut().downcast_mut::<AdminSession>() {
                            session.loaded.insert(id);
                        }
                    })),
                    add_allowed_servers: None,
                };
            }
            _ => {}
        }
        let Some(tool) = TOOLS
            .iter()
            .find(|tool| tool.name == name && Self::available(tool, session, &grant))
        else {
            return VirtualToolCallResult::ToolError(
                "Tool unavailable. Load its skill first.".into(),
            );
        };
        match self.execute(tool, args, client_id, client_name).await {
            Ok(value) => VirtualToolCallResult::Success(result(value)),
            Err(error) => VirtualToolCallResult::ToolError(error),
        }
    }

    fn build_instructions(&self, state: &dyn VirtualSessionState) -> Option<VirtualInstructions> {
        self.list_tools(state).iter().any(|tool| tool.name == GET).then(|| VirtualInstructions {
            section_title: "localrouter_admin".into(),
            content: "Call lr_skills_list to see available LocalRouter skills. To create a client, call lr_skills_get with skill client-setup to unlock lr_clients_create. To work with providers or models, load providers to unlock those tools. For routing load model-routing, for access rules load firewall, and for held requests load active-intercept. Loading a skill changes this session's tool list. Refresh tools/list if the client does not do so automatically.".into(),
            tool_names: vec![STATUS.into(), LIST.into(), GET.into()],
            priority: 10,
        })
    }

    fn create_session_state(&self, client: &Client) -> Box<dyn VirtualSessionState> {
        Box::new(AdminSession {
            client_id: client.id.clone(),
            loaded: BTreeSet::new(),
        })
    }

    fn update_session_state(&self, _: &mut dyn VirtualSessionState, _: &Client) {
        // The session owns its loaded groups. Permissions are read live.
    }

    fn all_tool_names(&self) -> Vec<String> {
        [STATUS, LIST, GET]
            .into_iter()
            .chain(TOOLS.iter().map(|tool| tool.name))
            .map(str::to_owned)
            .collect()
    }

    fn is_tool_indexable(&self, _: &str) -> bool {
        false
    }
}
