use serde_json::json;

use super::types::*;
use crate::protocol::{McpPrompt, McpResource, McpTool};

/// An MCP server's info for instruction building (using human-readable names, not UUIDs)
#[derive(Clone)]
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
#[derive(Clone)]
pub struct UnavailableServerInfo {
    /// Human-readable name
    pub name: String,
    /// Error message
    pub error: String,
}

/// Context for building gateway instructions
#[derive(Clone)]
pub struct InstructionsContext {
    /// Available MCP servers with their info
    pub servers: Vec<McpServerInstructionInfo>,
    /// Unavailable MCP servers
    pub unavailable_servers: Vec<UnavailableServerInfo>,
    /// Whether context management is enabled
    pub context_management_enabled: bool,
    /// Catalog compression plan (computed when context management is enabled)
    pub catalog_compression: Option<CatalogCompressionPlan>,
    /// Instructions from virtual servers
    pub virtual_instructions: Vec<super::virtual_server::VirtualInstructions>,
    /// Configured search tool name (for search hints in instructions)
    pub search_tool_name: String,
    /// Byte size of each tool/resource/prompt definition as serialized JSON.
    /// Key is the namespaced name (e.g., "filesystem__read_file").
    pub item_definition_sizes: std::collections::HashMap<String, usize>,
}

