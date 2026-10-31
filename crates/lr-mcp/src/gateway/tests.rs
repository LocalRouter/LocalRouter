#[cfg(test)]
#[allow(clippy::module_inception)]
mod tests {
    use super::super::*;
    use crate::protocol::{McpPrompt, McpResource, McpTool};
    use serde_json::json;

    #[test]
    fn test_namespace_parsing() {
        // Valid namespaces
        assert_eq!(
            types::parse_namespace("filesystem__read_file"),
            Some(("filesystem".to_string(), "read_file".to_string()))
        );

        assert_eq!(
            types::parse_namespace("github__create_issue"),
            Some(("github".to_string(), "create_issue".to_string()))
        );

        // Edge cases
        assert_eq!(types::parse_namespace("no_separator"), None);
        assert_eq!(types::parse_namespace("__no_server"), None);
        assert_eq!(types::parse_namespace("no_tool__"), None);
        assert_eq!(types::parse_namespace(""), None);
    }

    #[test]
    fn test_namespace_application() {
        assert_eq!(
            types::apply_namespace("filesystem", "read_file"),
            "filesystem__read_file"
        );

        assert_eq!(
            types::apply_namespace("github", "create_issue"),
            "github__create_issue"
        );
    }

    #[test]
    fn test_namespace_roundtrip() {
        let server = "filesystem";
        let tool = "read_file";
        let namespaced = types::apply_namespace(server, tool);
        let (parsed_server, parsed_tool) = types::parse_namespace(&namespaced).unwrap();

        assert_eq!(parsed_server, server);
        assert_eq!(parsed_tool, tool);
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

        let merged = merger::merge_tools(server_tools, &[], None);

        assert_eq!(merged.len(), 2);
        assert_eq!(merged[0].name, "filesystem__read_file");
        assert_eq!(merged[0].server_id, "filesystem");
        assert_eq!(merged[0].original_name, "read_file");
        assert_eq!(merged[1].name, "github__create_issue");
        assert_eq!(merged[1].server_id, "github");
        assert_eq!(merged[1].original_name, "create_issue");
    }

    #[test]
    fn test_merge_resources() {
        let resource1 = McpResource {
            name: "config".to_string(),
            uri: "file:///config.json".to_string(),
            description: Some("Config file".to_string()),
            mime_type: Some("application/json".to_string()),
        };

        let resource2 = McpResource {
            name: "repo".to_string(),
            uri: "github://myrepo".to_string(),
            description: Some("Repository".to_string()),
            mime_type: None,
        };

        let server_resources = vec![
            ("filesystem".to_string(), vec![resource1]),
            ("github".to_string(), vec![resource2]),
        ];

        let merged = merger::merge_resources(server_resources, &[], None);

        assert_eq!(merged.len(), 2);
        assert_eq!(merged[0].name, "filesystem__config");
        assert_eq!(merged[0].uri, "file:///config.json");
        assert_eq!(merged[1].name, "github__repo");
        assert_eq!(merged[1].uri, "github://myrepo");
    }

    #[test]
    fn test_merge_prompts() {
        let prompt1 = McpPrompt {
            name: "commit_template".to_string(),
            description: Some("Git commit template".to_string()),
            arguments: None,
        };

        let prompt2 = McpPrompt {
            name: "pr_template".to_string(),
            description: Some("PR template".to_string()),
            arguments: None,
        };

        let server_prompts = vec![
            ("git".to_string(), vec![prompt1]),
            ("github".to_string(), vec![prompt2]),
        ];

        let merged = merger::merge_prompts(server_prompts, &[], None);

        assert_eq!(merged.len(), 2);
        assert_eq!(merged[0].name, "git__commit_template");
        assert_eq!(merged[1].name, "github__pr_template");
    }

    #[test]
    fn test_merge_initialize_results() {
        let result1 = types::InitializeResult {
            protocol_version: "2024-11-05".to_string(),
            capabilities: types::ServerCapabilities {
                tools: Some(types::ToolsCapability {
                    list_changed: Some(true),
                }),
                ..Default::default()
            },
            server_info: types::ServerInfo {
                name: "Filesystem Server".to_string(),
                version: "1.0.0".to_string(),
                description: Some("File operations".to_string()),
            },
            instructions: None,
        };

        let result2 = types::InitializeResult {
            protocol_version: "2024-11-05".to_string(),
            capabilities: types::ServerCapabilities {
                resources: Some(types::ResourcesCapability {
                    list_changed: Some(true),
                    subscribe: Some(true),
                }),
                ..Default::default()
            },
            server_info: types::ServerInfo {
                name: "GitHub Server".to_string(),
                version: "1.0.0".to_string(),
                description: None,
            },
            instructions: None,
        };

        let results = vec![
            ("filesystem".to_string(), result1),
            ("github".to_string(), result2),
        ];

        let merged = merger::merge_initialize_results(results, vec![]);

        assert_eq!(merged.protocol_version, "2024-11-05");
        assert!(merged.capabilities.tools.is_some());
        assert!(merged.capabilities.resources.is_some());
        assert_eq!(merged.server_info.name, "LocalRouter Unified Gateway");
    }

