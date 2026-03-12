// Empty import section - using json! macro from types.rs

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

        for name in &server.tool_names {
            inst.push_str(&format!("- `{}` (tool)\n", name));
        }
        for name in &server.resource_names {
            inst.push_str(&format!("- `{}` (resource)\n", name));
        }
        for name in &server.prompt_names {
            inst.push_str(&format!("- `{}` (prompt)\n", name));
        }

        if let Some(desc) = &server.description {
            inst.push('\n');
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
                    "ctx_execute_file".to_string(),
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
                content: "Call a skill's `get_info` tool to view its instructions, then use `ctx_execute_file` with the absolute script path to run it.\n".to_string(),
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
                "ctx_execute_file".to_string(),
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
            content: "Call a skill's `get_info` tool to view its instructions, then use `ctx_execute_file` with the absolute script path to run it.\n".to_string(),
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

            let savings = def_size.saturating_sub(batch_output_size);
            if savings > 0 && !batches.is_empty() {
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
        // Tool listing with type annotations inside XML block
        assert!(instructions.contains("<filesystem>"));
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
                content: "Call a skill's `get_info` tool to view its instructions, then use `ctx_execute_file` with the absolute script path to run it.\n"
                    .to_string(),
                tool_names: vec!["skill_get_info".to_string()],
                priority: 30,
            }],
            ..Default::default()
        };

        let instructions = build_gateway_instructions(&ctx).unwrap();
        // Virtual tool listing in XML block (no bold header, tag is enough)
        assert!(instructions.contains("<skills>"));
        assert!(instructions.contains("`skill_get_info` (tool)"));
        assert!(instructions.contains("ctx_execute_file"));
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
        assert!(instructions.contains("`barebones__tool` (tool)"));
        // Servers with tools now always get XML blocks in the unified format
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
            context_management_enabled: false,
            catalog_compression: None,
            virtual_instructions: vec![
                VirtualInstructions {
                    section_title: "Skills".to_string(),
                    content: "Call a skill's `get_info` tool to view its instructions, then use `ctx_execute_file` with the absolute script path to run it.\n".to_string(),
                    tool_names: vec![
                        "skill_get_info".to_string(),
                    ],
                    priority: 30,
                },
                VirtualInstructions {
                    section_title: "Marketplace".to_string(),
                    content: "Use marketplace tools to discover and install new MCP servers and skills.\n".to_string(),
                    tool_names: vec![
                        "marketplace__search".to_string(),
                        "marketplace__install".to_string(),
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

        // Tool annotations
        assert!(instructions.contains("`skill_get_info` (tool)"));
        assert!(instructions.contains("`marketplace__search` (tool)"));
        assert!(instructions.contains("`filesystem__read_file` (tool)"));
        assert!(instructions.contains("`knowledge__search` (tool)"));
        assert!(instructions.contains("`knowledge__docs` (resource)"));
        assert!(instructions.contains("`knowledge__summarize` (prompt)"));

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
}