impl Default for InstructionsContext {
    fn default() -> Self {
        Self {
            servers: Vec::new(),
            unavailable_servers: Vec::new(),
            context_management_enabled: false,
            catalog_compression: None,
            virtual_instructions: Vec::new(),
            search_tool_name: "IndexSearch".to_string(),
            item_definition_sizes: std::collections::HashMap::new(),
        }
    }
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
/// 3. Instructions in XML tags for virtual and regular servers
pub fn build_gateway_instructions(ctx: &InstructionsContext) -> Option<String> {
    let has_servers = !ctx.servers.is_empty();
    let has_unavailable = !ctx.unavailable_servers.is_empty();
    let has_virtual = !ctx.virtual_instructions.is_empty();

    // Nothing to describe
    if !has_servers && !has_unavailable && !has_virtual {
        return None;
    }

    // Context management path — uses compression plan
    if ctx.context_management_enabled {
        return build_context_managed_instructions(ctx);
    }

    let mut inst = String::new();

    // --- 1. Header ---
    build_header(&mut inst, has_servers, has_virtual);

    // --- 2. Unified per-server XML blocks ---
    build_unified_server_blocks(&mut inst, ctx);

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

/// Build the header line based on what's available.
fn build_header(inst: &mut String, has_mcp_servers: bool, has_virtual: bool) {
    if !has_mcp_servers && !has_virtual {
        // Only unavailable servers — nothing usable
        inst.push_str("Unified MCP Gateway: no servers or tools are currently available.\n\n");
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

/// Build all servers as unified XML blocks: virtual servers first, then regular, then unavailable.
fn build_unified_server_blocks(inst: &mut String, ctx: &InstructionsContext) {
    // --- Virtual servers (always full, never compressed) ---
    for vsi in &ctx.virtual_instructions {
        let tag = slugify(&vsi.section_title);
        inst.push_str(&format!("<{}>\n", tag));
        for name in &vsi.tool_names {
            inst.push_str(&format!("- `{}` (tool)\n", name));
        }
        if !vsi.content.is_empty() {
            inst.push('\n');
            inst.push_str(&vsi.content);
            if !vsi.content.ends_with('\n') {
                inst.push('\n');
            }
        }
        inst.push_str(&format!("</{}>\n\n", tag));
    }

    // --- Regular MCP servers ---
    for server in &ctx.servers {
        let tag = slugify(&server.name);
        let total_items =
            server.tool_names.len() + server.resource_names.len() + server.prompt_names.len();
        let has_content = server.instructions.is_some() || server.description.is_some();

        if total_items == 0 && !has_content {
            inst.push_str(&format!("**{}** (no capabilities)\n\n", server.name));
            continue;
        }

        inst.push_str(&format!("<{}>\n", tag));

        if let Some(desc) = &server.description {
            inst.push_str(desc);
            if !desc.ends_with('\n') {
                inst.push('\n');
            }
        }

        if let Some(instructions) = &server.instructions {
            inst.push('\n');
            inst.push_str(instructions);
            if !instructions.ends_with('\n') {
                inst.push('\n');
            }
        }

        inst.push_str(&format!("</{}>\n\n", tag));
    }

    // --- Unavailable servers (no XML block, just bold + error) ---
    for server in &ctx.unavailable_servers {
        inst.push_str(&format!(
            "**{}** — unavailable: {}\n\n",
            server.name, server.error
        ));
    }
}

// ─── Context Management: Compression + Welcome Text ────────────────────────

/// Build welcome text when context management is enabled.
/// Applies the catalog compression plan to produce compressed output.
/// Uses unified per-server XML blocks with 4 compression phases.
fn build_context_managed_instructions(ctx: &InstructionsContext) -> Option<String> {
    let mut inst = String::new();

    // Header
    let server_count = ctx.servers.len();
    if server_count == 0 && ctx.virtual_instructions.is_empty() {
        inst.push_str("Unified MCP Gateway: no servers or tools are currently available.\n\n");
    } else {
        inst.push_str(
            "Unified MCP Gateway. Tools from MCP servers are namespaced \
             with a `servername__` prefix.\n",
        );
        if server_count > 0 {
            inst.push_str(&format!(
                "Use {} to discover capabilities and retrieve compressed content.\n",
                ctx.search_tool_name
            ));
        }
        inst.push('\n');
    }

    let plan = ctx.catalog_compression.as_ref();

    // Build lookup sets from the new plan structure
    let indexed_welcome_slugs: std::collections::HashMap<&str, &IndexedWelcome> = plan
        .map(|p| {
            p.indexed_welcomes
                .iter()
                .map(|w| (w.server_slug.as_str(), w))
                .collect()
        })
        .unwrap_or_default();
    let deferred_server_slugs: std::collections::HashMap<&str, &DeferredServer> = plan
        .map(|p| {
            p.deferred_servers
                .iter()
                .map(|d| (d.server_slug.as_str(), d))
                .collect()
        })
        .unwrap_or_default();
    let welcome_toc_dropped: std::collections::HashSet<&str> = plan
        .map(|p| p.welcome_toc_dropped.iter().map(|s| s.as_str()).collect())
        .unwrap_or_default();
    let batch_toc_dropped: std::collections::HashSet<&str> = plan
        .map(|p| p.batch_toc_dropped.iter().map(|s| s.as_str()).collect())
        .unwrap_or_default();

    // Virtual servers first (always full, never compressed) — in XML blocks
    for vsi in &ctx.virtual_instructions {
        let tag = slugify(&vsi.section_title);
        inst.push_str(&format!("<{}>\n", tag));
        for name in &vsi.tool_names {
            inst.push_str(&format!("- `{}` (tool)\n", name));
        }
        if !vsi.content.is_empty() {
            inst.push('\n');
            inst.push_str(&vsi.content);
            if !vsi.content.ends_with('\n') {
                inst.push('\n');
            }
        }
        inst.push_str(&format!("</{}>\n\n", tag));
    }

    // Regular MCP servers — in XML blocks with compression phases
    for server in &ctx.servers {
        let server_slug = slugify(&server.name);
        let slug_str = server_slug.as_str();

        inst.push_str(&format!("<{}>\n", server_slug));

        let has_indexed_welcome = indexed_welcome_slugs.contains_key(slug_str);
        let has_deferred = deferred_server_slugs.contains_key(slug_str);
        let toc_dropped = welcome_toc_dropped.contains(slug_str);
        let batch_toc_drop = batch_toc_dropped.contains(slug_str);

        // Welcome section
        if has_indexed_welcome {
            let welcome = indexed_welcome_slugs[slug_str];
            inst.push_str(&welcome.summary);
            inst.push('\n');
            if !toc_dropped {
                inst.push('\n');
                inst.push_str(&welcome.toc);
                inst.push('\n');
            }
        } else {
            // No compression: raw description + instructions (no tool listing)
            let has_content = server.instructions.is_some() || server.description.is_some();
            if has_content {
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
            }
        }

        // Batch section (Phase 2 output)
        if has_deferred {
            let deferred = deferred_server_slugs[slug_str];
            for batch in &deferred.batches {
                inst.push('\n');
                inst.push_str(&batch.batch_summary);
                inst.push('\n');
                if !batch_toc_drop {
                    inst.push('\n');
                    inst.push_str(&batch.batch_toc);
                    inst.push('\n');
                }
            }
        }

        inst.push_str(&format!("</{}>\n\n", server_slug));
    }

    // Unavailable servers (no XML block)
    for server in &ctx.unavailable_servers {
        inst.push_str(&format!(
            "**{}** — unavailable: {}\n\n",
            server.name, server.error
        ));
    }

    Some(inst)
}

/// Build the full content for a server — used for FTS5 indexing of compressed content.
/// Combines the tool listing + description + instructions into a single string.
pub fn build_full_server_content(server: &McpServerInstructionInfo) -> String {
    let mut content = String::new();

    // Server name and capabilities
    content.push_str(&format!("Server: {}\n", server.name));

    if let Some(desc) = &server.description {
        content.push_str(desc);
        content.push('\n');
    }

    if let Some(instructions) = &server.instructions {
        content.push_str(instructions);
        content.push('\n');
    }

    // Tool listing
    if !server.tool_names.is_empty() {
        content.push_str("\nTools:\n");
        for name in &server.tool_names {
            content.push_str(&format!("- {}\n", name));
        }
    }

    if !server.resource_names.is_empty() {
        content.push_str("\nResources:\n");
        for name in &server.resource_names {
            content.push_str(&format!("- {}\n", name));
        }
    }

    if !server.prompt_names.is_empty() {
        content.push_str("\nPrompts:\n");
        for name in &server.prompt_names {
            content.push_str(&format!("- {}\n", name));
        }
    }

    content
}

/// Compute item definition sizes: `serde_json::to_value(item).to_string().len()` per item.
pub fn compute_item_definition_sizes(
    tools: &[NamespacedTool],
    resources: &[NamespacedResource],
    prompts: &[NamespacedPrompt],
) -> std::collections::HashMap<String, usize> {
    let mut sizes = std::collections::HashMap::new();
    for tool in tools {
        let size = serde_json::to_value(tool)
            .map(|v| v.to_string().len())
            .unwrap_or(0);
        sizes.insert(tool.name.clone(), size);
    }
    for resource in resources {
        let size = serde_json::to_value(resource)
            .map(|v| v.to_string().len())
            .unwrap_or(0);
        sizes.insert(resource.name.clone(), size);
    }
    for prompt in prompts {
        let size = serde_json::to_value(prompt)
            .map(|v| v.to_string().len())
            .unwrap_or(0);
        sizes.insert(prompt.name.clone(), size);
    }
    sizes
}

/// Format a tool definition as markdown for indexing.
/// Includes description + all property descriptions + enum values.
pub fn format_tool_as_markdown(tool: &NamespacedTool) -> String {
    let mut md = format!("# {}\n\n", tool.name);

    if let Some(desc) = &tool.description {
        md.push_str(desc);
        md.push_str("\n\n");
    }

    // Extract properties from input_schema
    if let Some(props) = tool
        .input_schema
        .get("properties")
        .and_then(|p| p.as_object())
    {
        let required: std::collections::HashSet<&str> = tool
            .input_schema
            .get("required")
            .and_then(|r| r.as_array())
            .map(|arr| arr.iter().filter_map(|v| v.as_str()).collect())
            .unwrap_or_default();

        md.push_str("## Parameters\n");
        for (name, schema) in props {
            let type_str = schema.get("type").and_then(|t| t.as_str()).unwrap_or("any");
            let is_required = required.contains(name.as_str());
            let req_str = if is_required { ", required" } else { "" };

            md.push_str(&format!("- **{}** ({}{})", name, type_str, req_str));

            if let Some(desc) = schema.get("description").and_then(|d| d.as_str()) {
                md.push_str(&format!(": {}", desc));
            }

            if let Some(enum_vals) = schema.get("enum").and_then(|e| e.as_array()) {
                let vals: Vec<String> = enum_vals
                    .iter()
                    .filter_map(|v| v.as_str().map(|s| s.to_string()))
                    .collect();
                if !vals.is_empty() {
                    md.push_str(&format!(". Options: {}", vals.join(", ")));
                }
            }

            md.push('\n');
        }
    }

    md
}

/// Format a resource as markdown for indexing.
pub fn format_resource_as_markdown(resource: &NamespacedResource) -> String {
    let mut md = format!("# {}\n\n", resource.name);
    md.push_str(&format!("URI: {}\n", resource.uri));
    if let Some(desc) = &resource.description {
        md.push_str(desc);
        md.push('\n');
    }
    if let Some(mime) = &resource.mime_type {
        md.push_str(&format!("MIME type: {}\n", mime));
    }
    md
}

/// Format a prompt as markdown for indexing.
pub fn format_prompt_as_markdown(prompt: &NamespacedPrompt) -> String {
    let mut md = format!("# {}\n\n", prompt.name);
    if let Some(desc) = &prompt.description {
        md.push_str(desc);
        md.push_str("\n\n");
    }
    if let Some(args) = &prompt.arguments {
        md.push_str("## Arguments\n");
        for arg in args {
            let req_str = if arg.required.unwrap_or(false) {
                ", required"
            } else {
                ""
            };
            md.push_str(&format!("- **{}**{}", arg.name, req_str));
            if let Some(desc) = &arg.description {
                md.push_str(&format!(": {}", desc));
            }
            md.push('\n');
        }
    }
    md
}

/// Compress a tool definition in-place: replace description with index summary,
/// strip property descriptions from inputSchema.
pub fn compress_tool_definition(
    tool: &NamespacedTool,
    summary: &str,
    include_toc: bool,
    toc: &str,
) -> NamespacedTool {
    let mut compressed = tool.clone();

    // Replace description with index summary + optional TOC
    if include_toc && !toc.is_empty() {
        compressed.description = Some(format!("{}\n{}", summary, toc));
    } else {
        compressed.description = Some(summary.to_string());
    }

    // Strip descriptions from inputSchema properties
    if let Some(props) = compressed
        .input_schema
        .get_mut("properties")
        .and_then(|p| p.as_object_mut())
    {
        for (_name, schema) in props.iter_mut() {
            if let Some(obj) = schema.as_object_mut() {
                obj.remove("description");
            }
        }
    }

    compressed
}

/// Build a mock `InstructionsContext` for the compression preview UI.
/// Contains realistic hardcoded data so the preview is meaningful without
/// a live MCP session.
pub fn build_preview_instructions_context() -> InstructionsContext {
    use super::virtual_server::VirtualInstructions;

    InstructionsContext {
        servers: vec![
            McpServerInstructionInfo {
                name: "Filesystem".to_string(),
                description: Some("Provides filesystem operations for reading, writing, and managing files and directories on the local system.".to_string()),
                instructions: Some("Use read_file to read files, write_file to write, list_directory to browse, and search_files for content search. Always use absolute paths.".to_string()),
                tool_names: vec![
                    "filesystem__read_file".to_string(),
                    "filesystem__write_file".to_string(),
                    "filesystem__list_directory".to_string(),
                    "filesystem__search_files".to_string(),
                    "filesystem__get_file_info".to_string(),
                    "filesystem__create_directory".to_string(),
                    "filesystem__move_file".to_string(),
                    "filesystem__delete_file".to_string(),
                ],
                resource_names: vec!["filesystem__cwd".to_string()],
                prompt_names: Vec::new(),
            },
            McpServerInstructionInfo {
                name: "GitHub".to_string(),
                description: Some("GitHub API integration for managing repositories, issues, pull requests, and code.".to_string()),
                instructions: None,
                tool_names: vec![
                    "github__create_issue".to_string(),
                    "github__list_issues".to_string(),
                    "github__create_pull_request".to_string(),
                    "github__get_file_contents".to_string(),
                    "github__search_code".to_string(),
                    "github__list_repos".to_string(),
                ],
                resource_names: Vec::new(),
                prompt_names: vec!["github__review_pr".to_string()],
            },
            McpServerInstructionInfo {
                name: "Database".to_string(),
                description: Some("PostgreSQL database access for querying and managing data.".to_string()),
                instructions: Some("Use query for SELECT operations and execute for INSERT/UPDATE/DELETE. Always use parameterized queries to prevent SQL injection.".to_string()),
                tool_names: vec![
                    "database__query".to_string(),
                    "database__execute".to_string(),
                    "database__list_tables".to_string(),
                    "database__describe_table".to_string(),
                ],
                resource_names: vec!["database__schema".to_string()],
                prompt_names: Vec::new(),
            },
        ],
        unavailable_servers: vec![UnavailableServerInfo {
            name: "Slack".to_string(),
            error: "Connection refused".to_string(),
        }],
        context_management_enabled: true,
        catalog_compression: None, // computed by caller
        virtual_instructions: vec![
            VirtualInstructions {
                section_title: "Context Management".to_string(),
                content: "Use IndexSearch to discover MCP capabilities and retrieve compressed content.".to_string(),
                tool_names: vec![
                    "IndexSearch".to_string(),
                    "ctx_execute".to_string(),
                    "ctx_batch_execute".to_string(),
                    "ctx_index".to_string(),
                    "ctx_fetch_and_index".to_string(),
                ],
                priority: 0,
            },
            VirtualInstructions {
                section_title: "Coding Agents".to_string(),
                content: "You have access to **Claude Code** as a coding agent. Use the unified tools: `AgentStart`, `AgentSay`, `AgentStatus`, `AgentList`.\n".to_string(),
                tool_names: vec![
                    "AgentStart".to_string(),
                    "AgentSay".to_string(),
                    "AgentStatus".to_string(),
                    "AgentList".to_string(),
                ],
                priority: 10,
            },
            VirtualInstructions {
                section_title: "Marketplace".to_string(),
                content: "Use marketplace tools to discover and install new MCP servers and skills.\n".to_string(),
                tool_names: vec![
                    "marketplace_search".to_string(),
                    "marketplace_install".to_string(),
                ],
                priority: 20,
            },
            VirtualInstructions {
                section_title: "Skills".to_string(),
                content: "Call a skill's `get_info` tool to view its instructions.\n".to_string(),
                tool_names: vec![
                    "skill_get_info".to_string(),
                ],
                priority: 30,
            },
        ],
        ..Default::default()
    }
}

/// Standard virtual instructions shared across all preview presets.
fn preview_virtual_instructions() -> Vec<super::virtual_server::VirtualInstructions> {
    use super::virtual_server::VirtualInstructions;
    vec![
        VirtualInstructions {
            section_title: "Context Management".to_string(),
            content: "Use IndexSearch to discover MCP capabilities and retrieve compressed content.".to_string(),
            tool_names: vec![
                "IndexSearch".to_string(),
                "ctx_execute".to_string(),
                "ctx_batch_execute".to_string(),
                "ctx_index".to_string(),
                "ctx_fetch_and_index".to_string(),
            ],
            priority: 0,
        },
        VirtualInstructions {
            section_title: "Coding Agents".to_string(),
            content: "You have access to **Claude Code** as a coding agent. Use the unified tools: `coding_agent_start`, `coding_agent_say`, `coding_agent_status`.\n".to_string(),
            tool_names: vec![
                "coding_agent_start".to_string(),
                "coding_agent_say".to_string(),
                "coding_agent_status".to_string(),
                "coding_agent_respond".to_string(),
                "coding_agent_interrupt".to_string(),
                "coding_agent_list".to_string(),
            ],
            priority: 10,
        },
        VirtualInstructions {
            section_title: "Marketplace".to_string(),
            content: "Use marketplace tools to discover and install new MCP servers and skills.\n".to_string(),
            tool_names: vec![
                "marketplace_search".to_string(),
                "marketplace_install".to_string(),
            ],
            priority: 20,
        },
        VirtualInstructions {
            section_title: "Skills".to_string(),
            content: "Call a skill's `get_info` tool to view its instructions.\n".to_string(),
            tool_names: vec![
                "skill_get_info".to_string(),
            ],
            priority: 30,
        },
    ]
}

/// Build a realistic mock `InstructionsContext` for the compression preview UI.
/// Modeled after real MCP servers (GitHub, Atlassian, Filesystem, PostgreSQL, Slack)
/// with verbose multi-line descriptions and instructions like real servers have.
pub fn build_preview_mock_realistic() -> InstructionsContext {
    InstructionsContext {
        servers: vec![
            // -- GitHub MCP Server (modeled after github/github-mcp-server) --
            McpServerInstructionInfo {
                name: "GitHub".to_string(),
                description: Some(
                    "GitHub's official MCP server for repository management, issues, pull requests, \
                     code search, actions workflows, and code security scanning. Provides full \
                     read/write access to the authenticated user's repositories and organizations."
                        .to_string(),
                ),
                instructions: Some(
                    "## Issues\n\
                     - Use `github__issue_read` with method='get' to get issue details, method='get_comments' for \
                     comments, method='get_sub_issues' for sub-issues, or method='get_labels' for labels.\n\
                     - Use `github__issue_write` to create or update issues. Always set the `method` parameter:\n\
                       - 'create' — create a new issue (requires owner, repo, title)\n\
                       - 'update' — update an existing issue (requires owner, repo, issue_number)\n\
                       - 'close' — close an issue\n\
                       - 'reopen' — reopen a closed issue\n\
                     - Use `github__add_issue_comment` to add a comment. This also works for pull requests \
                     (pass the PR number as issue_number), but only if the user is not asking specifically \
                     to add review comments.\n\n\
                     ## Pull Requests\n\
                     - Use `github__pull_request_read` to get PR data. The `method` parameter controls what data:\n\
                       1. 'get' — Get details of a specific pull request\n\
                       2. 'get_diff' — Get the diff of a pull request\n\
                       3. 'get_status' — Get combined commit status of the head commit\n\
                       4. 'get_files' — Get the list of files changed. Use pagination to control results.\n\
                       5. 'get_reviews' — Get reviews on a pull request\n\
                       6. 'get_review_comments' — Get review comments on a pull request\n\
                     - `github__create_pull_request` creates a new PR. Requires owner, repo, title, head, base.\n\
                     - `github__update_pull_request` modifies title, body, state, base, or maintainer_can_modify.\n\
                     - `github__merge_pull_request` merges via 'merge', 'squash', or 'rebase' method.\n\n\
                     ## Repository & Code\n\
                     - `github__get_file_contents` retrieves file/directory contents. For directories it returns \
                     a listing; for files it returns the content. Always specify the `ref` parameter for \
                     branch-specific reads.\n\
                     - `github__create_or_update_file` creates or updates a single file. If updating, provide the \
                     SHA of the existing file. To obtain the SHA: `git rev-parse <branch>:<path>`.\n\
                     - `github__push_files` commits and pushes multiple files in a single commit.\n\
                     - `github__search_code` searches code across repositories using GitHub code search syntax.\n\n\
                     ## Actions\n\
                     - `github__list_workflow_runs` lists workflow runs with optional filtering by status, branch, \
                     or event type. Returns at least 30 results per page.\n\
                     - `github__get_workflow_run_logs` retrieves logs for a specific run (may be large).\n\
                     - `github__rerun_workflow` re-runs a failed or completed workflow run.\n"
                        .to_string(),
                ),
                tool_names: vec![
                    "github__issue_read".to_string(),
                    "github__issue_write".to_string(),
                    "github__search_issues".to_string(),
                    "github__list_issues".to_string(),
                    "github__add_issue_comment".to_string(),
                    "github__sub_issue_write".to_string(),
                    "github__pull_request_read".to_string(),
                    "github__create_pull_request".to_string(),
                    "github__update_pull_request".to_string(),
                    "github__merge_pull_request".to_string(),
                    "github__add_pull_request_review_comment".to_string(),
                    "github__get_file_contents".to_string(),
                    "github__create_or_update_file".to_string(),
                    "github__push_files".to_string(),
                    "github__search_code".to_string(),
                    "github__search_repositories".to_string(),
                    "github__list_commits".to_string(),
                    "github__list_branches".to_string(),
                    "github__create_branch".to_string(),
                    "github__list_workflow_runs".to_string(),
                    "github__get_workflow_run_logs".to_string(),
                    "github__rerun_workflow".to_string(),
                    "github__get_code_scanning_alerts".to_string(),
                    "github__get_me".to_string(),
                ],
                resource_names: Vec::new(),
                prompt_names: Vec::new(),
            },
            // -- Atlassian MCP Server (Jira + Confluence) --
            McpServerInstructionInfo {
                name: "Atlassian".to_string(),
                description: Some(
                    "Atlassian Cloud integration providing access to Jira (project tracking, issues, sprints, \
                     workflows) and Confluence (wiki pages, spaces, comments). Supports both read and write \
                     operations across all accessible Atlassian Cloud sites."
                        .to_string(),
                ),
                instructions: Some(
                    "## Jira\n\
                     - Use `atlassian__searchJiraIssuesUsingJql` to find issues. JQL examples:\n\
                       - `project = PROJ AND status = 'In Progress'`\n\
                       - `assignee = currentUser() AND resolution = Unresolved ORDER BY priority DESC`\n\
                       - `labels in (bug, critical) AND created >= -7d`\n\
                     - `atlassian__getJiraIssue` retrieves a single issue by key (e.g., 'PROJ-123'). Returns \
                     fields including summary, description, status, assignee, priority, labels, components, \
                     fix versions, and custom fields.\n\
                     - `atlassian__createJiraIssue` creates an issue. Required: project key, issue type, summary. \
                     Use `atlassian__getJiraProjectIssueTypesMetadata` first to discover valid issue types and \
                     required fields for the target project.\n\
                     - `atlassian__editJiraIssue` updates issue fields. Pass only the fields you want to change.\n\
                     - `atlassian__transitionJiraIssue` moves an issue through workflow states. Use \
                     `atlassian__getTransitionsForJiraIssue` first to discover available transitions from \
                     the current state.\n\
                     - `atlassian__addCommentToJiraIssue` adds a comment using Atlassian Document Format (ADF).\n\
                     - `atlassian__addWorklogToJiraIssue` logs time spent. Specify timeSpentSeconds or use \
                     timeSpent string format (e.g., '2h 30m').\n\n\
                     ## Confluence\n\
                     - `atlassian__searchConfluenceUsingCql` searches pages/blogs using CQL. Examples:\n\
                       - `type = page AND space = DEV AND text ~ 'architecture'`\n\
                       - `creator = currentUser() AND lastModified > now('-7d')`\n\
                     - `atlassian__getConfluencePage` retrieves page content in 'storage' (raw HTML) or 'atlas_doc_format' \
                     (structured JSON). For reading, prefer 'atlas_doc_format'.\n\
                     - `atlassian__createConfluencePage` creates a page. Requires spaceId, title, and body in either \
                     'storage' or 'atlas_doc_format'. Set parentId to nest under an existing page.\n\
                     - `atlassian__updateConfluencePage` updates page content. You must pass the current version \
                     number (from getConfluencePage) to avoid conflicts.\n\
                     - Comments: Use `atlassian__createConfluenceFooterComment` for page-level comments and \
                     `atlassian__createConfluenceInlineComment` for inline annotations on specific content.\n"
                        .to_string(),
                ),
                tool_names: vec![
                    "atlassian__getJiraIssue".to_string(),
                    "atlassian__createJiraIssue".to_string(),
                    "atlassian__editJiraIssue".to_string(),
                    "atlassian__searchJiraIssuesUsingJql".to_string(),
                    "atlassian__transitionJiraIssue".to_string(),
                    "atlassian__getTransitionsForJiraIssue".to_string(),
                    "atlassian__addCommentToJiraIssue".to_string(),
                    "atlassian__addWorklogToJiraIssue".to_string(),
                    "atlassian__getJiraProjectIssueTypesMetadata".to_string(),
                    "atlassian__lookupJiraAccountId".to_string(),
                    "atlassian__getConfluencePage".to_string(),
                    "atlassian__createConfluencePage".to_string(),
                    "atlassian__updateConfluencePage".to_string(),
                    "atlassian__searchConfluenceUsingCql".to_string(),
                    "atlassian__getConfluenceSpaces".to_string(),
                    "atlassian__createConfluenceFooterComment".to_string(),
                    "atlassian__createConfluenceInlineComment".to_string(),
                    "atlassian__getConfluencePageDescendants".to_string(),
                ],
                resource_names: Vec::new(),
                prompt_names: Vec::new(),
            },
            // -- Filesystem MCP Server --
            McpServerInstructionInfo {
                name: "Filesystem".to_string(),
                description: Some(
                    "Secure filesystem operations with configurable access controls. Provides tools for reading, \
                     writing, creating, moving, and searching files and directories within allowed paths. All \
                     operations are sandboxed to the configured root directories."
                        .to_string(),
                ),
                instructions: Some(
                    "- `filesystem__read_file` reads the complete contents of a file. Returns text content \
                     with UTF-8 encoding. For binary files, returns base64-encoded content.\n\
                     - `filesystem__read_multiple_files` reads several files at once. More efficient than \
                     multiple individual read_file calls. Returns results in the same order as requested paths.\n\
                     - `filesystem__write_file` creates or overwrites a file. Creates parent directories if \
                     they don't exist. Content must be a string (use base64 for binary).\n\
                     - `filesystem__edit_file` applies targeted edits using a diff-like format. Supports \
                     multiple edits in a single call. Each edit specifies oldText (must match exactly) and \
                     newText. More reliable than write_file for partial modifications.\n\
                     - `filesystem__create_directory` creates a directory (and parents). No error if it exists.\n\
                     - `filesystem__list_directory` lists entries in a directory. Returns [FILE] or [DIR] prefix \
                     for each entry. Does not recurse into subdirectories.\n\
                     - `filesystem__directory_tree` returns a recursive tree structure of a directory up to a \
                     configurable depth. Useful for understanding project layout.\n\
                     - `filesystem__move_file` moves or renames a file or directory. Fails if destination exists.\n\
                     - `filesystem__search_files` searches for files matching a glob pattern (e.g., '**/*.ts'). \
                     Searches recursively from the given path.\n\
                     - `filesystem__get_file_info` returns metadata: size, creation time, modification time, \
                     permissions, and whether the path is a file or directory.\n"
                        .to_string(),
                ),
                tool_names: vec![
                    "filesystem__read_file".to_string(),
                    "filesystem__read_multiple_files".to_string(),
                    "filesystem__write_file".to_string(),
                    "filesystem__edit_file".to_string(),
                    "filesystem__create_directory".to_string(),
                    "filesystem__list_directory".to_string(),
                    "filesystem__directory_tree".to_string(),
                    "filesystem__move_file".to_string(),
                    "filesystem__search_files".to_string(),
                    "filesystem__get_file_info".to_string(),
                ],
                resource_names: Vec::new(),
                prompt_names: Vec::new(),
            },
            // -- PostgreSQL MCP Server --
            McpServerInstructionInfo {
                name: "PostgreSQL".to_string(),
                description: Some(
                    "PostgreSQL database integration for executing queries, managing schemas, analyzing \
                     query performance, and browsing database structure. Connected to the project's \
                     development database with read-write access."
                        .to_string(),
                ),
                instructions: Some(
                    "- `postgres__query` executes a read-only SQL query (SELECT, EXPLAIN, SHOW). Returns \
                     results as JSON rows. Use LIMIT to avoid returning excessive data. Parameterized \
                     queries are supported: pass `params` as an array of values and use $1, $2, etc.\n\
                     - `postgres__execute` runs a write SQL statement (INSERT, UPDATE, DELETE, CREATE, ALTER, \
                     DROP). Returns the number of affected rows. Always use parameterized queries for \
                     user-provided values to prevent SQL injection.\n\
                     - `postgres__list_schemas` returns all schemas in the database with their descriptions.\n\
                     - `postgres__list_tables` lists tables in a schema with row counts and descriptions. \
                     Defaults to the 'public' schema if not specified.\n\
                     - `postgres__describe_table` returns column definitions (name, type, nullable, default, \
                     constraints) for a given table. Also shows indexes, foreign keys, and check constraints.\n\
                     - `postgres__explain_query` runs EXPLAIN ANALYZE on a query and returns the execution \
                     plan. Useful for optimizing slow queries. The query is executed within a rolled-back \
                     transaction, so no data is modified.\n"
                        .to_string(),
                ),
                tool_names: vec![
                    "postgres__query".to_string(),
                    "postgres__execute".to_string(),
                    "postgres__list_schemas".to_string(),
                    "postgres__list_tables".to_string(),
                    "postgres__describe_table".to_string(),
                    "postgres__explain_query".to_string(),
                ],
                resource_names: vec![
                    "postgres__schema://public".to_string(),
                ],
                prompt_names: Vec::new(),
            },
            // -- Slack MCP Server --
            McpServerInstructionInfo {
                name: "Slack".to_string(),
                description: Some(
                    "Slack workspace integration for messaging, channel management, user lookups, \
                     file sharing, and conversation search. Operates in the authenticated user's \
                     workspace with permissions scoped to their access level."
                        .to_string(),
                ),
                instructions: Some(
                    "- `slack__send_message` posts a message to a channel or DM. Use the channel ID (not name). \
                     Supports Slack mrkdwn formatting. For threads, include `thread_ts` parameter.\n\
                     - `slack__list_channels` returns workspace channels with IDs, names, topics, and member \
                     counts. Use `types` parameter to filter: 'public_channel', 'private_channel', 'im', 'mpim'. \
                     Results are paginated — use `cursor` parameter for subsequent pages.\n\
                     - `slack__search_messages` performs a full-text search across messages the user has access to. \
                     Supports Slack search modifiers: `in:#channel`, `from:@user`, `before:2024-01-01`, \
                     `has:link`, `has:reaction`. Returns message text, channel, timestamp, and permalink.\n\
                     - `slack__get_thread` retrieves all replies in a thread given a channel ID and thread \
                     timestamp. Returns messages in chronological order.\n\
                     - `slack__get_channel_history` fetches recent messages from a channel. Use `oldest` and \
                     `latest` parameters (Unix timestamps) to specify a time range.\n\
                     - `slack__get_users` lists workspace members with display names, real names, email, \
                     and status. Use to resolve user IDs for mentions.\n\
                     - `slack__add_reaction` adds an emoji reaction to a message. Requires channel and timestamp.\n\
                     - `slack__upload_file` uploads a file to a channel with optional initial comment.\n"
                        .to_string(),
                ),
                tool_names: vec![
                    "slack__send_message".to_string(),
                    "slack__list_channels".to_string(),
                    "slack__search_messages".to_string(),
                    "slack__get_thread".to_string(),
                    "slack__get_channel_history".to_string(),
                    "slack__get_users".to_string(),
                    "slack__add_reaction".to_string(),
                    "slack__upload_file".to_string(),
                ],
                resource_names: Vec::new(),
                prompt_names: Vec::new(),
            },
        ],
        unavailable_servers: vec![
            UnavailableServerInfo {
                name: "Sentry".to_string(),
                error: "Connection refused — is the Sentry MCP server running?".to_string(),
            },
        ],
        context_management_enabled: true,
        catalog_compression: None,
        virtual_instructions: preview_virtual_instructions(),
        ..Default::default()
    }
}

/// Build a mock tool catalog for the compression preview UI.
/// Returns NamespacedTool entries with verbose descriptions and inputSchemas
/// for all tools in the realistic mock context, so the compression preview
/// can display full tool properties even when no real servers are running.
pub fn build_preview_mock_tool_catalog() -> Vec<NamespacedTool> {
    fn tool(server: &str, name: &str, desc: &str, schema: serde_json::Value) -> NamespacedTool {
        NamespacedTool {
            name: format!("{}__{}", server, name),
            original_name: name.to_string(),
            server_id: server.to_string(),
            description: Some(desc.to_string()),
            input_schema: schema,
        }
    }

    vec![
        // ── GitHub ──────────────────────────────────────────────────────
        tool("github", "issue_read", "Read detailed information about a GitHub issue including its title, body, labels, assignees, milestone, and timeline events. Supports multiple read methods: 'get' for full issue details, 'get_comments' for all comments on the issue, 'get_sub_issues' for linked sub-issues, and 'get_labels' for available labels. Returns comprehensive issue metadata including creation date, last update, state reason, and linked pull requests.", json!({
            "type": "object",
            "properties": {
                "owner": { "type": "string", "description": "The account owner of the repository. This is the GitHub username or organization name that owns the repository (e.g., 'octocat' or 'my-org')." },
                "repo": { "type": "string", "description": "The name of the repository without the owner prefix (e.g., 'hello-world', not 'octocat/hello-world')." },
                "issue_number": { "type": "integer", "description": "The unique issue number within the repository. This is the number shown in the issue URL (e.g., 42 from github.com/owner/repo/issues/42)." },
                "method": { "type": "string", "description": "The type of data to retrieve. Must be one of: 'get' (full issue details), 'get_comments' (all comments), 'get_sub_issues' (linked sub-issues), or 'get_labels' (available labels for the repository).", "enum": ["get", "get_comments", "get_sub_issues", "get_labels"] },
                "page": { "type": "integer", "description": "Page number for paginated results. Defaults to 1. Each page returns up to 30 items." },
                "per_page": { "type": "integer", "description": "Number of results per page (max 100). Defaults to 30." }
            },
            "required": ["owner", "repo", "issue_number", "method"]
        })),
        tool("github", "issue_write", "Create or update GitHub issues. Use method 'create' to open a new issue with title, body, labels, assignees, and milestone. Use 'update' to modify an existing issue's fields — only fields you pass will be changed. Use 'close' or 'reopen' to change an issue's state. When creating issues, the authenticated user must have push access or the repository must allow issue creation by the public.", json!({
            "type": "object",
            "properties": {
                "owner": { "type": "string", "description": "The account owner of the repository (username or organization)." },
                "repo": { "type": "string", "description": "The repository name without the owner prefix." },
                "method": { "type": "string", "description": "The write operation to perform: 'create' to open a new issue, 'update' to modify an existing issue, 'close' to close an issue, or 'reopen' to reopen a closed issue.", "enum": ["create", "update", "close", "reopen"] },
                "issue_number": { "type": "integer", "description": "Required for update/close/reopen. The issue number to modify." },
                "title": { "type": "string", "description": "Issue title. Required for 'create', optional for 'update'." },
                "body": { "type": "string", "description": "Issue body content in GitHub-flavored Markdown. Supports task lists, mentions, and cross-references." },
                "labels": { "type": "array", "items": { "type": "string" }, "description": "Array of label names to apply to the issue. Labels must already exist in the repository." },
                "assignees": { "type": "array", "items": { "type": "string" }, "description": "Array of GitHub usernames to assign to the issue. Users must have push access to the repository." },
                "milestone": { "type": "integer", "description": "Milestone number to associate with the issue. Use null to remove the milestone." },
                "state_reason": { "type": "string", "description": "Reason for closing: 'completed' or 'not_planned'. Only used with method 'close'.", "enum": ["completed", "not_planned"] }
            },
            "required": ["owner", "repo", "method"]
        })),
        tool("github", "search_issues", "Search for issues and pull requests across GitHub repositories using GitHub's search syntax. Supports qualifiers like 'is:issue', 'is:pr', 'state:open', 'label:bug', 'author:username', 'repo:owner/name', and date ranges. Results are sorted by best match by default but can be sorted by creation date, update date, or number of comments.", json!({
            "type": "object",
            "properties": {
                "query": { "type": "string", "description": "GitHub search query string. Supports qualifiers like 'is:issue is:open label:bug repo:owner/repo'. See GitHub search syntax documentation for full list of qualifiers." },
                "sort": { "type": "string", "description": "Sort field: 'created', 'updated', 'comments', or 'best-match' (default).", "enum": ["created", "updated", "comments", "best-match"] },
                "order": { "type": "string", "description": "Sort order: 'asc' or 'desc' (default).", "enum": ["asc", "desc"] },
                "per_page": { "type": "integer", "description": "Results per page (max 100, default 30)." },
                "page": { "type": "integer", "description": "Page number for paginated results (default 1)." }
            },
            "required": ["query"]
        })),
        tool("github", "list_issues", "List issues in a GitHub repository with optional filtering by state, labels, assignee, milestone, and sort order. Returns paginated results with full issue metadata. Only returns issues (not pull requests) unless explicitly filtered.", json!({
            "type": "object",
            "properties": {
                "owner": { "type": "string", "description": "Repository owner (username or organization)." },
                "repo": { "type": "string", "description": "Repository name." },
                "state": { "type": "string", "description": "Filter by state: 'open', 'closed', or 'all'. Defaults to 'open'.", "enum": ["open", "closed", "all"] },
                "labels": { "type": "string", "description": "Comma-separated list of label names to filter by (e.g., 'bug,priority:high')." },
                "assignee": { "type": "string", "description": "Filter by assignee username. Use '*' for any assignee, 'none' for unassigned." },
                "sort": { "type": "string", "description": "Sort field: 'created', 'updated', or 'comments'.", "enum": ["created", "updated", "comments"] },
                "direction": { "type": "string", "description": "Sort direction: 'asc' or 'desc'.", "enum": ["asc", "desc"] },
                "per_page": { "type": "integer", "description": "Results per page (max 100, default 30)." },
                "page": { "type": "integer", "description": "Page number (default 1)." }
            },
            "required": ["owner", "repo"]
        })),
        tool("github", "add_issue_comment", "Add a comment to an existing GitHub issue or pull request. The comment body supports GitHub-flavored Markdown including task lists, code blocks, mentions (@username), issue references (#123), and cross-repository references (owner/repo#123). The authenticated user must have read access to the repository.", json!({
            "type": "object",
            "properties": {
                "owner": { "type": "string", "description": "Repository owner (username or organization)." },
                "repo": { "type": "string", "description": "Repository name." },
                "issue_number": { "type": "integer", "description": "The issue or pull request number to comment on." },
                "body": { "type": "string", "description": "Comment body in GitHub-flavored Markdown." }
            },
            "required": ["owner", "repo", "issue_number", "body"]
        })),
        tool("github", "sub_issue_write", "Manage sub-issues (child issues) linked to a parent issue. Supports adding existing issues as sub-issues, removing sub-issue relationships, and reordering sub-issues within the parent. Sub-issues appear as a checklist in the parent issue's body.", json!({
            "type": "object",
            "properties": {
                "owner": { "type": "string", "description": "Repository owner." },
                "repo": { "type": "string", "description": "Repository name." },
                "issue_number": { "type": "integer", "description": "The parent issue number." },
                "method": { "type": "string", "description": "Operation: 'add' to link a sub-issue, 'remove' to unlink, 'reorder' to change position.", "enum": ["add", "remove", "reorder"] },
                "sub_issue_number": { "type": "integer", "description": "The issue number to add/remove as a sub-issue." },
                "position": { "type": "integer", "description": "Position index for reordering (0-based). Only used with 'reorder' method." }
            },
            "required": ["owner", "repo", "issue_number", "method", "sub_issue_number"]
        })),
        tool("github", "pull_request_read", "Read detailed information about a GitHub pull request. Supports multiple methods: 'get' for full PR details including merge status and review state, 'get_diff' for the unified diff, 'get_status' for CI/CD check status of the head commit, 'get_files' for the list of changed files with patches, 'get_reviews' for submitted reviews, and 'get_review_comments' for inline code comments.", json!({
            "type": "object",
            "properties": {
                "owner": { "type": "string", "description": "Repository owner." },
                "repo": { "type": "string", "description": "Repository name." },
                "pull_number": { "type": "integer", "description": "Pull request number." },
                "method": { "type": "string", "description": "Data to retrieve: 'get', 'get_diff', 'get_status', 'get_files', 'get_reviews', or 'get_review_comments'.", "enum": ["get", "get_diff", "get_status", "get_files", "get_reviews", "get_review_comments"] },
                "page": { "type": "integer", "description": "Page number for paginated results." },
                "per_page": { "type": "integer", "description": "Results per page (max 100)." }
            },
            "required": ["owner", "repo", "pull_number", "method"]
        })),
        tool("github", "create_pull_request", "Create a new pull request in a GitHub repository. Requires a head branch with commits to merge into the base branch. The title and body describe the changes for reviewers. Supports draft PRs, auto-merge enablement, and maintainer edit permissions.", json!({
            "type": "object",
            "properties": {
                "owner": { "type": "string", "description": "Repository owner." },
                "repo": { "type": "string", "description": "Repository name." },
                "title": { "type": "string", "description": "Pull request title. Should be concise and descriptive of the changes." },
                "body": { "type": "string", "description": "Pull request description in GitHub-flavored Markdown. Should explain what changed and why." },
                "head": { "type": "string", "description": "The branch containing the changes. For cross-repo PRs, use 'username:branch' format." },
                "base": { "type": "string", "description": "The branch to merge changes into (e.g., 'main' or 'develop')." },
                "draft": { "type": "boolean", "description": "If true, create as a draft PR that cannot be merged until marked ready." },
                "maintainer_can_modify": { "type": "boolean", "description": "If true, allow maintainers of the base repository to push to the head branch." }
            },
            "required": ["owner", "repo", "title", "head", "base"]
        })),
        tool("github", "update_pull_request", "Update an existing pull request's title, body, state, base branch, or maintainer edit permissions. Only the fields you provide will be changed — omitted fields remain unchanged. Use state 'closed' to close without merging, or 'open' to reopen a closed PR.", json!({
            "type": "object",
            "properties": {
                "owner": { "type": "string", "description": "Repository owner." },
                "repo": { "type": "string", "description": "Repository name." },
                "pull_number": { "type": "integer", "description": "Pull request number to update." },
                "title": { "type": "string", "description": "New title for the pull request." },
                "body": { "type": "string", "description": "New body/description for the pull request." },
                "state": { "type": "string", "description": "New state: 'open' or 'closed'.", "enum": ["open", "closed"] },
                "base": { "type": "string", "description": "New base branch to merge into." },
                "maintainer_can_modify": { "type": "boolean", "description": "Allow maintainers to push to the head branch." }
            },
            "required": ["owner", "repo", "pull_number"]
        })),
        tool("github", "merge_pull_request", "Merge a pull request using one of three merge strategies: 'merge' creates a merge commit, 'squash' combines all commits into one, and 'rebase' replays commits on top of the base branch. The PR must have passing status checks and no merge conflicts. Optionally specify a custom commit title and message.", json!({
            "type": "object",
            "properties": {
                "owner": { "type": "string", "description": "Repository owner." },
                "repo": { "type": "string", "description": "Repository name." },
                "pull_number": { "type": "integer", "description": "Pull request number to merge." },
                "merge_method": { "type": "string", "description": "Merge strategy: 'merge' (merge commit), 'squash' (single commit), or 'rebase' (replay commits).", "enum": ["merge", "squash", "rebase"] },
                "commit_title": { "type": "string", "description": "Custom title for the merge commit. Defaults to the PR title." },
                "commit_message": { "type": "string", "description": "Custom message for the merge commit. Defaults to the PR body." },
                "sha": { "type": "string", "description": "SHA of the head commit to verify. Merge will fail if this doesn't match the current head." }
            },
            "required": ["owner", "repo", "pull_number"]
        })),
        tool("github", "add_pull_request_review_comment", "Add an inline review comment to a specific line or range of lines in a pull request diff. Comments are attached to the diff at a specific commit and file path. Supports single-line and multi-line comments with suggestions using GitHub's suggestion syntax.", json!({
            "type": "object",
            "properties": {
                "owner": { "type": "string", "description": "Repository owner." },
                "repo": { "type": "string", "description": "Repository name." },
                "pull_number": { "type": "integer", "description": "Pull request number." },
                "body": { "type": "string", "description": "Comment body in Markdown. Use ```suggestion blocks for code suggestions." },
                "path": { "type": "string", "description": "Relative path of the file to comment on (e.g., 'src/main.rs')." },
                "line": { "type": "integer", "description": "The line number in the diff to attach the comment to." },
                "side": { "type": "string", "description": "Which side of the diff: 'LEFT' (deletion) or 'RIGHT' (addition).", "enum": ["LEFT", "RIGHT"] },
                "start_line": { "type": "integer", "description": "For multi-line comments, the first line of the range." },
                "commit_id": { "type": "string", "description": "SHA of the commit to comment on. Defaults to the latest commit." }
            },
            "required": ["owner", "repo", "pull_number", "body", "path", "line"]
        })),
        tool("github", "get_file_contents", "Retrieve the contents of a file or directory from a GitHub repository at a specific branch, tag, or commit. For files, returns the decoded content. For directories, returns a listing of entries with their types and sizes. Large files (>1MB) return a download URL instead of content.", json!({
            "type": "object",
            "properties": {
                "owner": { "type": "string", "description": "Repository owner." },
                "repo": { "type": "string", "description": "Repository name." },
                "path": { "type": "string", "description": "Path to the file or directory within the repository (e.g., 'src/lib.rs' or 'docs/')." },
                "ref": { "type": "string", "description": "Branch name, tag, or commit SHA. Defaults to the repository's default branch. Always specify this for branch-specific reads." }
            },
            "required": ["owner", "repo", "path"]
        })),
        tool("github", "create_or_update_file", "Create a new file or update an existing file in a GitHub repository by committing directly. For updates, you must provide the SHA of the existing file to prevent overwriting concurrent changes. Creates a new commit on the specified branch.", json!({
            "type": "object",
            "properties": {
                "owner": { "type": "string", "description": "Repository owner." },
                "repo": { "type": "string", "description": "Repository name." },
                "path": { "type": "string", "description": "Path for the file in the repository." },
                "content": { "type": "string", "description": "New file content (will be Base64 encoded automatically)." },
                "message": { "type": "string", "description": "Commit message describing the change." },
                "branch": { "type": "string", "description": "Branch to commit to. Defaults to the default branch." },
                "sha": { "type": "string", "description": "Required for updates: SHA blob of the existing file. Obtain via get_file_contents." }
            },
            "required": ["owner", "repo", "path", "content", "message"]
        })),
        tool("github", "push_files", "Commit and push multiple files in a single commit to a GitHub repository. More efficient than multiple create_or_update_file calls. Creates a new tree with all file changes and a single commit pointing to it.", json!({
            "type": "object",
            "properties": {
                "owner": { "type": "string", "description": "Repository owner." },
                "repo": { "type": "string", "description": "Repository name." },
                "branch": { "type": "string", "description": "Branch to push to." },
                "message": { "type": "string", "description": "Commit message for all the file changes." },
                "files": { "type": "array", "description": "Array of file objects to commit. Each object has 'path' (string) and 'content' (string).", "items": { "type": "object", "properties": { "path": { "type": "string" }, "content": { "type": "string" } } } }
            },
            "required": ["owner", "repo", "branch", "message", "files"]
        })),
        tool("github", "search_code", "Search for code across GitHub repositories using GitHub's code search syntax. Supports qualifiers like 'language:rust', 'repo:owner/name', 'path:src/', 'extension:rs', and boolean operators. Returns matching file fragments with highlighting.", json!({
            "type": "object",
            "properties": {
                "query": { "type": "string", "description": "Code search query. Supports qualifiers: language:, repo:, path:, extension:, filename:, and boolean operators AND, OR, NOT." },
                "per_page": { "type": "integer", "description": "Results per page (max 100, default 30)." },
                "page": { "type": "integer", "description": "Page number (default 1)." }
            },
            "required": ["query"]
        })),
        tool("github", "search_repositories", "Search for GitHub repositories by name, description, topic, language, stars, forks, and other criteria. Supports GitHub search qualifiers and sorting by stars, forks, help-wanted-issues, or update date.", json!({
            "type": "object",
            "properties": {
                "query": { "type": "string", "description": "Repository search query. Supports qualifiers: language:, stars:>100, topic:, license:, archived:, fork:, is:public/private." },
                "sort": { "type": "string", "description": "Sort by: 'stars', 'forks', 'help-wanted-issues', 'updated', or best-match.", "enum": ["stars", "forks", "help-wanted-issues", "updated"] },
                "order": { "type": "string", "description": "Sort order: 'asc' or 'desc'.", "enum": ["asc", "desc"] },
                "per_page": { "type": "integer", "description": "Results per page (max 100, default 30)." },
                "page": { "type": "integer", "description": "Page number (default 1)." }
            },
            "required": ["query"]
        })),
        tool("github", "list_commits", "List commits on a repository branch with optional filtering by author, date range, and path. Returns commit SHA, message, author info, date, and changed file statistics. Results are paginated.", json!({
            "type": "object",
            "properties": {
                "owner": { "type": "string", "description": "Repository owner." },
                "repo": { "type": "string", "description": "Repository name." },
                "sha": { "type": "string", "description": "Branch name or commit SHA to list commits from. Defaults to the default branch." },
                "author": { "type": "string", "description": "Filter by commit author (GitHub username or email)." },
                "since": { "type": "string", "description": "Only commits after this date (ISO 8601 format, e.g., '2024-01-01T00:00:00Z')." },
                "until": { "type": "string", "description": "Only commits before this date (ISO 8601 format)." },
                "path": { "type": "string", "description": "Only commits affecting this file path." },
                "per_page": { "type": "integer", "description": "Results per page (max 100, default 30)." },
                "page": { "type": "integer", "description": "Page number (default 1)." }
            },
            "required": ["owner", "repo"]
        })),
        tool("github", "list_branches", "List branches in a GitHub repository with their latest commit SHA and protection status. Supports filtering by whether branches are protected. Results are paginated alphabetically.", json!({
            "type": "object",
            "properties": {
                "owner": { "type": "string", "description": "Repository owner." },
                "repo": { "type": "string", "description": "Repository name." },
                "protected_only": { "type": "boolean", "description": "If true, only return branches with protection rules enabled." },
                "per_page": { "type": "integer", "description": "Results per page (max 100, default 30)." },
                "page": { "type": "integer", "description": "Page number (default 1)." }
            },
            "required": ["owner", "repo"]
        })),
        tool("github", "create_branch", "Create a new branch in a GitHub repository from a specified source commit or branch. The new branch name must not already exist. Useful for creating feature branches before making changes.", json!({
            "type": "object",
            "properties": {
                "owner": { "type": "string", "description": "Repository owner." },
                "repo": { "type": "string", "description": "Repository name." },
                "branch": { "type": "string", "description": "Name for the new branch (e.g., 'feature/my-feature')." },
                "from_branch": { "type": "string", "description": "Source branch or commit SHA to create from. Defaults to the default branch." }
            },
            "required": ["owner", "repo", "branch"]
        })),
        tool("github", "list_workflow_runs", "List GitHub Actions workflow runs for a repository with optional filtering by workflow, branch, actor, status, and event type. Returns run ID, status, conclusion, timing, and the triggering commit. Supports pagination with at least 30 results per page.", json!({
            "type": "object",
            "properties": {
                "owner": { "type": "string", "description": "Repository owner." },
                "repo": { "type": "string", "description": "Repository name." },
                "workflow_id": { "type": "string", "description": "Filter by workflow file name (e.g., 'ci.yml') or workflow ID number." },
                "branch": { "type": "string", "description": "Filter by branch name." },
                "actor": { "type": "string", "description": "Filter by the user who triggered the run." },
                "status": { "type": "string", "description": "Filter by status: 'queued', 'in_progress', 'completed', 'waiting', 'requested'.", "enum": ["queued", "in_progress", "completed", "waiting", "requested"] },
                "event": { "type": "string", "description": "Filter by event type: 'push', 'pull_request', 'schedule', 'workflow_dispatch', etc." },
                "per_page": { "type": "integer", "description": "Results per page (max 100, default 30)." },
                "page": { "type": "integer", "description": "Page number (default 1)." }
            },
            "required": ["owner", "repo"]
        })),
        tool("github", "get_workflow_run_logs", "Download and return the logs for a specific GitHub Actions workflow run. Logs can be very large for complex workflows with many jobs and steps. Returns the combined log output from all jobs in the run.", json!({
            "type": "object",
            "properties": {
                "owner": { "type": "string", "description": "Repository owner." },
                "repo": { "type": "string", "description": "Repository name." },
                "run_id": { "type": "integer", "description": "The unique workflow run ID. Obtain from list_workflow_runs." }
            },
            "required": ["owner", "repo", "run_id"]
        })),
        tool("github", "rerun_workflow", "Re-run a completed or failed GitHub Actions workflow run. This creates a new run with the same inputs and configuration as the original. Useful for retrying after transient failures or infrastructure issues.", json!({
            "type": "object",
            "properties": {
                "owner": { "type": "string", "description": "Repository owner." },
                "repo": { "type": "string", "description": "Repository name." },
                "run_id": { "type": "integer", "description": "The workflow run ID to re-run." },
                "enable_debug_logging": { "type": "boolean", "description": "If true, enable debug logging for the re-run. Produces more verbose output." }
            },
            "required": ["owner", "repo", "run_id"]
        })),
        tool("github", "get_code_scanning_alerts", "List code scanning alerts for a repository detected by GitHub Advanced Security (CodeQL, third-party tools). Returns alert details including severity, rule, location, and dismissal status. Requires GitHub Advanced Security to be enabled.", json!({
            "type": "object",
            "properties": {
                "owner": { "type": "string", "description": "Repository owner." },
                "repo": { "type": "string", "description": "Repository name." },
                "state": { "type": "string", "description": "Filter by alert state: 'open', 'closed', 'dismissed', or 'fixed'.", "enum": ["open", "closed", "dismissed", "fixed"] },
                "severity": { "type": "string", "description": "Filter by severity level: 'critical', 'high', 'medium', 'low', 'warning', 'note', or 'error'.", "enum": ["critical", "high", "medium", "low", "warning", "note", "error"] },
                "ref": { "type": "string", "description": "Git reference (branch, tag, or SHA) to filter alerts for." },
                "per_page": { "type": "integer", "description": "Results per page (max 100, default 30)." },
                "page": { "type": "integer", "description": "Page number (default 1)." }
            },
            "required": ["owner", "repo"]
        })),
        tool("github", "get_me", "Get the authenticated user's GitHub profile information including username, display name, email, bio, company, location, avatar URL, and account statistics (public repos, followers, following). Useful for identifying the current user in workflows.", json!({
            "type": "object",
            "properties": {},
            "required": []
        })),

        // ── Atlassian ───────────────────────────────────────────────────
        tool("atlassian", "getJiraIssue", "Retrieve a single Jira issue by its key (e.g., 'PROJ-123'). Returns all standard and custom fields including summary, description (in Atlassian Document Format), status, priority, assignee, reporter, labels, components, fix versions, sprint information, story points, and the full change history. Related issues such as blockers, duplicates, and parent epics are included in the response.", json!({
            "type": "object",
            "properties": {
                "issueIdOrKey": { "type": "string", "description": "The issue key (e.g., 'PROJ-123') or numeric issue ID. Issue keys are case-insensitive." },
                "fields": { "type": "array", "items": { "type": "string" }, "description": "Optional list of field names to return. If omitted, all navigable fields are returned. Use this to reduce response size for large issues. Common fields: 'summary', 'status', 'assignee', 'priority', 'description'." },
                "expand": { "type": "string", "description": "Comma-separated list of entities to expand in the response. Options: 'renderedFields' (HTML), 'changelog' (full history), 'transitions' (available transitions), 'operations' (available actions)." }
            },
            "required": ["issueIdOrKey"]
        })),
        tool("atlassian", "createJiraIssue", "Create a new Jira issue in a specified project. Requires the project key, issue type, and summary at minimum. Additional fields depend on the project's configuration and field schemes — use getJiraProjectIssueTypesMetadata to discover required and optional fields before creating. Supports setting description in Atlassian Document Format (ADF), labels, components, priority, assignee, and custom fields.", json!({
            "type": "object",
            "properties": {
                "projectKey": { "type": "string", "description": "The project key (e.g., 'PROJ'). Must be an existing, accessible project." },
                "issueType": { "type": "string", "description": "Issue type name (e.g., 'Bug', 'Story', 'Task', 'Epic'). Must be a valid type for the target project." },
                "summary": { "type": "string", "description": "Issue summary/title. Should be concise and descriptive." },
                "description": { "type": "object", "description": "Issue description in Atlassian Document Format (ADF). Use type 'doc' with content array of paragraph, heading, codeBlock, and other ADF nodes." },
                "priority": { "type": "string", "description": "Priority name: 'Highest', 'High', 'Medium', 'Low', or 'Lowest'. Defaults to the project's default priority." },
                "assignee": { "type": "string", "description": "Atlassian account ID of the assignee. Use lookupJiraAccountId to find account IDs from display names." },
                "labels": { "type": "array", "items": { "type": "string" }, "description": "Array of label strings to apply to the new issue." },
                "components": { "type": "array", "items": { "type": "object" }, "description": "Array of component objects with 'name' field. Components must exist in the project." },
                "parentKey": { "type": "string", "description": "Parent issue key for creating sub-tasks or child issues (e.g., 'PROJ-100')." }
            },
            "required": ["projectKey", "issueType", "summary"]
        })),
        tool("atlassian", "editJiraIssue", "Update fields on an existing Jira issue. Pass only the fields you want to change — omitted fields remain unchanged. Supports updating summary, description, status, priority, assignee, labels, components, fix versions, and any custom fields configured in the project.", json!({
            "type": "object",
            "properties": {
                "issueIdOrKey": { "type": "string", "description": "The issue key (e.g., 'PROJ-123') or numeric issue ID to update." },
                "summary": { "type": "string", "description": "New summary/title for the issue." },
                "description": { "type": "object", "description": "New description in Atlassian Document Format (ADF)." },
                "priority": { "type": "string", "description": "New priority name." },
                "assignee": { "type": "string", "description": "Atlassian account ID of the new assignee, or null to unassign." },
                "labels": { "type": "array", "items": { "type": "string" }, "description": "Complete list of labels (replaces existing labels)." },
                "components": { "type": "array", "items": { "type": "object" }, "description": "Complete list of component objects." }
            },
            "required": ["issueIdOrKey"]
        })),
        tool("atlassian", "searchJiraIssuesUsingJql", "Search for Jira issues using JQL (Jira Query Language). Supports complex queries with fields, operators, keywords, and functions. Examples: 'project = PROJ AND status = \"In Progress\"', 'assignee = currentUser() AND resolution = Unresolved ORDER BY priority DESC', 'labels in (bug, critical) AND created >= -7d'. Returns paginated results with configurable fields.", json!({
            "type": "object",
            "properties": {
                "jql": { "type": "string", "description": "JQL query string. Supports fields (project, status, assignee, priority, labels, created, updated, etc.), operators (=, !=, IN, NOT IN, ~, >=, <=), keywords (AND, OR, NOT, ORDER BY), and functions (currentUser(), now(), startOfDay(), endOfWeek(), etc.)." },
                "fields": { "type": "array", "items": { "type": "string" }, "description": "List of field names to include in results. Defaults to all navigable fields. Use '*all' for everything including custom fields." },
                "maxResults": { "type": "integer", "description": "Maximum number of results to return (default 50, max 100)." },
                "startAt": { "type": "integer", "description": "Index of the first result to return (0-based). Use for pagination." }
            },
            "required": ["jql"]
        })),
        tool("atlassian", "transitionJiraIssue", "Move a Jira issue to a new workflow state by executing a transition. Transitions represent the allowed state changes for an issue (e.g., 'To Do' → 'In Progress' → 'Done'). You must use getTransitionsForJiraIssue first to discover which transitions are available from the current state, as available transitions depend on the workflow configuration.", json!({
            "type": "object",
            "properties": {
                "issueIdOrKey": { "type": "string", "description": "The issue key or ID to transition." },
                "transitionId": { "type": "string", "description": "The ID of the transition to execute. Get available transition IDs from getTransitionsForJiraIssue." },
                "comment": { "type": "string", "description": "Optional comment to add during the transition, explaining why the state was changed." },
                "resolution": { "type": "string", "description": "Resolution name when transitioning to a resolved/done state (e.g., 'Done', 'Won\\'t Do', 'Duplicate')." }
            },
            "required": ["issueIdOrKey", "transitionId"]
        })),
        tool("atlassian", "getTransitionsForJiraIssue", "Retrieve the available workflow transitions for a Jira issue from its current state. Returns transition IDs, names, and target status for each available transition. Use this before transitionJiraIssue to discover valid transitions — available transitions depend on the issue's current status and the project's workflow configuration.", json!({
            "type": "object",
            "properties": {
                "issueIdOrKey": { "type": "string", "description": "The issue key (e.g., 'PROJ-123') or numeric issue ID." }
            },
            "required": ["issueIdOrKey"]
        })),
        tool("atlassian", "addCommentToJiraIssue", "Add a comment to an existing Jira issue. The comment body uses Atlassian Document Format (ADF), which supports rich text, mentions, code blocks, tables, and inline media. Comments are visible to all users with access to the issue and appear in the issue's activity feed.", json!({
            "type": "object",
            "properties": {
                "issueIdOrKey": { "type": "string", "description": "The issue key or ID to comment on." },
                "body": { "type": "object", "description": "Comment body in Atlassian Document Format (ADF). Root node must be type 'doc' with a 'content' array of block nodes (paragraph, heading, codeBlock, table, etc.)." }
            },
            "required": ["issueIdOrKey", "body"]
        })),
        tool("atlassian", "addWorklogToJiraIssue", "Log time spent working on a Jira issue. Creates a worklog entry with the specified duration, start time, and optional description. Supports both seconds-based and string-based time formats. Worklogs contribute to the issue's time tracking and are visible in the issue's worklog tab.", json!({
            "type": "object",
            "properties": {
                "issueIdOrKey": { "type": "string", "description": "The issue key or ID to log work against." },
                "timeSpentSeconds": { "type": "integer", "description": "Time spent in seconds. Either this or timeSpent string must be provided." },
                "timeSpent": { "type": "string", "description": "Time spent as a human-readable string (e.g., '2h 30m', '1d', '45m'). Either this or timeSpentSeconds must be provided." },
                "started": { "type": "string", "description": "When the work was started, in ISO 8601 format (e.g., '2024-03-15T09:00:00.000+0000'). Defaults to the current time." },
                "comment": { "type": "object", "description": "Optional description of the work performed, in Atlassian Document Format (ADF)." }
            },
            "required": ["issueIdOrKey"]
        })),
        tool("atlassian", "getJiraProjectIssueTypesMetadata", "Retrieve the available issue types and their field metadata for a Jira project. Returns each issue type (Bug, Story, Task, Epic, etc.) with its required fields, optional fields, allowed values, and default values. Essential to call before createJiraIssue to discover what fields are available and required for the target project.", json!({
            "type": "object",
            "properties": {
                "projectKey": { "type": "string", "description": "The project key (e.g., 'PROJ') to retrieve issue type metadata for." }
            },
            "required": ["projectKey"]
        })),
        tool("atlassian", "lookupJiraAccountId", "Look up an Atlassian account ID from a display name or email address. Account IDs are required for assigning issues, adding watchers, and other user-referencing operations. Returns matching users with their account ID, display name, email, and avatar URL.", json!({
            "type": "object",
            "properties": {
                "query": { "type": "string", "description": "Search query: a display name, email address, or partial match. Searches across all accessible Atlassian Cloud sites." },
                "maxResults": { "type": "integer", "description": "Maximum number of matching users to return (default 10)." }
            },
            "required": ["query"]
        })),
        tool("atlassian", "getConfluencePage", "Retrieve a Confluence page's content, metadata, and version information. Supports two content representations: 'storage' format (raw XHTML used by Confluence) and 'atlas_doc_format' (structured JSON). The atlas_doc_format is recommended for reading as it provides a structured document tree. Also returns page title, space, version number, ancestors, and labels.", json!({
            "type": "object",
            "properties": {
                "pageId": { "type": "string", "description": "The numeric ID of the Confluence page to retrieve." },
                "bodyFormat": { "type": "string", "description": "Content representation format: 'storage' (raw XHTML) or 'atlas_doc_format' (structured JSON, recommended for reading).", "enum": ["storage", "atlas_doc_format"] }
            },
            "required": ["pageId"]
        })),
        tool("atlassian", "createConfluencePage", "Create a new page in a Confluence space. Requires a space ID, title, and body content in either 'storage' (XHTML) or 'atlas_doc_format' (structured JSON). Optionally set a parent page to create the page as a child in the page hierarchy. The page is created as the latest version (version 1).", json!({
            "type": "object",
            "properties": {
                "spaceId": { "type": "string", "description": "The ID of the Confluence space to create the page in." },
                "title": { "type": "string", "description": "Page title. Must be unique within the space." },
                "body": { "type": "string", "description": "Page content in the specified representation format." },
                "bodyFormat": { "type": "string", "description": "Content format: 'storage' (XHTML) or 'atlas_doc_format' (JSON).", "enum": ["storage", "atlas_doc_format"] },
                "parentId": { "type": "string", "description": "ID of the parent page. If omitted, the page is created at the space root." },
                "status": { "type": "string", "description": "Page status: 'current' (published) or 'draft'.", "enum": ["current", "draft"] }
            },
            "required": ["spaceId", "title", "body"]
        })),
        tool("atlassian", "updateConfluencePage", "Update the content, title, or status of an existing Confluence page. You must provide the current version number (obtained from getConfluencePage) to prevent overwriting concurrent edits. The version number is automatically incremented. Supports the same content formats as createConfluencePage.", json!({
            "type": "object",
            "properties": {
                "pageId": { "type": "string", "description": "The numeric ID of the page to update." },
                "title": { "type": "string", "description": "New page title." },
                "body": { "type": "string", "description": "New page content in the specified representation format." },
                "bodyFormat": { "type": "string", "description": "Content format: 'storage' or 'atlas_doc_format'.", "enum": ["storage", "atlas_doc_format"] },
                "version": { "type": "integer", "description": "Current version number of the page. Required to detect conflicts. Obtain from getConfluencePage." },
                "status": { "type": "string", "description": "New page status: 'current' or 'draft'.", "enum": ["current", "draft"] }
            },
            "required": ["pageId", "title", "body", "version"]
        })),
        tool("atlassian", "searchConfluenceUsingCql", "Search Confluence content using CQL (Confluence Query Language). Supports searching pages, blog posts, comments, and attachments with filters on space, type, creator, label, last modified date, and full-text content matching. Examples: 'type = page AND space = DEV AND text ~ \"architecture\"', 'creator = currentUser() AND lastModified > now(\"-7d\")'.", json!({
            "type": "object",
            "properties": {
                "cql": { "type": "string", "description": "CQL query string. Supports fields (type, space, creator, label, ancestor, title, text, lastModified), operators (=, !=, ~, IN, NOT IN, >, <), and functions (currentUser(), now())." },
                "limit": { "type": "integer", "description": "Maximum results to return (default 25, max 100)." },
                "start": { "type": "integer", "description": "Starting index for pagination (0-based)." },
                "expand": { "type": "string", "description": "Comma-separated entities to expand: 'content.body.storage', 'content.metadata.labels', etc." }
            },
            "required": ["cql"]
        })),
        tool("atlassian", "getConfluenceSpaces", "List all Confluence spaces accessible to the authenticated user. Returns space key, name, type (global/personal), description, and homepage ID. Use the space key or ID when creating pages or searching within a specific space.", json!({
            "type": "object",
            "properties": {
                "type": { "type": "string", "description": "Filter by space type: 'global' or 'personal'.", "enum": ["global", "personal"] },
                "limit": { "type": "integer", "description": "Maximum results to return (default 25, max 100)." },
                "start": { "type": "integer", "description": "Starting index for pagination (0-based)." }
            },
            "required": []
        })),
        tool("atlassian", "createConfluenceFooterComment", "Add a footer (page-level) comment to a Confluence page. Footer comments appear at the bottom of the page and are visible to all users with page access. The comment body uses the same content format as pages.", json!({
            "type": "object",
            "properties": {
                "pageId": { "type": "string", "description": "The ID of the page to comment on." },
                "body": { "type": "string", "description": "Comment body in storage format (XHTML) or atlas_doc_format (JSON)." },
                "bodyFormat": { "type": "string", "description": "Content format: 'storage' or 'atlas_doc_format'.", "enum": ["storage", "atlas_doc_format"] }
            },
            "required": ["pageId", "body"]
        })),
        tool("atlassian", "createConfluenceInlineComment", "Add an inline comment to a specific section of a Confluence page's content. Inline comments are anchored to the text they reference and appear as highlights in the page. Useful for targeted feedback on specific paragraphs or sentences.", json!({
            "type": "object",
            "properties": {
                "pageId": { "type": "string", "description": "The ID of the page to add an inline comment to." },
                "body": { "type": "string", "description": "Comment body in storage format or atlas_doc_format." },
                "bodyFormat": { "type": "string", "description": "Content format: 'storage' or 'atlas_doc_format'.", "enum": ["storage", "atlas_doc_format"] },
                "inlineCommentProperties": { "type": "object", "description": "Object specifying the text selection to anchor the comment to. Contains 'textSelection' (the selected text) and 'textSelectionMatchCount' (which occurrence to match if text appears multiple times)." }
            },
            "required": ["pageId", "body", "inlineCommentProperties"]
        })),
        tool("atlassian", "getConfluencePageDescendants", "Retrieve all descendant pages of a Confluence page in the page hierarchy. Returns child pages, grandchild pages, and deeper descendants. Useful for understanding the page tree structure under a parent page. Results include page ID, title, status, and depth level.", json!({
            "type": "object",
            "properties": {
                "pageId": { "type": "string", "description": "The ID of the parent page to list descendants for." },
                "depth": { "type": "string", "description": "How deep to traverse: 'all' for all descendants, or a number (e.g., '1' for direct children only)." },
                "limit": { "type": "integer", "description": "Maximum results to return (default 25, max 100)." },
                "start": { "type": "integer", "description": "Starting index for pagination (0-based)." }
            },
            "required": ["pageId"]
        })),

        // ── Filesystem ──────────────────────────────────────────────────
        tool("filesystem", "read_file", "Read the complete contents of a file from the filesystem. Returns the file content as a UTF-8 encoded string. For binary files, the content is returned as base64-encoded data. The file path must be within the server's configured allowed directories for security.", json!({
            "type": "object",
            "properties": {
                "path": { "type": "string", "description": "Absolute or relative path to the file to read. Must be within the server's configured allowed directories. Supports both forward slashes and backslashes on Windows." }
            },
            "required": ["path"]
        })),
        tool("filesystem", "read_multiple_files", "Read the contents of multiple files in a single operation. More efficient than making separate read_file calls for each file. Returns results in the same order as the requested paths. If any file fails to read, its entry contains an error message while other files are still returned.", json!({
            "type": "object",
            "properties": {
                "paths": { "type": "array", "items": { "type": "string" }, "description": "Array of file paths to read. Each path must be within the server's allowed directories." }
            },
            "required": ["paths"]
        })),
        tool("filesystem", "write_file", "Create a new file or completely overwrite an existing file with the provided content. Parent directories are created automatically if they don't exist. Content must be provided as a UTF-8 string; use base64 encoding for binary content. The file path must be within the server's allowed directories.", json!({
            "type": "object",
            "properties": {
                "path": { "type": "string", "description": "Path for the file to write. Parent directories will be created if needed." },
                "content": { "type": "string", "description": "Complete file content to write. For binary files, provide base64-encoded content." }
            },
            "required": ["path", "content"]
        })),
        tool("filesystem", "edit_file", "Apply targeted edits to a file using a diff-like format. Supports multiple edits in a single call, each specifying an 'oldText' string to find (must match exactly) and a 'newText' replacement. More reliable than write_file for partial modifications because it validates that the expected content exists before replacing.", json!({
            "type": "object",
            "properties": {
                "path": { "type": "string", "description": "Path to the file to edit." },
                "edits": { "type": "array", "description": "Array of edit operations. Each edit has 'oldText' (exact string to find) and 'newText' (replacement string).", "items": { "type": "object", "properties": { "oldText": { "type": "string" }, "newText": { "type": "string" } }, "required": ["oldText", "newText"] } },
                "dryRun": { "type": "boolean", "description": "If true, validate that all edits can be applied without actually modifying the file. Useful for verification." }
            },
            "required": ["path", "edits"]
        })),
        tool("filesystem", "create_directory", "Create a new directory at the specified path, including any necessary parent directories. Does not return an error if the directory already exists. The path must be within the server's allowed directories.", json!({
            "type": "object",
            "properties": {
                "path": { "type": "string", "description": "Path for the new directory. Parent directories will be created as needed." }
            },
            "required": ["path"]
        })),
        tool("filesystem", "list_directory", "List all entries in a directory. Returns each entry prefixed with [FILE] or [DIR] to indicate its type. Does not recurse into subdirectories — use directory_tree for recursive listing. Entries are sorted alphabetically with directories listed first.", json!({
            "type": "object",
            "properties": {
                "path": { "type": "string", "description": "Path to the directory to list." }
            },
            "required": ["path"]
        })),
        tool("filesystem", "directory_tree", "Generate a recursive tree structure of a directory showing all files and subdirectories up to a configurable depth. Useful for understanding project layout and file organization. Returns an indented text representation similar to the Unix 'tree' command.", json!({
            "type": "object",
            "properties": {
                "path": { "type": "string", "description": "Root directory path to generate the tree from." },
                "depth": { "type": "integer", "description": "Maximum depth to recurse. Defaults to 3. Use -1 for unlimited depth (may be slow for large directories)." }
            },
            "required": ["path"]
        })),
        tool("filesystem", "move_file", "Move or rename a file or directory. The source path must exist and the destination must not already exist. Both paths must be within the server's allowed directories. Works across directories on the same filesystem.", json!({
            "type": "object",
            "properties": {
                "source": { "type": "string", "description": "Current path of the file or directory to move." },
                "destination": { "type": "string", "description": "New path for the file or directory. Must not already exist." }
            },
            "required": ["source", "destination"]
        })),
        tool("filesystem", "search_files", "Search for files matching a glob pattern (e.g., '**/*.ts', 'src/**/*.test.js'). Searches recursively from the given path and returns all matching file paths. Supports standard glob syntax including wildcards (*), recursive matching (**), character classes ([abc]), and brace expansion ({a,b}).", json!({
            "type": "object",
            "properties": {
                "path": { "type": "string", "description": "Root directory to search from." },
                "pattern": { "type": "string", "description": "Glob pattern to match files against (e.g., '**/*.rs', 'src/**/*.{ts,tsx}')." },
                "excludePatterns": { "type": "array", "items": { "type": "string" }, "description": "Glob patterns to exclude from results (e.g., ['node_modules/**', '*.min.js'])." }
            },
            "required": ["path", "pattern"]
        })),
        tool("filesystem", "get_file_info", "Get detailed metadata about a file or directory including size in bytes, creation timestamp, last modification timestamp, last access timestamp, POSIX permissions, and whether the path is a file, directory, or symbolic link.", json!({
            "type": "object",
            "properties": {
                "path": { "type": "string", "description": "Path to the file or directory to get information about." }
            },
            "required": ["path"]
        })),

        // ── PostgreSQL ──────────────────────────────────────────────────
        tool("postgres", "query", "Execute a read-only SQL query (SELECT, EXPLAIN, SHOW, WITH) against the PostgreSQL database. Returns results as an array of JSON objects where each object represents a row with column names as keys. Use LIMIT to control result size and avoid returning excessive data. Parameterized queries are supported for safe value interpolation.", json!({
            "type": "object",
            "properties": {
                "sql": { "type": "string", "description": "SQL query to execute. Must be a read-only statement (SELECT, EXPLAIN, SHOW, WITH). Write operations will be rejected." },
                "params": { "type": "array", "items": {}, "description": "Optional array of parameter values for parameterized queries. Use $1, $2, etc. as placeholders in the SQL. Prevents SQL injection." }
            },
            "required": ["sql"]
        })),
        tool("postgres", "execute", "Execute a write SQL statement (INSERT, UPDATE, DELETE, CREATE, ALTER, DROP, TRUNCATE) against the PostgreSQL database. Returns the number of affected rows. Always use parameterized queries with the params array for any user-provided values to prevent SQL injection attacks.", json!({
            "type": "object",
            "properties": {
                "sql": { "type": "string", "description": "SQL statement to execute. Must be a write operation. Use $1, $2, etc. for parameter placeholders." },
                "params": { "type": "array", "items": {}, "description": "Array of parameter values corresponding to $1, $2, etc. placeholders in the SQL." }
            },
            "required": ["sql"]
        })),
        tool("postgres", "list_schemas", "List all schemas in the connected PostgreSQL database. Returns schema names and their descriptions (if set via COMMENT ON SCHEMA). Useful for discovering the database organization before exploring specific schemas.", json!({
            "type": "object",
            "properties": {},
            "required": []
        })),
        tool("postgres", "list_tables", "List all tables in a specified schema with their approximate row counts, size on disk, and descriptions (if set via COMMENT ON TABLE). Defaults to the 'public' schema if not specified. Also shows views and materialized views in the schema.", json!({
            "type": "object",
            "properties": {
                "schema": { "type": "string", "description": "Schema name to list tables from. Defaults to 'public' if omitted.", "default": "public" }
            },
            "required": []
        })),
        tool("postgres", "describe_table", "Get the complete schema definition of a table including all columns (name, data type, nullability, default value, character length limits), primary key, unique constraints, foreign key relationships, check constraints, and indexes. Essential for understanding table structure before writing queries.", json!({
            "type": "object",
            "properties": {
                "table": { "type": "string", "description": "Table name. For tables not in the 'public' schema, use schema-qualified form: 'schema.table'." }
            },
            "required": ["table"]
        })),
        tool("postgres", "explain_query", "Run EXPLAIN ANALYZE on a SQL query and return the query execution plan with actual timing statistics. The query is executed inside a rolled-back transaction so no data is permanently modified, making it safe for analyzing write operations. Shows sequential vs. index scans, join methods, sort operations, and actual row counts vs. estimates.", json!({
            "type": "object",
            "properties": {
                "sql": { "type": "string", "description": "SQL query to analyze. Can be any valid SQL including SELECT, INSERT, UPDATE, DELETE." },
                "params": { "type": "array", "items": {}, "description": "Optional parameter values for parameterized queries." },
                "format": { "type": "string", "description": "Output format: 'text' (default, human-readable), 'json' (structured), 'yaml', or 'xml'.", "enum": ["text", "json", "yaml", "xml"] }
            },
            "required": ["sql"]
        })),

        // ── Slack ────────────────────────────────────────────────────────
        tool("slack", "send_message", "Post a message to a Slack channel, group, or direct message conversation. The message body supports Slack's mrkdwn formatting including bold (*text*), italic (_text_), strikethrough (~text~), code (`code`), code blocks (```code```), links (<url|text>), and user mentions (<@USER_ID>). For threaded replies, include the thread_ts parameter.", json!({
            "type": "object",
            "properties": {
                "channel": { "type": "string", "description": "Channel ID (not name) to post to. Use list_channels to find channel IDs. For DMs, use the user's DM channel ID." },
                "text": { "type": "string", "description": "Message text with optional Slack mrkdwn formatting. This is also used as the notification text." },
                "thread_ts": { "type": "string", "description": "Timestamp of the parent message for threaded replies. Format: '1234567890.123456'." },
                "unfurl_links": { "type": "boolean", "description": "If true, enable URL previews/unfurling in the message." },
                "unfurl_media": { "type": "boolean", "description": "If true, enable media content unfurling (images, videos)." }
            },
            "required": ["channel", "text"]
        })),
        tool("slack", "list_channels", "List channels in the Slack workspace that the authenticated user has access to. Returns channel IDs, names, topics, purposes, member counts, and creation dates. Results are paginated — use the cursor parameter for subsequent pages. Supports filtering by channel type.", json!({
            "type": "object",
            "properties": {
                "types": { "type": "string", "description": "Comma-separated channel types to include: 'public_channel', 'private_channel', 'im' (direct messages), 'mpim' (group DMs). Defaults to 'public_channel'." },
                "limit": { "type": "integer", "description": "Maximum channels per page (default 100, max 1000)." },
                "cursor": { "type": "string", "description": "Pagination cursor from a previous response's 'next_cursor' field." },
                "exclude_archived": { "type": "boolean", "description": "If true, exclude archived channels from results. Defaults to false." }
            },
            "required": []
        })),
        tool("slack", "search_messages", "Search for messages across all channels the authenticated user has access to. Supports Slack search modifiers: 'in:#channel' to limit to a channel, 'from:@user' to filter by sender, 'before:2024-01-01' and 'after:2024-01-01' for date ranges, 'has:link' for messages with URLs, 'has:reaction' for reacted messages, and 'has:pin' for pinned messages. Returns message text, channel, timestamp, and permalink.", json!({
            "type": "object",
            "properties": {
                "query": { "type": "string", "description": "Search query string with optional Slack search modifiers (in:, from:, before:, after:, has:, is:)." },
                "sort": { "type": "string", "description": "Sort order: 'score' (relevance, default) or 'timestamp' (newest first).", "enum": ["score", "timestamp"] },
                "count": { "type": "integer", "description": "Number of results per page (default 20, max 100)." },
                "page": { "type": "integer", "description": "Page number for pagination (default 1)." }
            },
            "required": ["query"]
        })),
        tool("slack", "get_thread", "Retrieve all replies in a message thread. Returns messages in chronological order including the parent message. Each message includes sender ID, text, timestamp, reactions, and file attachments. Useful for following up on threaded conversations.", json!({
            "type": "object",
            "properties": {
                "channel": { "type": "string", "description": "Channel ID containing the thread." },
                "thread_ts": { "type": "string", "description": "Timestamp of the parent message that started the thread." },
                "limit": { "type": "integer", "description": "Maximum replies to return (default 100, max 1000)." },
                "cursor": { "type": "string", "description": "Pagination cursor from a previous response." }
            },
            "required": ["channel", "thread_ts"]
        })),
        tool("slack", "get_channel_history", "Fetch recent messages from a Slack channel in reverse chronological order. Supports filtering by time range using Unix timestamps. Returns message text, sender, timestamp, reactions, thread info, and attachments. Does not include threaded replies — use get_thread for those.", json!({
            "type": "object",
            "properties": {
                "channel": { "type": "string", "description": "Channel ID to fetch history from." },
                "oldest": { "type": "string", "description": "Only messages after this Unix timestamp (e.g., '1234567890.000000')." },
                "latest": { "type": "string", "description": "Only messages before this Unix timestamp." },
                "limit": { "type": "integer", "description": "Maximum messages to return (default 100, max 1000)." },
                "cursor": { "type": "string", "description": "Pagination cursor from a previous response." }
            },
            "required": ["channel"]
        })),
        tool("slack", "get_users", "List members of the Slack workspace with their display names, real names, email addresses, status text and emoji, timezone, and whether they are a bot, admin, or owner. Results are paginated. Useful for resolving user IDs needed for mentions and DMs.", json!({
            "type": "object",
            "properties": {
                "limit": { "type": "integer", "description": "Maximum users per page (default 100, max 1000)." },
                "cursor": { "type": "string", "description": "Pagination cursor from a previous response." }
            },
            "required": []
        })),
        tool("slack", "add_reaction", "Add an emoji reaction to a message in a Slack channel. The reaction is added on behalf of the authenticated user. The emoji name should not include colons (e.g., use 'thumbsup' not ':thumbsup:'). Each user can only add one of each reaction to a message.", json!({
            "type": "object",
            "properties": {
                "channel": { "type": "string", "description": "Channel ID containing the message to react to." },
                "timestamp": { "type": "string", "description": "Timestamp of the message to react to (e.g., '1234567890.123456')." },
                "name": { "type": "string", "description": "Emoji name without colons (e.g., 'thumbsup', 'rocket', 'white_check_mark')." }
            },
            "required": ["channel", "timestamp", "name"]
        })),
        tool("slack", "upload_file", "Upload a file to one or more Slack channels with an optional initial comment. Supports any file type. The file content is provided as a string (text files) or base64-encoded data (binary files). Returns the file ID, URL, and sharing details.", json!({
            "type": "object",
            "properties": {
                "channels": { "type": "string", "description": "Comma-separated channel IDs to share the file to." },
                "content": { "type": "string", "description": "File content as a string. For binary files, provide base64-encoded data." },
                "filename": { "type": "string", "description": "Name for the uploaded file (e.g., 'report.csv', 'screenshot.png')." },
                "title": { "type": "string", "description": "Title displayed for the file in Slack. Defaults to the filename." },
                "initial_comment": { "type": "string", "description": "Message text to post alongside the file upload." },
                "filetype": { "type": "string", "description": "File type identifier (e.g., 'csv', 'png', 'pdf'). Auto-detected if omitted." }
            },
            "required": ["channels", "content", "filename"]
        })),
    ]
}

/// Minimum content size (bytes) for indexing. Content shorter than this is not worth indexing.
const INDEX_MIN_BYTES: usize = 256;

/// Compute a catalog compression plan using the progressive 4-phase algorithm.
///
/// Phases:
/// 1. Index welcome messages (selective, by welcome size desc) → summary + TOC
/// 2. Defer tool definitions (selective, by definition savings desc) → batch-index, remove from tools/list
/// 3. Drop welcome TOC (keep summary only)
/// 4. Drop batch TOC (keep summary only)
pub fn compute_catalog_compression_plan(
    ctx: &InstructionsContext,
    threshold: usize,
    supports_tools_list_changed: bool,
    _supports_resources_list_changed: bool,
    _supports_prompts_list_changed: bool,
) -> CatalogCompressionPlan {
    let mut plan = CatalogCompressionPlan::default();

    let full_size = estimate_catalog_size(ctx);

    if full_size <= threshold {
        return plan;
    }

    let bytes_to_save = full_size - threshold;
    let mut saved = 0usize;

    // ── Phase 1: Index Welcome Messages ──
    // Sort servers by welcome size descending
    let mut welcome_candidates: Vec<(String, usize)> = ctx
        .servers
        .iter()
        .filter_map(|server| {
            let slug = slugify(&server.name);
            let welcome_size = server.description.as_ref().map(|d| d.len()).unwrap_or(0)
                + server.instructions.as_ref().map(|i| i.len()).unwrap_or(0);
            if welcome_size >= INDEX_MIN_BYTES {
                Some((slug, welcome_size))
            } else {
                None
            }
        })
        .collect();
    welcome_candidates.sort_by(|a, b| b.1.cmp(&a.1));

    // Performance shortcut: if total Phase 1 savings < bytes_to_save, apply all at once
    let total_phase1_savings: usize = welcome_candidates
        .iter()
        .map(|(_, size)| {
            // Estimate summary + TOC at ~500 bytes
            size.saturating_sub(500)
        })
        .sum();
    let apply_all_phase1 = total_phase1_savings < bytes_to_save;

    for (slug, welcome_size) in &welcome_candidates {
        if !apply_all_phase1 && saved >= bytes_to_save {
            break;
        }

        // Build welcome content for indexing
        let server = ctx.servers.iter().find(|s| slugify(&s.name) == *slug);
        let server = match server {
            Some(s) => s,
            None => continue,
        };

        let content = build_full_server_content(server);
        let store = lr_context::ContentStore::new().unwrap();
        let label = format!("mcp/{}", slug);
        let index_result = match store.index(&label, &content) {
            Ok(r) => r,
            Err(_) => continue,
        };

        let summary = index_result.summary();
        let toc = index_result.toc(None);
        let compressed_size = summary.len() + toc.len();
        let savings = welcome_size.saturating_sub(compressed_size);

        if savings > 0 {
            plan.indexed_welcomes.push(IndexedWelcome {
                server_slug: slug.clone(),
                summary,
                toc,
                original_size: *welcome_size,
            });
            saved += savings;
        }
    }

    if saved >= bytes_to_save {
        return plan;
    }

    // ── Phase 2: Defer Tool Definitions ──
    // Only when client supports tools/listChanged (always true when context management is enabled)
    if supports_tools_list_changed {
        // Collect per-server definition sizes, sorted by total definition savings desc
        let mut server_def_candidates: Vec<(String, usize)> = ctx
            .servers
            .iter()
            .map(|server| {
                let slug = slugify(&server.name);
                let total_def_size: usize = server
                    .tool_names
                    .iter()
                    .chain(server.resource_names.iter())
                    .chain(server.prompt_names.iter())
                    .filter_map(|name| ctx.item_definition_sizes.get(name))
                    .sum();
                (slug, total_def_size)
            })
            .filter(|(_, size)| *size > 0)
            .collect();
        server_def_candidates.sort_by(|a, b| b.1.cmp(&a.1));

        // Performance shortcut
        let total_phase2_savings: usize = server_def_candidates
            .iter()
            .map(|(_, size)| size.saturating_sub(200))
            .sum();
        let apply_all_phase2 = total_phase2_savings < bytes_to_save.saturating_sub(saved);

        for (slug, def_size) in &server_def_candidates {
            if !apply_all_phase2 && saved >= bytes_to_save {
                break;
            }

            let server = ctx.servers.iter().find(|s| slugify(&s.name) == *slug);
            let server = match server {
                Some(s) => s,
                None => continue,
            };

            let store = lr_context::ContentStore::new().unwrap();
            let mut batches = Vec::new();
            let mut batch_output_size = 0usize;

            // Batch-index tools
            if !server.tool_names.is_empty() {
                let items: Vec<(&str, String)> = server
                    .tool_names
                    .iter()
                    .map(|name| {
                        // Generate a placeholder markdown for size estimation
                        let md = format!("# {}\n\nTool definition placeholder.\n", name);
                        (name.as_str(), md)
                    })
                    .collect();
                let items_ref: Vec<(&str, &str)> =
                    items.iter().map(|(n, c)| (*n, c.as_str())).collect();
                let root = format!("mcp/{}/tool/", slug);
                if let Ok(result) = store.batch_index(&root, &items_ref) {
                    let summary = result.summary();
                    let toc = result.toc(Some(1));
                    batch_output_size += summary.len() + toc.len();
                    batches.push(DeferredServerBatch {
                        batch_summary: summary,
                        batch_toc: toc,
                    });
                }
            }

            // Batch-index resources
            if !server.resource_names.is_empty() {
                let items: Vec<(&str, String)> = server
                    .resource_names
                    .iter()
                    .map(|name| {
                        let md = format!("# {}\n\nResource definition placeholder.\n", name);
                        (name.as_str(), md)
                    })
                    .collect();
                let items_ref: Vec<(&str, &str)> =
                    items.iter().map(|(n, c)| (*n, c.as_str())).collect();
                let root = format!("mcp/{}/resource/", slug);
                if let Ok(result) = store.batch_index(&root, &items_ref) {
                    let summary = result.summary();
                    let toc = result.toc(Some(1));
                    batch_output_size += summary.len() + toc.len();
                    batches.push(DeferredServerBatch {
                        batch_summary: summary,
                        batch_toc: toc,
                    });
                }
            }

            // Batch-index prompts
            if !server.prompt_names.is_empty() {
                let items: Vec<(&str, String)> = server
                    .prompt_names
                    .iter()
                    .map(|name| {
                        let md = format!("# {}\n\nPrompt definition placeholder.\n", name);
                        (name.as_str(), md)
                    })
                    .collect();
                let items_ref: Vec<(&str, &str)> =
                    items.iter().map(|(n, c)| (*n, c.as_str())).collect();
                let root = format!("mcp/{}/prompt/", slug);
                if let Ok(result) = store.batch_index(&root, &items_ref) {
                    let summary = result.summary();
                    let toc = result.toc(Some(1));
                    batch_output_size += summary.len() + toc.len();
                    batches.push(DeferredServerBatch {
                        batch_summary: summary,
                        batch_toc: toc,
                    });
                }
            }

            if !batches.is_empty() {
                let savings = def_size.saturating_sub(batch_output_size);
                plan.deferred_servers.push(DeferredServer {
                    server_slug: slug.clone(),
                    batches,
                    definition_savings: savings,
                });
                saved += savings;
            }
        }
    }

    if saved >= bytes_to_save {
        return plan;
    }

    // ── Phase 3: Drop Welcome TOC ──
    for indexed in &plan.indexed_welcomes {
        if saved >= bytes_to_save {
            break;
        }
        let toc_bytes = indexed.toc.len();
        if toc_bytes > 0 {
            plan.welcome_toc_dropped.push(indexed.server_slug.clone());
            saved += toc_bytes;
        }
    }

    if saved >= bytes_to_save {
        return plan;
    }

    // ── Phase 4: Drop Batch TOC ──
    for deferred in &plan.deferred_servers {
        if saved >= bytes_to_save {
            break;
        }
        let toc_bytes: usize = deferred.batches.iter().map(|b| b.batch_toc.len()).sum();
        if toc_bytes > 0 {
            plan.batch_toc_dropped.push(deferred.server_slug.clone());
            saved += toc_bytes;
        }
    }

    plan
}

/// Estimate the total byte size of the catalog (welcome text + tool definitions).
fn estimate_catalog_size(ctx: &InstructionsContext) -> usize {
    let mut size = 100; // header overhead

    for server in &ctx.servers {
        size += server.name.len() + 10; // server header
                                        // Server description + instructions (welcome)
        if let Some(desc) = &server.description {
            size += desc.len() + 20; // XML tags
        }
        if let Some(inst) = &server.instructions {
            size += inst.len() + 20;
        }
    }

    for server in &ctx.unavailable_servers {
        size += server.name.len() + server.error.len() + 30;
    }

    for vsi in &ctx.virtual_instructions {
        size += vsi.section_title.len() + vsi.content.len() + 20;
        for name in &vsi.tool_names {
            size += name.len() + 15;
        }
    }

    // Add tool definition sizes
    size += ctx.item_definition_sizes.values().sum::<usize>();

    size
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
            context_management_enabled: false,
            catalog_compression: None,
            virtual_instructions: Vec::new(),
            ..Default::default()
        };

        let instructions = build_gateway_instructions(&ctx).unwrap();
        // Header
        assert!(instructions.contains("Unified MCP Gateway"));
        assert!(instructions.contains("servername__"));
        // Server instructions in XML tags (no tool listing)
        assert!(instructions.contains("<filesystem>"));
        assert!(!instructions.contains("`filesystem__read_file` (tool)"));
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
            context_management_enabled: false,
            catalog_compression: None,
            virtual_instructions: Vec::new(),
            ..Default::default()
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
            context_management_enabled: false,
            catalog_compression: None,
            virtual_instructions: Vec::new(),
            ..Default::default()
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
            context_management_enabled: false,
            catalog_compression: None,
            virtual_instructions: vec![VirtualInstructions {
                section_title: "Skills".to_string(),
                content: "Call a skill's `get_info` tool to view its instructions.\n".to_string(),
                tool_names: vec!["skill_get_info".to_string()],
                priority: 30,
            }],
            ..Default::default()
        };

        let instructions = build_gateway_instructions(&ctx).unwrap();
        // Virtual tool listing in XML block (no bold header, tag is enough)
        assert!(instructions.contains("<skills>"));
        assert!(instructions.contains("`skill_get_info` (tool)"));
        assert!(instructions.contains("get_info"));
        assert!(instructions.contains("</skills>"));
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
            context_management_enabled: false,
            catalog_compression: None,
            virtual_instructions: vec![VirtualInstructions {
                section_title: "Skills".to_string(),
                content: "Call get_info to unlock.\n".to_string(),
                tool_names: vec!["skill_get_info".to_string()],
                priority: 30,
            }],
            ..Default::default()
        };

        let instructions = build_gateway_instructions(&ctx).unwrap();
        // Header
        assert!(instructions.contains("Unified MCP Gateway"));
        // Virtual server listed FIRST (use XML tags for position check)
        let skills_pos = instructions.find("<skills>").unwrap();
        let github_pos = instructions.find("<github>").unwrap();
        assert!(skills_pos < github_pos, "Virtual servers should come first");
        // Virtual tool listing
        assert!(instructions.contains("`skill_get_info` (tool)"));
        // Regular tools NOT listed in welcome (they are in tools/list)
        assert!(!instructions.contains("`github__create_issue` (tool)"));
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
            context_management_enabled: false,
            catalog_compression: None,
            virtual_instructions: Vec::new(),
            ..Default::default()
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
            context_management_enabled: false,
            catalog_compression: None,
            virtual_instructions: Vec::new(),
            ..Default::default()
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
            context_management_enabled: false,
            catalog_compression: None,
            virtual_instructions: Vec::new(),
            ..Default::default()
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
            context_management_enabled: false,
            catalog_compression: None,
            virtual_instructions: Vec::new(),
            ..Default::default()
        };

