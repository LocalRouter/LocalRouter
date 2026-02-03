//! Marketplace types - listings, install requests/responses

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use thiserror::Error;

/// Convert a human-readable name to a tokenized ID
/// e.g., "My Marketplace" -> "my-marketplace"
pub fn tokenize_name(name: &str) -> String {
    name.to_lowercase()
        .chars()
        .map(|c| if c.is_alphanumeric() { c } else { '-' })
        .collect::<String>()
        .split('-')
        .filter(|s| !s.is_empty())
        .collect::<Vec<_>>()
        .join("-")
}

/// The source ID for the official MCP registry
pub const MCP_REGISTRY_SOURCE_ID: &str = "mcp-registry";

/// Marketplace error types
#[derive(Debug, Error)]
pub enum MarketplaceError {
    #[error("Registry request failed: {0}")]
    RegistryError(String),

    #[error("Skill source request failed: {0}")]
    SkillSourceError(String),

    #[error("Install failed: {0}")]
    InstallError(String),

    #[error("Install cancelled by user")]
    InstallCancelled,

    #[error("Install timed out")]
    InstallTimeout,

    #[error("Invalid tool name: {0}")]
    InvalidToolName(String),

    #[error("Invalid arguments: {0}")]
    InvalidArguments(String),

    #[error("Network error: {0}")]
    NetworkError(String),

    #[error("Parse error: {0}")]
    ParseError(String),

    #[error("Internal error: {0}")]
    Internal(String),
}

impl From<reqwest::Error> for MarketplaceError {
    fn from(err: reqwest::Error) -> Self {
        MarketplaceError::NetworkError(err.to_string())
    }
}

impl From<serde_json::Error> for MarketplaceError {
    fn from(err: serde_json::Error) -> Self {
        MarketplaceError::ParseError(err.to_string())
    }
}

/// MCP server listing from the registry
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct McpServerListing {
    /// Package/server name
    pub name: String,

    /// Human-readable description
    pub description: String,

    /// Source ID (tokenized marketplace name, e.g., "mcp-registry")
    pub source_id: String,

    /// Homepage or repository URL
    #[serde(skip_serializing_if = "Option::is_none")]
    pub homepage: Option<String>,

    /// Vendor/author
    #[serde(skip_serializing_if = "Option::is_none")]
    pub vendor: Option<String>,

    /// Available packages (npm, pypi, etc.)
    #[serde(default)]
    pub packages: Vec<McpPackageInfo>,

    /// Hosted/remote options (direct URLs)
    #[serde(default)]
    pub remotes: Vec<McpRemoteInfo>,

    /// Available transports based on packages/remotes
    #[serde(default)]
    pub available_transports: Vec<String>,

    /// Install hint for the AI/user
    #[serde(skip_serializing_if = "Option::is_none")]
    pub install_hint: Option<String>,
}

/// Package info for an MCP server (npm, pypi, etc.)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct McpPackageInfo {
    /// Package manager (npm, pypi, cargo, etc.)
    pub registry: String,

    /// Package name in that registry
    pub name: String,

    /// Latest version
    #[serde(skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,

    /// Runtime (node, python, etc.)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub runtime: Option<String>,

    /// Default command to run
    #[serde(skip_serializing_if = "Option::is_none")]
    pub command: Option<String>,

    /// Default arguments
    #[serde(default)]
    pub args: Vec<String>,

    /// Required environment variables
    #[serde(default)]
    pub env: HashMap<String, String>,
}

/// Remote/hosted info for an MCP server
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct McpRemoteInfo {
    /// Transport type (sse, http, websocket)
    pub transport: String,

    /// Server URL
    pub url: String,

    /// Auth required (none, bearer, oauth)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub auth: Option<String>,

    /// OAuth endpoints (if auth=oauth)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub oauth: Option<McpRemoteOAuthInfo>,
}

/// OAuth info for a remote MCP server
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct McpRemoteOAuthInfo {
    /// Authorization URL
    pub authorization_url: String,

    /// Token URL
    pub token_url: String,

    /// Required scopes
    #[serde(default)]
    pub scopes: Vec<String>,
}

