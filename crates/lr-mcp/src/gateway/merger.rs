// Empty import section - using json! macro from types.rs

use super::types::*;
use crate::protocol::{McpPrompt, McpResource, McpTool};

/// An MCP server's info for instruction building (using human-readable names, not UUIDs)
pub struct McpServerInstructionInfo {
    /// Human-readable name (e.g., "My Filesystem" as configured by the user)
    pub name: String,
    /// The server's own instructions (from MCP `instructions` field), if any
    pub instructions: Option<String>,
    /// The server's description (from MCP `serverInfo.description`), if any
    pub description: Option<String>,
    /// Tool names (already namespaced, e.g., "my-filesystem__read_file")
    pub tool_names: Vec<String>,
    /// Resource names (already namespaced)
    pub resource_names: Vec<String>,
    /// Prompt names (already namespaced)
    pub prompt_names: Vec<String>,
}

/// An unavailable server for instruction building
pub struct UnavailableServerInfo {
    /// Human-readable name
    pub name: String,
    /// Error message
    pub error: String,
}

/// Context for building gateway instructions
pub struct InstructionsContext {
    /// Available MCP servers with their info
    pub servers: Vec<McpServerInstructionInfo>,
    /// Unavailable MCP servers
    pub unavailable_servers: Vec<UnavailableServerInfo>,
    /// Whether deferred loading is enabled
    pub deferred_loading: bool,
    /// Instructions from virtual servers
    pub virtual_instructions: Vec<super::virtual_server::VirtualInstructions>,
}

/// Merge initialize results from multiple servers
pub fn merge_initialize_results(
    results: Vec<(String, InitializeResult)>,
    failures: Vec<ServerFailure>,
) -> MergedCapabilities {
    // Find minimum protocol version
    let protocol_version = results
        .iter()
        .map(|(_, result)| &result.protocol_version)
        .min()
        .cloned()
        .unwrap_or_else(|| "2024-11-05".to_string());

    // Merge capabilities (union of all capabilities)
    let mut merged_capabilities = ServerCapabilities::default();

    for (_, result) in &results {
        if let Some(tools) = &result.capabilities.tools {
            let existing = merged_capabilities
                .tools
                .get_or_insert(ToolsCapability { list_changed: None });
            if tools.list_changed.unwrap_or(false) {
                existing.list_changed = Some(true);
            }
        }

        if let Some(resources) = &result.capabilities.resources {
            let existing = merged_capabilities
                .resources
                .get_or_insert(ResourcesCapability {
                    list_changed: None,
                    subscribe: None,
                });
            if resources.list_changed.unwrap_or(false) {
                existing.list_changed = Some(true);
            }
            if resources.subscribe.unwrap_or(false) {
                existing.subscribe = Some(true);
            }
        }

        if let Some(prompts) = &result.capabilities.prompts {
            let existing = merged_capabilities
                .prompts
                .get_or_insert(PromptsCapability { list_changed: None });
            if prompts.list_changed.unwrap_or(false) {
                existing.list_changed = Some(true);
            }
        }

        if result.capabilities.logging.is_some() {
            merged_capabilities.logging = Some(LoggingCapability {});
        }
    }

    // Build server description with catalog listing
    let description = build_server_description(&results, &failures);

    let server_info = ServerInfo {
        name: "LocalRouter Unified Gateway".to_string(),
        version: env!("CARGO_PKG_VERSION").to_string(),
        description: Some(description),
    };

    MergedCapabilities {
        protocol_version,
        capabilities: merged_capabilities,
        server_info,
        failures,
        instructions: None, // Set later via build_gateway_instructions when full context is available
    }
}

/// Build comprehensive server description with catalog listings
fn build_server_description(
    results: &[(String, InitializeResult)],
    failures: &[ServerFailure],
) -> String {
    let mut desc =
        "Unified gateway aggregating multiple MCP servers.\n\nAvailable servers:\n\n".to_string();

    for (i, (server_id, result)) in results.iter().enumerate() {
        desc.push_str(&format!(
            "{}. {} ({})\n",
            i + 1,
            server_id,
            result.server_info.name
        ));

        if let Some(server_desc) = &result.server_info.description {
            desc.push_str(&format!("   Description: {}\n", server_desc));
        }

        // Note: Tool/resource/prompt counts will be populated during list operations
        // For now, just show server is available
        desc.push('\n');
    }

    // Add failure information
    if !failures.is_empty() {
        desc.push_str("\nFailed servers:\n");
        for failure in failures {
            desc.push_str(&format!("  - {}: {}\n", failure.server_id, failure.error));
        }
    }

    desc
}

