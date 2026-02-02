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
    fn test_search_tool_relevance() {
        let tools = vec![
            types::NamespacedTool {
                name: "filesystem__read_file".to_string(),
                original_name: "read_file".to_string(),
                server_id: "filesystem".to_string(),
                description: Some("Read a file from disk".to_string()),
                input_schema: json!({}),
            },
            types::NamespacedTool {
                name: "filesystem__write_file".to_string(),
                original_name: "write_file".to_string(),
                server_id: "filesystem".to_string(),
                description: Some("Write a file to disk".to_string()),
                input_schema: json!({}),
            },
            types::NamespacedTool {
                name: "github__read_issue".to_string(),
                original_name: "read_issue".to_string(),
                server_id: "github".to_string(),
                description: Some("Read an issue from GitHub".to_string()),
                input_schema: json!({}),
            },
        ];

        let results = deferred::search_tools("read", &tools, 10);

        // Should return tools with "read" in name or description
        assert!(!results.is_empty());
        assert!(results.iter().any(|(tool, _)| tool.name.contains("read")));
    }

    #[test]
    fn test_search_tool_minimum_activations() {
        let tools = vec![
            types::NamespacedTool {
                name: "tool1".to_string(),
                original_name: "tool1".to_string(),
                server_id: "server".to_string(),
                description: Some("related".to_string()),
                input_schema: json!({}),
            },
            types::NamespacedTool {
                name: "tool2".to_string(),
                original_name: "tool2".to_string(),
                server_id: "server".to_string(),
                description: Some("also related".to_string()),
                input_schema: json!({}),
            },
            types::NamespacedTool {
                name: "tool3".to_string(),
                original_name: "tool3".to_string(),
                server_id: "server".to_string(),
                description: Some("related too".to_string()),
                input_schema: json!({}),
            },
        ];

        let results = deferred::search_tools("related", &tools, 10);

        // Should activate at least MIN_ACTIVATIONS (3) if available
        assert!(results.len() >= 3 || results.len() == tools.len());
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
            false,
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
            false,
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
}
