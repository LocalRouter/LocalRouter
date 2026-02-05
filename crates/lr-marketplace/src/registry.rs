//! MCP Registry client
//!
//! Queries the official MCP server registry at registry.modelcontextprotocol.io

use crate::types::{
    MarketplaceError, McpPackageInfo, McpRemoteInfo, McpServerListing, MCP_REGISTRY_SOURCE_ID,
};
use parking_lot::RwLock;
use serde::Deserialize;
use std::collections::HashMap;
use std::sync::Arc;
use tracing::{debug, warn};

/// Default registry URL
pub const DEFAULT_REGISTRY_URL: &str = "https://registry.modelcontextprotocol.io/v0.1/servers";

/// MCP Registry API client
pub struct McpRegistryClient {
    http_client: reqwest::Client,
    registry_url: Arc<RwLock<String>>,
}

impl McpRegistryClient {
    /// Create a new registry client
    pub fn new(registry_url: String) -> Self {
        Self {
            http_client: reqwest::Client::builder()
                .timeout(std::time::Duration::from_secs(30))
                .user_agent("LocalRouter/1.0")
                .build()
                .expect("Failed to create HTTP client"),
            registry_url: Arc::new(RwLock::new(registry_url)),
        }
    }

    /// Update the registry URL
    pub fn set_registry_url(&self, url: String) {
        *self.registry_url.write() = url;
    }

    /// Search for MCP servers
    pub async fn search(
        &self,
        query: &str,
        limit: Option<u32>,
    ) -> Result<Vec<McpServerListing>, MarketplaceError> {
        let registry_url = self.registry_url.read().clone();
        let limit = limit.unwrap_or(10).min(50);

        let url = format!(
            "{}?search={}&limit={}&version=latest",
            registry_url,
            urlencoding::encode(query),
            limit
        );

        debug!("Searching MCP registry: {}", url);

        let response = self
            .http_client
            .get(&url)
            .send()
            .await
            .map_err(|e| MarketplaceError::RegistryError(format!("Request failed: {}", e)))?;

        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            return Err(MarketplaceError::RegistryError(format!(
                "Registry returned {}: {}",
                status, body
            )));
        }

        let registry_response: RegistryResponse = response.json().await.map_err(|e| {
            MarketplaceError::ParseError(format!("Failed to parse response: {}", e))
        })?;

        // Convert registry response to our listing format
        let listings = registry_response
            .servers
            .into_iter()
            .map(|entry| convert_registry_server(entry.server))
            .collect();

        Ok(listings)
    }
}

impl Clone for McpRegistryClient {
    fn clone(&self) -> Self {
        Self {
            http_client: self.http_client.clone(),
            registry_url: self.registry_url.clone(),
        }
    }
}

impl Default for McpRegistryClient {
    fn default() -> Self {
        Self::new(DEFAULT_REGISTRY_URL.to_string())
    }
}

/// Registry API response format
#[derive(Debug, Deserialize)]
struct RegistryResponse {
    #[serde(default)]
    servers: Vec<RegistryServerEntry>,
}

/// Server entry wrapper (contains nested server object)
#[derive(Debug, Deserialize)]
struct RegistryServerEntry {
    server: RegistryServer,
}

/// Server entry from the registry
#[derive(Debug, Deserialize)]
struct RegistryServer {
    /// Server/package name
    name: String,

    /// Description
    #[serde(default)]
    description: String,

    /// Version
    #[serde(default)]
    #[allow(dead_code)]
    version: Option<String>,

    /// Vendor/author
    #[serde(default)]
    vendor: Option<String>,

    /// Homepage URL
    #[serde(default)]
    homepage: Option<String>,

    /// Repository info (object with url field)
    #[serde(default)]
    repository: Option<RegistryRepository>,

    /// Available packages (npm, pypi, etc.)
    #[serde(default)]
    packages: Vec<RegistryPackage>,

    /// Hosted/remote endpoints
    #[serde(default)]
    remotes: Vec<RegistryRemote>,
}

/// Repository info from registry
#[derive(Debug, Deserialize)]
struct RegistryRepository {
    /// Repository URL
    #[serde(default)]
    url: Option<String>,
}

/// Package info from registry
#[derive(Debug, Deserialize)]
struct RegistryPackage {
    /// Package registry type (npm, pypi, oci, etc.)
    #[serde(rename = "registryType")]
    registry_type: String,

    /// Package identifier/name
    identifier: String,

    /// Version
    #[serde(default)]
    version: Option<String>,

    /// Transport configuration
    #[serde(default)]
    transport: Option<RegistryTransport>,

    /// Runtime (node, python, etc.) - may be in transport
    #[serde(default)]
    runtime: Option<String>,

    /// Environment variables
    #[serde(default)]
    env: HashMap<String, String>,
}

/// Transport configuration within a package
#[derive(Debug, Deserialize)]
struct RegistryTransport {
    /// Transport type (stdio, sse, etc.)
    #[serde(rename = "type")]
    transport_type: String,

    /// Args for the command
    #[serde(default)]
    args: Vec<String>,
}

