#![allow(dead_code)]

use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashSet;
use std::sync::Arc;
use std::time::Instant;

// AppError not directly used in this file

/// Namespace separator for MCP gateway (double underscore is MCP spec compliant)
pub const NAMESPACE_SEPARATOR: &str = "__";

/// Dynamic cache TTL manager
///
/// Tracks cache invalidation frequency and adjusts TTL accordingly:
/// - Low invalidation rate (< 5 per hour) → Keep 5 minute TTL
/// - Medium rate (5-20 per hour) → Reduce to 2 minute TTL
/// - High rate (> 20 per hour) → Reduce to 1 minute TTL
#[derive(Debug)]
pub struct DynamicCacheTTL {
    /// Base TTL in seconds (configured value)
    base_ttl_seconds: u64,

    /// Invalidation counter (atomic for thread safety)
    invalidation_count: std::sync::atomic::AtomicU32,

    /// Last reset time for invalidation counter
    last_reset: Arc<parking_lot::RwLock<Instant>>,
}

impl DynamicCacheTTL {
    pub fn new(base_ttl_seconds: u64) -> Self {
        Self {
            base_ttl_seconds,
            invalidation_count: std::sync::atomic::AtomicU32::new(0),
            last_reset: Arc::new(parking_lot::RwLock::new(Instant::now())),
        }
    }

    /// Record a cache invalidation event
    pub fn record_invalidation(&self) {
        self.invalidation_count
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    }

    /// Get the current TTL based on invalidation frequency
    pub fn get_ttl(&self) -> std::time::Duration {
        // Reset counter every hour
        let now = Instant::now();

        // Check if reset is needed (read lock only)
        let needs_reset = {
            let last_reset = self.last_reset.read();
            now.duration_since(*last_reset) >= std::time::Duration::from_secs(3600)
        };

        if needs_reset {
            // Try to acquire write lock to reset
            // Only one thread will successfully reset, others will skip
            if let Some(mut last_reset) = self.last_reset.try_write() {
                // Double-check elapsed time after acquiring write lock
                if now.duration_since(*last_reset) >= std::time::Duration::from_secs(3600) {
                    // Reset counter after an hour
                    self.invalidation_count
                        .store(0, std::sync::atomic::Ordering::Relaxed);
                    *last_reset = now;
                }
            }
            // If we couldn't get the lock, another thread is resetting - that's fine
        }

        // Calculate TTL based on invalidation frequency
        let invalidations = self
            .invalidation_count
            .load(std::sync::atomic::Ordering::Relaxed);

        if invalidations > 20 {
            // High invalidation rate - use short TTL
            std::time::Duration::from_secs(60) // 1 minute
        } else if invalidations > 5 {
            // Medium invalidation rate - use moderate TTL
            std::time::Duration::from_secs(120) // 2 minutes
        } else {
            // Low invalidation rate - use base TTL
            std::time::Duration::from_secs(self.base_ttl_seconds)
        }
    }
}

impl Clone for DynamicCacheTTL {
    fn clone(&self) -> Self {
        Self {
            base_ttl_seconds: self.base_ttl_seconds,
            invalidation_count: std::sync::atomic::AtomicU32::new(
                self.invalidation_count
                    .load(std::sync::atomic::Ordering::Relaxed),
            ),
            last_reset: self.last_reset.clone(),
        }
    }
}

/// Gateway configuration
#[derive(Debug, Clone)]
pub struct GatewayConfig {
    /// Session TTL in seconds (default: 3600 = 1 hour)
    pub session_ttl_seconds: u64,

    /// Server timeout in seconds (default: 10)
    pub server_timeout_seconds: u64,

    /// Allow partial failures (continue with working servers)
    pub allow_partial_failures: bool,

    /// Cache TTL in seconds (default: 300 = 5 minutes)
    pub cache_ttl_seconds: u64,

    /// Max retry attempts for failed requests (default: 1)
    pub max_retry_attempts: u8,

    /// Whether async skill script execution is enabled (default: false)
    pub skills_async_enabled: bool,
}

