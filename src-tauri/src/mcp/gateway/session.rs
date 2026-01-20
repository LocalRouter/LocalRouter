use std::collections::HashMap;
use std::time::Instant;

use super::types::*;

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

    /// Tool name mapping: namespaced_name -> (server_id, original_name)
    pub tool_mapping: HashMap<String, (String, String)>,

    /// Resource name mapping: namespaced_name -> (server_id, original_name)
    pub resource_mapping: HashMap<String, (String, String)>,

    /// Prompt name mapping: namespaced_name -> (server_id, original_name)
    pub prompt_mapping: HashMap<String, (String, String)>,

    /// Deferred loading state (if enabled)
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
}

impl GatewaySession {
    /// Create a new session
    pub fn new(client_id: String, allowed_servers: Vec<String>, ttl: std::time::Duration) -> Self {
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
            tool_mapping: HashMap::new(),
            resource_mapping: HashMap::new(),
            prompt_mapping: HashMap::new(),
            deferred_loading: None,
            cached_tools: None,
            cached_resources: None,
            cached_prompts: None,
            created_at: now,
            last_activity: now,
            ttl,
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
        for resource in resources {
            self.resource_mapping.insert(
                resource.name.clone(),
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
}