        let instructions = build_gateway_instructions(&ctx).unwrap();
        // Tools NOT listed in welcome (they are in tools/list)
        assert!(!instructions.contains("`barebones__tool` (tool)"));
        // Servers with tools still get XML blocks (even if empty)
        assert!(instructions.contains("<barebones>"));
        assert!(instructions.contains("</barebones>"));
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
            context_management_enabled: false,
            catalog_compression: None,
            virtual_instructions: Vec::new(),
            ..Default::default()
        };

        let instructions = build_gateway_instructions(&ctx).unwrap();
        // Tool/resource/prompt names NOT listed in welcome (they are in tools/list)
        assert!(!instructions.contains("`knowledge__search` (tool)"));
        assert!(!instructions.contains("`knowledge__docs` (resource)"));
        assert!(!instructions.contains("`knowledge__faq` (resource)"));
        assert!(!instructions.contains("`knowledge__summarize` (prompt)"));
        // But the description is still present
        assert!(instructions.contains("Knowledge base server"));
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
            context_management_enabled: false,
            catalog_compression: None,
            virtual_instructions: vec![
                VirtualInstructions {
                    section_title: "Skills".to_string(),
                    content: "Call a skill's `get_info` tool to view its instructions.\n".to_string(),
                    tool_names: vec![
                        "skill_get_info".to_string(),
                    ],
                    priority: 30,
                },
                VirtualInstructions {
                    section_title: "Marketplace".to_string(),
                    content: "Use marketplace tools to discover and install new MCP servers and skills.\n".to_string(),
                    tool_names: vec![
                        "MarketplaceSearch".to_string(),
                        "MarketplaceInstall".to_string(),
                    ],
                    priority: 20,
                },
            ],
            ..Default::default()
        };

        let instructions = build_gateway_instructions(&ctx).unwrap();

        // Virtual servers come first (use XML tags for position check)
        let skills_pos = instructions.find("<skills>").unwrap();
        let marketplace_pos = instructions.find("<marketplace>").unwrap();
        let filesystem_pos = instructions.find("<filesystem>").unwrap();
        let broken_pos = instructions.find("**broken-server**").unwrap();
        assert!(skills_pos < marketplace_pos);
        assert!(marketplace_pos < filesystem_pos);
        assert!(filesystem_pos < broken_pos);

        // Virtual tool annotations (still listed)
        assert!(instructions.contains("`skill_get_info` (tool)"));
        assert!(instructions.contains("`MarketplaceSearch` (tool)"));
        // Regular MCP tools/resources/prompts NOT listed in welcome
        assert!(!instructions.contains("`filesystem__read_file` (tool)"));
        assert!(!instructions.contains("`knowledge__search` (tool)"));
        assert!(!instructions.contains("`knowledge__docs` (resource)"));
        assert!(!instructions.contains("`knowledge__summarize` (prompt)"));

        // XML instructions for virtual servers
        assert!(instructions.contains("<skills>"));
        assert!(instructions.contains("</skills>"));
        assert!(instructions.contains("<marketplace>"));
        assert!(instructions.contains("</marketplace>"));

        // XML blocks for regular servers (all servers with tools get XML blocks)
        assert!(instructions.contains("<filesystem>"));
        assert!(instructions.contains("</filesystem>"));
        // knowledge also gets XML block (unified format)
        assert!(instructions.contains("<knowledge>"));
        assert!(instructions.contains("</knowledge>"));

        // Unavailable server
        assert!(instructions.contains("**broken-server** — unavailable: Connection refused"));
    }

    #[test]
    fn test_virtual_only_instructions_snapshot() {
        use crate::gateway::virtual_server::VirtualInstructions;

        let ctx = InstructionsContext {
            servers: Vec::new(),
            unavailable_servers: Vec::new(),
            context_management_enabled: false,
            catalog_compression: None,
            virtual_instructions: vec![VirtualInstructions {
                section_title: "Skills".to_string(),
                content: "Call get_info to unlock skills.\n".to_string(),
                tool_names: vec!["skill_get_info".to_string()],
                priority: 30,
            }],
            ..Default::default()
        };

        let instructions = build_gateway_instructions(&ctx).unwrap();
        assert!(instructions.contains("<skills>"));
        assert!(instructions.contains("`skill_get_info` (tool)"));
        assert!(instructions.contains("Call get_info to unlock skills."));
        assert!(instructions.contains("</skills>"));
        // No regular server content
        assert!(!instructions.contains("servername__"));
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

    // ── Context Management: estimate_catalog_size ───────────────────

    #[test]
    fn test_estimate_catalog_size_empty() {
        let ctx = InstructionsContext {
            servers: vec![],
            unavailable_servers: vec![],
            context_management_enabled: true,
            catalog_compression: None,
            virtual_instructions: vec![],
            ..Default::default()
        };
        // Only header overhead (100)
        assert_eq!(estimate_catalog_size(&ctx), 100);
    }

    #[test]
    fn test_estimate_catalog_size_with_servers() {
        let ctx = InstructionsContext {
            servers: vec![McpServerInstructionInfo {
                name: "filesystem".to_string(),
                description: Some("A file server".to_string()),
                instructions: None,
                tool_names: vec!["fs__read".to_string(), "fs__write".to_string()],
                resource_names: vec!["fs__config".to_string()],
                prompt_names: vec![],
            }],
            unavailable_servers: vec![],
            context_management_enabled: true,
            catalog_compression: None,
            virtual_instructions: vec![],
            ..Default::default()
        };
        let size = estimate_catalog_size(&ctx);
        // 100 + server header + tool lines + resource line + description + XML tags
        assert!(size > 100);
    }

    // ── Context Management: compute_catalog_compression_plan ────────

    fn make_large_server(name: &str, num_tools: usize) -> McpServerInstructionInfo {
        McpServerInstructionInfo {
            name: name.to_string(),
            description: Some("A".repeat(500)),
            instructions: Some("B".repeat(500)),
            tool_names: (0..num_tools)
                .map(|i| format!("{}__tool_{}", name, i))
                .collect(),
            resource_names: vec![format!("{}__config", name)],
            prompt_names: vec![format!("{}__ask", name)],
        }
    }

    #[test]
    fn test_no_compression_under_threshold() {
        let ctx = InstructionsContext {
            servers: vec![McpServerInstructionInfo {
                name: "tiny".to_string(),
                description: None,
                instructions: None,
                tool_names: vec!["tiny__a".to_string()],
                resource_names: vec![],
                prompt_names: vec![],
            }],
            unavailable_servers: vec![],
            context_management_enabled: true,
            catalog_compression: None,
            virtual_instructions: vec![],
            ..Default::default()
        };

        let plan = compute_catalog_compression_plan(&ctx, 100_000, true, true, true);
        assert!(plan.indexed_welcomes.is_empty());
        assert!(plan.deferred_servers.is_empty());
        assert!(plan.welcome_toc_dropped.is_empty());
        assert!(plan.batch_toc_dropped.is_empty());
    }

    #[test]
    fn test_phase1_indexes_large_welcomes() {
        let ctx = InstructionsContext {
            servers: vec![make_large_server("big", 5)],
            unavailable_servers: vec![],
            context_management_enabled: true,
            catalog_compression: None,
            virtual_instructions: vec![],
            ..Default::default()
        };

        // Set threshold low enough to trigger phase 1
        let full_size = estimate_catalog_size(&ctx);
        let threshold = full_size - 200;

        let plan = compute_catalog_compression_plan(&ctx, threshold, true, true, true);

        assert!(
            !plan.indexed_welcomes.is_empty(),
            "Phase 1 should index large welcomes"
        );
        assert_eq!(
            plan.indexed_welcomes[0].server_slug, "big",
            "Should index the 'big' server welcome"
        );
        assert!(
            plan.deferred_servers.is_empty(),
            "Phase 2 should not trigger"
        );
    }

    #[test]
    fn test_phase1_skips_short_welcomes() {
        let ctx = InstructionsContext {
            servers: vec![McpServerInstructionInfo {
                name: "tiny".to_string(),
                description: Some("Short.".to_string()), // < 256 bytes
                instructions: None,
                tool_names: vec!["tiny__a".to_string()],
                resource_names: vec![],
                prompt_names: vec![],
            }],
            ..Default::default()
        };

        let plan = compute_catalog_compression_plan(&ctx, 1, true, true, true);
        // Short welcome should not be indexed (< INDEX_MIN_BYTES)
        assert!(
            plan.indexed_welcomes.is_empty(),
            "Short welcomes should not be indexed"
        );
    }

    #[test]
    fn test_phase2_defers_servers() {
        let mut ctx = InstructionsContext {
            servers: vec![make_large_server("big", 20), make_large_server("small", 3)],
            unavailable_servers: vec![],
            context_management_enabled: true,
            catalog_compression: None,
            virtual_instructions: vec![],
            ..Default::default()
        };

        // Add mock definition sizes to trigger phase 2
        for s in &ctx.servers {
            for name in s
                .tool_names
                .iter()
                .chain(s.resource_names.iter())
                .chain(s.prompt_names.iter())
            {
                ctx.item_definition_sizes.insert(name.clone(), 200);
            }
        }

        // Set threshold very low to force phase 2
        let plan = compute_catalog_compression_plan(&ctx, 100, true, true, true);

        assert!(
            !plan.deferred_servers.is_empty(),
            "Phase 2 should defer servers when threshold is very low"
        );
        let deferred_slugs: Vec<&str> = plan
            .deferred_servers
            .iter()
            .map(|d| d.server_slug.as_str())
            .collect();
        assert!(
            deferred_slugs.contains(&"big"),
            "Largest server should be deferred first"
        );
    }

    #[test]
    fn test_phase2_no_deferral_without_list_changed() {
        let mut ctx = InstructionsContext {
            servers: vec![make_large_server("server", 20)],
            ..Default::default()
        };
        for name in &ctx.servers[0].tool_names.clone() {
            ctx.item_definition_sizes.insert(name.clone(), 200);
        }

        // No list_changed support → phase 2 does nothing
        let plan = compute_catalog_compression_plan(&ctx, 100, false, false, false);
        assert!(
            plan.deferred_servers.is_empty(),
            "Phase 2 should not defer without list_changed support"
        );
    }

    #[test]
    fn test_phase3_drops_welcome_toc() {
        let mut ctx = InstructionsContext {
            servers: vec![make_large_server("s1", 50), make_large_server("s2", 50)],
            ..Default::default()
        };
        for s in &ctx.servers {
            for name in s
                .tool_names
                .iter()
                .chain(s.resource_names.iter())
                .chain(s.prompt_names.iter())
            {
                ctx.item_definition_sizes.insert(name.clone(), 200);
            }
        }

        // Very low threshold should trigger all phases
        let plan = compute_catalog_compression_plan(&ctx, 10, true, true, true);

        // Phase 3 should drop welcome TOC for indexed servers
        if !plan.indexed_welcomes.is_empty() {
            assert!(
                !plan.welcome_toc_dropped.is_empty(),
                "Phase 3 should drop welcome TOC. Plan: {:?}",
                plan
            );
        }
    }

    #[test]
    fn test_phase4_drops_batch_toc() {
        let mut ctx = InstructionsContext {
            servers: vec![make_large_server("s1", 50), make_large_server("s2", 50)],
            ..Default::default()
        };
        for s in &ctx.servers {
            for name in s
                .tool_names
                .iter()
                .chain(s.resource_names.iter())
                .chain(s.prompt_names.iter())
            {
                ctx.item_definition_sizes.insert(name.clone(), 200);
            }
        }

        let plan = compute_catalog_compression_plan(&ctx, 1, true, true, true);

        // With extremely low threshold, all phases should trigger
        if !plan.deferred_servers.is_empty() {
            assert!(
                !plan.batch_toc_dropped.is_empty(),
                "Phase 4 should drop batch TOC. Plan: {:?}",
                plan
            );
        }
    }

    // ── Context Management: build_context_managed_instructions ──────

    #[test]
    fn test_cm_instructions_no_compression() {
        let ctx = InstructionsContext {
            servers: vec![McpServerInstructionInfo {
                name: "filesystem".to_string(),
                description: Some("File system server".to_string()),
                instructions: None,
                tool_names: vec!["filesystem__read_file".to_string()],
                resource_names: vec!["filesystem__config".to_string()],
                prompt_names: vec![],
            }],
            unavailable_servers: vec![],
            context_management_enabled: true,
            catalog_compression: Some(CatalogCompressionPlan::default()),
            virtual_instructions: vec![],
            ..Default::default()
        };

        let inst = build_context_managed_instructions(&ctx).unwrap();
        assert!(inst.contains("Unified MCP Gateway"));
        assert!(inst.contains("servername__"));
        // Uncompressed: raw description shown
        assert!(inst.contains("<filesystem>"));
        assert!(inst.contains("File system server"));
        assert!(inst.contains("</filesystem>"));
    }

    #[test]
    fn test_cm_instructions_with_indexed_welcome() {
        let plan = CatalogCompressionPlan {
            indexed_welcomes: vec![IndexedWelcome {
                server_slug: "filesystem".to_string(),
                summary: "Indexed \"mcp/filesystem\" \u{2014} 10 lines, 1.0KB, 3 chunks (0 code)".to_string(),
                toc: "## Contents\n- [L1] Description\n- [L5] Tools\n\nUse search(queries: [...]) to find specific content.\nUse read(source: \"mcp/filesystem\", offset: \"1\") to read sections.".to_string(),
                original_size: 1000,
            }],
            ..Default::default()
        };

        let ctx = InstructionsContext {
            servers: vec![McpServerInstructionInfo {
                name: "filesystem".to_string(),
                description: Some("Detailed docs about filesystem server".to_string()),
                instructions: Some("Use read_file to read files".to_string()),
                tool_names: vec!["filesystem__read_file".to_string()],
                resource_names: vec![],
                prompt_names: vec![],
            }],
            context_management_enabled: true,
            catalog_compression: Some(plan),
            ..Default::default()
        };

        let inst = build_context_managed_instructions(&ctx).unwrap();
        // Should show summary line
        assert!(
            inst.contains("Indexed \"mcp/filesystem\""),
            "Should contain index summary. Got:\n{}",
            inst
        );
        // Should show TOC
        assert!(
            inst.contains("## Contents"),
            "Should contain TOC. Got:\n{}",
            inst
        );
        // XML block present
        assert!(inst.contains("<filesystem>"));
        // Original instructions NOT inline
        assert!(!inst.contains("Detailed docs about filesystem server"));
    }

    #[test]
    fn test_cm_instructions_with_deferred_server() {
        let plan = CatalogCompressionPlan {
            deferred_servers: vec![DeferredServer {
                server_slug: "filesystem".to_string(),
                batches: vec![DeferredServerBatch {
                    batch_summary: "Indexed 2 items at \"mcp/filesystem/tool/\" \u{2014} 10 lines, 0.5KB, 4 chunks".to_string(),
                    batch_toc: "## Contents\n- filesystem__read_file\n- filesystem__write_file\n\nUse search(queries: [...], source: \"mcp/filesystem/tool/\") to discover items.".to_string(),
                }],
                definition_savings: 500,
            }],
            ..Default::default()
        };

        let ctx = InstructionsContext {
            servers: vec![McpServerInstructionInfo {
                name: "filesystem".to_string(),
                description: None,
                instructions: None,
                tool_names: vec![
                    "filesystem__read_file".to_string(),
                    "filesystem__write_file".to_string(),
                ],
                resource_names: vec![],
                prompt_names: vec![],
            }],
            context_management_enabled: true,
            catalog_compression: Some(plan),
            ..Default::default()
        };

        let inst = build_context_managed_instructions(&ctx).unwrap();
        // Should show batch summary
        assert!(
            inst.contains("Indexed 2 items"),
            "Should contain batch summary. Got:\n{}",
            inst
        );
        // Should show batch TOC
        assert!(
            inst.contains("filesystem__read_file"),
            "Should list deferred tools in TOC. Got:\n{}",
            inst
        );
    }

    #[test]
    fn test_cm_instructions_virtual_servers_never_compressed() {
        let plan = CatalogCompressionPlan::default();

        let ctx = InstructionsContext {
            servers: vec![],
            unavailable_servers: vec![],
            context_management_enabled: true,
            catalog_compression: Some(plan),
            virtual_instructions: vec![crate::gateway::virtual_server::VirtualInstructions {
                section_title: "Context Management".to_string(),
                content: "Use IndexSearch to find things".to_string(),
                tool_names: vec!["IndexSearch".to_string(), "ctx_execute".to_string()],
                priority: 0,
            }],
            ..Default::default()
        };

        let inst = build_context_managed_instructions(&ctx).unwrap();
        assert!(inst.contains("`IndexSearch` (tool)"));
        assert!(inst.contains("`ctx_execute` (tool)"));
        assert!(inst.contains("<context-management>"));
        assert!(inst.contains("Use IndexSearch to find things"));
    }

    #[test]
    fn test_cm_instructions_unavailable_servers() {
        let ctx = InstructionsContext {
            servers: vec![],
            unavailable_servers: vec![UnavailableServerInfo {
                name: "broken-server".to_string(),
                error: "Connection refused".to_string(),
            }],
            context_management_enabled: true,
            catalog_compression: Some(CatalogCompressionPlan::default()),
            virtual_instructions: vec![],
            ..Default::default()
        };

        let inst = build_context_managed_instructions(&ctx).unwrap();
        assert!(inst.contains("broken-server"));
        assert!(inst.contains("unavailable"));
        assert!(inst.contains("Connection refused"));
    }

    // ── End-to-end: compression plan → instructions ─────────────────

    #[test]
    fn test_e2e_compression_plan_applied_to_instructions() {
        let ctx = InstructionsContext {
            servers: vec![
                McpServerInstructionInfo {
                    name: "filesystem".to_string(),
                    description: Some("X".repeat(2000)),
                    instructions: Some("Y".repeat(2000)),
                    tool_names: (0..30).map(|i| format!("filesystem__tool_{}", i)).collect(),
                    resource_names: (0..5).map(|i| format!("filesystem__res_{}", i)).collect(),
                    prompt_names: vec!["filesystem__ask".to_string()],
                },
                McpServerInstructionInfo {
                    name: "github".to_string(),
                    description: Some("Z".repeat(1000)),
                    instructions: None,
                    tool_names: vec![
                        "github__create_issue".to_string(),
                        "github__list_prs".to_string(),
                    ],
                    resource_names: vec![],
                    prompt_names: vec![],
                },
            ],
            unavailable_servers: vec![],
            context_management_enabled: true,
            catalog_compression: None,
            virtual_instructions: vec![],
            ..Default::default()
        };

        let full_size = estimate_catalog_size(&ctx);
        assert!(
            full_size > 5000,
            "Full catalog should be large. Got: {}",
            full_size
        );

        let threshold = full_size * 2 / 5;
        let plan = compute_catalog_compression_plan(&ctx, threshold, true, true, true);

        assert!(
            !plan.indexed_welcomes.is_empty(),
            "Should have indexed welcomes"
        );

        let ctx_with_plan = InstructionsContext {
            catalog_compression: Some(plan),
            ..ctx.clone()
        };

        let inst = build_gateway_instructions(&ctx_with_plan).unwrap();
        assert!(
            inst.contains("Unified MCP Gateway"),
            "Should use CM instructions path"
        );
        assert!(
            inst.contains("Indexed"),
            "Output should contain Indexed markers"
        );
        assert!(
            inst.contains("IndexSearch"),
            "Should reference IndexSearch for discovery"
        );

        let uncompressed_ctx = InstructionsContext {
            context_management_enabled: false,
            catalog_compression: None,
            ..ctx
        };
        let uncompressed = build_gateway_instructions(&uncompressed_ctx).unwrap();
        assert!(
            inst.len() < uncompressed.len(),
            "Compressed instructions ({} bytes) should be smaller than uncompressed ({} bytes)",
            inst.len(),
            uncompressed.len()
        );
    }

    #[test]
    fn test_e2e_no_compression_when_small_catalog() {
        let ctx = InstructionsContext {
            servers: vec![McpServerInstructionInfo {
                name: "tiny".to_string(),
                description: None,
                instructions: None,
                tool_names: vec!["tiny__do_thing".to_string()],
                resource_names: vec![],
                prompt_names: vec![],
            }],
            context_management_enabled: true,
            ..Default::default()
        };

        let plan = compute_catalog_compression_plan(&ctx, 100_000, true, true, true);
        assert!(plan.indexed_welcomes.is_empty());
        assert!(plan.deferred_servers.is_empty());

        let ctx_with_plan = InstructionsContext {
            catalog_compression: Some(plan),
            ..ctx
        };

        let inst = build_gateway_instructions(&ctx_with_plan).unwrap();
        assert!(!inst.contains("Indexed"));
    }

    #[test]
    fn test_estimate_includes_definition_sizes() {
        let mut ctx = InstructionsContext {
            servers: vec![McpServerInstructionInfo {
                name: "test".to_string(),
                description: None,
                instructions: None,
                tool_names: vec!["test__tool".to_string()],
                resource_names: vec![],
                prompt_names: vec![],
            }],
            ..Default::default()
        };
        let base_size = estimate_catalog_size(&ctx);

        ctx.item_definition_sizes
            .insert("test__tool".to_string(), 1000);
        let with_defs = estimate_catalog_size(&ctx);
        assert_eq!(
            with_defs,
            base_size + 1000,
            "Should include definition sizes"
        );
    }

    #[test]
    fn test_path_scheme_mcp_prefix() {
        let ctx = InstructionsContext {
            servers: vec![make_large_server("github", 5)],
            ..Default::default()
        };
        let plan = compute_catalog_compression_plan(&ctx, 1, true, true, true);

        // All indexed welcomes should use mcp/ paths
        for w in &plan.indexed_welcomes {
            assert!(
                w.summary.contains("mcp/"),
                "IndexedWelcome summary should use mcp/ path. Got: {}",
                w.summary
            );
        }
    }

    // ── Comprehensive compression algorithm tests ──────────────────────

    #[test]
    fn test_phase1_sorts_by_welcome_size_descending() {
        let ctx = InstructionsContext {
            servers: vec![
                McpServerInstructionInfo {
                    name: "small".to_string(),
                    description: Some("S".repeat(300)),
                    instructions: None,
                    tool_names: vec!["small__a".to_string()],
                    resource_names: vec![],
                    prompt_names: vec![],
                },
                McpServerInstructionInfo {
                    name: "large".to_string(),
                    description: Some("L".repeat(1000)),
                    instructions: Some("I".repeat(1000)),
                    tool_names: vec!["large__a".to_string()],
                    resource_names: vec![],
                    prompt_names: vec![],
                },
                McpServerInstructionInfo {
                    name: "medium".to_string(),
                    description: Some("M".repeat(600)),
                    instructions: None,
                    tool_names: vec!["medium__a".to_string()],
                    resource_names: vec![],
                    prompt_names: vec![],
                },
            ],
            ..Default::default()
        };

        // Set threshold to force indexing but only partially
        let full_size = estimate_catalog_size(&ctx);
        let threshold = full_size - 500; // only need ~500 bytes savings

        let plan = compute_catalog_compression_plan(&ctx, threshold, true, true, true);

        assert!(
            !plan.indexed_welcomes.is_empty(),
            "Should have indexed at least one welcome"
        );
        // The largest server should be indexed first
        assert_eq!(
            plan.indexed_welcomes[0].server_slug, "large",
            "Largest welcome should be indexed first"
        );
    }

    #[test]
    fn test_phase2_sorts_by_definition_size_descending() {
        let mut ctx = InstructionsContext {
            servers: vec![
                McpServerInstructionInfo {
                    name: "few tools".to_string(),
                    description: Some("D".repeat(300)),
                    instructions: None,
                    tool_names: vec!["few-tools__a".to_string()],
                    resource_names: vec![],
                    prompt_names: vec![],
                },
                McpServerInstructionInfo {
                    name: "many tools".to_string(),
                    description: Some("D".repeat(300)),
                    instructions: None,
                    tool_names: (0..20).map(|i| format!("many-tools__tool_{}", i)).collect(),
                    resource_names: vec![],
                    prompt_names: vec![],
                },
            ],
            ..Default::default()
        };
        // Small definition for "few tools", large for "many tools"
        ctx.item_definition_sizes
            .insert("few-tools__a".to_string(), 100);
        for i in 0..20 {
            ctx.item_definition_sizes
                .insert(format!("many-tools__tool_{}", i), 500);
        }

        // Very low threshold to trigger phase 2
        let plan = compute_catalog_compression_plan(&ctx, 1, true, true, true);

        if plan.deferred_servers.len() >= 2 {
            // "many tools" has more definition savings, should be deferred first
            assert_eq!(
                plan.deferred_servers[0].server_slug, "many-tools",
                "Server with largest definitions should be deferred first"
            );
        }
    }

    #[test]
    fn test_phase1_stops_early_when_enough_saved() {
        let ctx = InstructionsContext {
            servers: vec![
                McpServerInstructionInfo {
                    name: "big1".to_string(),
                    description: Some("A".repeat(2000)),
                    instructions: None,
                    tool_names: vec!["big1__a".to_string()],
                    resource_names: vec![],
                    prompt_names: vec![],
                },
                McpServerInstructionInfo {
                    name: "big2".to_string(),
                    description: Some("B".repeat(2000)),
                    instructions: None,
                    tool_names: vec!["big2__a".to_string()],
                    resource_names: vec![],
                    prompt_names: vec![],
                },
            ],
            ..Default::default()
        };

        let full_size = estimate_catalog_size(&ctx);
        // Set threshold so indexing just one server is enough
        let threshold = full_size - 500;

        let plan = compute_catalog_compression_plan(&ctx, threshold, true, true, true);

        // May stop after first if savings were enough
        // At minimum, phase 2 should NOT trigger
        assert!(
            plan.deferred_servers.is_empty(),
            "Phase 2 should not trigger when Phase 1 saved enough"
        );
    }

    #[test]
    fn test_phase2_skips_servers_with_zero_definition_size() {
        let ctx = InstructionsContext {
            servers: vec![
                make_large_server("with-defs", 10),
                McpServerInstructionInfo {
                    name: "no defs".to_string(),
                    description: Some("D".repeat(500)),
                    instructions: None,
                    tool_names: vec!["no-defs__tool".to_string()],
                    resource_names: vec![],
                    prompt_names: vec![],
                },
            ],
            // Note: item_definition_sizes is empty, so no server has definition sizes
            ..Default::default()
        };

        let plan = compute_catalog_compression_plan(&ctx, 1, true, true, true);

        // No servers should be deferred since none have definition sizes
        assert!(
            plan.deferred_servers.is_empty(),
            "Should not defer servers with 0 definition size"
        );
    }

    #[test]
    fn test_server_can_be_both_indexed_and_deferred() {
        let mut ctx = InstructionsContext {
            servers: vec![make_large_server("server", 20)],
            ..Default::default()
        };
        // Add definition sizes so phase 2 can trigger
        for name in ctx.servers[0]
            .tool_names
            .iter()
            .chain(ctx.servers[0].resource_names.iter())
            .chain(ctx.servers[0].prompt_names.iter())
        {
            ctx.item_definition_sizes.insert(name.clone(), 500);
        }

        // Very low threshold should trigger both phases
        let plan = compute_catalog_compression_plan(&ctx, 1, true, true, true);

        let indexed_slugs: Vec<&str> = plan
            .indexed_welcomes
            .iter()
            .map(|w| w.server_slug.as_str())
            .collect();
        let deferred_slugs: Vec<&str> = plan
            .deferred_servers
            .iter()
            .map(|d| d.server_slug.as_str())
            .collect();

        // The same server should appear in both
        assert!(
            indexed_slugs.contains(&"server"),
            "Server should be indexed (Phase 1)"
        );
        assert!(
            deferred_slugs.contains(&"server"),
            "Server should also be deferred (Phase 2)"
        );
    }

    #[test]
    fn test_all_four_phases_with_very_low_threshold() {
        let mut ctx = InstructionsContext {
            servers: vec![make_large_server("s1", 30), make_large_server("s2", 30)],
            ..Default::default()
        };
        for s in &ctx.servers {
            for name in s
                .tool_names
                .iter()
                .chain(s.resource_names.iter())
                .chain(s.prompt_names.iter())
            {
                ctx.item_definition_sizes.insert(name.clone(), 300);
            }
        }

        // threshold=0 should trigger maximum compression
        let plan = compute_catalog_compression_plan(&ctx, 0, true, true, true);

        assert!(
            !plan.indexed_welcomes.is_empty(),
            "Phase 1 should index welcomes"
        );
        assert!(
            !plan.deferred_servers.is_empty(),
            "Phase 2 should defer servers"
        );
        // Phases 3 and 4 should also trigger since threshold=0
        assert!(
            !plan.welcome_toc_dropped.is_empty(),
            "Phase 3 should drop welcome TOC"
        );
        assert!(
            !plan.batch_toc_dropped.is_empty(),
            "Phase 4 should drop batch TOC"
        );
    }

    #[test]
    fn test_mixed_compression_states() {
        let mut ctx = InstructionsContext {
            servers: vec![
                // Large welcome + large definitions -> both indexed and deferred
                McpServerInstructionInfo {
                    name: "big".to_string(),
                    description: Some("B".repeat(1000)),
                    instructions: Some("I".repeat(1000)),
                    tool_names: (0..10).map(|i| format!("big__tool_{}", i)).collect(),
                    resource_names: vec![],
                    prompt_names: vec![],
                },
                // Small welcome + no definitions -> visible (no compression)
                McpServerInstructionInfo {
                    name: "tiny".to_string(),
                    description: Some("Tiny server.".to_string()),
                    instructions: None,
                    tool_names: vec!["tiny__do".to_string()],
                    resource_names: vec![],
                    prompt_names: vec![],
                },
            ],
            ..Default::default()
        };
        for i in 0..10 {
            ctx.item_definition_sizes
                .insert(format!("big__tool_{}", i), 500);
        }
        // No definition size for "tiny__do"

        let plan = compute_catalog_compression_plan(&ctx, 1, true, true, true);

        let indexed_slugs: Vec<&str> = plan
            .indexed_welcomes
            .iter()
            .map(|w| w.server_slug.as_str())
            .collect();
        let deferred_slugs: Vec<&str> = plan
            .deferred_servers
            .iter()
            .map(|d| d.server_slug.as_str())
            .collect();

        // "big" should be indexed and deferred
        assert!(
            indexed_slugs.contains(&"big"),
            "Big server should be indexed"
        );
        assert!(
            deferred_slugs.contains(&"big"),
            "Big server should be deferred"
        );
        // "tiny" should NOT be indexed (welcome too small) or deferred (no definitions)
        assert!(
            !indexed_slugs.contains(&"tiny"),
            "Tiny server should NOT be indexed (welcome < 256 bytes)"
        );
        assert!(
            !deferred_slugs.contains(&"tiny"),
            "Tiny server should NOT be deferred (no definition sizes)"
        );
    }

    #[test]
    fn test_deferred_server_definition_savings_correct() {
        let mut ctx = InstructionsContext {
            servers: vec![McpServerInstructionInfo {
                name: "server".to_string(),
                description: Some("D".repeat(500)),
                instructions: None,
                tool_names: vec!["server__tool_a".to_string(), "server__tool_b".to_string()],
                resource_names: vec!["server__res".to_string()],
                prompt_names: vec![],
            }],
            ..Default::default()
        };
        ctx.item_definition_sizes
            .insert("server__tool_a".to_string(), 400);
        ctx.item_definition_sizes
            .insert("server__tool_b".to_string(), 600);
        ctx.item_definition_sizes
            .insert("server__res".to_string(), 200);

        let plan = compute_catalog_compression_plan(&ctx, 1, true, true, true);

        if !plan.deferred_servers.is_empty() {
            let deferred = &plan.deferred_servers[0];
            // definition_savings should be total_def_size - batch_output_size
            // total_def_size = 400 + 600 + 200 = 1200
            assert!(
                deferred.definition_savings > 0,
                "Should have positive savings"
            );
            assert!(
                deferred.definition_savings <= 1200,
                "Savings cannot exceed total definition size"
            );
        }
    }

    // ── build_context_managed_instructions with all phases ──────────────

    #[test]
    fn test_cm_instructions_with_welcome_toc_dropped() {
        let plan = CatalogCompressionPlan {
            indexed_welcomes: vec![IndexedWelcome {
                server_slug: "filesystem".to_string(),
                summary: "Indexed \"mcp/filesystem\" — summary".to_string(),
                toc: "## Contents\n- Line 1\n- Line 5".to_string(),
                original_size: 1000,
            }],
            welcome_toc_dropped: vec!["filesystem".to_string()],
            ..Default::default()
        };

        let ctx = InstructionsContext {
            servers: vec![McpServerInstructionInfo {
                name: "filesystem".to_string(),
                description: Some("Full description here".to_string()),
                instructions: None,
                tool_names: vec!["filesystem__read_file".to_string()],
                resource_names: vec![],
                prompt_names: vec![],
            }],
            context_management_enabled: true,
            catalog_compression: Some(plan),
            ..Default::default()
        };

        let inst = build_context_managed_instructions(&ctx).unwrap();
        // Summary should be present
        assert!(
            inst.contains("Indexed \"mcp/filesystem\""),
            "Should contain summary"
        );
        // TOC should be dropped
        assert!(
            !inst.contains("## Contents"),
            "TOC should be dropped in Phase 3"
        );
        // Original description should NOT be shown (replaced by summary)
        assert!(!inst.contains("Full description here"));
    }

    #[test]
    fn test_cm_instructions_with_batch_toc_dropped() {
        let plan = CatalogCompressionPlan {
            deferred_servers: vec![DeferredServer {
                server_slug: "filesystem".to_string(),
                batches: vec![DeferredServerBatch {
                    batch_summary: "Indexed 2 tools at mcp/filesystem/tool/".to_string(),
                    batch_toc: "## Tool List\n- read_file\n- write_file".to_string(),
                }],
                definition_savings: 500,
            }],
            batch_toc_dropped: vec!["filesystem".to_string()],
            ..Default::default()
        };

        let ctx = InstructionsContext {
            servers: vec![McpServerInstructionInfo {
                name: "filesystem".to_string(),
                description: None,
                instructions: None,
                tool_names: vec![
                    "filesystem__read_file".to_string(),
                    "filesystem__write_file".to_string(),
                ],
                resource_names: vec![],
                prompt_names: vec![],
            }],
            context_management_enabled: true,
            catalog_compression: Some(plan),
            ..Default::default()
        };

        let inst = build_context_managed_instructions(&ctx).unwrap();
        // Batch summary should be present
        assert!(
            inst.contains("Indexed 2 tools"),
            "Should contain batch summary"
        );
        // Batch TOC should be dropped
        assert!(
            !inst.contains("## Tool List"),
            "Batch TOC should be dropped in Phase 4"
        );
    }

    #[test]
    fn test_cm_instructions_indexed_and_deferred_same_server() {
        let plan = CatalogCompressionPlan {
            indexed_welcomes: vec![IndexedWelcome {
                server_slug: "github".to_string(),
                summary: "Indexed \"mcp/github\" — compressed summary".to_string(),
                toc: "## Welcome TOC\n- issues\n- prs".to_string(),
                original_size: 2000,
            }],
            deferred_servers: vec![DeferredServer {
                server_slug: "github".to_string(),
                batches: vec![DeferredServerBatch {
                    batch_summary: "Indexed 10 tools at mcp/github/tool/".to_string(),
                    batch_toc: "## Tool Index\n- create_issue\n- list_prs".to_string(),
                }],
                definition_savings: 1500,
            }],
            ..Default::default()
        };

        let ctx = InstructionsContext {
            servers: vec![McpServerInstructionInfo {
                name: "github".to_string(),
                description: Some("Full GitHub server description".to_string()),
                instructions: Some("Full GitHub instructions".to_string()),
                tool_names: (0..10).map(|i| format!("github__tool_{}", i)).collect(),
                resource_names: vec![],
                prompt_names: vec![],
            }],
            context_management_enabled: true,
            catalog_compression: Some(plan),
            ..Default::default()
        };

        let inst = build_context_managed_instructions(&ctx).unwrap();
        // Welcome should show summary, not original text
        assert!(inst.contains("compressed summary"));
        assert!(!inst.contains("Full GitHub server description"));
        // Welcome TOC should be present (not in welcome_toc_dropped)
        assert!(inst.contains("## Welcome TOC"));
        // Batch summary and TOC should be present
        assert!(inst.contains("Indexed 10 tools"));
        assert!(inst.contains("## Tool Index"));
    }

    #[test]
    fn test_cm_instructions_preserves_uncompressed_servers() {
        let plan = CatalogCompressionPlan {
            indexed_welcomes: vec![IndexedWelcome {
                server_slug: "big".to_string(),
                summary: "Indexed \"mcp/big\" — summary".to_string(),
                toc: "## TOC".to_string(),
                original_size: 1000,
            }],
            ..Default::default()
        };

        let ctx = InstructionsContext {
            servers: vec![
                McpServerInstructionInfo {
                    name: "big".to_string(),
                    description: Some("Big description".to_string()),
                    instructions: None,
                    tool_names: vec!["big__tool".to_string()],
                    resource_names: vec![],
                    prompt_names: vec![],
                },
                McpServerInstructionInfo {
                    name: "small".to_string(),
                    description: Some("Small description".to_string()),
                    instructions: Some("Small instructions".to_string()),
                    tool_names: vec!["small__tool".to_string()],
                    resource_names: vec![],
                    prompt_names: vec![],
                },
            ],
            context_management_enabled: true,
            catalog_compression: Some(plan),
            ..Default::default()
        };

        let inst = build_context_managed_instructions(&ctx).unwrap();
        // "big" should show summary
        assert!(inst.contains("Indexed \"mcp/big\""));
        assert!(!inst.contains("Big description"));
        // "small" should show original description and instructions
        assert!(inst.contains("Small description"));
        assert!(inst.contains("Small instructions"));
    }

    #[test]
    fn test_estimate_catalog_size_with_virtual_and_unavailable() {
        use crate::gateway::virtual_server::VirtualInstructions;

        let ctx = InstructionsContext {
            servers: vec![McpServerInstructionInfo {
                name: "server".to_string(),
                description: Some("desc".to_string()),
                instructions: None,
                tool_names: vec!["server__tool".to_string()],
                resource_names: vec![],
                prompt_names: vec![],
            }],
            unavailable_servers: vec![UnavailableServerInfo {
                name: "dead".to_string(),
                error: "Connection refused".to_string(),
            }],
            virtual_instructions: vec![VirtualInstructions {
                section_title: "Context Management".to_string(),
                content: "Use IndexSearch.".to_string(),
                tool_names: vec!["IndexSearch".to_string()],
                priority: 0,
            }],
            ..Default::default()
        };

        let size = estimate_catalog_size(&ctx);
        // Should include all components
        assert!(
            size > 100, // base overhead
            "Size should include server + unavailable + virtual"
        );
        // Check it includes the unavailable server
        let ctx_without_unavailable = InstructionsContext {
            unavailable_servers: vec![],
            ..ctx.clone()
        };
        assert!(
            size > estimate_catalog_size(&ctx_without_unavailable),
            "Unavailable servers should add to the estimate"
        );
    }

    #[test]
    fn test_phase2_creates_batches_per_item_type() {
        let mut ctx = InstructionsContext {
            servers: vec![McpServerInstructionInfo {
                name: "multi".to_string(),
                description: Some("D".repeat(500)),
                instructions: None,
                tool_names: vec!["multi__tool1".to_string(), "multi__tool2".to_string()],
                resource_names: vec!["multi__res1".to_string()],
                prompt_names: vec!["multi__prompt1".to_string()],
            }],
            ..Default::default()
        };
        ctx.item_definition_sizes
            .insert("multi__tool1".to_string(), 300);
        ctx.item_definition_sizes
            .insert("multi__tool2".to_string(), 300);
        ctx.item_definition_sizes
            .insert("multi__res1".to_string(), 200);
        ctx.item_definition_sizes
            .insert("multi__prompt1".to_string(), 200);

        let plan = compute_catalog_compression_plan(&ctx, 1, true, true, true);

        if !plan.deferred_servers.is_empty() {
            let deferred = &plan.deferred_servers[0];
            // Should have 3 batches: one for tools, one for resources, one for prompts
            assert_eq!(
                deferred.batches.len(),
                3,
                "Should create separate batches for tools, resources, and prompts"
            );
        }
    }

    #[test]
    fn test_compression_plan_empty_servers() {
        let ctx = InstructionsContext {
            servers: vec![],
            ..Default::default()
        };

        let plan = compute_catalog_compression_plan(&ctx, 0, true, true, true);
        assert!(plan.indexed_welcomes.is_empty());
        assert!(plan.deferred_servers.is_empty());
        assert!(plan.welcome_toc_dropped.is_empty());
        assert!(plan.batch_toc_dropped.is_empty());
    }

    #[test]
    fn test_threshold_exactly_at_catalog_size() {
        let ctx = InstructionsContext {
            servers: vec![make_large_server("server", 5)],
            ..Default::default()
        };

        let exact_size = estimate_catalog_size(&ctx);
        let plan = compute_catalog_compression_plan(&ctx, exact_size, true, true, true);

        // At exactly the threshold, no compression needed
        assert!(
            plan.indexed_welcomes.is_empty(),
            "No compression when threshold equals catalog size"
        );
    }

    #[test]
    fn test_threshold_one_below_catalog_size() {
        let ctx = InstructionsContext {
            servers: vec![make_large_server("server", 5)],
            ..Default::default()
        };

        let exact_size = estimate_catalog_size(&ctx);
        let plan = compute_catalog_compression_plan(&ctx, exact_size - 1, true, true, true);

        // Just 1 byte over should still trigger compression
        assert!(
            !plan.indexed_welcomes.is_empty(),
            "Should compress when 1 byte over threshold"
        );
    }

    #[test]
    fn test_e2e_compressed_output_smaller_with_all_phases() {
        let mut ctx = InstructionsContext {
            servers: vec![make_large_server("s1", 20), make_large_server("s2", 20)],
            context_management_enabled: true,
            ..Default::default()
        };
        for s in &ctx.servers {
            for name in s
                .tool_names
                .iter()
                .chain(s.resource_names.iter())
                .chain(s.prompt_names.iter())
            {
                ctx.item_definition_sizes.insert(name.clone(), 200);
            }
        }

        // Build uncompressed
        ctx.catalog_compression = None;
        let uncompressed = build_gateway_instructions(&ctx).unwrap();

        // Build compressed with threshold=0
        let plan = compute_catalog_compression_plan(&ctx, 0, true, true, true);
        ctx.catalog_compression = Some(plan);
        let compressed = build_gateway_instructions(&ctx).unwrap();

        assert!(
            compressed.len() < uncompressed.len(),
            "Compressed ({} bytes) should be smaller than uncompressed ({} bytes)",
            compressed.len(),
            uncompressed.len()
        );
    }
}
