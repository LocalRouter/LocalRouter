//! Marketplace tool definitions and handler
//!
//! Defines the 2 marketplace tools (search + install) and handles tool calls.

use crate::{MarketplaceError, MarketplaceService};
use serde_json::{json, Value};
use tracing::{debug, info};

/// Returns the appropriate noun phrase for the enabled marketplace features.
fn feature_label(mcp: bool, skills: bool) -> &'static str {
    match (mcp, skills) {
        (true, true) => "MCP servers and skills",
        (true, false) => "MCP servers",
        (false, true) => "skills",
        (false, false) => "marketplace items",
    }
}

/// Returns the appropriate article + noun for install descriptions.
fn install_label(mcp: bool, skills: bool) -> &'static str {
    match (mcp, skills) {
        (true, true) => "an MCP server or skill",
        (true, false) => "an MCP server",
        (false, true) => "a skill",
        (false, false) => "an item",
    }
}

/// Build the search tool `type` enum and description based on enabled features.
fn search_type_schema(mcp: bool, skills: bool) -> Value {
    match (mcp, skills) {
        (true, true) => json!({
            "type": "string",
            "enum": ["mcp", "skill", "all"],
            "description": "Type of items to search for: 'mcp' for MCP servers, 'skill' for skills, 'all' for both (default: 'all')"
        }),
        (true, false) => json!({
            "type": "string",
            "enum": ["mcp"],
            "description": "Type of items to search for (only 'mcp' is available)"
        }),
        (false, true) => json!({
            "type": "string",
            "enum": ["skill"],
            "description": "Type of items to search for (only 'skill' is available)"
        }),
        (false, false) => json!({
            "type": "string",
            "enum": ["mcp", "skill", "all"],
            "description": "Type of items to search for"
        }),
    }
}

/// Build the install tool `type` enum and description based on enabled features.
fn install_type_schema(mcp: bool, skills: bool) -> Value {
    match (mcp, skills) {
        (true, true) => json!({
            "type": "string",
            "enum": ["mcp", "skill"],
            "description": "Type of item to install: 'mcp' for MCP server, 'skill' for skill"
        }),
        (true, false) => json!({
            "type": "string",
            "enum": ["mcp"],
            "description": "Type of item to install (only 'mcp' is available)"
        }),
        (false, true) => json!({
            "type": "string",
            "enum": ["skill"],
            "description": "Type of item to install (only 'skill' is available)"
        }),
        (false, false) => json!({
            "type": "string",
            "enum": ["mcp", "skill"],
            "description": "Type of item to install"
        }),
    }
}

/// List all marketplace tools as JSON tool definitions.
///
/// Tool descriptions and type enums adapt based on which features are enabled.
/// Tool names are provided by the caller (from `MarketplaceConfig`).
pub fn list_tools(
    search_tool_name: &str,
    install_tool_name: &str,
    mcp_enabled: bool,
    skills_enabled: bool,
) -> Vec<Value> {
    let search_desc = format!(
        "Search the marketplace for available {}. Returns matching results with descriptions and installation options.",
        feature_label(mcp_enabled, skills_enabled)
    );
    let install_desc = format!(
        "Install {} from the marketplace. The user will be prompted to confirm the installation. Use the name, source, and type from search results.",
        install_label(mcp_enabled, skills_enabled)
    );

    vec![
        json!({
            "name": search_tool_name,
            "description": search_desc,
            "inputSchema": {
                "type": "object",
                "properties": {
                    "query": {
                        "type": "string",
                        "description": "Search query (e.g., 'filesystem', 'database', 'github')"
                    },
                    "type": search_type_schema(mcp_enabled, skills_enabled),
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
            "name": install_tool_name,
            "description": install_desc,
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
                    "type": install_type_schema(mcp_enabled, skills_enabled)
                },
                "required": ["name", "source", "type"]
            }
        }),
    ]
}

/// Build the search result hint text based on enabled features.
pub fn search_hint(install_tool_name: &str, mcp_enabled: bool, skills_enabled: bool) -> String {
    match (mcp_enabled, skills_enabled) {
        (true, true) => {
            format!(
                "Use {} with type 'mcp' or 'skill' to install an item",
                install_tool_name
            )
        }
        (true, false) => {
            format!(
                "Use {} with type 'mcp' to install a server",
                install_tool_name
            )
        }
        (false, true) => {
            format!(
                "Use {} with type 'skill' to install a skill",
                install_tool_name
            )
        }
        (false, false) => format!("Use {} to install an item", install_tool_name),
    }
}

