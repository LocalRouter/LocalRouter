//! Centralized access control module
//!
//! Resolves `PermissionState` into access decisions for all resource types:
//! MCP tools, skills, marketplace, and models.

use lr_config::{McpPermissions, ModelPermissions, PermissionState, SkillsPermissions};

/// Access decision resolved from PermissionState
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AccessDecision {
    /// Access is allowed without restriction
    Allow,
    /// Access requires user approval (popup)
    Ask,
    /// Access is denied
    Deny,
}

impl From<&PermissionState> for AccessDecision {
    fn from(p: &PermissionState) -> Self {
        match p {
            PermissionState::Allow => AccessDecision::Allow,
            PermissionState::Ask => AccessDecision::Ask,
            PermissionState::Off => AccessDecision::Deny,
        }
    }
}

impl From<PermissionState> for AccessDecision {
    fn from(p: PermissionState) -> Self {
        AccessDecision::from(&p)
    }
}

/// Check access for an MCP tool call using mcp_permissions hierarchy.
///
/// Resolution order: tool -> server -> global (handled by McpPermissions::resolve_tool)
pub fn check_mcp_tool_access(
    perms: &McpPermissions,
    server_id: &str,
    tool_name: &str,
) -> AccessDecision {
    perms.resolve_tool(server_id, tool_name).into()
}

/// Check access for a skill tool call using skills_permissions hierarchy.
///
/// Resolution order: tool -> skill -> global (handled by SkillsPermissions::resolve_tool)
pub fn check_skill_tool_access(
    perms: &SkillsPermissions,
    skill_name: &str,
    tool_name: &str,
) -> AccessDecision {
    perms.resolve_tool(skill_name, tool_name).into()
}

/// Check access for marketplace.
pub fn check_marketplace_access(perm: &PermissionState) -> AccessDecision {
    AccessDecision::from(perm)
}

