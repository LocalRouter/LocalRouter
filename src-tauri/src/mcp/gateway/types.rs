use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashSet;
use std::time::Instant;

// AppError not directly used in this file

/// Namespace separator for MCP gateway (double underscore is MCP spec compliant)
pub const NAMESPACE_SEPARATOR: &str = "__";

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
}

impl Default for GatewayConfig {
    fn default() -> Self {
        Self {
            session_ttl_seconds: 3600,
            server_timeout_seconds: 10,
            allow_partial_failures: true,
            cache_ttl_seconds: 300,
            max_retry_attempts: 1,
        }
    }
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
    /// Whether deferred loading is enabled
    pub enabled: bool,

    /// Activated tools (persist for session lifetime)
    pub activated_tools: HashSet<String>,

    /// Full catalog of all available tools
    pub full_catalog: Vec<NamespacedTool>,

    /// Activated resources (optional)
    pub activated_resources: HashSet<String>,

    /// Activated prompts (optional)
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
}