    #[test]
    fn test_should_broadcast() {
        assert!(router::should_broadcast("initialize"));
        assert!(router::should_broadcast("tools/list"));
        assert!(router::should_broadcast("resources/list"));
        assert!(router::should_broadcast("prompts/list"));
        assert!(router::should_broadcast("logging/setLevel"));
        assert!(router::should_broadcast("ping"));

        assert!(!router::should_broadcast("tools/call"));
        assert!(!router::should_broadcast("resources/read"));
        assert!(!router::should_broadcast("prompts/get"));
    }

    #[test]
    fn test_session_creation() {
        use std::time::Duration;

        let session = session::GatewaySession::new(
            "client-123".to_string(),
            vec!["filesystem".to_string(), "github".to_string()],
            Duration::from_secs(3600),
            300,
            Vec::new(),
        );

        assert_eq!(session.client_id, "client-123");
        assert_eq!(session.allowed_servers.len(), 2);
        assert_eq!(session.server_init_status.len(), 2);
        assert!(!session.is_expired());
    }

    #[test]
    fn test_session_expiration() {
        use std::time::Duration;

        let mut session = session::GatewaySession::new(
            "client-123".to_string(),
            vec!["filesystem".to_string()],
            Duration::from_millis(100),
            300,
            Vec::new(),
        );

        assert!(!session.is_expired());

        // Wait for expiration
        std::thread::sleep(Duration::from_millis(150));
        assert!(session.is_expired());

        // Touch should reset expiration
        session.touch();
        assert!(!session.is_expired());
    }

    #[test]
    fn test_cached_list_validity() {
        use std::time::Duration;

        let cached = types::CachedList::new(vec!["item1".to_string()], Duration::from_millis(100));

        assert!(cached.is_valid());

        std::thread::sleep(Duration::from_millis(150));
        assert!(!cached.is_valid());
    }

    #[test]
    fn test_gateway_config_default_includes_mcp_settings() {
        let config = types::GatewayConfig::default();
        assert_eq!(config.sampling, lr_config::SamplingBehavior::Passthrough);
        assert_eq!(
            config.elicitation_mode,
            lr_config::ElicitationMode::Passthrough
        );
    }

    #[test]
    fn test_server_capabilities_with_completions() {
        let capabilities = types::ServerCapabilities {
            completions: Some(types::CompletionsCapability {}),
            ..Default::default()
        };
        assert!(capabilities.completions.is_some());

        // Default should have no completions
        let default_caps = types::ServerCapabilities::default();
        assert!(default_caps.completions.is_none());
    }

    #[test]
    fn test_completions_capability_merging() {
        // Server 1 has tools, no completions
        let result1 = types::InitializeResult {
            protocol_version: "2024-11-05".to_string(),
            capabilities: types::ServerCapabilities {
                tools: Some(types::ToolsCapability {
                    list_changed: Some(true),
                }),
                ..Default::default()
            },
            server_info: types::ServerInfo {
                name: "Server A".to_string(),
                version: "1.0.0".to_string(),
                description: None,
            },
            instructions: None,
        };

        // Server 2 has completions
        let result2 = types::InitializeResult {
            protocol_version: "2024-11-05".to_string(),
            capabilities: types::ServerCapabilities {
                completions: Some(types::CompletionsCapability {}),
                ..Default::default()
            },
            server_info: types::ServerInfo {
                name: "Server B".to_string(),
                version: "1.0.0".to_string(),
                description: None,
            },
            instructions: None,
        };

        let results = vec![
            ("server-a".to_string(), result1),
            ("server-b".to_string(), result2),
        ];

        let merged = merger::merge_initialize_results(results, vec![]);

        // Merged should have both tools and completions
        assert!(merged.capabilities.tools.is_some());
        assert!(merged.capabilities.completions.is_some());
    }

    fn init_with(capabilities: serde_json::Value) -> types::InitializeResult {
        serde_json::from_value(json!({
            "protocolVersion": "2025-11-25",
            "capabilities": capabilities,
            "serverInfo": {"name": "s", "version": "1"}
        }))
        .unwrap()
    }

