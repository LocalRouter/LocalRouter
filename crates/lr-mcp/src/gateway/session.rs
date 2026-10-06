#![allow(dead_code)]

use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::time::Instant;

use super::context_mode::ContextModeSessionState;
use super::types::*;
use super::virtual_server::VirtualSessionState;
use crate::protocol::Root;
use crate::transport::SessionTransportSet;

/// Gateway session (one per client)
pub struct GatewaySession {
    /// Client ID
    pub client_id: String,

    /// Session key (SSE connection UUID for SSE, client_id for non-SSE)
    pub session_key: String,

    /// Servers the session was created for (sorted). Compared against each
    /// request's server list to detect permission/config changes; unlike
    /// `allowed_servers` it is not narrowed when a server fails to start.
    pub requested_servers: Vec<String>,

    /// Servers this session currently routes to: the requested servers minus
    /// any that failed to start.
    pub allowed_servers: Vec<String>,

    /// Initialization status for each server
    pub server_init_status: HashMap<String, InitStatus>,

    /// Merged capabilities (cached after successful initialization)
    pub merged_capabilities: Option<MergedCapabilities>,

    /// Client capabilities (from initialize request params)
    pub client_capabilities: Option<ClientCapabilities>,

    /// Tool name mapping: namespaced_name -> (server_id, original_name)
    pub tool_mapping: HashMap<String, (String, String)>,

    /// Resource name mapping: namespaced_name -> (server_id, original_name)
    pub resource_mapping: HashMap<String, (String, String)>,

    /// Resource URI mapping: uri -> (server_id, original_name)
    /// Used for routing resources/read by URI instead of namespaced name
    pub resource_uri_mapping: HashMap<String, (String, String)>,

    /// Prompt name mapping: namespaced_name -> (server_id, original_name)
    pub prompt_mapping: HashMap<String, (String, String)>,

    /// Cached tools list
    pub cached_tools: Option<CachedList<NamespacedTool>>,

    /// Cached resources list
    pub cached_resources: Option<CachedList<NamespacedResource>>,

    /// Cached prompts list
    pub cached_prompts: Option<CachedList<NamespacedPrompt>>,

    /// Session creation time
    pub created_at: Instant,

    /// Last activity time (for TTL)
    pub last_activity: Instant,

    /// Session TTL
    pub ttl: std::time::Duration,

    /// Dynamic cache TTL manager
    pub cache_ttl_manager: DynamicCacheTTL,

    /// Last broadcast failures (for exposing in responses)
    pub last_broadcast_failures: Vec<ServerFailure>,

    /// Track if resources/list was fetched (to avoid redundant auto-fetches in URI fallback)
    pub resources_list_fetched: bool,

    /// Filesystem roots for this session (advisory boundaries)
    /// Merged from global config + per-client overrides
    pub roots: Vec<Root>,

    /// Subscribed resource URIs (uri -> server_id)
    /// Tracks which resources this session has subscribed to for change notifications
    pub subscribed_resources: HashMap<String, String>,

    /// MCP permissions snapshot for this client (for change detection)
    pub mcp_permissions: lr_config::McpPermissions,

    /// Skills permissions for this client (hierarchical Allow/Ask/Off)
    pub skills_permissions: lr_config::SkillsPermissions,

    /// Human-readable client name (for firewall approval display)
    pub client_name: String,

    /// Tools approved during this session via "Allow for Session" action
    pub firewall_session_approvals: HashSet<String>,

    /// Tools denied during this session via "Deny for Session" action
    pub firewall_session_denials: HashSet<String>,

    /// Per-virtual-server session state (server_id -> state)
    pub virtual_server_state: HashMap<String, Box<dyn VirtualSessionState>>,

    /// Context management overrides snapshot (for change detection / cache invalidation)
    pub context_management_overrides: Option<lr_config::ContextManagementOverrides>,

    /// Catalog compression plan (computed during initialize when context management is enabled).
    /// Used to filter deferred items from tools/resources/prompts lists and compress descriptions.
    pub catalog_compression: Option<CatalogCompressionPlan>,

    /// Instructions context snapshot (without compression plan) from initialization.
    /// Used by the compression preview UI to re-compute plans at different thresholds.
    pub instructions_context: Option<super::merger::InstructionsContext>,

    /// Sampling permission for this client session (Allow/Ask/Off)
    /// Deprecated: now global via McpGatewaySettings, kept for backward compat
    pub mcp_sampling_permission: lr_config::PermissionState,