/// Skill listing from a GitHub source
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SkillListing {
    /// Skill name (directory name)
    pub name: String,

    /// Human-readable description (from SKILL.md frontmatter)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,

    /// Source ID (tokenized source label, e.g., "anthropic", "awesome-claude-skills")
    pub source_id: String,

    /// Author
    #[serde(skip_serializing_if = "Option::is_none")]
    pub author: Option<String>,

    /// Version
    #[serde(skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,

    /// Tags/categories
    #[serde(default)]
    pub tags: Vec<String>,

    /// Source label (e.g., "Anthropic", "Community")
    pub source_label: String,

    /// Source repository URL
    pub source_repo: String,

    /// Path within the repo
    pub source_path: String,

    /// Branch
    pub source_branch: String,

    /// Raw URL to download SKILL.md
    pub skill_md_url: String,

    /// Whether this is a multi-file skill (has scripts/, references/, etc.)
    pub is_multi_file: bool,

    /// List of files to download (for multi-file skills)
    #[serde(default)]
    pub files: Vec<SkillFileInfo>,
}

/// Info about a file in a skill
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SkillFileInfo {
    /// Relative path within the skill directory
    pub path: String,

    /// Raw download URL
    pub url: String,
}

/// Request to install an MCP server (sent to popup)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct McpInstallRequest {
    /// The listing being installed
    pub listing: McpServerListing,

    /// Client ID that requested the install
    pub client_id: String,

    /// Client name for display
    pub client_name: String,
}

/// User-provided config for MCP server installation
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct McpInstallConfig {
    /// Server name (user can customize)
    pub name: String,

    /// Transport type (stdio, http_sse, websocket)
    pub transport: String,

    /// For stdio: command to run
    #[serde(skip_serializing_if = "Option::is_none")]
    pub command: Option<String>,

    /// For stdio: command arguments
    #[serde(default)]
    pub args: Vec<String>,

    /// For http/websocket: server URL
    #[serde(skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,

    /// Environment variables
    #[serde(default)]
    pub env: HashMap<String, String>,

    /// Auth type (none, bearer, oauth)
    #[serde(default = "default_auth_none")]
    pub auth_type: String,

    /// Bearer token (if auth_type = bearer)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bearer_token: Option<String>,
}

fn default_auth_none() -> String {
    "none".to_string()
}

/// Request to install a skill (sent to popup)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SkillInstallRequest {
    /// The listing being installed
    pub listing: SkillListing,

    /// Client ID that requested the install
    pub client_id: String,

    /// Client name for display
    pub client_name: String,
}

/// Result of installing an MCP server
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InstalledServer {
    /// The server ID in config
    pub server_id: String,

    /// Server name
    pub name: String,
}

/// Result of installing a skill
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InstalledSkill {
    /// Skill name
    pub name: String,

    /// Path where skill was installed
    pub path: String,
}

/// Cache entry for MCP server listings
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct McpCacheEntry {
    /// Query that was cached
    pub query: String,
    /// Cached server listings
    pub servers: Vec<McpServerListing>,
    /// When the cache was created
    pub cached_at: DateTime<Utc>,
}

/// Cache entry for skill listings
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SkillsCacheEntry {
    /// Source repo URL
    pub source_url: String,
    /// Cached skill listings
    pub skills: Vec<SkillListing>,
    /// When the cache was created
    pub cached_at: DateTime<Utc>,
}

/// Marketplace cache stored on disk
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct MarketplaceCache {
    /// MCP server cache entries by query
    #[serde(default)]
    pub mcp_cache: HashMap<String, McpCacheEntry>,
    /// Skills cache entries by source URL
    #[serde(default)]
    pub skills_cache: HashMap<String, SkillsCacheEntry>,
    /// Last time MCP was refreshed (any query)
    #[serde(default)]
    pub mcp_last_refresh: Option<DateTime<Utc>>,
    /// Last time skills were refreshed
    #[serde(default)]
    pub skills_last_refresh: Option<DateTime<Utc>>,
}

/// Cache TTL in days
pub const CACHE_TTL_DAYS: i64 = 7;

/// Cache status information
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CacheStatus {
    /// Last time MCP servers were refreshed
    pub mcp_last_refresh: Option<DateTime<Utc>>,
    /// Last time skills were refreshed
    pub skills_last_refresh: Option<DateTime<Utc>>,
    /// Number of cached MCP queries
    pub mcp_cached_queries: usize,
    /// Number of cached skill sources
    pub skills_cached_sources: usize,
}

impl MarketplaceCache {
    /// Check if an MCP cache entry is still valid
    pub fn is_mcp_cache_valid(&self, query: &str) -> bool {
        if let Some(entry) = self.mcp_cache.get(query) {
            let age = Utc::now() - entry.cached_at;
            age.num_days() < CACHE_TTL_DAYS
        } else {
            false
        }
    }