impl Default for GatewayConfig {
    fn default() -> Self {
        Self {
            session_ttl_seconds: 3600,
            server_timeout_seconds: 10,
            allow_partial_failures: true,
            cache_ttl_seconds: 300,
            max_retry_attempts: 1,
            skills_async_enabled: false,
        }
    }
}

/// Slugify a name for use as a namespace prefix and XML tag.
///
/// Produces a consistent kebab-case identifier from a human-readable name.
/// Used for both tool namespace prefixes (e.g., `everything-mcp-server__echo`)
/// and XML instruction tags (e.g., `<everything-mcp-server>`).
///
/// Examples:
/// - "My MCP Server" → "my-mcp-server"
/// - "filesystem" → "filesystem"
/// - "GitHub  API" → "github-api"
pub fn slugify(name: &str) -> String {
    let mut slug = String::with_capacity(name.len());
    let mut last_was_separator = true; // avoid leading dash
    for ch in name.chars() {
        if ch.is_ascii_alphanumeric() {
            slug.push(ch.to_ascii_lowercase());
            last_was_separator = false;
        } else if !last_was_separator {
            slug.push('-');
            last_was_separator = true;
        }
    }
    // trim trailing dash
    if slug.ends_with('-') {
        slug.pop();
    }
    slug
}

/// Parse namespaced name into (server_id, original_name)
/// Example: "filesystem__read_file" -> ("filesystem", "read_file")
pub fn parse_namespace(namespaced: &str) -> Option<(String, String)> {
    let idx = namespaced.find(NAMESPACE_SEPARATOR)?;
    let server_id = &namespaced[..idx];
    let original_name = &namespaced[idx + NAMESPACE_SEPARATOR.len()..];

    if server_id.is_empty() || original_name.is_empty() {
        return None;
    }

    Some((server_id.to_string(), original_name.to_string()))
}

/// Apply namespace to name
/// Example: ("filesystem", "read_file") -> "filesystem__read_file"
pub fn apply_namespace(server_id: &str, name: &str) -> String {
    format!("{}{}{}", server_id, NAMESPACE_SEPARATOR, name)
}

/// Namespaced tool with metadata
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct NamespacedTool {
    /// Namespaced name: "filesystem__read_file"
    pub name: String,

    /// Original name without namespace: "read_file"
    #[serde(skip_serializing)]
    pub original_name: String,

    /// Server ID: "filesystem"
    #[serde(skip_serializing)]
    pub server_id: String,

    /// Description (unchanged from backend)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,

    /// Input schema
    #[serde(rename = "inputSchema")]
    pub input_schema: Value,
}

/// Namespaced resource with metadata
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct NamespacedResource {
    /// Namespaced name: "filesystem__config"
    pub name: String,

    /// Original name without namespace: "config"
    #[serde(skip_serializing)]
    pub original_name: String,

    /// Server ID: "filesystem"
    #[serde(skip_serializing)]
    pub server_id: String,

    /// Resource URI (unchanged)
    pub uri: String,

    /// Description (unchanged from backend)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,

    /// MIME type
    #[serde(rename = "mimeType", skip_serializing_if = "Option::is_none")]
    pub mime_type: Option<String>,
}

/// Namespaced prompt with metadata
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct NamespacedPrompt {
    /// Namespaced name: "github__pr_template"
    pub name: String,

    /// Original name without namespace: "pr_template"
    #[serde(skip_serializing)]
    pub original_name: String,

    /// Server ID: "github"
    #[serde(skip_serializing)]
    pub server_id: String,

    /// Description (unchanged from backend)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,

    /// Arguments schema
    #[serde(skip_serializing_if = "Option::is_none")]
    pub arguments: Option<Vec<PromptArgument>>,
}

/// Prompt argument definition
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PromptArgument {
    pub name: String,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub required: Option<bool>,
}

/// Cached list with TTL
#[derive(Debug, Clone)]
pub struct CachedList<T> {
    pub data: Vec<T>,
    pub cached_at: Instant,
    pub ttl: std::time::Duration,
}

impl<T> CachedList<T> {
    pub fn new(data: Vec<T>, ttl: std::time::Duration) -> Self {
        Self {
            data,
            cached_at: Instant::now(),
            ttl,
        }
    }