/// Handle a marketplace tool call.
///
/// Matches `tool_name` against the configured search and install tool names.
pub async fn handle_tool_call(
    service: &MarketplaceService,
    tool_name: &str,
    search_tool_name: &str,
    install_tool_name: &str,
    arguments: Value,
    client_id: &str,
    client_name: &str,
) -> Result<Value, MarketplaceError> {
    debug!(
        "Handling marketplace tool call: {} for client {}",
        tool_name, client_id
    );

    if tool_name == search_tool_name {
        handle_search(service, install_tool_name, arguments).await
    } else if tool_name == install_tool_name {
        handle_install(service, search_tool_name, arguments, client_id, client_name).await
    } else {
        Err(MarketplaceError::InvalidToolName(tool_name.to_string()))
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
    install_tool_name: &str,
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

    result["hint"] = json!(search_hint(
        install_tool_name,
        service.is_mcp_enabled(),
        service.is_skills_enabled()
    ));

    Ok(result)
}

async fn handle_install(
    service: &MarketplaceService,
    search_tool_name: &str,
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
            handle_install_mcp_server(
                service,
                search_tool_name,
                name,
                source,
                client_id,
                client_name,
            )
            .await
        }
        "skill" => {
            if !service.is_skills_enabled() {
                return Err(MarketplaceError::NotEnabled(
                    "Skills marketplace is not enabled".to_string(),
                ));
            }
            handle_install_skill(
                service,
                search_tool_name,
                name,
                source,
                client_id,
                client_name,
            )
            .await
        }
        _ => Err(MarketplaceError::InvalidArguments(format!(
            "Invalid type '{}'. Must be 'mcp' or 'skill'",
            install_type
        ))),
    }
}

async fn handle_install_mcp_server(
    service: &MarketplaceService,
    search_tool_name: &str,
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
                "Server '{}' from source '{}' not found. Use {} first.",
                name, source, search_tool_name
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

            // Perform actual installation via callback
            if let Some(callback) = service.mcp_install_callback() {
                let result = callback(
                    listing,
                    config,
                    client_id.to_string(),
                    client_name.to_string(),
                )
                .await
                .map_err(MarketplaceError::InstallError)?;

                let tools_summary: Vec<Value> = result
                    .tools
                    .iter()
                    .map(|t| {
                        json!({
                            "name": t.name,
                            "description": t.description,
                        })
                    })
                    .collect();

                let tool_names: Vec<&str> = result.tools.iter().map(|t| t.name.as_str()).collect();

                Ok(json!({
                    "status": "installed",
                    "server_id": result.server_id,
                    "server_name": result.server_name,
                    "tools": tools_summary,
                    "instructions": result.instructions,
                    "message": format!(
                        "MCP server '{}' installed and ready. Available tools: {}. These tools are now available for immediate use.",
                        result.server_name,
                        tool_names.join(", ")
                    )
                }))
            } else {
                // No callback set — fall back to old behavior (approved but not installed)
                Ok(json!({
                    "status": "approved",
                    "message": format!("Installation of '{}' from '{}' approved by user", name, source),
                    "config": config,
                    "next_step": "The server is being installed and will be available shortly"
                }))
            }
        }
        crate::install_popup::InstallAction::Cancel => Err(MarketplaceError::InstallCancelled),
    }
}