    /// Elicitation permission for this client session (Allow/Ask/Off)
    /// Deprecated: now global via McpGatewaySettings, kept for backward compat
    pub mcp_elicitation_permission: lr_config::PermissionState,

    /// Client mode for this session (Both/LlmOnly/McpOnly/McpViaLlm)
    pub client_mode: lr_config::ClientMode,

    /// Pending requests: request_id -> server_id (for forwarding notifications/cancelled)
    pub pending_requests: HashMap<String, String>,

    /// Monitor session ID for grouping events from one API request.
    /// Set by MCP-via-LLM to link tool call events to the parent LLM call.
    pub monitor_session_id: Option<String>,

    /// Per-session MCP server transports (owned by this session).
    /// Created during `handle_initialize` (or lazily on first use for
    /// stateless clients), closed when the session ends.
    /// Wrapped in `Arc` so it can be extracted with a brief read lock and used
    /// without holding the session lock during requests.
    pub transports: Option<Arc<SessionTransportSet>>,

    /// Protocol revision the downstream client speaks.
    /// Legacy (2025-11-25) by default; upgraded when a request declares
    /// `io.modelcontextprotocol/protocolVersion: 2026-07-28` in `_meta`.
    pub protocol_revision: crate::protocol::ProtocolRevision,

    /// Per-request log level from `_meta` (2026-07-28 replaces
    /// `logging/setLevel`); last seen value.
    pub log_level: Option<String>,

    /// Serializes lazy backend initialization for stateless clients so
    /// concurrent first requests can't double-create transports (which
    /// would leak the first set's stdio processes).
    pub lazy_init_lock: Arc<tokio::sync::Mutex<()>>,
}

impl GatewaySession {
    /// Create a new session
    pub fn new(
        client_id: String,
        allowed_servers: Vec<String>,
        ttl: std::time::Duration,
        base_cache_ttl_seconds: u64,
        roots: Vec<Root>,
    ) -> Self {
        let now = Instant::now();
        let mut server_init_status = HashMap::new();

        // Initialize all allowed servers as NotStarted
        for server_id in &allowed_servers {
            server_init_status.insert(server_id.clone(), InitStatus::NotStarted);
        }

        let mut requested_servers = allowed_servers.clone();
        requested_servers.sort();

        Self {
            session_key: client_id.clone(), // default; overridden by gateway for SSE
            client_id,
            requested_servers,
            allowed_servers,
            server_init_status,
            merged_capabilities: None,
            client_capabilities: None,
            tool_mapping: HashMap::new(),
            resource_mapping: HashMap::new(),
            resource_uri_mapping: HashMap::new(),
            prompt_mapping: HashMap::new(),
            cached_tools: None,
            cached_resources: None,
            cached_prompts: None,
            created_at: now,
            last_activity: now,
            ttl,
            cache_ttl_manager: DynamicCacheTTL::new(base_cache_ttl_seconds),
            last_broadcast_failures: Vec::new(),
            resources_list_fetched: false,
            roots,
            subscribed_resources: HashMap::new(),
            mcp_permissions: lr_config::McpPermissions::default(),
            skills_permissions: lr_config::SkillsPermissions::default(),
            client_name: String::new(),
            firewall_session_approvals: HashSet::new(),
            firewall_session_denials: HashSet::new(),
            virtual_server_state: HashMap::new(),
            context_management_overrides: None,
            catalog_compression: None,
            instructions_context: None,
            mcp_sampling_permission: lr_config::PermissionState::default(),
            mcp_elicitation_permission: lr_config::PermissionState::default(),
            client_mode: lr_config::ClientMode::default(),
            pending_requests: HashMap::new(),
            monitor_session_id: None,
            transports: None,
            protocol_revision: crate::protocol::ProtocolRevision::default(),
            log_level: None,
            lazy_init_lock: Arc::new(tokio::sync::Mutex::new(())),
        }
    }

    /// Get activated tools from context-mode session state (if any).
    pub fn activated_tools(&self) -> Option<&HashSet<String>> {
        self.virtual_server_state
            .get("_context_mode")
            .and_then(|s| s.as_any().downcast_ref::<ContextModeSessionState>())
            .map(|cm| &cm.activated_tools)
    }