    /// Check if a skills cache entry is still valid
    pub fn is_skills_cache_valid(&self, source_url: &str) -> bool {
        if let Some(entry) = self.skills_cache.get(source_url) {
            let age = Utc::now() - entry.cached_at;
            age.num_days() < CACHE_TTL_DAYS
        } else {
            false
        }
    }

    /// Get cached MCP servers for a query
    pub fn get_mcp_servers(&self, query: &str) -> Option<&Vec<McpServerListing>> {
        if self.is_mcp_cache_valid(query) {
            self.mcp_cache.get(query).map(|e| &e.servers)
        } else {
            None
        }
    }

    /// Get cached skills for a source
    pub fn get_skills(&self, source_url: &str) -> Option<&Vec<SkillListing>> {
        if self.is_skills_cache_valid(source_url) {
            self.skills_cache.get(source_url).map(|e| &e.skills)
        } else {
            None
        }
    }

    /// Cache MCP servers for a query
    pub fn cache_mcp_servers(&mut self, query: String, servers: Vec<McpServerListing>) {
        let now = Utc::now();
        self.mcp_cache.insert(
            query.clone(),
            McpCacheEntry {
                query,
                servers,
                cached_at: now,
            },
        );
        self.mcp_last_refresh = Some(now);
    }

    /// Cache skills for a source
    pub fn cache_skills(&mut self, source_url: String, skills: Vec<SkillListing>) {
        let now = Utc::now();
        self.skills_cache.insert(
            source_url.clone(),
            SkillsCacheEntry {
                source_url,
                skills,
                cached_at: now,
            },
        );
        self.skills_last_refresh = Some(now);
    }

    /// Clear MCP cache
    pub fn clear_mcp_cache(&mut self) {
        self.mcp_cache.clear();
        self.mcp_last_refresh = None;
    }

    /// Clear skills cache
    pub fn clear_skills_cache(&mut self) {
        self.skills_cache.clear();
        self.skills_last_refresh = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_mcp_server_listing_serialization() {
        let listing = McpServerListing {
            name: "test-server".to_string(),
            description: "A test server".to_string(),
            source_id: MCP_REGISTRY_SOURCE_ID.to_string(),
            homepage: Some("https://example.com".to_string()),
            vendor: Some("Test Vendor".to_string()),
            packages: vec![McpPackageInfo {
                registry: "npm".to_string(),
                name: "@test/server".to_string(),
                version: Some("1.0.0".to_string()),
                runtime: Some("node".to_string()),
                command: Some("npx".to_string()),
                args: vec!["-y".to_string(), "@test/server".to_string()],
                env: HashMap::new(),
            }],
            remotes: vec![],
            available_transports: vec!["stdio".to_string()],
            install_hint: None,
        };

        let json = serde_json::to_string(&listing).unwrap();
        let parsed: McpServerListing = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.name, "test-server");
        assert_eq!(parsed.source_id, MCP_REGISTRY_SOURCE_ID);
    }

    #[test]
    fn test_skill_listing_serialization() {
        let listing = SkillListing {
            name: "test-skill".to_string(),
            description: Some("A test skill".to_string()),
            source_id: "test".to_string(),
            author: Some("Test Author".to_string()),
            version: Some("1.0.0".to_string()),
            tags: vec!["test".to_string()],
            source_label: "Test".to_string(),
            source_repo: "https://github.com/test/skills".to_string(),
            source_path: "skills/test-skill".to_string(),
            source_branch: "main".to_string(),
            skill_md_url:
                "https://raw.githubusercontent.com/test/skills/main/skills/test-skill/SKILL.md"
                    .to_string(),
            is_multi_file: false,
            files: vec![],
        };

        let json = serde_json::to_string(&listing).unwrap();
        let parsed: SkillListing = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.name, "test-skill");
        assert_eq!(parsed.source_id, "test");
    }

    #[test]
    fn test_tokenize_name() {
        assert_eq!(tokenize_name("My Marketplace"), "my-marketplace");
        assert_eq!(tokenize_name("Anthropic"), "anthropic");
        assert_eq!(tokenize_name("Awesome Claude Skills"), "awesome-claude-skills");
        assert_eq!(tokenize_name("foo__bar  baz"), "foo-bar-baz");
        assert_eq!(tokenize_name("TEST-123"), "test-123");
    }
}