/// Build comprehensive gateway instructions based on available capabilities.
///
/// Structure:
/// 1. Header
/// 2. Capability listing: virtual servers first, then regular servers, then unavailable
/// 3. Instructions in XML tags: virtual always, regular only in non-deferred mode
pub fn build_gateway_instructions(ctx: &InstructionsContext) -> Option<String> {
    let has_servers = !ctx.servers.is_empty();
    let has_unavailable = !ctx.unavailable_servers.is_empty();
    let has_virtual = !ctx.virtual_instructions.is_empty();

    // Nothing to describe
    if !has_servers && !has_unavailable && !has_virtual {
        return None;
    }

    let mut inst = String::new();

    // --- 1. Header ---
    build_header(&mut inst, has_servers, has_virtual, ctx.deferred_loading);

    // --- 2. Capability listing ---
    build_capability_listing(&mut inst, ctx);

    // --- 3. Instructions in XML tags ---
    build_all_instructions_section(&mut inst, ctx);

    Some(inst)
}

/// Slugify a server name for use as an XML tag (e.g., "My MCP Server" -> "my-mcp-server")
fn slugify(name: &str) -> String {
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

/// Maximum number of tool names shown per server in deferred mode.
const DEFERRED_TOOL_PREVIEW_LIMIT: usize = 20;

/// Build the header line based on what's available.
fn build_header(inst: &mut String, has_mcp_servers: bool, has_virtual: bool, deferred: bool) {
    if !has_mcp_servers && !has_virtual {
        // Only unavailable servers — nothing usable
        inst.push_str(
            "Unified MCP Gateway: no servers or tools are currently available.\n\n",
        );
    } else if deferred {
        inst.push_str(
            "Unified MCP Gateway. Tools are loaded on demand \
             — use the `search` tool to discover and activate them by keyword. \
             Use `server_info` to get a server's full tool list and detailed instructions.\n\n",
        );
    } else if has_mcp_servers {
        inst.push_str(
            "Unified MCP Gateway. Tools from MCP servers are namespaced \
             with a `servername__` prefix.\n\n",
        );
    } else {
        // Only virtual servers
        inst.push_str("Unified MCP Gateway.\n\n");
    }
}

/// Build the capability listing: virtual servers first, then regular, then unavailable.
fn build_capability_listing(inst: &mut String, ctx: &InstructionsContext) {
    // --- Virtual servers first ---
    for vsi in &ctx.virtual_instructions {
        inst.push_str(&format!("**{}**\n", vsi.section_title));
        for name in &vsi.tool_names {
            inst.push_str(&format!("- `{}` (tool)\n", name));
        }
        inst.push('\n');
    }

    // --- Regular MCP servers ---
    for server in &ctx.servers {
        let total_items = server.tool_names.len()
            + server.resource_names.len()
            + server.prompt_names.len();

        inst.push_str(&format!("**{}**", server.name));
        if total_items == 0 {
            inst.push_str(" (no capabilities)\n");
        } else if ctx.deferred_loading && total_items > DEFERRED_TOOL_PREVIEW_LIMIT {
            // Show count summary in deferred mode when truncated
            let mut parts = Vec::new();
            if !server.tool_names.is_empty() {
                parts.push(format!("{} tools", server.tool_names.len()));
            }
            if !server.resource_names.is_empty() {
                parts.push(format!("{} resources", server.resource_names.len()));
            }
            if !server.prompt_names.is_empty() {
                parts.push(format!("{} prompts", server.prompt_names.len()));
            }
            inst.push_str(&format!(" ({})", parts.join(", ")));
            inst.push('\n');

            // Show first N items
            let mut shown = 0;
            for name in &server.tool_names {
                if shown >= DEFERRED_TOOL_PREVIEW_LIMIT {
                    break;
                }
                inst.push_str(&format!("- `{}` (tool)\n", name));
                shown += 1;
            }
            for name in &server.resource_names {
                if shown >= DEFERRED_TOOL_PREVIEW_LIMIT {
                    break;
                }
                inst.push_str(&format!("- `{}` (resource)\n", name));
                shown += 1;
            }
            for name in &server.prompt_names {
                if shown >= DEFERRED_TOOL_PREVIEW_LIMIT {
                    break;
                }
                inst.push_str(&format!("- `{}` (prompt)\n", name));
                shown += 1;
            }
            inst.push_str(
                "- ... (use `search` to find more, or `server_info` for full list)\n",
            );
        } else {
            inst.push('\n');
            for name in &server.tool_names {
                inst.push_str(&format!("- `{}` (tool)\n", name));
            }
            for name in &server.resource_names {
                inst.push_str(&format!("- `{}` (resource)\n", name));
            }
            for name in &server.prompt_names {
                inst.push_str(&format!("- `{}` (prompt)\n", name));
            }
        }
        inst.push('\n');
    }

    // --- Unavailable servers ---
    for server in &ctx.unavailable_servers {
        inst.push_str(&format!(
            "**{}** — unavailable: {}\n\n",
            server.name, server.error
        ));
    }
}

/// Build all instructions wrapped in XML tags.
///
/// Virtual server instructions are always included.
/// Regular server instructions are only included in non-deferred mode.
fn build_all_instructions_section(inst: &mut String, ctx: &InstructionsContext) {
    // Virtual server instructions (always included)
    for vsi in &ctx.virtual_instructions {
        if vsi.content.is_empty() {
            continue;
        }
        let tag = slugify(&vsi.section_title);
        inst.push_str(&format!("\n<{}>\n", tag));
        inst.push_str(&vsi.content);
        if !vsi.content.ends_with('\n') {
            inst.push('\n');
        }
        inst.push_str(&format!("</{}>\n", tag));
    }

    // Regular server instructions (omitted in deferred mode)
    if !ctx.deferred_loading {
        for server in &ctx.servers {
            let has_content = server.instructions.is_some() || server.description.is_some();
            if !has_content {
                continue;
            }

            let tag = slugify(&server.name);
            inst.push_str(&format!("\n<{}>\n", tag));

            if let Some(desc) = &server.description {
                inst.push_str(desc);
                if !desc.ends_with('\n') {
                    inst.push('\n');
                }
            }

            if let Some(instructions) = &server.instructions {
                if server.description.is_some() {
                    inst.push('\n');
                }
                inst.push_str(instructions);
                if !instructions.ends_with('\n') {
                    inst.push('\n');
                }
            }

            inst.push_str(&format!("</{}>\n", tag));
        }
    }
}

/// Merge tools from multiple servers with namespacing
///
/// # Arguments
/// * `server_tools` - Vec of (server_id, tools) tuples
/// * `_failures` - Server failures (for potential future use)
/// * `server_id_to_name` - Optional mapping from server ID (UUID) to human-readable name.
///   If provided, uses the name for the namespace prefix (e.g., "filesystem__read_file").
///   If not provided, uses the server ID (UUID) as the prefix.
pub fn merge_tools(
    server_tools: Vec<(String, Vec<McpTool>)>,
    _failures: &[ServerFailure],
    server_id_to_name: Option<&std::collections::HashMap<String, String>>,
) -> Vec<NamespacedTool> {
    let mut merged_tools = Vec::new();

    for (server_id, tools) in server_tools {
        // Use the human-readable name for the namespace if available, otherwise use server_id
        let display_name = server_id_to_name
            .and_then(|map| map.get(&server_id))
            .cloned()
            .unwrap_or_else(|| server_id.clone());

        for tool in tools {
            let namespaced_name = apply_namespace(&display_name, &tool.name);

            merged_tools.push(NamespacedTool {
                name: namespaced_name,
                original_name: tool.name.clone(),
                server_id: server_id.clone(), // Keep UUID for routing
                description: tool.description.clone(),
                input_schema: tool.input_schema.clone(),
            });
        }
    }

    // Sort by server_id, then by tool name for consistent ordering
    merged_tools.sort_by(|a, b| {
        a.server_id
            .cmp(&b.server_id)
            .then_with(|| a.name.cmp(&b.name))
    });

    merged_tools
}

/// Merge resources from multiple servers with namespacing
///
/// # Arguments
/// * `server_resources` - Vec of (server_id, resources) tuples
/// * `_failures` - Server failures (for potential future use)
/// * `server_id_to_name` - Optional mapping from server ID (UUID) to human-readable name.
///   If provided, uses the name for the namespace prefix.
pub fn merge_resources(
    server_resources: Vec<(String, Vec<McpResource>)>,
    _failures: &[ServerFailure],
    server_id_to_name: Option<&std::collections::HashMap<String, String>>,
) -> Vec<NamespacedResource> {
    let mut merged_resources = Vec::new();

    for (server_id, resources) in server_resources {
        // Use the human-readable name for the namespace if available
        let display_name = server_id_to_name
            .and_then(|map| map.get(&server_id))
            .cloned()
            .unwrap_or_else(|| server_id.clone());

        for resource in resources {
            let namespaced_name = apply_namespace(&display_name, &resource.name);

            merged_resources.push(NamespacedResource {
                name: namespaced_name,
                original_name: resource.name.clone(),
                server_id: server_id.clone(), // Keep UUID for routing
                uri: resource.uri.clone(),
                description: resource.description.clone(),
                mime_type: resource.mime_type.clone(),
            });
        }
    }

    // Sort by server_id, then by resource name
    merged_resources.sort_by(|a, b| {
        a.server_id
            .cmp(&b.server_id)
            .then_with(|| a.name.cmp(&b.name))
    });

    merged_resources
}

/// Merge prompts from multiple servers with namespacing
///
/// # Arguments
/// * `server_prompts` - Vec of (server_id, prompts) tuples
/// * `_failures` - Server failures (for potential future use)
/// * `server_id_to_name` - Optional mapping from server ID (UUID) to human-readable name.
///   If provided, uses the name for the namespace prefix.
pub fn merge_prompts(
    server_prompts: Vec<(String, Vec<McpPrompt>)>,
    _failures: &[ServerFailure],
    server_id_to_name: Option<&std::collections::HashMap<String, String>>,
) -> Vec<NamespacedPrompt> {
    let mut merged_prompts = Vec::new();

    for (server_id, prompts) in server_prompts {
        // Use the human-readable name for the namespace if available
        let display_name = server_id_to_name
            .and_then(|map| map.get(&server_id))
            .cloned()
            .unwrap_or_else(|| server_id.clone());

        for prompt in prompts {
            let namespaced_name = apply_namespace(&display_name, &prompt.name);

            // Convert arguments
            let arguments = prompt.arguments.map(|args| {
                args.into_iter()
                    .map(|arg| PromptArgument {
                        name: arg.name,
                        description: arg.description,
                        required: arg.required,
                    })
                    .collect()
            });

            merged_prompts.push(NamespacedPrompt {
                name: namespaced_name,
                original_name: prompt.name.clone(),
                server_id: server_id.clone(), // Keep UUID for routing
                description: prompt.description.clone(),
                arguments,
            });
        }
    }

    // Sort by server_id, then by prompt name
    merged_prompts.sort_by(|a, b| {
        a.server_id
            .cmp(&b.server_id)
            .then_with(|| a.name.cmp(&b.name))
    });

    merged_prompts
}

/// Update server description with catalog counts (call after lists are populated)
#[allow(dead_code)]
pub fn update_server_description_with_catalog(
    _base_description: &str,
    tools: &[NamespacedTool],
    resources: &[NamespacedResource],
    prompts: &[NamespacedPrompt],
) -> String {
    let mut desc =
        "Unified gateway aggregating multiple MCP servers.\n\nAvailable servers:\n\n".to_string();

    // Group by server_id
    let mut server_stats = std::collections::HashMap::new();

    for tool in tools {
        let stats = server_stats.entry(&tool.server_id).or_insert((
            0,
            0,
            0,
            Vec::new(),
            Vec::new(),
            Vec::new(),
        ));
        stats.0 += 1;
        stats.3.push(tool.name.clone());
    }

    for resource in resources {
        let stats = server_stats.entry(&resource.server_id).or_insert((
            0,
            0,
            0,
            Vec::new(),
            Vec::new(),
            Vec::new(),
        ));
        stats.1 += 1;
        stats.4.push(resource.name.clone());
    }

    for prompt in prompts {
        let stats = server_stats.entry(&prompt.server_id).or_insert((
            0,
            0,
            0,
            Vec::new(),
            Vec::new(),
            Vec::new(),
        ));
        stats.2 += 1;
        stats.5.push(prompt.name.clone());
    }

    let mut server_ids: Vec<_> = server_stats.keys().cloned().collect();
    server_ids.sort();

    for (i, server_id) in server_ids.iter().enumerate() {
        let (tool_count, resource_count, prompt_count, tool_names, resource_names, prompt_names) =
            server_stats.get(server_id).unwrap();

        desc.push_str(&format!(
            "{}. {} ({} tools, {} resources, {} prompts)\n",
            i + 1,
            server_id,
            tool_count,
            resource_count,
            prompt_count
        ));

        if !tool_names.is_empty() {
            desc.push_str(&format!("   Tools: {}\n", tool_names.join(", ")));
        }
        if !resource_names.is_empty() {
            desc.push_str(&format!("   Resources: {}\n", resource_names.join(", ")));
        }
        if !prompt_names.is_empty() {
            desc.push_str(&format!("   Prompts: {}\n", prompt_names.join(", ")));
        }
        desc.push('\n');
    }

    desc
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn test_merge_initialize_results() {
        let results = vec![
            (
                "filesystem".to_string(),
                InitializeResult {
                    protocol_version: "2024-11-05".to_string(),
                    capabilities: ServerCapabilities {
                        tools: Some(ToolsCapability {
                            list_changed: Some(true),
                        }),
                        ..Default::default()
                    },
                    server_info: ServerInfo {
                        name: "Filesystem Server".to_string(),
                        version: "1.0.0".to_string(),
                        description: Some("File operations".to_string()),
                    },
                    instructions: None,
                },
            ),
            (
                "github".to_string(),
                InitializeResult {
                    protocol_version: "2024-11-05".to_string(),
                    capabilities: ServerCapabilities {
                        resources: Some(ResourcesCapability {
                            list_changed: Some(true),
                            subscribe: Some(true),
                        }),
                        ..Default::default()
                    },
                    server_info: ServerInfo {
                        name: "GitHub Server".to_string(),
                        version: "1.0.0".to_string(),
                        description: None,
                    },
                    instructions: None,
                },
            ),
        ];

        let merged = merge_initialize_results(results, vec![]);

        assert_eq!(merged.protocol_version, "2024-11-05");
        assert!(merged.capabilities.tools.is_some());
        assert!(merged.capabilities.resources.is_some());
        assert_eq!(merged.server_info.name, "LocalRouter Unified Gateway");
    }

    #[test]
    fn test_slugify() {
        assert_eq!(slugify("My MCP Server"), "my-mcp-server");
        assert_eq!(slugify("filesystem"), "filesystem");
        assert_eq!(slugify("GitHub  API"), "github-api");
        assert_eq!(slugify("  leading-trailing  "), "leading-trailing");
        assert_eq!(slugify("CamelCase"), "camelcase");
    }

    #[test]
    fn test_build_gateway_instructions_servers_only() {
        let ctx = InstructionsContext {
            servers: vec![McpServerInstructionInfo {
                name: "filesystem".to_string(),
                instructions: Some("Use read_file to read and write_file to write.".to_string()),
                description: Some("File operations server".to_string()),
                tool_names: vec![
                    "filesystem__read_file".to_string(),
                    "filesystem__write_file".to_string(),
                ],
                resource_names: Vec::new(),
                prompt_names: Vec::new(),
            }],
            unavailable_servers: Vec::new(),
            deferred_loading: false,
            virtual_instructions: Vec::new(),
        };

        let instructions = build_gateway_instructions(&ctx).unwrap();
        // Header
        assert!(instructions.contains("Unified MCP Gateway"));
        assert!(instructions.contains("servername__"));
        // Tool listing with type annotations
        assert!(instructions.contains("**filesystem**"));
        assert!(instructions.contains("`filesystem__read_file` (tool)"));
        assert!(instructions.contains("`filesystem__write_file` (tool)"));
        // Server instructions in XML tags
        assert!(instructions.contains("<filesystem>"));
        assert!(instructions.contains("Use read_file to read"));
        assert!(instructions.contains("</filesystem>"));
        // Should NOT contain deferred loading text
        assert!(!instructions.contains("on demand"));
    }

    #[test]
    fn test_build_gateway_instructions_server_description_fallback() {
        let ctx = InstructionsContext {
            servers: vec![McpServerInstructionInfo {
                name: "github".to_string(),
                instructions: None,
                description: Some("GitHub API access".to_string()),
                tool_names: vec!["github__create_issue".to_string()],
                resource_names: Vec::new(),
                prompt_names: Vec::new(),
            }],
            unavailable_servers: Vec::new(),
            deferred_loading: false,
            virtual_instructions: Vec::new(),
        };

        let instructions = build_gateway_instructions(&ctx).unwrap();
        assert!(instructions.contains("<github>"));
        assert!(instructions.contains("GitHub API access"));
        assert!(instructions.contains("</github>"));
    }

    #[test]
    fn test_build_gateway_instructions_both_description_and_instructions() {
        let ctx = InstructionsContext {
            servers: vec![McpServerInstructionInfo {
                name: "filesystem".to_string(),
                instructions: Some("Use read_file to read files.".to_string()),
                description: Some("File operations server".to_string()),
                tool_names: vec!["filesystem__read_file".to_string()],
                resource_names: Vec::new(),
                prompt_names: Vec::new(),
            }],
            unavailable_servers: Vec::new(),
            deferred_loading: false,
            virtual_instructions: Vec::new(),
        };

        let instructions = build_gateway_instructions(&ctx).unwrap();
        assert!(instructions.contains("<filesystem>"));
        assert!(instructions.contains("File operations server"));
        assert!(instructions.contains("Use read_file to read files."));
        assert!(instructions.contains("</filesystem>"));
    }

    #[test]
    fn test_build_gateway_instructions_virtual_only() {
        use crate::gateway::virtual_server::VirtualInstructions;

        let ctx = InstructionsContext {
            servers: Vec::new(),
            unavailable_servers: Vec::new(),
            deferred_loading: false,
            virtual_instructions: vec![VirtualInstructions {
                section_title: "Skills".to_string(),
                content: "Call a skill's `get_info` tool.\n\n- **code-review**: `skill_code_review_get_info` — Automated code review\n"
                    .to_string(),
                tool_names: vec!["skill_code_review_get_info".to_string()],
            }],
        };

        let instructions = build_gateway_instructions(&ctx).unwrap();
        // Virtual tool listing
        assert!(instructions.contains("**Skills**"));
        assert!(instructions.contains("`skill_code_review_get_info` (tool)"));
        // Virtual instructions in XML tags
        assert!(instructions.contains("<skills>"));
        assert!(instructions.contains("Automated code review"));
        assert!(instructions.contains("</skills>"));
    }

    #[test]
    fn test_build_gateway_instructions_deferred_loading() {
        let ctx = InstructionsContext {
            servers: vec![McpServerInstructionInfo {
                name: "filesystem".to_string(),
                instructions: Some("Detailed file system instructions.".to_string()),
                description: None,
                tool_names: vec![
                    "filesystem__read_file".to_string(),
                    "filesystem__write_file".to_string(),
                ],
                resource_names: Vec::new(),
                prompt_names: Vec::new(),
            }],
            unavailable_servers: Vec::new(),
            deferred_loading: true,
            virtual_instructions: Vec::new(),
        };

        let instructions = build_gateway_instructions(&ctx).unwrap();
        assert!(instructions.contains("on demand"));
        assert!(instructions.contains("`search`"));
        assert!(instructions.contains("`server_info`"));
        assert!(instructions.contains("`filesystem__read_file` (tool)"));
        // Deferred mode omits regular server instructions
        assert!(!instructions.contains("<filesystem>"));
        assert!(!instructions.contains("Detailed file system instructions."));
    }

    #[test]
    fn test_build_gateway_instructions_both_servers_and_virtual() {
        use crate::gateway::virtual_server::VirtualInstructions;

        let ctx = InstructionsContext {
            servers: vec![McpServerInstructionInfo {
                name: "github".to_string(),
                instructions: None,
                description: Some("GitHub API access".to_string()),
                tool_names: vec!["github__create_issue".to_string()],
                resource_names: Vec::new(),
                prompt_names: Vec::new(),
            }],
            unavailable_servers: Vec::new(),
            deferred_loading: false,
            virtual_instructions: vec![VirtualInstructions {
                section_title: "Skills".to_string(),
                content: "Call get_info to unlock.\n".to_string(),
                tool_names: vec!["skill_deploy_get_info".to_string()],
            }],
        };

        let instructions = build_gateway_instructions(&ctx).unwrap();
        // Header
        assert!(instructions.contains("Unified MCP Gateway"));
        // Virtual server listed FIRST
        let skills_pos = instructions.find("**Skills**").unwrap();
        let github_pos = instructions.find("**github**").unwrap();
        assert!(skills_pos < github_pos, "Virtual servers should come first");
        // Virtual tool listing
        assert!(instructions.contains("`skill_deploy_get_info` (tool)"));
        // Regular tool listing
        assert!(instructions.contains("`github__create_issue` (tool)"));
        // Virtual instructions in XML
        assert!(instructions.contains("<skills>"));
        // Regular instructions in XML
        assert!(instructions.contains("<github>"));
    }

    #[test]
    fn test_build_gateway_instructions_empty() {
        let ctx = InstructionsContext {
            servers: Vec::new(),
            unavailable_servers: Vec::new(),
            deferred_loading: false,
            virtual_instructions: Vec::new(),
        };

        assert!(build_gateway_instructions(&ctx).is_none());
    }

    #[test]
    fn test_build_gateway_instructions_with_unavailable_servers() {
        let ctx = InstructionsContext {
            servers: Vec::new(),
            unavailable_servers: vec![UnavailableServerInfo {
                name: "broken-server".to_string(),
                error: "Connection refused".to_string(),
            }],
            deferred_loading: false,
            virtual_instructions: Vec::new(),
        };

        let instructions = build_gateway_instructions(&ctx).unwrap();
        assert!(instructions.contains("**broken-server**"));
        assert!(instructions.contains("unavailable"));
        assert!(instructions.contains("Connection refused"));
    }

    #[test]
    fn test_build_gateway_instructions_xml_tag_slugified() {
        let ctx = InstructionsContext {
            servers: vec![McpServerInstructionInfo {
                name: "My MCP Server".to_string(),
                instructions: Some("Some instructions.".to_string()),
                description: None,
                tool_names: vec!["my-mcp-server__do_thing".to_string()],
                resource_names: Vec::new(),
                prompt_names: Vec::new(),
            }],
            unavailable_servers: Vec::new(),
            deferred_loading: false,
            virtual_instructions: Vec::new(),
        };

        let instructions = build_gateway_instructions(&ctx).unwrap();
        assert!(instructions.contains("<my-mcp-server>"));
        assert!(instructions.contains("Some instructions."));
        assert!(instructions.contains("</my-mcp-server>"));
    }

    #[test]
    fn test_build_gateway_instructions_no_xml_when_no_content() {
        let ctx = InstructionsContext {
            servers: vec![McpServerInstructionInfo {
                name: "barebones".to_string(),
                instructions: None,
                description: None,
                tool_names: vec!["barebones__tool".to_string()],
                resource_names: Vec::new(),
                prompt_names: Vec::new(),
            }],
            unavailable_servers: Vec::new(),
            deferred_loading: false,
            virtual_instructions: Vec::new(),
        };

        let instructions = build_gateway_instructions(&ctx).unwrap();
        assert!(instructions.contains("`barebones__tool` (tool)"));
        assert!(!instructions.contains("<barebones>"));
    }

    #[test]
    fn test_build_gateway_instructions_with_resources_and_prompts() {
        let ctx = InstructionsContext {
            servers: vec![McpServerInstructionInfo {
                name: "knowledge".to_string(),
                instructions: None,
                description: Some("Knowledge base server".to_string()),
                tool_names: vec!["knowledge__search".to_string()],
                resource_names: vec!["knowledge__docs".to_string(), "knowledge__faq".to_string()],
                prompt_names: vec!["knowledge__summarize".to_string()],
            }],
            unavailable_servers: Vec::new(),
            deferred_loading: false,
            virtual_instructions: Vec::new(),
        };

        let instructions = build_gateway_instructions(&ctx).unwrap();
        assert!(instructions.contains("`knowledge__search` (tool)"));
        assert!(instructions.contains("`knowledge__docs` (resource)"));
        assert!(instructions.contains("`knowledge__faq` (resource)"));
        assert!(instructions.contains("`knowledge__summarize` (prompt)"));
    }

    // --- Snapshot-style tests ---

    #[test]
    fn test_full_instructions_snapshot() {
        use crate::gateway::virtual_server::VirtualInstructions;

        let ctx = InstructionsContext {
            servers: vec![
                McpServerInstructionInfo {
                    name: "filesystem".to_string(),
                    instructions: Some(
                        "Use read_file to read and write_file to write.".to_string(),
                    ),
                    description: Some("File operations server".to_string()),
                    tool_names: vec![
                        "filesystem__read_file".to_string(),
                        "filesystem__write_file".to_string(),
                    ],
                    resource_names: Vec::new(),
                    prompt_names: Vec::new(),
                },
                McpServerInstructionInfo {
                    name: "knowledge".to_string(),
                    instructions: None,
                    description: None,
                    tool_names: vec!["knowledge__search".to_string()],
                    resource_names: vec![
                        "knowledge__docs".to_string(),
                        "knowledge__faq".to_string(),
                    ],
                    prompt_names: vec!["knowledge__summarize".to_string()],
                },
            ],
            unavailable_servers: vec![UnavailableServerInfo {
                name: "broken-server".to_string(),
                error: "Connection refused".to_string(),
            }],
            deferred_loading: false,
            virtual_instructions: vec![
                VirtualInstructions {
                    section_title: "Skills".to_string(),
                    content: "Call a skill's `get_info` tool.\n\n- **code-review**: `skill_code_review_get_info` — Automated code review\n".to_string(),
                    tool_names: vec![
                        "skill_code_review_get_info".to_string(),
                        "skill_deploy_get_info".to_string(),
                    ],
                },
                VirtualInstructions {
                    section_title: "Marketplace".to_string(),
                    content: "Use marketplace tools to discover and install new MCP servers.\n".to_string(),
                    tool_names: vec![
                        "marketplace__search_mcp_servers".to_string(),
                        "marketplace__install_mcp_server".to_string(),
                    ],
                },
            ],
        };

        let instructions = build_gateway_instructions(&ctx).unwrap();

        // Virtual servers come first
        let skills_pos = instructions.find("**Skills**").unwrap();
        let marketplace_pos = instructions.find("**Marketplace**").unwrap();
        let filesystem_pos = instructions.find("**filesystem**").unwrap();
        let broken_pos = instructions.find("**broken-server**").unwrap();
        assert!(skills_pos < marketplace_pos);
        assert!(marketplace_pos < filesystem_pos);
        assert!(filesystem_pos < broken_pos);

        // Tool annotations
        assert!(instructions.contains("`skill_code_review_get_info` (tool)"));
        assert!(instructions.contains("`marketplace__search_mcp_servers` (tool)"));
        assert!(instructions.contains("`filesystem__read_file` (tool)"));
        assert!(instructions.contains("`knowledge__search` (tool)"));
        assert!(instructions.contains("`knowledge__docs` (resource)"));
        assert!(instructions.contains("`knowledge__summarize` (prompt)"));

        // XML instructions for virtual servers
        assert!(instructions.contains("<skills>"));
        assert!(instructions.contains("</skills>"));
        assert!(instructions.contains("<marketplace>"));
        assert!(instructions.contains("</marketplace>"));

        // XML instructions for regular servers
        assert!(instructions.contains("<filesystem>"));
        assert!(instructions.contains("</filesystem>"));
        // knowledge has no instructions/description, so no XML
        assert!(!instructions.contains("<knowledge>"));

        // Unavailable server
        assert!(instructions.contains("**broken-server** — unavailable: Connection refused"));
    }

    #[test]
    fn test_virtual_only_instructions_snapshot() {
        use crate::gateway::virtual_server::VirtualInstructions;

        let ctx = InstructionsContext {
            servers: Vec::new(),
            unavailable_servers: Vec::new(),
            deferred_loading: false,
            virtual_instructions: vec![VirtualInstructions {
                section_title: "Skills".to_string(),
                content: "Call get_info to unlock skills.\n".to_string(),
                tool_names: vec![
                    "skill_code_review_get_info".to_string(),
                    "skill_deploy_get_info".to_string(),
                ],
            }],
        };

        let instructions = build_gateway_instructions(&ctx).unwrap();
        assert!(instructions.contains("**Skills**"));
        assert!(instructions.contains("`skill_code_review_get_info` (tool)"));
        assert!(instructions.contains("`skill_deploy_get_info` (tool)"));
        assert!(instructions.contains("<skills>"));
        assert!(instructions.contains("Call get_info to unlock skills."));
        assert!(instructions.contains("</skills>"));
        // No regular server content
        assert!(!instructions.contains("servername__"));
    }

    #[test]
    fn test_deferred_instructions_snapshot() {
        use crate::gateway::virtual_server::VirtualInstructions;

        // Build a server with >20 tools to test truncation
        let tool_names: Vec<String> = (1..=25)
            .map(|i| format!("big-server__tool_{}", i))
            .collect();

        let ctx = InstructionsContext {
            servers: vec![
                McpServerInstructionInfo {
                    name: "big-server".to_string(),
                    instructions: Some("Detailed instructions for big server.".to_string()),
                    description: None,
                    tool_names,
                    resource_names: Vec::new(),
                    prompt_names: Vec::new(),
                },
                McpServerInstructionInfo {
                    name: "small".to_string(),
                    instructions: Some("Small server instructions.".to_string()),
                    description: None,
                    tool_names: vec!["small__tool_a".to_string(), "small__tool_b".to_string()],
                    resource_names: Vec::new(),
                    prompt_names: Vec::new(),
                },
            ],
            unavailable_servers: Vec::new(),
            deferred_loading: true,
            virtual_instructions: vec![VirtualInstructions {
                section_title: "Skills".to_string(),
                content: "Skill instructions always shown.\n".to_string(),
                tool_names: vec!["skill_test_get_info".to_string()],
            }],
        };

        let instructions = build_gateway_instructions(&ctx).unwrap();

        // Header mentions deferred
        assert!(instructions.contains("on demand"));
        assert!(instructions.contains("`search`"));
        assert!(instructions.contains("`server_info`"));

        // Virtual instructions always included
        assert!(instructions.contains("<skills>"));
        assert!(instructions.contains("Skill instructions always shown."));
        assert!(instructions.contains("</skills>"));

        // Regular server instructions omitted in deferred mode
        assert!(!instructions.contains("<big-server>"));
        assert!(!instructions.contains("Detailed instructions for big server."));
        assert!(!instructions.contains("<small>"));
        assert!(!instructions.contains("Small server instructions."));

        // Big server truncated with count summary
        assert!(instructions.contains("**big-server** (25 tools)"));
        assert!(instructions.contains("`big-server__tool_1` (tool)"));
        assert!(instructions.contains("`big-server__tool_20` (tool)"));
        // Tool 21 should NOT appear (past limit)
        assert!(!instructions.contains("`big-server__tool_21` (tool)"));
        // Truncation hint
        assert!(instructions.contains("use `search` to find more"));

        // Small server NOT truncated (only 2 tools, under limit)
        assert!(instructions.contains("**small**\n"));
        assert!(instructions.contains("`small__tool_a` (tool)"));
        assert!(instructions.contains("`small__tool_b` (tool)"));
    }

    #[test]
    fn test_merge_tools() {
        let tool1 = McpTool {
            name: "read_file".to_string(),
            description: Some("Read a file".to_string()),
            input_schema: json!({"type": "object"}),
        };

        let tool2 = McpTool {
            name: "create_issue".to_string(),
            description: Some("Create an issue".to_string()),
            input_schema: json!({"type": "object"}),
        };

        let server_tools = vec![
            ("filesystem".to_string(), vec![tool1]),
            ("github".to_string(), vec![tool2]),
        ];

        // Test without name mapping (uses server_id as-is)
        let merged = merge_tools(server_tools.clone(), &[], None);

        assert_eq!(merged.len(), 2);
        assert_eq!(merged[0].name, "filesystem__read_file");
        assert_eq!(merged[1].name, "github__create_issue");
        assert_eq!(merged[0].server_id, "filesystem");
        assert_eq!(merged[1].server_id, "github");
    }

    #[test]
    fn test_merge_tools_with_name_mapping() {
        let tool1 = McpTool {
            name: "read_file".to_string(),
            description: Some("Read a file".to_string()),
            input_schema: json!({"type": "object"}),
        };

        let tool2 = McpTool {
            name: "create_issue".to_string(),
            description: Some("Create an issue".to_string()),
            input_schema: json!({"type": "object"}),
        };

        let server_tools = vec![
            ("uuid-123-abc".to_string(), vec![tool1]),
            ("uuid-456-def".to_string(), vec![tool2]),
        ];

        // Create name mapping (UUID -> human-readable name)
        let mut name_map = std::collections::HashMap::new();
        name_map.insert("uuid-123-abc".to_string(), "filesystem".to_string());
        name_map.insert("uuid-456-def".to_string(), "github".to_string());

        let merged = merge_tools(server_tools, &[], Some(&name_map));

        assert_eq!(merged.len(), 2);
        // Display name uses human-readable name
        assert_eq!(merged[0].name, "filesystem__read_file");
        assert_eq!(merged[1].name, "github__create_issue");
        // But server_id still contains UUID for routing
        assert_eq!(merged[0].server_id, "uuid-123-abc");
        assert_eq!(merged[1].server_id, "uuid-456-def");
    }
}
