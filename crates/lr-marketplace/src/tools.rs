//! Marketplace tool definitions and handler
//!
//! Defines the 4 marketplace tools and handles tool calls.

use crate::{MarketplaceError, MarketplaceService, TOOL_PREFIX};
use serde_json::{json, Value};
use tracing::{debug, info};

/// Tool names (without prefix)
pub const SEARCH_MCP_SERVERS: &str = "search_mcp_servers";
pub const INSTALL_MCP_SERVER: &str = "install_mcp_server";
pub const SEARCH_SKILLS: &str = "search_skills";
pub const INSTALL_SKILL: &str = "install_skill";

/// List all marketplace tools as JSON tool definitions
pub fn list_tools() -> Vec<Value> {
    vec![
        json!({
            "name": format!("{}{}", TOOL_PREFIX, SEARCH_MCP_SERVERS),
            "description": "Search the MCP server registry for available servers. Returns a list of servers matching the query with their descriptions and installation options.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "query": {
                        "type": "string",
                        "description": "Search query to find MCP servers (e.g., 'filesystem', 'database', 'github')"
                    },
                    "limit": {
                        "type": "integer",
                        "description": "Maximum number of results to return (default: 10, max: 50)",
                        "minimum": 1,
                        "maximum": 50
                    }
                },
                "required": ["query"]
            }
        }),
        json!({
            "name": format!("{}{}", TOOL_PREFIX, INSTALL_MCP_SERVER),
            "description": "Install an MCP server from the registry. This will prompt the user to confirm and configure the installation. After installation, the server will be available for use. Use the name and source from search results.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "name": {
                        "type": "string",
                        "description": "Name of the MCP server to install (from search results)"
                    },
                    "source": {
                        "type": "string",
                        "description": "Source ID of the marketplace (e.g., 'mcp-registry' from search results)"
                    }
                },
                "required": ["name", "source"]
            }
        }),
        json!({
            "name": format!("{}{}", TOOL_PREFIX, SEARCH_SKILLS),
            "description": "Browse available skills from configured skill sources. Returns a list of skills with their descriptions and metadata.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "query": {
                        "type": "string",
                        "description": "Optional search query to filter skills by name or description"
                    },
                    "source": {
                        "type": "string",
                        "description": "Optional source label to filter skills (e.g., 'Anthropic', 'Community')"
                    }
                }
            }
        }),
        json!({
            "name": format!("{}{}", TOOL_PREFIX, INSTALL_SKILL),
            "description": "Install a skill from a configured skill source. This will download the skill files and make it available for use. The user will be prompted to confirm the installation. Use the name and source from search results.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "name": {
                        "type": "string",
                        "description": "Name of the skill to install (from search results)"
                    },
                    "source": {
                        "type": "string",
                        "description": "Source ID of the marketplace (e.g., 'anthropic' from search results)"
                    }
                },
                "required": ["name", "source"]
            }
        }),
    ]
}

/// Handle a marketplace tool call
pub async fn handle_tool_call(
    service: &MarketplaceService,
    tool_name: &str,
    arguments: Value,
    client_id: &str,
    client_name: &str,
) -> Result<Value, MarketplaceError> {
    // Strip prefix if present
    let tool = tool_name.strip_prefix(TOOL_PREFIX).unwrap_or(tool_name);

    debug!(
        "Handling marketplace tool call: {} for client {}",
        tool, client_id
    );

    match tool {
        SEARCH_MCP_SERVERS => handle_search_mcp_servers(service, arguments).await,
        INSTALL_MCP_SERVER => {
            handle_install_mcp_server(service, arguments, client_id, client_name).await
        }
        SEARCH_SKILLS => handle_search_skills(service, arguments).await,
        INSTALL_SKILL => handle_install_skill(service, arguments, client_id, client_name).await,
        _ => Err(MarketplaceError::InvalidToolName(tool_name.to_string())),
    }
}

async fn handle_search_mcp_servers(
    service: &MarketplaceService,
    arguments: Value,
) -> Result<Value, MarketplaceError> {
    let query = arguments
        .get("query")
        .and_then(|v| v.as_str())
        .ok_or_else(|| MarketplaceError::InvalidArguments("query is required".to_string()))?;

    let limit = arguments
        .get("limit")
        .and_then(|v| v.as_u64())
        .map(|v| v.min(50) as u32);

    let results = service.search_mcp_servers(query, limit).await?;

    info!("Found {} MCP servers matching '{}'", results.len(), query);

    Ok(json!({
        "servers": results,
        "count": results.len(),
        "hint": "Use marketplace__install_mcp_server to install a server"
    }))
}

