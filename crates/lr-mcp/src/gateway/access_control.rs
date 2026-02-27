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

// ============================================================================
// Unified Firewall Check
// ============================================================================

/// Result of checking whether a firewall approval popup is needed
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FirewallCheckResult {
    /// No popup needed, allow the request
    Allow,
    /// No popup needed, deny the request
    Deny,
    /// Popup needed — user must approve or deny
    Ask,
}

/// Context for checking whether a firewall approval is needed.
/// Each variant provides the type-specific inputs for permission resolution.
pub enum FirewallCheckContext<'a> {
    McpTool {
        permissions: &'a McpPermissions,
        server_id: &'a str,
        original_tool_name: &'a str,
        session_approved: bool,
        session_denied: bool,
    },
    SkillTool {
        permissions: &'a SkillsPermissions,
        skill_name: &'a str,
        tool_name: &'a str,
        session_approved: bool,
        session_denied: bool,
    },
    Model {
        permissions: &'a ModelPermissions,
        provider: &'a str,
        model_id: &'a str,
        has_time_based_approval: bool,
    },
    Guardrail {
        has_time_based_bypass: bool,
        has_time_based_denial: bool,
        category_actions_empty: bool,
    },
}

/// Single source of truth: determines whether a request needs a firewall approval popup.
///
/// Used by:
/// - Original trigger sites (gateway_tools.rs, chat.rs) to decide whether to show a popup
/// - Re-evaluation (commands_clients.rs) to auto-resolve pending popups after permission changes
pub fn check_needs_approval(ctx: &FirewallCheckContext) -> FirewallCheckResult {
    match ctx {
        FirewallCheckContext::McpTool {
            permissions,
            server_id,
            original_tool_name,
            session_approved,
            session_denied,
        } => {
            let decision = check_mcp_tool_access(permissions, server_id, original_tool_name);
            match decision {
                AccessDecision::Allow => FirewallCheckResult::Allow,
                AccessDecision::Deny => FirewallCheckResult::Deny,
                AccessDecision::Ask => {
                    if *session_denied {
                        FirewallCheckResult::Deny
                    } else if *session_approved {
                        FirewallCheckResult::Allow
                    } else {
                        FirewallCheckResult::Ask
                    }
                }
            }
        }
        FirewallCheckContext::SkillTool {
            permissions,
            skill_name,
            tool_name,
            session_approved,
            session_denied,
        } => {
            let decision = check_skill_tool_access(permissions, skill_name, tool_name);
            match decision {
                AccessDecision::Allow => FirewallCheckResult::Allow,
                AccessDecision::Deny => FirewallCheckResult::Deny,
                AccessDecision::Ask => {
                    if *session_denied {
                        FirewallCheckResult::Deny
                    } else if *session_approved {
                        FirewallCheckResult::Allow
                    } else {
                        FirewallCheckResult::Ask
                    }
                }
            }
        }
        FirewallCheckContext::Model {
            permissions,
            provider,
            model_id,
            has_time_based_approval,
        } => {
            let decision = check_model_access(permissions, provider, model_id);
            match decision {
                AccessDecision::Allow => FirewallCheckResult::Allow,
                AccessDecision::Deny => FirewallCheckResult::Deny,
                AccessDecision::Ask => {
                    if *has_time_based_approval {
                        FirewallCheckResult::Allow
                    } else {
                        FirewallCheckResult::Ask
                    }
                }
            }
        }
        FirewallCheckContext::Guardrail {
            has_time_based_bypass,
            has_time_based_denial,
            category_actions_empty,
        } => {
            if *has_time_based_bypass || *category_actions_empty {
                FirewallCheckResult::Allow
            } else if *has_time_based_denial {
                FirewallCheckResult::Deny
            } else {
                FirewallCheckResult::Ask
            }
        }
    }
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

    // ========================================================================
    // check_needs_approval tests
    // ========================================================================

    // --- MCP tool variants ---

    #[test]
    fn test_check_mcp_tool_global_allow() {
        let perms = McpPermissions {
            global: PermissionState::Allow,
            ..Default::default()
        };
        let ctx = FirewallCheckContext::McpTool {
            permissions: &perms,
            server_id: "srv1",
            original_tool_name: "read_file",
            session_approved: false,
            session_denied: false,
        };
        assert_eq!(check_needs_approval(&ctx), FirewallCheckResult::Allow);
    }

    #[test]
    fn test_check_mcp_tool_global_off() {
        let perms = McpPermissions {
            global: PermissionState::Off,
            ..Default::default()
        };
        let ctx = FirewallCheckContext::McpTool {
            permissions: &perms,
            server_id: "srv1",
            original_tool_name: "read_file",
            session_approved: false,
            session_denied: false,
        };
        assert_eq!(check_needs_approval(&ctx), FirewallCheckResult::Deny);
    }

    #[test]
    fn test_check_mcp_tool_global_ask_no_session() {
        let perms = McpPermissions {
            global: PermissionState::Ask,
            ..Default::default()
        };
        let ctx = FirewallCheckContext::McpTool {
            permissions: &perms,
            server_id: "srv1",
            original_tool_name: "read_file",
            session_approved: false,
            session_denied: false,
        };
        assert_eq!(check_needs_approval(&ctx), FirewallCheckResult::Ask);
    }

    #[test]
    fn test_check_mcp_tool_ask_session_approved() {
        let perms = McpPermissions {
            global: PermissionState::Ask,
            ..Default::default()
        };
        let ctx = FirewallCheckContext::McpTool {
            permissions: &perms,
            server_id: "srv1",
            original_tool_name: "read_file",
            session_approved: true,
            session_denied: false,
        };
        assert_eq!(check_needs_approval(&ctx), FirewallCheckResult::Allow);
    }

    #[test]
    fn test_check_mcp_tool_ask_session_denied() {
        let perms = McpPermissions {
            global: PermissionState::Ask,
            ..Default::default()
        };
        let ctx = FirewallCheckContext::McpTool {
            permissions: &perms,
            server_id: "srv1",
            original_tool_name: "read_file",
            session_approved: false,
            session_denied: true,
        };
        assert_eq!(check_needs_approval(&ctx), FirewallCheckResult::Deny);
    }

    #[test]
    fn test_check_mcp_tool_server_override_allow() {
        let mut servers = HashMap::new();
        servers.insert("srv1".to_string(), PermissionState::Allow);
        let perms = McpPermissions {
            global: PermissionState::Ask,
            servers,
            ..Default::default()
        };
        let ctx = FirewallCheckContext::McpTool {
            permissions: &perms,
            server_id: "srv1",
            original_tool_name: "read_file",
            session_approved: false,
            session_denied: false,
        };
        assert_eq!(check_needs_approval(&ctx), FirewallCheckResult::Allow);
    }

    #[test]
    fn test_check_mcp_tool_tool_override_off() {
        let mut tools = HashMap::new();
        tools.insert("srv1__write_file".to_string(), PermissionState::Off);
        let perms = McpPermissions {
            global: PermissionState::Allow,
            tools,
            ..Default::default()
        };
        let ctx = FirewallCheckContext::McpTool {
            permissions: &perms,
            server_id: "srv1",
            original_tool_name: "write_file",
            session_approved: false,
            session_denied: false,
        };
        assert_eq!(check_needs_approval(&ctx), FirewallCheckResult::Deny);
    }

    #[test]
    fn test_check_mcp_tool_tool_override_allow_overrides_server_ask() {
        let mut servers = HashMap::new();
        servers.insert("srv1".to_string(), PermissionState::Ask);
        let mut tools = HashMap::new();
        tools.insert("srv1__read_file".to_string(), PermissionState::Allow);
        let perms = McpPermissions {
            global: PermissionState::Off,
            servers,
            tools,
            ..Default::default()
        };
        let ctx = FirewallCheckContext::McpTool {
            permissions: &perms,
            server_id: "srv1",
            original_tool_name: "read_file",
            session_approved: false,
            session_denied: false,
        };
        assert_eq!(check_needs_approval(&ctx), FirewallCheckResult::Allow);
    }

    // --- Skill tool variants ---

    #[test]
    fn test_check_skill_tool_global_ask() {
        let perms = SkillsPermissions {
            global: PermissionState::Ask,
            ..Default::default()
        };
        let ctx = FirewallCheckContext::SkillTool {
            permissions: &perms,
            skill_name: "weather",
            tool_name: "skill_weather_run_main",
            session_approved: false,
            session_denied: false,
        };
        assert_eq!(check_needs_approval(&ctx), FirewallCheckResult::Ask);
    }

    #[test]
    fn test_check_skill_tool_skill_allow() {
        let mut skills = HashMap::new();
        skills.insert("weather".to_string(), PermissionState::Allow);
        let perms = SkillsPermissions {
            global: PermissionState::Ask,
            skills,
            ..Default::default()
        };
        let ctx = FirewallCheckContext::SkillTool {
            permissions: &perms,
            skill_name: "weather",
            tool_name: "skill_weather_run_main",
            session_approved: false,
            session_denied: false,
        };
        assert_eq!(check_needs_approval(&ctx), FirewallCheckResult::Allow);
    }

    #[test]
    fn test_check_skill_tool_tool_override() {
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
        let ctx = FirewallCheckContext::SkillTool {
            permissions: &perms,
            skill_name: "weather",
            tool_name: "skill_weather_run_dangerous",
            session_approved: false,
            session_denied: false,
        };
        assert_eq!(check_needs_approval(&ctx), FirewallCheckResult::Deny);
    }

    #[test]
    fn test_check_skill_tool_ask_session_approved() {
        let perms = SkillsPermissions {
            global: PermissionState::Ask,
            ..Default::default()
        };
        let ctx = FirewallCheckContext::SkillTool {
            permissions: &perms,
            skill_name: "weather",
            tool_name: "skill_weather_run_main",
            session_approved: true,
            session_denied: false,
        };
        assert_eq!(check_needs_approval(&ctx), FirewallCheckResult::Allow);
    }

    #[test]
    fn test_check_skill_tool_ask_session_denied() {
        let perms = SkillsPermissions {
            global: PermissionState::Ask,
            ..Default::default()
        };
        let ctx = FirewallCheckContext::SkillTool {
            permissions: &perms,
            skill_name: "weather",
            tool_name: "skill_weather_run_main",
            session_approved: false,
            session_denied: true,
        };
        assert_eq!(check_needs_approval(&ctx), FirewallCheckResult::Deny);
    }

    // --- Model variants ---

    #[test]
    fn test_check_model_global_allow() {
        let perms = ModelPermissions {
            global: PermissionState::Allow,
            ..Default::default()
        };
        let ctx = FirewallCheckContext::Model {
            permissions: &perms,
            provider: "openai",
            model_id: "gpt-4",
            has_time_based_approval: false,
        };
        assert_eq!(check_needs_approval(&ctx), FirewallCheckResult::Allow);
    }

    #[test]
    fn test_check_model_global_off() {
        let perms = ModelPermissions {
            global: PermissionState::Off,
            ..Default::default()
        };
        let ctx = FirewallCheckContext::Model {
            permissions: &perms,
            provider: "openai",
            model_id: "gpt-4",
            has_time_based_approval: false,
        };
        assert_eq!(check_needs_approval(&ctx), FirewallCheckResult::Deny);
    }

    #[test]
    fn test_check_model_global_ask_no_tracker() {
        let perms = ModelPermissions {
            global: PermissionState::Ask,
            ..Default::default()
        };
        let ctx = FirewallCheckContext::Model {
            permissions: &perms,
            provider: "openai",
            model_id: "gpt-4",
            has_time_based_approval: false,
        };
        assert_eq!(check_needs_approval(&ctx), FirewallCheckResult::Ask);
    }

    #[test]
    fn test_check_model_ask_with_time_based_approval() {
        let perms = ModelPermissions {
            global: PermissionState::Ask,
            ..Default::default()
        };
        let ctx = FirewallCheckContext::Model {
            permissions: &perms,
            provider: "openai",
            model_id: "gpt-4",
            has_time_based_approval: true,
        };
        assert_eq!(check_needs_approval(&ctx), FirewallCheckResult::Allow);
    }

    #[test]
    fn test_check_model_provider_override() {
        let mut providers = HashMap::new();
        providers.insert("openai".to_string(), PermissionState::Allow);
        let perms = ModelPermissions {
            global: PermissionState::Ask,
            providers,
            ..Default::default()
        };
        let ctx = FirewallCheckContext::Model {
            permissions: &perms,
            provider: "openai",
            model_id: "gpt-4",
            has_time_based_approval: false,
        };
        assert_eq!(check_needs_approval(&ctx), FirewallCheckResult::Allow);
    }

    #[test]
    fn test_check_model_model_override_off() {
        let mut models = HashMap::new();
        models.insert("openai__gpt-4".to_string(), PermissionState::Off);
        let perms = ModelPermissions {
            global: PermissionState::Allow,
            models,
            ..Default::default()
        };
        let ctx = FirewallCheckContext::Model {
            permissions: &perms,
            provider: "openai",
            model_id: "gpt-4",
            has_time_based_approval: false,
        };
        assert_eq!(check_needs_approval(&ctx), FirewallCheckResult::Deny);
    }

    // --- Guardrail variants ---

    #[test]
    fn test_check_guardrail_no_bypass_no_denial() {
        let ctx = FirewallCheckContext::Guardrail {
            has_time_based_bypass: false,
            has_time_based_denial: false,
            category_actions_empty: false,
        };
        assert_eq!(check_needs_approval(&ctx), FirewallCheckResult::Ask);
    }

    #[test]
    fn test_check_guardrail_has_bypass() {
        let ctx = FirewallCheckContext::Guardrail {
            has_time_based_bypass: true,
            has_time_based_denial: false,
            category_actions_empty: false,
        };
        assert_eq!(check_needs_approval(&ctx), FirewallCheckResult::Allow);
    }

    #[test]
    fn test_check_guardrail_has_denial() {
        let ctx = FirewallCheckContext::Guardrail {
            has_time_based_bypass: false,
            has_time_based_denial: true,
            category_actions_empty: false,
        };
        assert_eq!(check_needs_approval(&ctx), FirewallCheckResult::Deny);
    }

    #[test]
    fn test_check_guardrail_category_actions_empty() {
        let ctx = FirewallCheckContext::Guardrail {
            has_time_based_bypass: false,
            has_time_based_denial: false,
            category_actions_empty: true,
        };
        assert_eq!(check_needs_approval(&ctx), FirewallCheckResult::Allow);
    }

    #[test]
    fn test_check_guardrail_bypass_takes_priority_over_denial() {
        let ctx = FirewallCheckContext::Guardrail {
            has_time_based_bypass: true,
            has_time_based_denial: true,
            category_actions_empty: false,
        };
        assert_eq!(check_needs_approval(&ctx), FirewallCheckResult::Allow);
    }

    // --- Edge cases ---

    #[test]
    fn test_check_mixed_session_both_true() {
        // session_denied wins (checked first)
        let perms = McpPermissions {
            global: PermissionState::Ask,
            ..Default::default()
        };
        let ctx = FirewallCheckContext::McpTool {
            permissions: &perms,
            server_id: "srv1",
            original_tool_name: "read_file",
            session_approved: true,
            session_denied: true,
        };
        assert_eq!(check_needs_approval(&ctx), FirewallCheckResult::Deny);
    }

    #[test]
    fn test_check_guardrail_all_flags_false() {
        let ctx = FirewallCheckContext::Guardrail {
            has_time_based_bypass: false,
            has_time_based_denial: false,
            category_actions_empty: false,
        };
        assert_eq!(check_needs_approval(&ctx), FirewallCheckResult::Ask);
    }
}