    /// Get activated resources from context-mode session state (if any).
    pub fn activated_resources(&self) -> Option<&HashSet<String>> {
        self.virtual_server_state
            .get("_context_mode")
            .and_then(|s| s.as_any().downcast_ref::<ContextModeSessionState>())
            .map(|cm| &cm.activated_resources)
    }

    /// Get activated prompts from context-mode session state (if any).
    pub fn activated_prompts(&self) -> Option<&HashSet<String>> {
        self.virtual_server_state
            .get("_context_mode")
            .and_then(|s| s.as_any().downcast_ref::<ContextModeSessionState>())
            .map(|cm| &cm.activated_prompts)
    }

    /// Check if session is expired
    pub fn is_expired(&self) -> bool {
        self.last_activity.elapsed() > self.ttl
    }

    /// Update last activity timestamp
    pub fn touch(&mut self) {
        self.last_activity = Instant::now();
    }

    /// Update tool mappings from a freshly fetched list of namespaced tools.
    ///
    /// Entries of servers in `failures` are kept: a transient `tools/list`
    /// failure must not make that server's tools uncallable.
    pub fn update_tool_mappings(&mut self, tools: &[NamespacedTool], failures: &[ServerFailure]) {
        let failed = failed_server_ids(failures);
        self.tool_mapping
            .retain(|_, (server_id, _)| failed.contains(server_id.as_str()));
        for tool in tools {
            self.tool_mapping.insert(
                tool.name.clone(),
                (tool.server_id.clone(), tool.original_name.clone()),
            );
        }
    }

    /// Update resource mappings from a freshly fetched list of namespaced
    /// resources, keeping entries of servers in `failures`.
    pub fn update_resource_mappings(
        &mut self,
        resources: &[NamespacedResource],
        failures: &[ServerFailure],
    ) {
        let failed = failed_server_ids(failures);
        self.resource_mapping
            .retain(|_, (server_id, _)| failed.contains(server_id.as_str()));
        self.resource_uri_mapping
            .retain(|_, (server_id, _)| failed.contains(server_id.as_str()));
        for resource in resources {
            // Map by namespaced name
            self.resource_mapping.insert(
                resource.name.clone(),
                (resource.server_id.clone(), resource.original_name.clone()),
            );

            // Also map by URI for URI-based routing
            self.resource_uri_mapping.insert(
                resource.uri.clone(),
                (resource.server_id.clone(), resource.original_name.clone()),
            );
        }
    }

    /// Update prompt mappings from a freshly fetched list of namespaced
    /// prompts, keeping entries of servers in `failures`.
    pub fn update_prompt_mappings(
        &mut self,
        prompts: &[NamespacedPrompt],
        failures: &[ServerFailure],
    ) {
        let failed = failed_server_ids(failures);
        self.prompt_mapping
            .retain(|_, (server_id, _)| failed.contains(server_id.as_str()));
        for prompt in prompts {
            self.prompt_mapping.insert(
                prompt.name.clone(),
                (prompt.server_id.clone(), prompt.original_name.clone()),
            );
        }
    }

    /// The catalog compression plan, if catalog compression is currently
    /// enabled for this client. The plan is computed at initialize; turning
    /// the feature off mid-session stops deferral immediately.
    pub fn active_catalog_compression(&self) -> Option<&CatalogCompressionPlan> {
        let enabled = self
            .virtual_server_state
            .get("_context_mode")
            .and_then(|s| s.as_any().downcast_ref::<ContextModeSessionState>())
            .is_some_and(|cm| cm.catalog_compression_enabled);
        if enabled {
            self.catalog_compression.as_ref()
        } else {
            None
        }
    }

    /// Tools to advertise in `tools/list`: those the client's permissions
    /// enable, minus any deferred by catalog compression. Tools excluded from
    /// indexing are never deferred — search could not surface them.
    pub fn visible_tools(&self, tools: &[NamespacedTool]) -> Vec<NamespacedTool> {
        let permitted: Vec<NamespacedTool> = tools
            .iter()
            .filter(|t| {
                self.mcp_permissions
                    .resolve_tool(&t.server_id, &t.original_name)
                    .is_enabled()
            })
            .cloned()
            .collect();
        let Some(plan) = self.active_catalog_compression() else {
            return permitted;
        };
        let kept: HashSet<String> = super::gateway_tools::apply_catalog_compression_tools(
            &permitted,
            Some(plan),
            self.activated_tools(),
        )
        .into_iter()
        .map(|t| t.name)
        .collect();
        let indexing = self
            .virtual_server_state
            .get("_context_mode")
            .and_then(|s| s.as_any().downcast_ref::<ContextModeSessionState>())
            .map(|cm| &cm.gateway_indexing);
        permitted
            .into_iter()
            .filter(|t| {
                kept.contains(&t.name)
                    || indexing.is_some_and(|perms| {
                        let slug = t.name.split_once("__").map_or(t.name.as_str(), |(s, _)| s);
                        !perms.is_tool_eligible(slug, &t.original_name)
                    })
            })
            .collect()
    }