    /// `experimental`, top-level `events` and fields no revision defines yet
    /// survive deserialisation and serialisation unchanged.
    #[test]
    fn server_capabilities_keep_experimental_and_unknown_fields() {
        let raw = json!({
            "tools": {"listChanged": true},
            "events": {"listChanged": true},
            "experimental": {"claude/channel": {}, "claude/channel/permission": {}},
            "extensions": {"io.modelcontextprotocol/events": {"listChanged": true}},
            "futureCapability": {"nested": [1, 2, 3]}
        });
        let caps: types::ServerCapabilities = serde_json::from_value(raw.clone()).unwrap();
        assert!(caps.has_events());
        assert!(caps.has_claude_channel());
        assert!(!caps.has_resource_subscribe());
        assert_eq!(caps.other["futureCapability"], json!({"nested": [1, 2, 3]}));
        assert_eq!(serde_json::to_value(&caps).unwrap(), raw);

        // Either location declares events
        let top_level: types::ServerCapabilities =
            serde_json::from_value(json!({"events": {}})).unwrap();
        assert!(top_level.has_events());
        let extension: types::ServerCapabilities =
            serde_json::from_value(json!({"extensions": {"io.modelcontextprotocol/events": {}}}))
                .unwrap();
        assert!(extension.has_events());
        assert!(!types::ServerCapabilities::default().has_events());
    }

    /// Two upstreams declaring the same capabilities differently: each keeps
    /// its own declaration in `providers`; the merged capabilities sent
    /// downstream never carry upstream `experimental`, unknown fields or the
    /// events extension, and other extensions stay first-provider-wins.
    #[test]
    fn conflicting_providers_are_recorded_not_forwarded() {
        let a = init_with(json!({
            "tools": {},
            "events": {"from": "a"},
            "experimental": {"claude/channel": {}},
            "extensions": {
                "io.modelcontextprotocol/events": {"listChanged": true},
                "vendor/ext": {"version": "a"}
            }
        }));
        let b = init_with(json!({
            "resources": {"subscribe": true},
            "events": {"from": "b"},
            "experimental": {"other/thing": {"from": "b"}},
            "extensions": {
                "io.modelcontextprotocol/events": {"listChanged": false},
                "vendor/ext": {"version": "b"}
            }
        }));
        let merged = merger::merge_initialize_results(
            vec![("srv-a".into(), a), ("srv-b".into(), b)],
            vec![],
        );

        let downstream = serde_json::to_value(&merged.capabilities).unwrap();
        assert!(downstream.get("experimental").is_none());
        assert!(downstream.get("events").is_none());
        assert_eq!(
            downstream["extensions"],
            json!({"vendor/ext": {"version": "a"}})
        );
        assert_eq!(downstream["resources"]["subscribe"], true);

        let providers = &merged.providers;
        assert_eq!(providers.servers_with_events(), vec!["srv-a", "srv-b"]);
        assert_eq!(providers.servers_with_claude_channel(), vec!["srv-a"]);
        assert_eq!(
            providers.servers_with_extension("vendor/ext"),
            vec!["srv-a", "srv-b"]
        );
        let a = providers.get("srv-a").unwrap();
        let b = providers.get("srv-b").unwrap();
        assert_eq!(a.other["events"], json!({"from": "a"}));
        assert_eq!(b.other["events"], json!({"from": "b"}));
        assert_eq!(
            b.experimental_capability("other/thing"),
            Some(&json!({"from": "b"}))
        );
        assert_eq!(
            a.extension("io.modelcontextprotocol/events"),
            Some(&json!({"listChanged": true}))
        );
        assert!(b.has_resource_subscribe());
    }

    /// When the events extension was the only one, no empty map is sent.
    #[test]
    fn events_only_extensions_are_dropped_entirely() {
        let a = init_with(json!({
            "extensions": {"io.modelcontextprotocol/events": {}}
        }));
        let merged = merger::merge_initialize_results(vec![("a".into(), a)], vec![]);
        assert!(merged.capabilities.extensions.is_none());
        assert_eq!(merged.providers.servers_with_events(), vec!["a"]);
    }

    #[test]
    fn test_resource_templates_list_broadcast() {
        // resources/templates/list should be broadcast
        assert!(router::should_broadcast("resources/templates/list"));

        // Other template-related methods should not
        assert!(!router::should_broadcast("resources/templates/read"));
    }

    #[test]
    fn test_parse_namespace_for_completion() {
        // Completions use namespaced refs like "server__ref/name"
        let result = types::parse_namespace("filesystem__file:///path/to/file");
        assert!(result.is_some());
        let (server, original) = result.unwrap();
        assert_eq!(server, "filesystem");
        assert_eq!(original, "file:///path/to/file");

        // Prompt argument completion ref
        let result = types::parse_namespace("github__pr_template");
        assert!(result.is_some());
        let (server, original) = result.unwrap();
        assert_eq!(server, "github");
        assert_eq!(original, "pr_template");
    }
}