async fn handle_install_skill(
    service: &MarketplaceService,
    search_tool_name: &str,
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
                "Skill '{}' from source '{}' not found. Use {} first.",
                name, source, search_tool_name
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
            // Perform actual installation via callback
            if let Some(callback) = service.skill_install_callback() {
                let result = callback(listing, client_id.to_string(), client_name.to_string())
                    .await
                    .map_err(MarketplaceError::InstallError)?;

                let tools_summary: Vec<Value> = result
                    .tools
                    .iter()
                    .map(|t| {
                        json!({
                            "name": t.name,
                            "description": t.description,
                        })
                    })
                    .collect();

                let tool_names: Vec<&str> = result.tools.iter().map(|t| t.name.as_str()).collect();

                let message = if tool_names.is_empty() {
                    format!(
                        "Skill '{}' installed and ready. The skill's tools are now available for use.",
                        result.skill_name
                    )
                } else {
                    format!(
                        "Skill '{}' installed and ready. Available tools: {}. These tools are now available for immediate use.",
                        result.skill_name,
                        tool_names.join(", ")
                    )
                };

                Ok(json!({
                    "status": "installed",
                    "skill_name": result.skill_name,
                    "tools": tools_summary,
                    "instructions": result.instructions,
                    "message": message
                }))
            } else {
                // No callback set — fall back to old behavior
                Ok(json!({
                    "status": "approved",
                    "message": format!("Installation of skill '{}' from '{}' approved by user", name, source),
                    "listing": listing,
                    "next_step": "The skill is being downloaded and will be available shortly"
                }))
            }
        }
        crate::install_popup::InstallAction::Cancel => Err(MarketplaceError::InstallCancelled),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_list_tools_both_enabled() {
        let tools = list_tools("MarketplaceSearch", "MarketplaceInstall", true, true);
        assert_eq!(tools.len(), 2);

        let names: Vec<&str> = tools
            .iter()
            .map(|t| t.get("name").unwrap().as_str().unwrap())
            .collect();
        assert!(names.contains(&"MarketplaceSearch"));
        assert!(names.contains(&"MarketplaceInstall"));

        // Search tool should reference both
        let search = &tools[0];
        let desc = search.get("description").unwrap().as_str().unwrap();
        assert!(desc.contains("MCP servers and skills"));

        // Type enum should include all three options
        let type_enum = search["inputSchema"]["properties"]["type"]["enum"]
            .as_array()
            .unwrap();
        assert_eq!(type_enum.len(), 3);

        // Install tool should reference both
        let install = &tools[1];
        let desc = install.get("description").unwrap().as_str().unwrap();
        assert!(desc.contains("MCP server or skill"));

        let type_enum = install["inputSchema"]["properties"]["type"]["enum"]
            .as_array()
            .unwrap();
        assert_eq!(type_enum.len(), 2);
    }

    #[test]
    fn test_list_tools_mcp_only() {
        let tools = list_tools("MarketplaceSearch", "MarketplaceInstall", true, false);
        assert_eq!(tools.len(), 2);

        let search = &tools[0];
        let desc = search.get("description").unwrap().as_str().unwrap();
        assert!(desc.contains("MCP servers"));
        assert!(!desc.contains("skills"));

        let type_enum = search["inputSchema"]["properties"]["type"]["enum"]
            .as_array()
            .unwrap();
        assert_eq!(type_enum, &[json!("mcp")]);

        let install = &tools[1];
        let desc = install.get("description").unwrap().as_str().unwrap();
        assert!(desc.contains("MCP server"));
        assert!(!desc.contains("skill"));

        let type_enum = install["inputSchema"]["properties"]["type"]["enum"]
            .as_array()
            .unwrap();
        assert_eq!(type_enum, &[json!("mcp")]);
    }

    #[test]
    fn test_list_tools_skills_only() {
        let tools = list_tools("MarketplaceSearch", "MarketplaceInstall", false, true);
        assert_eq!(tools.len(), 2);

        let search = &tools[0];
        let desc = search.get("description").unwrap().as_str().unwrap();
        assert!(desc.contains("skills"));
        assert!(!desc.contains("MCP"));

        let type_enum = search["inputSchema"]["properties"]["type"]["enum"]
            .as_array()
            .unwrap();
        assert_eq!(type_enum, &[json!("skill")]);

        let install = &tools[1];
        let desc = install.get("description").unwrap().as_str().unwrap();
        assert!(desc.contains("skill"));
        assert!(!desc.contains("MCP"));

        let type_enum = install["inputSchema"]["properties"]["type"]["enum"]
            .as_array()
            .unwrap();
        assert_eq!(type_enum, &[json!("skill")]);
    }

    #[test]
    fn test_list_tools_custom_names() {
        let tools = list_tools("MySearch", "MyInstall", true, true);
        assert_eq!(tools.len(), 2);

        let names: Vec<&str> = tools
            .iter()
            .map(|t| t.get("name").unwrap().as_str().unwrap())
            .collect();
        assert!(names.contains(&"MySearch"));
        assert!(names.contains(&"MyInstall"));
    }

    #[test]
    fn test_tool_schemas() {
        let tools = list_tools("MarketplaceSearch", "MarketplaceInstall", true, true);

        for tool in &tools {
            assert!(tool.get("name").is_some());
            assert!(tool.get("description").is_some());
            assert!(tool.get("inputSchema").is_some());

            let schema = tool.get("inputSchema").unwrap();
            assert_eq!(schema.get("type").unwrap(), "object");
        }
    }

    #[test]
    fn test_search_hint() {
        let hint = search_hint("MarketplaceInstall", true, true);
        assert!(hint.contains("'mcp' or 'skill'"));
        assert!(hint.contains("MarketplaceInstall"));

        let hint = search_hint("MarketplaceInstall", true, false);
        assert!(hint.contains("'mcp'"));
        assert!(!hint.contains("skill"));

        let hint = search_hint("MarketplaceInstall", false, true);
        assert!(hint.contains("'skill'"));
        assert!(!hint.contains("mcp"));
    }
}
