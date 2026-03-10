//! Marketplace tool definitions and handler
//!
//! Defines the 2 marketplace tools (search + install) and handles tool calls.

use crate::{MarketplaceError, MarketplaceService, TOOL_PREFIX};
use serde_json::{json, Value};
use tracing::{debug, info};

/// Tool names (without prefix)
pub const SEARCH: &str = "search";
pub const INSTALL: &str = "install";

/// List all marketplace tools as JSON tool definitions
pub fn list_tools() -> Vec<Value> {
    vec![
        json!({
            "name": format!("{}{}", TOOL_PREFIX, SEARCH),
            "description": "Search the marketplace for available MCP servers and/or skills. Returns matching results with descriptions and installation options.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "query": {
                        "type": "string",
                        "description": "Search query (e.g., 'filesystem', 'database', 'github')"
                    },
                    "type": {
                        "type": "string",
                        "enum": ["mcp", "skill", "all"],
                        "description": "Type of items to search for: 'mcp' for MCP servers, 'skill' for skills, 'all' for both (default: 'all')"
                    },
                    "source": {
                        "type": "string",
                        "description": "Optional source label to filter results (e.g., 'Anthropic', 'Community')"
                    },
                    "limit": {
                        "type": "integer",
                        "description": "Maximum number of results per type (default: 10, max: 50)",
                        "minimum": 1,
                        "maximum": 50
                    }
                },
                "required": ["query"]
            }
        }),
        json!({
            "name": format!("{}{}", TOOL_PREFIX, INSTALL),
            "description": "Install an MCP server or skill from the marketplace. The user will be prompted to confirm the installation. Use the name, source, and type from search results.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "name": {
                        "type": "string",
                        "description": "Name of the item to install (from search results)"
                    },
                    "source": {
                        "type": "string",
                        "description": "Source ID of the marketplace (e.g., 'mcp-registry', 'anthropic' from search results)"
                    },
                    "type": {
                        "type": "string",
                        "enum": ["mcp", "skill"],
                        "description": "Type of item to install: 'mcp' for MCP server, 'skill' for skill"
                    }
                },
                "required": ["name", "source", "type"]
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
        SEARCH => handle_search(service, arguments).await,
        INSTALL => handle_install(service, arguments, client_id, client_name).await,
        _ => Err(MarketplaceError::InvalidToolName(tool_name.to_string())),
    }
}

/// Resolve the search type from arguments, defaulting to "all"
fn resolve_search_type(arguments: &Value) -> &str {
    arguments
        .get("type")
        .and_then(|v| v.as_str())
        .unwrap_or("all")
}

async fn handle_search(
    service: &MarketplaceService,
    arguments: Value,
) -> Result<Value, MarketplaceError> {
    let query = arguments
        .get("query")
        .and_then(|v| v.as_str())
        .ok_or_else(|| MarketplaceError::InvalidArguments("query is required".to_string()))?;

    let search_type = resolve_search_type(&arguments);
    let source = arguments.get("source").and_then(|v| v.as_str());
    let limit = arguments
        .get("limit")
        .and_then(|v| v.as_u64())
        .map(|v| v.min(50) as u32);

    let mut result = json!({});

    // Search MCP servers
    if (search_type == "all" || search_type == "mcp") && service.is_mcp_enabled() {
        let servers = service.search_mcp_servers(query, limit).await?;
        info!("Found {} MCP servers matching '{}'", servers.len(), query);
        result["servers"] = json!(servers);
        result["server_count"] = json!(servers.len());
    }

    // Search skills
    if (search_type == "all" || search_type == "skill") && service.is_skills_enabled() {
        let skills = service.search_skills(Some(query), source).await?;
        info!("Found {} skills matching '{}'", skills.len(), query);
        result["skills"] = json!(skills);
        result["skill_count"] = json!(skills.len());
    }

    result["hint"] = json!("Use marketplace__install with type 'mcp' or 'skill' to install an item");

    Ok(result)
}

async fn handle_install(
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

    let install_type = arguments
        .get("type")
        .and_then(|v| v.as_str())
        .ok_or_else(|| MarketplaceError::InvalidArguments("type is required".to_string()))?;

    match install_type {
        "mcp" => {
            if !service.is_mcp_enabled() {
                return Err(MarketplaceError::NotEnabled(
                    "MCP marketplace is not enabled".to_string(),
                ));
            }
            handle_install_mcp_server(service, name, source, client_id, client_name).await
        }
        "skill" => {
            if !service.is_skills_enabled() {
                return Err(MarketplaceError::NotEnabled(
                    "Skills marketplace is not enabled".to_string(),
                ));
            }
            handle_install_skill(service, name, source, client_id, client_name).await
        }
        _ => Err(MarketplaceError::InvalidArguments(format!(
            "Invalid type '{}'. Must be 'mcp' or 'skill'",
            install_type
        ))),
    }
}

async fn handle_install_mcp_server(
    service: &MarketplaceService,
    name: &str,
    source: &str,
    client_id: &str,
    client_name: &str,
) -> Result<Value, MarketplaceError> {
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
                "Server '{}' from source '{}' not found. Use marketplace__search first.",
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
            let config = response.config.ok_or_else(|| {
                MarketplaceError::InstallError("No config provided by user".to_string())
            })?;

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

async fn handle_install_skill(
    service: &MarketplaceService,
    name: &str,
    source: &str,
    client_id: &str,
    client_name: &str,
) -> Result<Value, MarketplaceError> {
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
                "Skill '{}' from source '{}' not found. Use marketplace__search first.",
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
        assert_eq!(tools.len(), 2);

        // Check tool names
        let names: Vec<&str> = tools
            .iter()
            .map(|t| t.get("name").unwrap().as_str().unwrap())
            .collect();

        assert!(names.contains(&"marketplace__search"));
        assert!(names.contains(&"marketplace__install"));
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