/// Remote/hosted endpoint from registry
#[derive(Debug, Deserialize)]
struct RegistryRemote {
    /// Transport type
    #[serde(rename = "type")]
    transport: String,

    /// URL
    url: String,

    /// Auth type
    #[serde(default)]
    auth: Option<String>,
}

/// Convert registry server to our listing format
fn convert_registry_server(server: RegistryServer) -> McpServerListing {
    let mut available_transports = Vec::new();

    // Convert packages
    let packages: Vec<McpPackageInfo> = server
        .packages
        .into_iter()
        .filter(|p| {
            // Filter out non-stdio packages like OCI for now
            p.transport
                .as_ref()
                .map(|t| t.transport_type == "stdio")
                .unwrap_or(true)
        })
        .map(|p| {
            // Get args from transport if available
            let transport_args = p
                .transport
                .as_ref()
                .map(|t| t.args.clone())
                .unwrap_or_default();

            // Generate default command based on registry type
            let (command, args) =
                generate_default_command(&p.registry_type, p.runtime.as_deref(), &p.identifier);

            // Merge transport args if we have them
            let final_args = if !transport_args.is_empty() {
                transport_args
            } else {
                args
            };

            McpPackageInfo {
                registry: p.registry_type,
                name: p.identifier,
                version: p.version,
                runtime: p.runtime,
                command,
                args: final_args,
                env: p.env,
            }
        })
        .collect();

    if !packages.is_empty() {
        available_transports.push("stdio".to_string());
    }

    // Convert remotes
    let remotes: Vec<McpRemoteInfo> = server
        .remotes
        .into_iter()
        .map(|r| {
            if !available_transports.contains(&r.transport) {
                available_transports.push(r.transport.clone());
            }

            McpRemoteInfo {
                transport: r.transport,
                url: r.url,
                auth: r.auth,
                oauth: None,
            }
        })
        .collect();

    // Generate install hint
    let install_hint = generate_install_hint(&packages, &remotes);

    // Extract repository URL from the nested object
    let repo_url = server.repository.and_then(|r| r.url);

    McpServerListing {
        name: server.name,
        description: server.description,
        source_id: MCP_REGISTRY_SOURCE_ID.to_string(),
        homepage: server.homepage.or(repo_url),
        vendor: server.vendor,
        packages,
        remotes,
        available_transports,
        install_hint,
    }
}

/// Generate default command based on registry/runtime
fn generate_default_command(
    registry: &str,
    runtime: Option<&str>,
    package_name: &str,
) -> (Option<String>, Vec<String>) {
    match (registry, runtime) {
        ("npm", _) | (_, Some("node")) => (
            Some("npx".to_string()),
            vec!["-y".to_string(), package_name.to_string()],
        ),
        ("pypi", _) | (_, Some("python")) => {
            (Some("uvx".to_string()), vec![package_name.to_string()])
        }
        _ => {
            warn!(
                "Unknown registry/runtime combination: {}/{}",
                registry,
                runtime.unwrap_or("unknown")
            );
            (None, vec![])
        }
    }
}

/// Generate an install hint for the AI
fn generate_install_hint(packages: &[McpPackageInfo], remotes: &[McpRemoteInfo]) -> Option<String> {
    if !packages.is_empty() {
        let pkg = &packages[0];
        if let Some(ref cmd) = pkg.command {
            let args = pkg.args.join(" ");
            return Some(format!("For stdio transport, run: {} {}", cmd, args));
        }
    }

    if !remotes.is_empty() {
        let remote = &remotes[0];
        return Some(format!(
            "For {} transport, connect to: {}",
            remote.transport, remote.url
        ));
    }

    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_generate_default_command_npm() {
        let (cmd, args) = generate_default_command("npm", None, "@anthropic/mcp-server");
        assert_eq!(cmd, Some("npx".to_string()));
        assert_eq!(args, vec!["-y", "@anthropic/mcp-server"]);
    }

    #[test]
    fn test_generate_default_command_pypi() {
        let (cmd, args) = generate_default_command("pypi", None, "mcp-server-sqlite");
        assert_eq!(cmd, Some("uvx".to_string()));
        assert_eq!(args, vec!["mcp-server-sqlite"]);
    }

    #[test]
    fn test_convert_registry_server() {
        let server = RegistryServer {
            name: "test-server".to_string(),
            description: "A test server".to_string(),
            version: Some("1.0.0".to_string()),
            vendor: Some("Test".to_string()),
            homepage: Some("https://example.com".to_string()),
            repository: None,
            packages: vec![RegistryPackage {
                registry_type: "npm".to_string(),
                identifier: "@test/server".to_string(),
                version: Some("1.0.0".to_string()),
                transport: Some(RegistryTransport {
                    transport_type: "stdio".to_string(),
                    args: vec![],
                }),
                runtime: Some("node".to_string()),
                env: HashMap::new(),
            }],
            remotes: vec![],
        };

        let listing = convert_registry_server(server);
        assert_eq!(listing.name, "test-server");
        assert_eq!(listing.source_id, MCP_REGISTRY_SOURCE_ID);
        assert_eq!(listing.packages.len(), 1);
        assert!(listing.available_transports.contains(&"stdio".to_string()));
    }
}