    /// Resources to advertise in `resources/list` (permission-filtered by
    /// URI, minus deferred).
    pub fn visible_resources(&self, resources: &[NamespacedResource]) -> Vec<NamespacedResource> {
        let permitted: Vec<NamespacedResource> = resources
            .iter()
            .filter(|r| {
                self.mcp_permissions
                    .resolve_resource(&r.server_id, &r.uri)
                    .is_enabled()
            })
            .cloned()
            .collect();
        super::gateway_tools::apply_catalog_compression_resources(
            &permitted,
            self.active_catalog_compression(),
            self.activated_resources(),
        )
    }

    /// Prompts to advertise in `prompts/list` (permission-filtered, minus
    /// deferred).
    pub fn visible_prompts(&self, prompts: &[NamespacedPrompt]) -> Vec<NamespacedPrompt> {
        let permitted: Vec<NamespacedPrompt> = prompts
            .iter()
            .filter(|p| {
                self.mcp_permissions
                    .resolve_prompt(&p.server_id, &p.original_name)
                    .is_enabled()
            })
            .cloned()
            .collect();
        super::gateway_tools::apply_catalog_compression_prompts(
            &permitted,
            self.active_catalog_compression(),
            self.activated_prompts(),
        )
    }

    /// Invalidate tools cache
    pub fn invalidate_tools_cache(&mut self) {
        self.cached_tools = None;
    }

    /// Invalidate resources cache
    pub fn invalidate_resources_cache(&mut self) {
        self.cached_resources = None;
    }

    /// Invalidate prompts cache
    pub fn invalidate_prompts_cache(&mut self) {
        self.cached_prompts = None;
    }

    /// Invalidate all caches
    pub fn invalidate_all_caches(&mut self) {
        self.invalidate_tools_cache();
        self.invalidate_resources_cache();
        self.invalidate_prompts_cache();
    }

    /// Check if all servers are initialized
    pub fn all_servers_initialized(&self) -> bool {
        self.server_init_status
            .values()
            .all(|status| matches!(status, InitStatus::Completed(_) | InitStatus::Failed { .. }))
    }

    /// Get list of successfully initialized servers
    pub fn get_initialized_servers(&self) -> Vec<String> {
        self.server_init_status
            .iter()
            .filter_map(|(server_id, status)| {
                if matches!(status, InitStatus::Completed(_)) {
                    Some(server_id.clone())
                } else {
                    None
                }
            })
            .collect()
    }

    /// Get list of failed servers
    pub fn get_failed_servers(&self) -> Vec<(String, String)> {
        self.server_init_status
            .iter()
            .filter_map(|(server_id, status)| {
                if let InitStatus::Failed { error, .. } = status {
                    Some((server_id.clone(), error.clone()))
                } else {
                    None
                }
            })
            .collect()
    }

    /// Subscribe to a resource
    ///
    /// # Arguments
    /// * `uri` - The resource URI to subscribe to
    /// * `server_id` - The server that owns this resource
    ///
    /// # Returns
    /// * `true` if this is a new subscription
    /// * `false` if already subscribed
    pub fn subscribe_resource(&mut self, uri: String, server_id: String) -> bool {
        use std::collections::hash_map::Entry;
        if let Entry::Vacant(e) = self.subscribed_resources.entry(uri) {
            e.insert(server_id);
            true
        } else {
            false
        }
    }

    /// Unsubscribe from a resource
    ///
    /// # Arguments
    /// * `uri` - The resource URI to unsubscribe from
    ///
    /// # Returns
    /// * `Some(server_id)` if was subscribed
    /// * `None` if was not subscribed
    pub fn unsubscribe_resource(&mut self, uri: &str) -> Option<String> {
        self.subscribed_resources.remove(uri)
    }

    /// Check if subscribed to a resource
    pub fn is_subscribed(&self, uri: &str) -> bool {
        self.subscribed_resources.contains_key(uri)
    }

