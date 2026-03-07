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
    /// Whether deferred loading is enabled (legacy)
    pub deferred_loading: bool,
    /// Whether context management is enabled (replaces deferred_loading)
    pub context_management_enabled: bool,
    /// Whether indexing tools (ctx_execute, etc.) are exposed
    pub indexing_tools_enabled: bool,
    /// Catalog compression plan (computed when context management is enabled)
    pub catalog_compression: Option<CatalogCompressionPlan>,
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

    // Context management path — uses compression plan
    if ctx.context_management_enabled {
        return build_context_managed_instructions(ctx);
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

// ─── Context Management: Compression + Welcome Text ────────────────────────

/// Build welcome text when context management is enabled.
/// Applies the catalog compression plan to produce compressed output.
fn build_context_managed_instructions(ctx: &InstructionsContext) -> Option<String> {
    let mut inst = String::new();

    // Header
    let server_count = ctx.servers.len();
    if server_count == 0 && ctx.virtual_instructions.is_empty() {
        inst.push_str(
            "Unified MCP Gateway — Context-Managed: no servers or tools are currently available.\n\n",
        );
    } else {
        inst.push_str(&format!(
            "Unified MCP Gateway — Context-Managed\n\n{} server{} connected. \
             Use ctx_search to discover MCP capabilities, retrieve compressed content, \
             and search server docs.\n\n",
            server_count,
            if server_count == 1 { "" } else { "s" }
        ));
    }

    let plan = ctx.catalog_compression.as_ref();
    let compressed_names: std::collections::HashSet<&str> = plan
        .map(|p| {
            p.compressed_descriptions
                .iter()
                .map(|c| c.namespaced_name.as_str())
                .collect()
        })
        .unwrap_or_default();
    let deferred_servers: std::collections::HashSet<&str> = plan
        .map(|p| {
            p.deferred_items
                .iter()
                .map(|d| d.server_slug.as_str())
                .collect()
        })
        .unwrap_or_default();
    let truncated_servers: std::collections::HashSet<&str> = plan
        .map(|p| p.truncated_servers.iter().map(|s| s.as_str()).collect())
        .unwrap_or_default();

    // Virtual server tools first (never compressed)
    for vsi in &ctx.virtual_instructions {
        inst.push_str(&format!("**{}**\n", vsi.section_title));
        for name in &vsi.tool_names {
            inst.push_str(&format!("- `{}` (tool)\n", name));
        }
        inst.push('\n');
    }

    // Regular MCP servers
    for server in &ctx.servers {
        let server_slug = slugify(&server.name);

        // Check if this server is fully truncated to counts
        if truncated_servers.contains(server_slug.as_str()) {
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
            inst.push_str(&format!(
                "- {}: {} — ctx_search(source=\"catalog:{}\") to explore\n",
                server.name,
                parts.join(", "),
                server_slug
            ));
            continue;
        }

        inst.push_str(&format!("**{}**\n", server.name));

        // List tools (compressed or full)
        for name in &server.tool_names {
            if compressed_names.contains(name.as_str()) {
                inst.push_str(&format!(
                    "- {} — [compressed] ctx_search(queries=[\"{}\"], source=\"catalog:{}\")\n",
                    name,
                    name.rsplit("__").next().unwrap_or(name),
                    name
                ));
            } else {
                inst.push_str(&format!("- `{}` (tool)\n", name));
            }
        }

        // List resources (compressed or full)
        for name in &server.resource_names {
            if compressed_names.contains(name.as_str()) {
                inst.push_str(&format!(
                    "- {} — [compressed] ctx_search(queries=[\"{}\"], source=\"catalog:{}\")\n",
                    name,
                    name.rsplit("__").next().unwrap_or(name),
                    name
                ));
            } else {
                inst.push_str(&format!("- `{}` (resource)\n", name));
            }
        }

        // List prompts (compressed or full)
        for name in &server.prompt_names {
            if compressed_names.contains(name.as_str()) {
                inst.push_str(&format!(
                    "- {} — [compressed] ctx_search(queries=[\"{}\"], source=\"catalog:{}\")\n",
                    name,
                    name.rsplit("__").next().unwrap_or(name),
                    name
                ));
            } else {
                inst.push_str(&format!("- `{}` (prompt)\n", name));
            }
        }

        inst.push('\n');
    }

    // Unavailable servers
    for server in &ctx.unavailable_servers {
        inst.push_str(&format!(
            "**{}** — unavailable: {}\n\n",
            server.name, server.error
        ));
    }

    // Virtual server instructions (always inline, never compressed)
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

    // Server instructions: inline unless compressed
    for server in &ctx.servers {
        let server_slug = slugify(&server.name);

        // Skip if server is truncated or its welcome text is compressed
        if truncated_servers.contains(server_slug.as_str()) {
            continue;
        }
        if compressed_names.contains(server_slug.as_str()) {
            inst.push_str(&format!(
                "\n- {} instructions — [compressed] ctx_search(queries=[\"{}\"], source=\"catalog:{}\")\n",
                server.name, server_slug, server_slug
            ));
            continue;
        }

        // Include inline (same as non-deferred mode)
        let has_content = server.instructions.is_some() || server.description.is_some();
        if !has_content {
            continue;
        }

        let tag = &server_slug;
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

/// Compute a catalog compression plan using the progressive algorithm.
///
/// Phases:
/// 1. Compress individual descriptions (largest first) — index + replace with one-liner
/// 2. Defer items entirely (hide from tools/list) — only if client supports *_changed
/// 3. Truncate server listings to counts only
pub fn compute_catalog_compression_plan(
    ctx: &InstructionsContext,
    threshold: usize,
    supports_tools_list_changed: bool,
    supports_resources_list_changed: bool,
    supports_prompts_list_changed: bool,
) -> CatalogCompressionPlan {
    let mut plan = CatalogCompressionPlan::default();

    // Build the full uncompressed catalog to measure size
    let full_size = estimate_catalog_size(ctx);

    if full_size <= threshold {
        // No compression needed
        return plan;
    }

    let bytes_to_save = full_size - threshold;

    // Phase 1: Collect all compressible items with their sizes, sorted descending
    let mut compressible_items: Vec<CompressibleCandidate> = Vec::new();

    for server in &ctx.servers {
        let server_slug = slugify(&server.name);

        // Server welcome/instructions text
        let welcome_size = server
            .description
            .as_ref()
            .map(|d| d.len())
            .unwrap_or(0)
            + server
                .instructions
                .as_ref()
                .map(|i| i.len())
                .unwrap_or(0);
        if welcome_size > 0 {
            compressible_items.push(CompressibleCandidate {
                namespaced_name: server_slug.clone(),
                source_label: format!("catalog:{}", server_slug),
                full_content: build_full_server_content(server),
                item_type: CompressedItemType::ServerWelcome,
                byte_size: welcome_size,
                server_slug: server_slug.clone(),
            });
        }

        // Individual tool descriptions (estimated ~80 bytes per tool line in listing)
        for name in &server.tool_names {
            let line_size = name.len() + 15; // "- `name` (tool)\n"
            compressible_items.push(CompressibleCandidate {
                namespaced_name: name.clone(),
                source_label: format!("catalog:{}", name),
                full_content: name.clone(), // Will be populated with full description at index time
                item_type: CompressedItemType::Tool,
                byte_size: line_size,
                server_slug: server_slug.clone(),
            });
        }

        for name in &server.resource_names {
            let line_size = name.len() + 19; // "- `name` (resource)\n"
            compressible_items.push(CompressibleCandidate {
                namespaced_name: name.clone(),
                source_label: format!("catalog:{}", name),
                full_content: name.clone(),
                item_type: CompressedItemType::Resource,
                byte_size: line_size,
                server_slug: server_slug.clone(),
            });
        }

        for name in &server.prompt_names {
            let line_size = name.len() + 17; // "- `name` (prompt)\n"
            compressible_items.push(CompressibleCandidate {
                namespaced_name: name.clone(),
                source_label: format!("catalog:{}", name),
                full_content: name.clone(),
                item_type: CompressedItemType::Prompt,
                byte_size: line_size,
                server_slug: server_slug.clone(),
            });
        }
    }

    // Sort by size descending — compress largest items first
    compressible_items.sort_by(|a, b| b.byte_size.cmp(&a.byte_size));

    // Batch-compress items until we've saved enough bytes
    let mut saved = 0usize;
    for candidate in &compressible_items {
        if saved >= bytes_to_save {
            break;
        }
        // Compressed one-liner is ~80 bytes (name + search hint)
        let one_liner_size = candidate.namespaced_name.len() + 80;
        let savings = candidate.byte_size.saturating_sub(one_liner_size);
        if savings == 0 {
            continue;
        }
        plan.compressed_descriptions.push(CompressedItem {
            source_label: candidate.source_label.clone(),
            full_content: candidate.full_content.clone(),
            item_type: candidate.item_type.clone(),
            namespaced_name: candidate.namespaced_name.clone(),
            byte_size: candidate.byte_size,
        });
        saved += savings;
    }

    if saved >= bytes_to_save {
        return plan;
    }

    // Phase 2: Defer items entirely (hide from listing)
    // Group remaining items by server, defer the server with the most items
    let mut server_item_counts: std::collections::HashMap<&str, usize> =
        std::collections::HashMap::new();
    for server in &ctx.servers {
        let slug = slugify(&server.name);
        let count = server.tool_names.len() + server.resource_names.len() + server.prompt_names.len();
        server_item_counts.insert(Box::leak(slug.into_boxed_str()), count);
    }

    // Sort servers by item count descending
    let mut server_slugs: Vec<String> = ctx.servers.iter().map(|s| slugify(&s.name)).collect();
    server_slugs.sort_by(|a, b| {
        let count_a = ctx
            .servers
            .iter()
            .find(|s| slugify(&s.name) == *a)
            .map(|s| s.tool_names.len() + s.resource_names.len() + s.prompt_names.len())
            .unwrap_or(0);
        let count_b = ctx
            .servers
            .iter()
            .find(|s| slugify(&s.name) == *b)
            .map(|s| s.tool_names.len() + s.resource_names.len() + s.prompt_names.len())
            .unwrap_or(0);
        count_b.cmp(&count_a)
    });

    for server_slug in &server_slugs {
        if saved >= bytes_to_save {
            break;
        }

        if supports_tools_list_changed {
            plan.deferred_items.push(DeferredItem {
                server_slug: server_slug.clone(),
                item_type: DeferredItemType::Tools,
            });
        }
        if supports_resources_list_changed {
            plan.deferred_items.push(DeferredItem {
                server_slug: server_slug.clone(),
                item_type: DeferredItemType::Resources,
            });
        }
        if supports_prompts_list_changed {
            plan.deferred_items.push(DeferredItem {
                server_slug: server_slug.clone(),
                item_type: DeferredItemType::Prompts,
            });
        }

        // Estimate savings from deferring this server's items (only count actually deferred types)
        if let Some(server) = ctx.servers.iter().find(|s| slugify(&s.name) == *server_slug) {
            let mut items_bytes = 0usize;
            if supports_tools_list_changed {
                items_bytes += server.tool_names.iter().map(|n| n.len() + 20).sum::<usize>();
            }
            if supports_resources_list_changed {
                items_bytes += server.resource_names.iter().map(|n| n.len() + 20).sum::<usize>();
            }
            if supports_prompts_list_changed {
                items_bytes += server.prompt_names.iter().map(|n| n.len() + 20).sum::<usize>();
            }
            saved += items_bytes;
        }
    }

    if saved >= bytes_to_save {
        return plan;
    }

    // Phase 3: Truncate remaining server listings to counts only
    for server_slug in &server_slugs {
        if saved >= bytes_to_save {
            break;
        }
        // Only truncate if not already fully deferred
        let already_deferred = plan
            .deferred_items
            .iter()
            .any(|d| d.server_slug == *server_slug);
        if already_deferred {
            continue;
        }

        plan.truncated_servers.push(server_slug.clone());

        // Estimate savings
        if let Some(server) = ctx.servers.iter().find(|s| slugify(&s.name) == *server_slug) {
            let items_bytes: usize = server
                .tool_names
                .iter()
                .chain(server.resource_names.iter())
                .chain(server.prompt_names.iter())
                .map(|n| n.len() + 20)
                .sum();
            // Truncated line is ~80 bytes
            saved += items_bytes.saturating_sub(80);
        }
    }

    plan
}

/// Estimate the total byte size of the catalog (welcome text).
fn estimate_catalog_size(ctx: &InstructionsContext) -> usize {
    let mut size = 100; // header overhead

    for server in &ctx.servers {
        size += server.name.len() + 10; // server header
        for name in &server.tool_names {
            size += name.len() + 15;
        }
        for name in &server.resource_names {
            size += name.len() + 19;
        }
        for name in &server.prompt_names {
            size += name.len() + 17;
        }
        // Server instructions
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

    size
}

/// Internal candidate for compression planning.
struct CompressibleCandidate {
    namespaced_name: String,
    source_label: String,
    full_content: String,
    item_type: CompressedItemType,
    byte_size: usize,
    #[allow(dead_code)]
    server_slug: String,
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
            context_management_enabled: false,
            indexing_tools_enabled: false,
            catalog_compression: None,
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
            context_management_enabled: false,
            indexing_tools_enabled: false,
            catalog_compression: None,
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
            context_management_enabled: false,
            indexing_tools_enabled: false,
            catalog_compression: None,
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
            context_management_enabled: false,
            indexing_tools_enabled: false,
            catalog_compression: None,
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
            context_management_enabled: false,
            indexing_tools_enabled: false,
            catalog_compression: None,
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
            context_management_enabled: false,
            indexing_tools_enabled: false,
            catalog_compression: None,
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
            context_management_enabled: false,
            indexing_tools_enabled: false,
            catalog_compression: None,
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
            context_management_enabled: false,
            indexing_tools_enabled: false,
            catalog_compression: None,
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
            context_management_enabled: false,
            indexing_tools_enabled: false,
            catalog_compression: None,
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
            context_management_enabled: false,
            indexing_tools_enabled: false,
            catalog_compression: None,
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
            context_management_enabled: false,
            indexing_tools_enabled: false,
            catalog_compression: None,
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
            context_management_enabled: false,
            indexing_tools_enabled: false,
            catalog_compression: None,
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
            context_management_enabled: false,
            indexing_tools_enabled: false,
            catalog_compression: None,
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
            context_management_enabled: false,
            indexing_tools_enabled: false,
            catalog_compression: None,
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

    // ── Context Management: estimate_catalog_size ───────────────────

    #[test]
    fn test_estimate_catalog_size_empty() {
        let ctx = InstructionsContext {
            servers: vec![],
            unavailable_servers: vec![],
            deferred_loading: false,
            context_management_enabled: true,
            indexing_tools_enabled: false,
            catalog_compression: None,
            virtual_instructions: vec![],
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
            deferred_loading: false,
            context_management_enabled: true,
            indexing_tools_enabled: false,
            catalog_compression: None,
            virtual_instructions: vec![],
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
            deferred_loading: false,
            context_management_enabled: true,
            indexing_tools_enabled: false,
            catalog_compression: None,
            virtual_instructions: vec![],
        };

        let plan = compute_catalog_compression_plan(&ctx, 100_000, true, true, true);
        assert!(plan.compressed_descriptions.is_empty());
        assert!(plan.deferred_items.is_empty());
        assert!(plan.truncated_servers.is_empty());
    }

    #[test]
    fn test_phase1_compresses_largest_first() {
        let ctx = InstructionsContext {
            servers: vec![make_large_server("big", 5)],
            unavailable_servers: vec![],
            deferred_loading: false,
            context_management_enabled: true,
            indexing_tools_enabled: false,
            catalog_compression: None,
            virtual_instructions: vec![],
        };

        // Set threshold low enough to trigger phase 1 but not phase 2
        let full_size = estimate_catalog_size(&ctx);
        // Remove ~200 bytes (threshold = full_size - 200) to trigger mild compression
        let threshold = full_size - 200;

        let plan = compute_catalog_compression_plan(&ctx, threshold, true, true, true);

        // Should compress descriptions but not defer
        assert!(
            !plan.compressed_descriptions.is_empty(),
            "Phase 1 should compress some descriptions"
        );
        // The largest item (server welcome at ~1000 bytes) should be compressed first
        assert_eq!(
            plan.compressed_descriptions[0].item_type,
            CompressedItemType::ServerWelcome,
            "Server welcome (largest) should be compressed first"
        );
        assert!(plan.deferred_items.is_empty(), "Phase 2 should not trigger");
        assert!(
            plan.truncated_servers.is_empty(),
            "Phase 3 should not trigger"
        );
    }

    #[test]
    fn test_phase2_defers_items() {
        let ctx = InstructionsContext {
            servers: vec![make_large_server("big", 20), make_large_server("small", 3)],
            unavailable_servers: vec![],
            deferred_loading: false,
            context_management_enabled: true,
            indexing_tools_enabled: false,
            catalog_compression: None,
            virtual_instructions: vec![],
        };

        // Set threshold very low to force phase 2
        let plan = compute_catalog_compression_plan(&ctx, 100, true, true, true);

        // Phase 2 should defer items
        assert!(
            !plan.deferred_items.is_empty(),
            "Phase 2 should defer items when threshold is very low"
        );
        // The server with more items should be deferred first
        let deferred_server_slugs: Vec<&str> = plan
            .deferred_items
            .iter()
            .map(|d| d.server_slug.as_str())
            .collect();
        assert!(
            deferred_server_slugs.contains(&"big"),
            "Largest server should be deferred first"
        );
    }

    #[test]
    fn test_phase2_respects_capabilities() {
        let ctx = InstructionsContext {
            servers: vec![make_large_server("server", 20)],
            unavailable_servers: vec![],
            deferred_loading: false,
            context_management_enabled: true,
            indexing_tools_enabled: false,
            catalog_compression: None,
            virtual_instructions: vec![],
        };

        // Only supports tools list_changed — not resources or prompts
        let plan = compute_catalog_compression_plan(&ctx, 100, true, false, false);

        let deferred_types: Vec<&DeferredItemType> =
            plan.deferred_items.iter().map(|d| &d.item_type).collect();

        // Should have tools deferred
        assert!(deferred_types.contains(&&DeferredItemType::Tools));
        // Should NOT have resources or prompts deferred (client doesn't support list_changed)
        assert!(!deferred_types.contains(&&DeferredItemType::Resources));
        assert!(!deferred_types.contains(&&DeferredItemType::Prompts));
    }

    #[test]
    fn test_phase3_truncates_servers() {
        // Create a context so large that phases 1+2 aren't enough
        let ctx = InstructionsContext {
            servers: vec![
                make_large_server("s1", 50),
                make_large_server("s2", 50),
                make_large_server("s3", 50),
            ],
            unavailable_servers: vec![],
            deferred_loading: false,
            context_management_enabled: true,
            indexing_tools_enabled: false,
            catalog_compression: None,
            virtual_instructions: vec![],
        };

        // Threshold of 10 is impossibly low — should trigger all 3 phases
        let plan = compute_catalog_compression_plan(&ctx, 10, false, false, false);

        // Phase 3 can only be tested when phase 2 doesn't fully defer all servers
        // With no list_changed support, phase 2 does nothing, so phase 3 kicks in
        assert!(
            !plan.truncated_servers.is_empty(),
            "Phase 3 should truncate servers. Plan: {:?}",
            plan
        );
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
            deferred_loading: false,
            context_management_enabled: true,
            indexing_tools_enabled: false,
            catalog_compression: Some(CatalogCompressionPlan::default()),
            virtual_instructions: vec![],
        };

        let inst = build_context_managed_instructions(&ctx).unwrap();
        assert!(inst.contains("Context-Managed"));
        assert!(inst.contains("1 server connected"));
        assert!(inst.contains("`filesystem__read_file` (tool)"));
        assert!(inst.contains("`filesystem__config` (resource)"));
        // Server instructions should be inline
        assert!(inst.contains("<filesystem>"));
        assert!(inst.contains("File system server"));
    }

    #[test]
    fn test_cm_instructions_with_compressed_items() {
        let plan = CatalogCompressionPlan {
            compressed_descriptions: vec![CompressedItem {
                source_label: "catalog:filesystem__read_file".to_string(),
                full_content: "Read a file from disk".to_string(),
                item_type: CompressedItemType::Tool,
                namespaced_name: "filesystem__read_file".to_string(),
                byte_size: 100,
            }],
            deferred_items: vec![],
            truncated_servers: vec![],
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
            unavailable_servers: vec![],
            deferred_loading: false,
            context_management_enabled: true,
            indexing_tools_enabled: false,
            catalog_compression: Some(plan),
            virtual_instructions: vec![],
        };

        let inst = build_context_managed_instructions(&ctx).unwrap();
        // Compressed tool should show [compressed] marker
        assert!(
            inst.contains("[compressed]"),
            "Compressed tool should show [compressed] marker. Got:\n{}",
            inst
        );
        assert!(inst.contains("ctx_search"));
        // Non-compressed tool should show backtick notation
        assert!(inst.contains("`filesystem__write_file` (tool)"));
    }

    #[test]
    fn test_cm_instructions_with_truncated_server() {
        let plan = CatalogCompressionPlan {
            compressed_descriptions: vec![],
            deferred_items: vec![],
            truncated_servers: vec!["big-server".to_string()],
        };

        let ctx = InstructionsContext {
            servers: vec![McpServerInstructionInfo {
                name: "Big Server".to_string(),
                description: Some("Lots of tools".to_string()),
                instructions: None,
                tool_names: (0..10)
                    .map(|i| format!("big-server__tool_{}", i))
                    .collect(),
                resource_names: vec!["big-server__data".to_string()],
                prompt_names: vec![],
            }],
            unavailable_servers: vec![],
            deferred_loading: false,
            context_management_enabled: true,
            indexing_tools_enabled: false,
            catalog_compression: Some(plan),
            virtual_instructions: vec![],
        };

        let inst = build_context_managed_instructions(&ctx).unwrap();
        // Truncated server shows counts + search hint
        assert!(
            inst.contains("10 tools"),
            "Truncated server should show tool count. Got:\n{}",
            inst
        );
        assert!(inst.contains("1 resources"));
        assert!(inst.contains("ctx_search"));
        // Individual tools should NOT be listed
        assert!(!inst.contains("big-server__tool_0"));
        // Server instructions should NOT be shown
        assert!(!inst.contains("Lots of tools"));
    }

    #[test]
    fn test_cm_instructions_compressed_server_welcome() {
        let plan = CatalogCompressionPlan {
            compressed_descriptions: vec![CompressedItem {
                source_label: "catalog:filesystem".to_string(),
                full_content: "Full server docs...".to_string(),
                item_type: CompressedItemType::ServerWelcome,
                namespaced_name: "filesystem".to_string(),
                byte_size: 500,
            }],
            deferred_items: vec![],
            truncated_servers: vec![],
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
            unavailable_servers: vec![],
            deferred_loading: false,
            context_management_enabled: true,
            indexing_tools_enabled: false,
            catalog_compression: Some(plan),
            virtual_instructions: vec![],
        };

        let inst = build_context_managed_instructions(&ctx).unwrap();
        // Server instructions should be [compressed], not inline
        assert!(
            inst.contains("instructions — [compressed]"),
            "Server welcome should be compressed. Got:\n{}",
            inst
        );
        assert!(!inst.contains("<filesystem>"));
        assert!(!inst.contains("Detailed docs about filesystem server"));
    }

    #[test]
    fn test_cm_instructions_virtual_servers_never_compressed() {
        let plan = CatalogCompressionPlan {
            compressed_descriptions: vec![],
            deferred_items: vec![],
            truncated_servers: vec![],
        };

        let ctx = InstructionsContext {
            servers: vec![],
            unavailable_servers: vec![],
            deferred_loading: false,
            context_management_enabled: true,
            indexing_tools_enabled: true,
            catalog_compression: Some(plan),
            virtual_instructions: vec![crate::gateway::virtual_server::VirtualInstructions {
                section_title: "Context Management".to_string(),
                content: "Use ctx_search to find things".to_string(),
                tool_names: vec!["ctx_search".to_string(), "ctx_execute".to_string()],
            }],
        };

        let inst = build_context_managed_instructions(&ctx).unwrap();
        assert!(inst.contains("`ctx_search` (tool)"));
        assert!(inst.contains("`ctx_execute` (tool)"));
        // Virtual instructions always inline
        assert!(inst.contains("<context-management>"));
        assert!(inst.contains("Use ctx_search to find things"));
    }

    #[test]
    fn test_cm_instructions_unavailable_servers() {
        let ctx = InstructionsContext {
            servers: vec![],
            unavailable_servers: vec![UnavailableServerInfo {
                name: "broken-server".to_string(),
                error: "Connection refused".to_string(),
            }],
            deferred_loading: false,
            context_management_enabled: true,
            indexing_tools_enabled: false,
            catalog_compression: Some(CatalogCompressionPlan::default()),
            virtual_instructions: vec![],
        };

        let inst = build_context_managed_instructions(&ctx).unwrap();
        assert!(inst.contains("broken-server"));
        assert!(inst.contains("unavailable"));
        assert!(inst.contains("Connection refused"));
    }

    // ── End-to-end: compression plan → instructions ─────────────────

    #[test]
    fn test_e2e_compression_plan_applied_to_instructions() {
        // Create a context with large servers that will trigger compression
        let ctx = InstructionsContext {
            servers: vec![
                McpServerInstructionInfo {
                    name: "filesystem".to_string(),
                    description: Some("X".repeat(2000)),
                    instructions: Some("Y".repeat(2000)),
                    tool_names: (0..30)
                        .map(|i| format!("filesystem__tool_{}", i))
                        .collect(),
                    resource_names: (0..5)
                        .map(|i| format!("filesystem__res_{}", i))
                        .collect(),
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
            deferred_loading: false,
            context_management_enabled: true,
            indexing_tools_enabled: false,
            catalog_compression: None,
            virtual_instructions: vec![],
        };

        // Step 1: Compute compression with a moderate threshold
        let full_size = estimate_catalog_size(&ctx);
        assert!(
            full_size > 5000,
            "Full catalog should be large. Got: {}",
            full_size
        );

        // Set threshold to ~40% of full size to trigger compression
        let threshold = full_size * 2 / 5;
        let plan = compute_catalog_compression_plan(&ctx, threshold, true, true, true);

        assert!(
            !plan.compressed_descriptions.is_empty(),
            "Should have compressed items"
        );

        // Step 2: Apply plan to build instructions
        let mut ctx_with_plan = InstructionsContext {
            servers: ctx.servers.clone(),
            unavailable_servers: vec![],
            deferred_loading: false,
            context_management_enabled: true,
            indexing_tools_enabled: false,
            catalog_compression: Some(plan.clone()),
            virtual_instructions: vec![],
        };
        ctx_with_plan.catalog_compression = Some(plan.clone());

        let inst = build_gateway_instructions(&ctx_with_plan).unwrap();

        // Verify: CM header
        assert!(
            inst.contains("Context-Managed"),
            "Should use CM instructions path"
        );

        // Verify: compressed items marked in output
        let has_compressed = inst.contains("[compressed]");
        assert!(has_compressed, "Output should contain [compressed] markers");

        // Verify: ctx_search mentioned for discovering content
        assert!(
            inst.contains("ctx_search"),
            "Should reference ctx_search for discovery"
        );

        // Verify: the output is smaller than uncompressed version
        let uncompressed_ctx = InstructionsContext {
            servers: ctx.servers.clone(),
            unavailable_servers: vec![],
            deferred_loading: false,
            context_management_enabled: false,
            indexing_tools_enabled: false,
            catalog_compression: None,
            virtual_instructions: vec![],
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
            unavailable_servers: vec![],
            deferred_loading: false,
            context_management_enabled: true,
            indexing_tools_enabled: false,
            catalog_compression: None,
            virtual_instructions: vec![],
        };

        let plan = compute_catalog_compression_plan(&ctx, 100_000, true, true, true);
        assert!(plan.compressed_descriptions.is_empty());
        assert!(plan.deferred_items.is_empty());
        assert!(plan.truncated_servers.is_empty());

        let ctx_with_plan = InstructionsContext {
            catalog_compression: Some(plan),
            ..ctx
        };

        let inst = build_gateway_instructions(&ctx_with_plan).unwrap();
        assert!(inst.contains("`tiny__do_thing` (tool)"));
        assert!(!inst.contains("[compressed]"));
    }

    #[test]
    fn test_e2e_mixed_compression_some_servers_compressed_some_not() {
        let ctx = InstructionsContext {
            servers: vec![
                McpServerInstructionInfo {
                    name: "big".to_string(),
                    description: Some("D".repeat(3000)),
                    instructions: Some("I".repeat(3000)),
                    tool_names: (0..20)
                        .map(|i| format!("big__tool_{}", i))
                        .collect(),
                    resource_names: vec![],
                    prompt_names: vec![],
                },
                McpServerInstructionInfo {
                    name: "small".to_string(),
                    description: None,
                    instructions: None,
                    tool_names: vec!["small__a".to_string()],
                    resource_names: vec![],
                    prompt_names: vec![],
                },
            ],
            unavailable_servers: vec![],
            deferred_loading: false,
            context_management_enabled: true,
            indexing_tools_enabled: false,
            catalog_compression: None,
            virtual_instructions: vec![],
        };

        let full_size = estimate_catalog_size(&ctx);
        // Threshold that compresses "big" but not "small"
        let threshold = full_size - 2000;
        let plan = compute_catalog_compression_plan(&ctx, threshold, true, true, true);

        let ctx_with_plan = InstructionsContext {
            catalog_compression: Some(plan),
            ..ctx
        };

        let inst = build_gateway_instructions(&ctx_with_plan).unwrap();

        // Small server should be uncompressed
        assert!(
            inst.contains("`small__a` (tool)"),
            "Small server tool should be uncompressed. Got:\n{}",
            inst
        );
    }
}