async fn handle_install_mcp_server(
    service: &MarketplaceService,
    arguments: Value,
    client_id: &str,
    client_name: &str,
) -> Result<Value, MarketplaceError> {
    let name = arguments
        .get("name")
        .and_then(|v| v.as_str())
        .ok_or_else(|| MarketplaceError::InvalidArguments("name is required".to_string()))?;

    let source = arguments
        .get("source")
        .and_then(|v| v.as_str())
        .ok_or_else(|| MarketplaceError::InvalidArguments("source is required".to_string()))?;

    info!(
        "Client {} requesting MCP server install: {} from {}",
        client_id, name, source
    );

    // First, search for the server to get full listing
    let results = service.search_mcp_servers(name, Some(50)).await?;

    let listing = results
        .into_iter()
        .find(|s| s.name == name && s.source_id == source)
        .ok_or_else(|| {
            MarketplaceError::InstallError(format!(
                "Server '{}' from source '{}' not found. Use marketplace__search_mcp_servers first.",
                name, source
            ))
        })?;

    // Request user approval via popup
    let install_manager = service.install_manager();
    let response = install_manager
        .request_mcp_install(
            listing.clone(),
            client_id.to_string(),
            client_name.to_string(),
        )
        .await?;

    match response.action {
        crate::install_popup::InstallAction::Install => {
            // User approved - perform the install
            let config = response.config.ok_or_else(|| {
                MarketplaceError::InstallError("No config provided by user".to_string())
            })?;

            // The actual installation is done by Tauri command (commands_marketplace.rs)
            // which has access to ConfigManager and McpServerManager
            // Here we just return success indicator that the popup was approved
            Ok(json!({
                "status": "approved",
                "message": format!("Installation of '{}' from '{}' approved by user", name, source),
                "config": config,
                "next_step": "The server is being installed and will be available shortly"
            }))
        }
        crate::install_popup::InstallAction::Cancel => Err(MarketplaceError::InstallCancelled),
    }
}

async fn handle_search_skills(
    service: &MarketplaceService,
    arguments: Value,
) -> Result<Value, MarketplaceError> {
    let query = arguments.get("query").and_then(|v| v.as_str());
    let source = arguments.get("source").and_then(|v| v.as_str());

    let results = service.search_skills(query, source).await?;

    info!(
        "Found {} skills matching query={:?}, source={:?}",
        results.len(),
        query,
        source
    );

    Ok(json!({
        "skills": results,
        "count": results.len(),
        "hint": "Use marketplace__install_skill to install a skill"
    }))
}

async fn handle_install_skill(
    service: &MarketplaceService,
    arguments: Value,
    client_id: &str,
    client_name: &str,
) -> Result<Value, MarketplaceError> {
    let name = arguments
        .get("name")
        .and_then(|v| v.as_str())
        .ok_or_else(|| MarketplaceError::InvalidArguments("name is required".to_string()))?;

    let source = arguments
        .get("source")
        .and_then(|v| v.as_str())
        .ok_or_else(|| MarketplaceError::InvalidArguments("source is required".to_string()))?;

    info!(
        "Client {} requesting skill install: {} from {}",
        client_id, name, source
    );

    // Search for the skill to get full listing
    let results = service.search_skills(Some(name), None).await?;

    let listing = results
        .into_iter()
        .find(|s| s.name == name && s.source_id == source)
        .ok_or_else(|| {
            MarketplaceError::InstallError(format!(
                "Skill '{}' from source '{}' not found. Use marketplace__search_skills first.",
                name, source
            ))
        })?;

    // Request user approval via popup
    let install_manager = service.install_manager();
    let response = install_manager
        .request_skill_install(
            listing.clone(),
            client_id.to_string(),
            client_name.to_string(),
        )
        .await?;

    match response.action {
        crate::install_popup::InstallAction::Install => {
            // User approved - the actual installation is done by Tauri command
            Ok(json!({
                "status": "approved",
                "message": format!("Installation of skill '{}' from '{}' approved by user", name, source),
                "listing": listing,
                "next_step": "The skill is being downloaded and will be available shortly"
            }))
        }
        crate::install_popup::InstallAction::Cancel => Err(MarketplaceError::InstallCancelled),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_list_tools() {
        let tools = list_tools();
        assert_eq!(tools.len(), 4);

        // Check tool names
        let names: Vec<&str> = tools
            .iter()
            .map(|t| t.get("name").unwrap().as_str().unwrap())
            .collect();

        assert!(names.contains(&"marketplace__search_mcp_servers"));
        assert!(names.contains(&"marketplace__install_mcp_server"));
        assert!(names.contains(&"marketplace__search_skills"));
        assert!(names.contains(&"marketplace__install_skill"));
    }

    #[test]
    fn test_tool_schemas() {
        let tools = list_tools();

        for tool in &tools {
            // Each tool should have name, description, inputSchema
            assert!(tool.get("name").is_some());
            assert!(tool.get("description").is_some());
            assert!(tool.get("inputSchema").is_some());

            let schema = tool.get("inputSchema").unwrap();
            assert_eq!(schema.get("type").unwrap(), "object");
        }
    }
}