    pub fn is_valid(&self) -> bool {
        self.cached_at.elapsed() < self.ttl
    }
}

/// Initialization status for a server
#[derive(Debug, Clone)]
pub enum InitStatus {
    NotStarted,
    InProgress,
    Completed(InitializeResult),
    Failed { error: String, retry_count: u8 },
}

/// Initialization result from a server
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InitializeResult {
    #[serde(rename = "protocolVersion")]
    pub protocol_version: String,

    pub capabilities: ServerCapabilities,

    #[serde(rename = "serverInfo")]
    pub server_info: ServerInfo,

    /// Optional instructions describing how to use this server
    #[serde(skip_serializing_if = "Option::is_none")]
    pub instructions: Option<String>,
}

/// Client capabilities (sent by MCP client during initialization)
/// Mirrors ServerCapabilities structure - declares what the client supports receiving
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ClientCapabilities {
    /// Tools capability - client can receive notifications/tools/list_changed
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tools: Option<ToolsCapability>,

    /// Resources capability - client can receive notifications/resources/list_changed
    #[serde(skip_serializing_if = "Option::is_none")]
    pub resources: Option<ResourcesCapability>,

    /// Prompts capability - client can receive notifications/prompts/list_changed
    #[serde(skip_serializing_if = "Option::is_none")]
    pub prompts: Option<PromptsCapability>,

    /// Roots capability - client can receive roots/list_changed notifications
    #[serde(skip_serializing_if = "Option::is_none")]
    pub roots: Option<ClientRootsCapability>,

    /// Sampling capability - client supports sampling/createMessage requests from server
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sampling: Option<serde_json::Value>,

    /// Elicitation capability - client supports elicitation requests from server
    /// Can include "form" for form-based elicitation
    #[serde(skip_serializing_if = "Option::is_none")]
    pub elicitation: Option<ClientElicitationCapability>,

    /// Experimental capabilities
    #[serde(skip_serializing_if = "Option::is_none")]
    pub experimental: Option<serde_json::Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClientRootsCapability {
    #[serde(rename = "listChanged", skip_serializing_if = "Option::is_none")]
    pub list_changed: Option<bool>,
}

/// Client elicitation capability
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClientElicitationCapability {
    /// Form-based elicitation support
    #[serde(skip_serializing_if = "Option::is_none")]
    pub form: Option<serde_json::Value>,
}

impl ClientCapabilities {
    /// Check if client explicitly declared support for notifications/tools/list_changed
    /// If not explicitly declared, defaults to false for safety.
    /// Deferred loading REQUIRES this capability.
    pub fn supports_tools_list_changed(&self) -> bool {
        self.tools
            .as_ref()
            .and_then(|t| t.list_changed)
            .unwrap_or(false)
    }

    /// Check if client explicitly declared support for notifications/resources/list_changed
    pub fn supports_resources_list_changed(&self) -> bool {
        self.resources
            .as_ref()
            .and_then(|r| r.list_changed)
            .unwrap_or(false)
    }

    /// Check if client explicitly declared support for notifications/prompts/list_changed
    pub fn supports_prompts_list_changed(&self) -> bool {
        self.prompts
            .as_ref()
            .and_then(|p| p.list_changed)
            .unwrap_or(false)
    }

    /// Check if client declared support for sampling/createMessage requests
    pub fn supports_sampling(&self) -> bool {
        self.sampling.is_some()
    }

    /// Check if client declared support for elicitation (any form)
    pub fn supports_elicitation(&self) -> bool {
        self.elicitation.is_some()
    }

    /// Check if client declared support for form-based elicitation
    pub fn supports_elicitation_form(&self) -> bool {
        self.elicitation
            .as_ref()
            .map(|e| e.form.is_some())
            .unwrap_or(false)
    }
}

/// Server capabilities
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ServerCapabilities {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tools: Option<ToolsCapability>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub resources: Option<ResourcesCapability>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub prompts: Option<PromptsCapability>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub logging: Option<LoggingCapability>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolsCapability {
    #[serde(rename = "listChanged", skip_serializing_if = "Option::is_none")]
    pub list_changed: Option<bool>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResourcesCapability {
    #[serde(rename = "listChanged", skip_serializing_if = "Option::is_none")]
    pub list_changed: Option<bool>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub subscribe: Option<bool>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PromptsCapability {
    #[serde(rename = "listChanged", skip_serializing_if = "Option::is_none")]
    pub list_changed: Option<bool>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LoggingCapability {}

/// Server info
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ServerInfo {
    pub name: String,
    pub version: String,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
}

/// Deferred loading state
#[derive(Debug, Clone)]
pub struct DeferredLoadingState {
    /// Whether deferred loading is enabled (tools are always deferred when this is true)
    pub enabled: bool,

    /// Whether resources are deferred (requires client resources.listChanged capability)
    pub resources_deferred: bool,

    /// Whether prompts are deferred (requires client prompts.listChanged capability)
    pub prompts_deferred: bool,

    /// Activated tools (persist for session lifetime)
    pub activated_tools: HashSet<String>,

    /// Full catalog of all available tools
    pub full_catalog: Vec<NamespacedTool>,

    /// Activated resources
    pub activated_resources: HashSet<String>,

    /// Activated prompts
    pub activated_prompts: HashSet<String>,

    /// Full resource catalog
    pub full_resource_catalog: Vec<NamespacedResource>,

    /// Full prompt catalog
    pub full_prompt_catalog: Vec<NamespacedPrompt>,
}

/// Server failure info for partial failure handling
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ServerFailure {
    pub server_id: String,
    pub error: String,
}

/// Merged capabilities from multiple servers
#[derive(Debug, Clone)]
pub struct MergedCapabilities {
    pub protocol_version: String,
    pub capabilities: ServerCapabilities,
    pub server_info: ServerInfo,
    pub failures: Vec<ServerFailure>,
    /// Instructions for the LLM on how to use this gateway
    pub instructions: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_namespace() {
        assert_eq!(
            parse_namespace("filesystem__read_file"),
            Some(("filesystem".to_string(), "read_file".to_string()))
        );

        assert_eq!(
            parse_namespace("github__create_issue"),
            Some(("github".to_string(), "create_issue".to_string()))
        );

        // Edge cases
        assert_eq!(parse_namespace("no_separator"), None);
        assert_eq!(parse_namespace("__no_server"), None);
        assert_eq!(parse_namespace("no_tool__"), None);
    }

    #[test]
    fn test_apply_namespace() {
        assert_eq!(
            apply_namespace("filesystem", "read_file"),
            "filesystem__read_file"
        );

        assert_eq!(
            apply_namespace("github", "create_issue"),
            "github__create_issue"
        );
    }

    #[test]
    fn test_roundtrip() {
        let original_server = "filesystem";
        let original_tool = "read_file";
        let namespaced = apply_namespace(original_server, original_tool);
        let (server, tool) = parse_namespace(&namespaced).unwrap();

        assert_eq!(server, original_server);
        assert_eq!(tool, original_tool);
    }

    #[test]
    fn test_slugify() {
        assert_eq!(slugify("My MCP Server"), "my-mcp-server");
        assert_eq!(slugify("filesystem"), "filesystem");
        assert_eq!(slugify("GitHub  API"), "github-api");
        assert_eq!(slugify("  leading-trailing  "), "leading-trailing");
        assert_eq!(slugify("CamelCase"), "camelcase");
        // Idempotent: slugifying an already-slugified name produces the same result
        assert_eq!(slugify("my-mcp-server"), "my-mcp-server");
    }

    #[test]
    fn test_slugify_used_for_namespace() {
        // Demonstrates that slugify + apply_namespace produces consistent tool names
        let server_name = "Everything MCP Server";
        let slug = slugify(server_name);
        assert_eq!(slug, "everything-mcp-server");
        assert_eq!(
            apply_namespace(&slug, "echo"),
            "everything-mcp-server__echo"
        );
        // And parsing roundtrips correctly
        let (server, tool) = parse_namespace("everything-mcp-server__echo").unwrap();
        assert_eq!(server, "everything-mcp-server");
        assert_eq!(tool, "echo");
    }
}