/// Check access for a model.
///
/// Resolution order: model -> provider -> global (handled by ModelPermissions::resolve_model)
pub fn check_model_access(
    perms: &ModelPermissions,
    provider: &str,
    model_id: &str,
) -> AccessDecision {
    perms.resolve_model(provider, model_id).into()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    #[test]
    fn test_access_decision_from_permission_state() {
        assert_eq!(
            AccessDecision::from(&PermissionState::Allow),
            AccessDecision::Allow
        );
        assert_eq!(
            AccessDecision::from(&PermissionState::Ask),
            AccessDecision::Ask
        );
        assert_eq!(
            AccessDecision::from(&PermissionState::Off),
            AccessDecision::Deny
        );
    }

    #[test]
    fn test_mcp_tool_access_global_allow() {
        let perms = McpPermissions {
            global: PermissionState::Allow,
            ..Default::default()
        };
        assert_eq!(
            check_mcp_tool_access(&perms, "server1", "read_file"),
            AccessDecision::Allow
        );
    }

    #[test]
    fn test_mcp_tool_access_global_ask() {
        let perms = McpPermissions {
            global: PermissionState::Ask,
            ..Default::default()
        };
        assert_eq!(
            check_mcp_tool_access(&perms, "server1", "read_file"),
            AccessDecision::Ask
        );
    }

    #[test]
    fn test_mcp_tool_access_global_off() {
        let perms = McpPermissions {
            global: PermissionState::Off,
            ..Default::default()
        };
        assert_eq!(
            check_mcp_tool_access(&perms, "server1", "read_file"),
            AccessDecision::Deny
        );
    }

    #[test]
    fn test_mcp_tool_access_server_override() {
        let mut servers = HashMap::new();
        servers.insert("server1".to_string(), PermissionState::Ask);

        let perms = McpPermissions {
            global: PermissionState::Allow,
            servers,
            ..Default::default()
        };
        // server1 overrides to Ask
        assert_eq!(
            check_mcp_tool_access(&perms, "server1", "read_file"),
            AccessDecision::Ask
        );
        // server2 falls through to global Allow
        assert_eq!(
            check_mcp_tool_access(&perms, "server2", "read_file"),
            AccessDecision::Allow
        );
    }

    #[test]
    fn test_mcp_tool_access_tool_override() {
        let mut servers = HashMap::new();
        servers.insert("server1".to_string(), PermissionState::Ask);

        let mut tools = HashMap::new();
        tools.insert("server1__write_file".to_string(), PermissionState::Off);

        let perms = McpPermissions {
            global: PermissionState::Allow,
            servers,
            tools,
            ..Default::default()
        };
        // Specific tool override -> Deny
        assert_eq!(
            check_mcp_tool_access(&perms, "server1", "write_file"),
            AccessDecision::Deny
        );
        // Other tool on same server -> Ask (server override)
        assert_eq!(
            check_mcp_tool_access(&perms, "server1", "read_file"),
            AccessDecision::Ask
        );
    }

    #[test]
    fn test_skill_tool_access_global() {
        let perms = SkillsPermissions {
            global: PermissionState::Ask,
            ..Default::default()
        };
        assert_eq!(
            check_skill_tool_access(&perms, "weather", "skill_weather_run_main"),
            AccessDecision::Ask
        );
    }

    #[test]
    fn test_skill_tool_access_skill_override() {
        let mut skills = HashMap::new();
        skills.insert("weather".to_string(), PermissionState::Allow);

        let perms = SkillsPermissions {
            global: PermissionState::Ask,
            skills,
            ..Default::default()
        };
        assert_eq!(
            check_skill_tool_access(&perms, "weather", "skill_weather_run_main"),
            AccessDecision::Allow
        );
        // Other skill falls through to global
        assert_eq!(
            check_skill_tool_access(&perms, "sysinfo", "skill_sysinfo_run_main"),
            AccessDecision::Ask
        );
    }

    #[test]
    fn test_skill_tool_access_tool_override() {
        let mut skills = HashMap::new();
        skills.insert("weather".to_string(), PermissionState::Allow);

        let mut tools = HashMap::new();
        tools.insert(
            "weather__skill_weather_run_dangerous".to_string(),
            PermissionState::Off,
        );

        let perms = SkillsPermissions {
            global: PermissionState::Ask,
            skills,
            tools,
        };
        assert_eq!(
            check_skill_tool_access(&perms, "weather", "skill_weather_run_dangerous"),
            AccessDecision::Deny
        );
        assert_eq!(
            check_skill_tool_access(&perms, "weather", "skill_weather_run_main"),
            AccessDecision::Allow
        );
    }

    #[test]
    fn test_marketplace_access() {
        assert_eq!(
            check_marketplace_access(&PermissionState::Allow),
            AccessDecision::Allow
        );
        assert_eq!(
            check_marketplace_access(&PermissionState::Ask),
            AccessDecision::Ask
        );
        assert_eq!(
            check_marketplace_access(&PermissionState::Off),
            AccessDecision::Deny
        );
    }

    #[test]
    fn test_model_access_global() {
        let perms = ModelPermissions {
            global: PermissionState::Ask,
            ..Default::default()
        };
        assert_eq!(
            check_model_access(&perms, "openai", "gpt-4"),
            AccessDecision::Ask
        );
    }

    #[test]
    fn test_model_access_provider_override() {
        let mut providers = HashMap::new();
        providers.insert("openai".to_string(), PermissionState::Allow);

        let perms = ModelPermissions {
            global: PermissionState::Ask,
            providers,
            ..Default::default()
        };
        assert_eq!(
            check_model_access(&perms, "openai", "gpt-4"),
            AccessDecision::Allow
        );
        assert_eq!(
            check_model_access(&perms, "anthropic", "claude-3"),
            AccessDecision::Ask
        );
    }

    #[test]
    fn test_model_access_model_override() {
        let mut providers = HashMap::new();
        providers.insert("openai".to_string(), PermissionState::Allow);

        let mut models = HashMap::new();
        models.insert("openai__gpt-4".to_string(), PermissionState::Off);

        let perms = ModelPermissions {
            global: PermissionState::Ask,
            providers,
            models,
        };
        assert_eq!(
            check_model_access(&perms, "openai", "gpt-4"),
            AccessDecision::Deny
        );
        assert_eq!(
            check_model_access(&perms, "openai", "gpt-3.5-turbo"),
            AccessDecision::Allow
        );
    }
}
