#![allow(dead_code)]

use std::collections::{HashMap, HashSet};
use std::time::Instant;

use super::types::*;
use crate::protocol::Root;

/// Gateway session (one per client)
#[derive(Debug)]
pub struct GatewaySession {
    /// Client ID
    pub client_id: String,

    /// List of MCP servers this client is allowed to access
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

    /// Whether deferred loading is requested by client config
    pub deferred_loading_requested: bool,

    /// Deferred loading state (if enabled after capability check)
    pub deferred_loading: Option<DeferredLoadingState>,

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

    /// Skills access control for this client
    pub skills_access: lr_config::SkillsAccess,

    /// Skills that have had get_info called (enables per-skill run/read tools)
    pub skills_info_loaded: HashSet<String>,

    /// Whether async skill tools are enabled
    pub skills_async_enabled: bool,

    /// Human-readable client name (for firewall approval display)
    pub client_name: String,

    /// Firewall rules for this session (copied from client config at session creation)
    pub firewall_rules: lr_config::FirewallRules,

    /// Tools approved during this session via "Allow for Session" action
    pub firewall_session_approvals: HashSet<String>,
}

impl GatewaySession {
    /// Create a new session
    pub fn new(
        client_id: String,
        allowed_servers: Vec<String>,
        ttl: std::time::Duration,
        base_cache_ttl_seconds: u64,
        roots: Vec<Root>,
        deferred_loading_requested: bool,
    ) -> Self {
        let now = Instant::now();
        let mut server_init_status = HashMap::new();

        // Initialize all allowed servers as NotStarted
        for server_id in &allowed_servers {
            server_init_status.insert(server_id.clone(), InitStatus::NotStarted);
        }

        Self {
            client_id,
            allowed_servers,
            server_init_status,
            merged_capabilities: None,
            client_capabilities: None,
            tool_mapping: HashMap::new(),
            resource_mapping: HashMap::new(),
            resource_uri_mapping: HashMap::new(),
            prompt_mapping: HashMap::new(),
            deferred_loading_requested,
            deferred_loading: None,
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
            skills_access: lr_config::SkillsAccess::None,
            skills_info_loaded: HashSet::new(),
            skills_async_enabled: false,
            client_name: String::new(),
            firewall_rules: lr_config::FirewallRules::default(),
            firewall_session_approvals: HashSet::new(),
        }
    }

    /// Check if session is expired
    pub fn is_expired(&self) -> bool {
        self.last_activity.elapsed() > self.ttl
    }

    /// Update last activity timestamp
    pub fn touch(&mut self) {
        self.last_activity = Instant::now();
    }

    /// Update tool mappings from a list of namespaced tools
    pub fn update_tool_mappings(&mut self, tools: &[NamespacedTool]) {
        self.tool_mapping.clear();
        for tool in tools {
            self.tool_mapping.insert(
                tool.name.clone(),
                (tool.server_id.clone(), tool.original_name.clone()),
            );
        }
    }

    /// Update resource mappings from a list of namespaced resources
    pub fn update_resource_mappings(&mut self, resources: &[NamespacedResource]) {
        self.resource_mapping.clear();
        self.resource_uri_mapping.clear();
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

    /// Update prompt mappings from a list of namespaced prompts
    pub fn update_prompt_mappings(&mut self, prompts: &[NamespacedPrompt]) {
        self.prompt_mapping.clear();
        for prompt in prompts {
            self.prompt_mapping.insert(
                prompt.name.clone(),
                (prompt.server_id.clone(), prompt.original_name.clone()),
            );
        }
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

    /// Mark a skill as having had get_info called
    pub fn mark_skill_info_loaded(&mut self, name: &str) {
        self.skills_info_loaded.insert(name.to_string());
    }

    /// Check if a skill has had get_info called
    pub fn is_skill_info_loaded(&self, name: &str) -> bool {
        self.skills_info_loaded.contains(name)
    }

    /// Get all subscribed resources
    pub fn get_all_subscriptions(&self) -> Vec<(String, String)> {
        self.subscribed_resources
            .iter()
            .map(|(uri, server_id)| (uri.clone(), server_id.clone()))
            .collect()
    }
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
            false,
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
            false,
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
            false,
        );

        let tools = vec![NamespacedTool {
            name: "filesystem__read_file".to_string(),
            original_name: "read_file".to_string(),
            server_id: "filesystem".to_string(),
            description: Some("Read a file".to_string()),
            input_schema: serde_json::json!({}),
        }];

        session.update_tool_mappings(&tools);

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
            false,
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
            false,
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
            false,
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
}