    /// Get all subscribed resources for a specific server
    pub fn get_subscriptions_for_server(&self, server_id: &str) -> Vec<String> {
        self.subscribed_resources
            .iter()
            .filter(|(_, sid)| *sid == server_id)
            .map(|(uri, _)| uri.clone())
            .collect()
    }

    /// Get all subscribed resources
    pub fn get_all_subscriptions(&self) -> Vec<(String, String)> {
        self.subscribed_resources
            .iter()
            .map(|(uri, server_id)| (uri.clone(), server_id.clone()))
            .collect()
    }
}

fn failed_server_ids(failures: &[ServerFailure]) -> HashSet<&str> {
    failures.iter().map(|f| f.server_id.as_str()).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn test_session_creation() {
        let session = GatewaySession::new(
            "client-123".to_string(),
            vec!["filesystem".to_string(), "github".to_string()],
            Duration::from_secs(3600),
            300,
            Vec::new(),
        );

        assert_eq!(session.client_id, "client-123");
        assert_eq!(session.allowed_servers.len(), 2);
        assert_eq!(session.server_init_status.len(), 2);
        assert!(!session.is_expired());
    }

    #[test]
    fn test_session_expiration() {
        let mut session = GatewaySession::new(
            "client-123".to_string(),
            vec!["filesystem".to_string()],
            Duration::from_millis(100),
            300,
            Vec::new(),
        );

        assert!(!session.is_expired());

        // Wait for expiration
        std::thread::sleep(Duration::from_millis(150));
        assert!(session.is_expired());

        // Touch should reset expiration
        session.touch();
        assert!(!session.is_expired());
    }

    #[test]
    fn test_tool_mapping_update() {
        let mut session = GatewaySession::new(
            "client-123".to_string(),
            vec!["filesystem".to_string()],
            Duration::from_secs(3600),
            300,
            Vec::new(),
        );

        let tools = vec![NamespacedTool {
            name: "filesystem__read_file".to_string(),
            original_name: "read_file".to_string(),
            server_id: "filesystem".to_string(),
            description: Some("Read a file".to_string()),
            input_schema: serde_json::json!({}),
        }];

        session.update_tool_mappings(&tools, &[]);

        assert_eq!(session.tool_mapping.len(), 1);
        assert_eq!(
            session.tool_mapping.get("filesystem__read_file"),
            Some(&("filesystem".to_string(), "read_file".to_string()))
        );
    }

    #[test]
    fn test_cache_invalidation() {
        let mut session = GatewaySession::new(
            "client-123".to_string(),
            vec!["filesystem".to_string()],
            Duration::from_secs(3600),
            300,
            Vec::new(),
        );

        // Set caches
        session.cached_tools = Some(CachedList::new(vec![], Duration::from_secs(300)));
        session.cached_resources = Some(CachedList::new(vec![], Duration::from_secs(300)));

        assert!(session.cached_tools.is_some());
        assert!(session.cached_resources.is_some());

        // Invalidate
        session.invalidate_all_caches();

        assert!(session.cached_tools.is_none());
        assert!(session.cached_resources.is_none());
    }

    #[test]
    fn test_resource_subscription() {
        let mut session = GatewaySession::new(
            "client-123".to_string(),
            vec!["filesystem".to_string()],
            Duration::from_secs(3600),
            300,
            Vec::new(),
        );

        // Subscribe to a resource
        let is_new = session.subscribe_resource(
            "file:///home/user/config.json".to_string(),
            "filesystem".to_string(),
        );
        assert!(is_new);
        assert!(session.is_subscribed("file:///home/user/config.json"));

        // Subscribe again (should return false)
        let is_new = session.subscribe_resource(
            "file:///home/user/config.json".to_string(),
            "filesystem".to_string(),
        );
        assert!(!is_new);

        // Unsubscribe
        let server_id = session.unsubscribe_resource("file:///home/user/config.json");
        assert_eq!(server_id, Some("filesystem".to_string()));
        assert!(!session.is_subscribed("file:///home/user/config.json"));

        // Unsubscribe again (should return None)
        let server_id = session.unsubscribe_resource("file:///home/user/config.json");
        assert_eq!(server_id, None);
    }

    #[test]
    fn test_get_subscriptions_for_server() {
        let mut session = GatewaySession::new(
            "client-123".to_string(),
            vec!["filesystem".to_string(), "github".to_string()],
            Duration::from_secs(3600),
            300,
            Vec::new(),
        );

        // Subscribe to resources from different servers
        session.subscribe_resource("file:///config.json".to_string(), "filesystem".to_string());
        session.subscribe_resource("file:///data.json".to_string(), "filesystem".to_string());
        session.subscribe_resource("github://repo/file".to_string(), "github".to_string());

        // Get subscriptions for filesystem
        let fs_subs = session.get_subscriptions_for_server("filesystem");
        assert_eq!(fs_subs.len(), 2);
        assert!(fs_subs.contains(&"file:///config.json".to_string()));
        assert!(fs_subs.contains(&"file:///data.json".to_string()));

        // Get subscriptions for github
        let gh_subs = session.get_subscriptions_for_server("github");
        assert_eq!(gh_subs.len(), 1);
        assert!(gh_subs.contains(&"github://repo/file".to_string()));

        // Get all subscriptions
        let all_subs = session.get_all_subscriptions();
        assert_eq!(all_subs.len(), 3);
    }

    // ── permission-aware visibility & mapping stability ─────────────

    fn tool(server_id: &str, server_slug: &str, name: &str) -> NamespacedTool {
        NamespacedTool {
            name: format!("{server_slug}__{name}"),
            original_name: name.to_string(),
            server_id: server_id.to_string(),
            description: None,
            input_schema: serde_json::json!({}),
        }
    }

    fn session_with_perms(perms: lr_config::McpPermissions) -> GatewaySession {
        let mut session = GatewaySession::new(
            "client".to_string(),
            vec!["srv-a".to_string(), "srv-b".to_string()],
            Duration::from_secs(3600),
            300,
            Vec::new(),
        );
        session.mcp_permissions = perms;
        session
    }

    /// Install context-mode state for a client with the given feature flags.
    fn install_context_mode(
        session: &mut GatewaySession,
        responses: Option<bool>,
        catalog: Option<bool>,
        config: lr_config::ContextManagementConfig,
    ) {
        use super::super::virtual_server::VirtualMcpServer;
        let vs = super::super::context_mode::ContextModeVirtualServer::new(config);
        let mut client = lr_config::Client::new_with_strategy("c".to_string(), "s".to_string());
        client.context_management_enabled = responses;
        client.catalog_compression_enabled = catalog;
        session.virtual_server_state.insert(
            "_context_mode".to_string(),
            vs.create_session_state(&client),
        );
    }

    fn defer_server(slug: &str) -> CatalogCompressionPlan {
        CatalogCompressionPlan {
            indexed_welcomes: Vec::new(),
            deferred_servers: vec![DeferredServer {
                server_slug: slug.to_string(),
                batches: Vec::new(),
                definition_savings: 0,
            }],
            welcome_toc_dropped: Vec::new(),
            batch_toc_dropped: Vec::new(),
        }
    }

    fn names(tools: &[NamespacedTool]) -> Vec<&str> {
        tools.iter().map(|t| t.name.as_str()).collect()
    }

    #[test]
    fn requested_servers_are_sorted_and_survive_trimming() {
        let mut session = GatewaySession::new(
            "client".to_string(),
            vec!["b".to_string(), "a".to_string()],
            Duration::from_secs(3600),
            300,
            Vec::new(),
        );
        assert_eq!(session.requested_servers, vec!["a", "b"]);
        // A server failing to start narrows routing but not the requested set
        session.allowed_servers.retain(|s| s != "b");
        assert_eq!(session.requested_servers, vec!["a", "b"]);
    }

    #[test]
    fn visible_tools_applies_tool_overrides_over_global_allow() {
        let mut perms = lr_config::McpPermissions {
            global: lr_config::PermissionState::Allow,
            ..Default::default()
        };
        perms
            .tools
            .insert("srv-a__delete".to_string(), lr_config::PermissionState::Off);
        perms
            .servers
            .insert("srv-b".to_string(), lr_config::PermissionState::Off);
        let session = session_with_perms(perms);

        let tools = vec![
            tool("srv-a", "a", "read"),
            tool("srv-a", "a", "delete"),
            tool("srv-b", "b", "read"),
        ];
        assert_eq!(names(&session.visible_tools(&tools)), vec!["a__read"]);
    }

    #[test]
    fn visible_tools_tool_allow_under_server_off_shows_only_that_tool() {
        let mut perms = lr_config::McpPermissions::default(); // global Off
        perms
            .servers
            .insert("srv-a".to_string(), lr_config::PermissionState::Off);
        perms
            .tools
            .insert("srv-a__read".to_string(), lr_config::PermissionState::Ask);
        let session = session_with_perms(perms);

        let tools = vec![tool("srv-a", "a", "read"), tool("srv-a", "a", "write")];
        assert_eq!(names(&session.visible_tools(&tools)), vec!["a__read"]);
    }

    #[test]
    fn visible_resources_and_prompts_follow_permissions() {
        let mut perms = lr_config::McpPermissions {
            global: lr_config::PermissionState::Allow,
            ..Default::default()
        };
        perms.resources.insert(
            "srv-a__file:///secret".to_string(),
            lr_config::PermissionState::Off,
        );
        perms
            .prompts
            .insert("srv-a__hidden".to_string(), lr_config::PermissionState::Off);
        let session = session_with_perms(perms);

        let resource = |uri: &str, name: &str| NamespacedResource {
            name: format!("a__{name}"),
            original_name: name.to_string(),
            server_id: "srv-a".to_string(),
            uri: uri.to_string(),
            description: None,
            mime_type: None,
        };
        let resources = vec![
            resource("file:///secret", "secret"),
            resource("file:///open", "open"),
        ];
        let visible: Vec<String> = session
            .visible_resources(&resources)
            .into_iter()
            .map(|r| r.uri)
            .collect();
        assert_eq!(visible, vec!["file:///open"]);

        let prompt = |name: &str| NamespacedPrompt {
            name: format!("a__{name}"),
            original_name: name.to_string(),
            server_id: "srv-a".to_string(),
            description: None,
            arguments: None,
        };
        let visible: Vec<String> = session
            .visible_prompts(&[prompt("hidden"), prompt("shown")])
            .into_iter()
            .map(|p| p.name)
            .collect();
        assert_eq!(visible, vec!["a__shown"]);
    }

    #[test]
    fn catalog_plan_is_ignored_when_catalog_compression_is_off() {
        let mut session = session_with_perms(lr_config::McpPermissions {
            global: lr_config::PermissionState::Allow,
            ..Default::default()
        });
        session.catalog_compression = Some(defer_server("a"));
        let tools = vec![tool("srv-a", "a", "read"), tool("srv-b", "b", "read")];

        // Response indexing on, catalog compression off → nothing deferred
        install_context_mode(
            &mut session,
            Some(true),
            Some(false),
            lr_config::ContextManagementConfig::default(),
        );
        assert!(session.active_catalog_compression().is_none());
        assert_eq!(
            names(&session.visible_tools(&tools)),
            vec!["a__read", "b__read"]
        );

        // Catalog compression on → server a deferred
        install_context_mode(
            &mut session,
            Some(false),
            Some(true),
            lr_config::ContextManagementConfig::default(),
        );
        assert!(session.active_catalog_compression().is_some());
        assert_eq!(names(&session.visible_tools(&tools)), vec!["b__read"]);
    }

    #[test]
    fn deferral_keeps_tools_excluded_from_indexing_visible() {
        let mut session = session_with_perms(lr_config::McpPermissions {
            global: lr_config::PermissionState::Allow,
            ..Default::default()
        });
        session.catalog_compression = Some(defer_server("a"));
        let mut config = lr_config::ContextManagementConfig::default();
        config
            .gateway_indexing
            .tools
            .insert("a__secret".to_string(), lr_config::IndexingState::Disable);
        install_context_mode(&mut session, None, Some(true), config);

        let tools = vec![tool("srv-a", "a", "read"), tool("srv-a", "a", "secret")];
        // `read` is deferred (searchable); `secret` can't be searched, so it stays listed
        assert_eq!(names(&session.visible_tools(&tools)), vec!["a__secret"]);
    }

    #[test]
    fn tool_mappings_survive_a_failed_refresh_of_their_server() {
        let mut session = session_with_perms(lr_config::McpPermissions::default());
        session.update_tool_mappings(
            &[tool("srv-a", "a", "read"), tool("srv-b", "b", "read")],
            &[],
        );

        // srv-b times out on the next tools/list; srv-a drops a tool
        session.update_tool_mappings(
            &[],
            &[ServerFailure {
                server_id: "srv-b".to_string(),
                error: "timeout".to_string(),
            }],
        );
        assert!(session.tool_mapping.contains_key("b__read"));
        assert!(!session.tool_mapping.contains_key("a__read"));
    }
}
