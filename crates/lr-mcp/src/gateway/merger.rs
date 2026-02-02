// Empty import section - using json! macro from types.rs

use super::types::*;
use crate::protocol::{McpPrompt, McpResource, McpTool};

/// Information about a skill, used when building gateway instructions
pub struct SkillInfo {
    pub name: String,
    pub description: Option<String>,
    pub get_info_tool: String,
}

/// Context for building gateway instructions
pub struct InstructionsContext {
    /// MCP server init results (server_id, result)
    pub servers: Vec<(String, InitializeResult)>,
    /// Skills accessible to this client
    pub skills: Vec<SkillInfo>,
    /// Whether deferred loading is enabled
    pub deferred_loading: bool,
    /// Deferred catalog: tool names (only relevant when deferred_loading is true)
    pub deferred_tool_names: Vec<String>,
    /// Deferred catalog: resource names
    pub deferred_resource_names: Vec<String>,
    /// Deferred catalog: prompt names
    pub deferred_prompt_names: Vec<String>,
    /// Server failures
    pub failures: Vec<ServerFailure>,
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
        version: "0.1.0".to_string(),
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
/// Generates instructions tailored to the combination of:
/// - MCP servers (with their own instructions/descriptions embedded)
/// - Skills (with discovery instructions)
/// - Deferred loading mode (catalog listing instead of full server details)
pub fn build_gateway_instructions(ctx: &InstructionsContext) -> Option<String> {
    let has_servers = !ctx.servers.is_empty();
    let has_skills = !ctx.skills.is_empty();

    // Nothing to describe
    if !has_servers && !has_skills {
        return None;
    }

    let mut inst = String::new();

    // --- MCP Servers section ---
    if has_servers {
        if ctx.deferred_loading {
            build_deferred_servers_section(&mut inst, ctx);
        } else {
            build_normal_servers_section(&mut inst, ctx);
        }
    }

    // --- Skills section ---
    if has_skills {
        if has_servers {
            inst.push('\n');
        }
        build_skills_section(&mut inst, &ctx.skills);
    }

    // --- Failures section ---
    if !ctx.failures.is_empty() {
        inst.push('\n');
        inst.push_str("## Unavailable Servers\n\n");
        for failure in &ctx.failures {
            inst.push_str(&format!("- **{}**: {}\n", failure.server_id, failure.error));
        }
    }

    Some(inst)
}

/// Build the MCP servers section for normal (non-deferred) mode.
/// Embeds each server's instructions/description directly.
fn build_normal_servers_section(inst: &mut String, ctx: &InstructionsContext) {
    inst.push_str("## MCP Servers\n\n");
    inst.push_str(
        "Tools, resources, and prompts from MCP servers are namespaced with a \
         `servername__` prefix (e.g., `filesystem__read_file`).\n\n",
    );

    for (server_id, result) in &ctx.servers {
        inst.push_str(&format!("### {}", server_id));
        if result.server_info.name != *server_id {
            inst.push_str(&format!(" ({})", result.server_info.name));
        }
        inst.push('\n');

        // Embed the server's own instructions (preferred) or description
        if let Some(server_instructions) = &result.instructions {
            inst.push('\n');
            inst.push_str(server_instructions);
            if !server_instructions.ends_with('\n') {
                inst.push('\n');
            }
        } else if let Some(server_desc) = &result.server_info.description {
            inst.push('\n');
            inst.push_str(server_desc);
            if !server_desc.ends_with('\n') {
                inst.push('\n');
            }
        }

        inst.push('\n');
    }
}

/// Build the MCP servers section for deferred loading mode.
/// Lists available tool/resource/prompt names without full instructions.
fn build_deferred_servers_section(inst: &mut String, ctx: &InstructionsContext) {
    inst.push_str("## MCP Servers (deferred loading)\n\n");
    inst.push_str(
        "Tools are loaded on demand. Use the `search` tool to discover and activate \
         capabilities by keyword before using them. Activated items remain available \
         for the rest of the session.\n\n",
    );

    // List servers briefly
    inst.push_str("Connected servers: ");
    let server_names: Vec<&str> = ctx.servers.iter().map(|(id, _)| id.as_str()).collect();
    inst.push_str(&server_names.join(", "));
    inst.push_str("\n\n");

    // List available tool names so the LLM knows what exists
    if !ctx.deferred_tool_names.is_empty() {
        inst.push_str(&format!(
            "**Available tools** ({}):\n",
            ctx.deferred_tool_names.len()
        ));
        for name in &ctx.deferred_tool_names {
            inst.push_str(&format!("- `{}`\n", name));
        }
        inst.push('\n');
    }

    if !ctx.deferred_resource_names.is_empty() {
        inst.push_str(&format!(
            "**Available resources** ({}):\n",
            ctx.deferred_resource_names.len()
        ));
        for name in &ctx.deferred_resource_names {
            inst.push_str(&format!("- `{}`\n", name));
        }
        inst.push('\n');
    }

    if !ctx.deferred_prompt_names.is_empty() {
        inst.push_str(&format!(
            "**Available prompts** ({}):\n",
            ctx.deferred_prompt_names.len()
        ));
        for name in &ctx.deferred_prompt_names {
            inst.push_str(&format!("- `{}`\n", name));
        }
        inst.push('\n');
    }
}

/// Build the skills section with discovery instructions.
fn build_skills_section(inst: &mut String, skills: &[SkillInfo]) {
    inst.push_str("## Skills\n\n");
    inst.push_str(
        "Skills provide specialized capabilities with scripts and resources. \
         Call a skill's `get_info` tool to view its full instructions and unlock \
         its run/read tools.\n\n",
    );

    for skill in skills {
        inst.push_str(&format!("- **{}**", skill.name));
        if let Some(desc) = &skill.description {
            inst.push_str(&format!(" — {}", desc));
        }
        inst.push_str(&format!(" → `{}`\n", skill.get_info_tool));
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
    fn test_build_gateway_instructions_servers_only() {
        let ctx = InstructionsContext {
            servers: vec![(
                "filesystem".to_string(),
                InitializeResult {
                    protocol_version: "2024-11-05".to_string(),
                    capabilities: ServerCapabilities::default(),
                    server_info: ServerInfo {
                        name: "Filesystem Server".to_string(),
                        version: "1.0.0".to_string(),
                        description: Some("File operations server".to_string()),
                    },
                    instructions: Some("Use read_file to read and write_file to write.".to_string()),
                },
            )],
            skills: Vec::new(),
            deferred_loading: false,
            deferred_tool_names: Vec::new(),
            deferred_resource_names: Vec::new(),
            deferred_prompt_names: Vec::new(),
            failures: Vec::new(),
        };

        let instructions = build_gateway_instructions(&ctx).unwrap();
        assert!(instructions.contains("## MCP Servers"));
        assert!(instructions.contains("### filesystem"));
        // Server's own instructions should be embedded
        assert!(instructions.contains("Use read_file to read"));
        // Should NOT contain deferred loading text
        assert!(!instructions.contains("deferred loading"));
    }

    #[test]
    fn test_build_gateway_instructions_skills_only() {
        let ctx = InstructionsContext {
            servers: Vec::new(),
            skills: vec![SkillInfo {
                name: "code-review".to_string(),
                description: Some("Automated code review".to_string()),
                get_info_tool: "skill_code_review_get_info".to_string(),
            }],
            deferred_loading: false,
            deferred_tool_names: Vec::new(),
            deferred_resource_names: Vec::new(),
            deferred_prompt_names: Vec::new(),
            failures: Vec::new(),
        };

        let instructions = build_gateway_instructions(&ctx).unwrap();
        assert!(instructions.contains("## Skills"));
        assert!(instructions.contains("code-review"));
        assert!(instructions.contains("skill_code_review_get_info"));
        assert!(!instructions.contains("## MCP Servers"));
    }

    #[test]
    fn test_build_gateway_instructions_deferred_loading() {
        let ctx = InstructionsContext {
            servers: vec![(
                "filesystem".to_string(),
                InitializeResult {
                    protocol_version: "2024-11-05".to_string(),
                    capabilities: ServerCapabilities::default(),
                    server_info: ServerInfo {
                        name: "Filesystem Server".to_string(),
                        version: "1.0.0".to_string(),
                        description: None,
                    },
                    instructions: None,
                },
            )],
            skills: Vec::new(),
            deferred_loading: true,
            deferred_tool_names: vec![
                "filesystem__read_file".to_string(),
                "filesystem__write_file".to_string(),
            ],
            deferred_resource_names: Vec::new(),
            deferred_prompt_names: Vec::new(),
            failures: Vec::new(),
        };

        let instructions = build_gateway_instructions(&ctx).unwrap();
        assert!(instructions.contains("deferred loading"));
        assert!(instructions.contains("`search`"));
        assert!(instructions.contains("`filesystem__read_file`"));
        assert!(instructions.contains("`filesystem__write_file`"));
    }

    #[test]
    fn test_build_gateway_instructions_both_servers_and_skills() {
        let ctx = InstructionsContext {
            servers: vec![(
                "github".to_string(),
                InitializeResult {
                    protocol_version: "2024-11-05".to_string(),
                    capabilities: ServerCapabilities::default(),
                    server_info: ServerInfo {
                        name: "GitHub".to_string(),
                        version: "1.0.0".to_string(),
                        description: Some("GitHub API access".to_string()),
                    },
                    instructions: None,
                },
            )],
            skills: vec![SkillInfo {
                name: "deploy".to_string(),
                description: None,
                get_info_tool: "skill_deploy_get_info".to_string(),
            }],
            deferred_loading: false,
            deferred_tool_names: Vec::new(),
            deferred_resource_names: Vec::new(),
            deferred_prompt_names: Vec::new(),
            failures: Vec::new(),
        };

        let instructions = build_gateway_instructions(&ctx).unwrap();
        assert!(instructions.contains("## MCP Servers"));
        assert!(instructions.contains("## Skills"));
        assert!(instructions.contains("### github"));
        assert!(instructions.contains("deploy"));
    }

    #[test]
    fn test_build_gateway_instructions_empty() {
        let ctx = InstructionsContext {
            servers: Vec::new(),
            skills: Vec::new(),
            deferred_loading: false,
            deferred_tool_names: Vec::new(),
            deferred_resource_names: Vec::new(),
            deferred_prompt_names: Vec::new(),
            failures: Vec::new(),
        };

        assert!(build_gateway_instructions(&ctx).is_none());
    }

    #[test]
    fn test_build_gateway_instructions_with_failures() {
        let ctx = InstructionsContext {
            servers: Vec::new(),
            skills: vec![SkillInfo {
                name: "test".to_string(),
                description: None,
                get_info_tool: "skill_test_get_info".to_string(),
            }],
            deferred_loading: false,
            deferred_tool_names: Vec::new(),
            deferred_resource_names: Vec::new(),
            deferred_prompt_names: Vec::new(),
            failures: vec![ServerFailure {
                server_id: "broken-server".to_string(),
                error: "Connection refused".to_string(),
            }],
        };

        let instructions = build_gateway_instructions(&ctx).unwrap();
        assert!(instructions.contains("## Unavailable Servers"));
        assert!(instructions.contains("broken-server"));
        assert!(instructions.contains("Connection refused"));
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
