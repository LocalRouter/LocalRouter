//! Tests for unified permission inheritance system (Allow/Ask/Off)
//!
//! Tests cover:
//! - MCP permissions: global -> server -> tool/resource/prompt
//! - Skills permissions: global -> skill -> tool
//! - Model permissions: global -> provider -> model
//! - Marketplace permissions: single state

use lr_config::{McpPermissions, ModelPermissions, PermissionState, SkillsPermissions};
use std::collections::HashMap;

// =============================================================================
// MCP Permission Tests
// =============================================================================

mod mcp_permissions {
    use super::*;

    #[test]
    fn test_global_allow_inherits_to_all() {
        let perms = McpPermissions {
            global: PermissionState::Allow,
            servers: HashMap::new(),
            tools: HashMap::new(),
            resources: HashMap::new(),
            prompts: HashMap::new(),
        };

        // Server inherits from global
        assert_eq!(perms.resolve_server("any-server"), PermissionState::Allow);

        // Tool inherits from server (which inherits from global)
        assert_eq!(
            perms.resolve_tool("any-server", "any-tool"),
            PermissionState::Allow
        );

        // Resource inherits from server
        assert_eq!(
            perms.resolve_resource("any-server", "any-resource"),
            PermissionState::Allow
        );

        // Prompt inherits from server
        assert_eq!(
            perms.resolve_prompt("any-server", "any-prompt"),
            PermissionState::Allow
        );
    }

    #[test]
    fn test_global_ask_inherits_to_all() {
        let perms = McpPermissions {
            global: PermissionState::Ask,
            servers: HashMap::new(),
            tools: HashMap::new(),
            resources: HashMap::new(),
            prompts: HashMap::new(),
        };

        assert_eq!(perms.resolve_server("any-server"), PermissionState::Ask);
        assert_eq!(
            perms.resolve_tool("any-server", "any-tool"),
            PermissionState::Ask
        );
        assert_eq!(
            perms.resolve_resource("any-server", "any-resource"),
            PermissionState::Ask
        );
        assert_eq!(
            perms.resolve_prompt("any-server", "any-prompt"),
            PermissionState::Ask
        );
    }

    #[test]
    fn test_global_off_inherits_to_all() {
        let perms = McpPermissions {
            global: PermissionState::Off,
            servers: HashMap::new(),
            tools: HashMap::new(),
            resources: HashMap::new(),
            prompts: HashMap::new(),
        };

        assert_eq!(perms.resolve_server("any-server"), PermissionState::Off);
        assert_eq!(
            perms.resolve_tool("any-server", "any-tool"),
            PermissionState::Off
        );
        assert_eq!(
            perms.resolve_resource("any-server", "any-resource"),
            PermissionState::Off
        );
        assert_eq!(
            perms.resolve_prompt("any-server", "any-prompt"),
            PermissionState::Off
        );
    }

    #[test]
    fn test_server_override_inherits_to_children() {
        let mut servers = HashMap::new();
        servers.insert("server-1".to_string(), PermissionState::Ask);

        let perms = McpPermissions {
            global: PermissionState::Allow,
            servers,
            tools: HashMap::new(),
            resources: HashMap::new(),
            prompts: HashMap::new(),
        };

        // Server has explicit override
        assert_eq!(perms.resolve_server("server-1"), PermissionState::Ask);

        // Other server inherits from global
        assert_eq!(perms.resolve_server("server-2"), PermissionState::Allow);

        // Tools on server-1 inherit from server override
        assert_eq!(
            perms.resolve_tool("server-1", "any-tool"),
            PermissionState::Ask
        );

        // Tools on server-2 inherit from global
        assert_eq!(
            perms.resolve_tool("server-2", "any-tool"),
            PermissionState::Allow
        );

        // Resources on server-1 inherit from server override
        assert_eq!(
            perms.resolve_resource("server-1", "any-resource"),
            PermissionState::Ask
        );

        // Prompts on server-1 inherit from server override
        assert_eq!(
            perms.resolve_prompt("server-1", "any-prompt"),
            PermissionState::Ask
        );
    }

    #[test]
    fn test_tool_override_takes_precedence() {
        let mut servers = HashMap::new();
        servers.insert("server-1".to_string(), PermissionState::Ask);

        let mut tools = HashMap::new();
        tools.insert("server-1__special-tool".to_string(), PermissionState::Off);

        let perms = McpPermissions {
            global: PermissionState::Allow,
            servers,
            tools,
            resources: HashMap::new(),
            prompts: HashMap::new(),
        };

        // Server has Ask
        assert_eq!(perms.resolve_server("server-1"), PermissionState::Ask);

        // Regular tool inherits from server (Ask)
        assert_eq!(
            perms.resolve_tool("server-1", "regular-tool"),
            PermissionState::Ask
        );

        // Special tool has explicit Off override
        assert_eq!(
            perms.resolve_tool("server-1", "special-tool"),
            PermissionState::Off
        );
    }

    #[test]
    fn test_resource_override_takes_precedence() {
        let mut servers = HashMap::new();
        servers.insert("server-1".to_string(), PermissionState::Allow);

        let mut resources = HashMap::new();
        resources.insert(
            "server-1__file:///secret.txt".to_string(),
            PermissionState::Off,
        );

        let perms = McpPermissions {
            global: PermissionState::Allow,
            servers,
            tools: HashMap::new(),
            resources,
            prompts: HashMap::new(),
        };

        // Regular resource inherits from server
        assert_eq!(
            perms.resolve_resource("server-1", "file:///normal.txt"),
            PermissionState::Allow
        );

        // Secret resource has explicit Off override
        assert_eq!(
            perms.resolve_resource("server-1", "file:///secret.txt"),
            PermissionState::Off
        );
    }

    #[test]
    fn test_prompt_override_takes_precedence() {
        let mut prompts = HashMap::new();
        prompts.insert(
            "server-1__dangerous-prompt".to_string(),
            PermissionState::Ask,
        );

        let perms = McpPermissions {
            global: PermissionState::Allow,
            servers: HashMap::new(),
            tools: HashMap::new(),
            resources: HashMap::new(),
            prompts,
        };

        // Regular prompt inherits from global
        assert_eq!(
            perms.resolve_prompt("server-1", "normal-prompt"),
            PermissionState::Allow
        );

        // Dangerous prompt has explicit Ask override
        assert_eq!(
            perms.resolve_prompt("server-1", "dangerous-prompt"),
            PermissionState::Ask
        );
    }

    #[test]
    fn test_mixed_inheritance_chain() {
        // Global: Allow
        // Server-1: Ask (override)
        // Server-1 tool-a: Off (override)
        // Server-1 tool-b: inherits Ask from server
        // Server-2: inherits Allow from global
        // Server-2 tool-c: Ask (override)

        let mut servers = HashMap::new();
        servers.insert("server-1".to_string(), PermissionState::Ask);

        let mut tools = HashMap::new();
        tools.insert("server-1__tool-a".to_string(), PermissionState::Off);
        tools.insert("server-2__tool-c".to_string(), PermissionState::Ask);

        let perms = McpPermissions {
            global: PermissionState::Allow,
            servers,
            tools,
            resources: HashMap::new(),
            prompts: HashMap::new(),
        };

        // Server-1 has Ask override
        assert_eq!(perms.resolve_server("server-1"), PermissionState::Ask);
        // Server-1 tool-a has Off override
        assert_eq!(
            perms.resolve_tool("server-1", "tool-a"),
            PermissionState::Off
        );
        // Server-1 tool-b inherits from server (Ask)
        assert_eq!(
            perms.resolve_tool("server-1", "tool-b"),
            PermissionState::Ask
        );

        // Server-2 inherits from global (Allow)
        assert_eq!(perms.resolve_server("server-2"), PermissionState::Allow);
        // Server-2 tool-c has Ask override
        assert_eq!(
            perms.resolve_tool("server-2", "tool-c"),
            PermissionState::Ask
        );
        // Server-2 tool-d inherits from server (Allow)
        assert_eq!(
            perms.resolve_tool("server-2", "tool-d"),
            PermissionState::Allow
        );
    }
}

// =============================================================================
// Skills Permission Tests
// =============================================================================

mod skills_permissions {
    use super::*;

    #[test]
    fn test_global_allow_inherits_to_all() {
        let perms = SkillsPermissions {
            global: PermissionState::Allow,
            skills: HashMap::new(),
            tools: HashMap::new(),
        };

        assert_eq!(perms.resolve_skill("any-skill"), PermissionState::Allow);
        assert_eq!(
            perms.resolve_tool("any-skill", "any-tool"),
            PermissionState::Allow
        );
    }

    #[test]
    fn test_global_ask_inherits_to_all() {
        let perms = SkillsPermissions {
            global: PermissionState::Ask,
            skills: HashMap::new(),
            tools: HashMap::new(),
        };

        assert_eq!(perms.resolve_skill("any-skill"), PermissionState::Ask);
        assert_eq!(
            perms.resolve_tool("any-skill", "any-tool"),
            PermissionState::Ask
        );
    }

    #[test]
    fn test_global_off_inherits_to_all() {
        let perms = SkillsPermissions {
            global: PermissionState::Off,
            skills: HashMap::new(),
            tools: HashMap::new(),
        };

        assert_eq!(perms.resolve_skill("any-skill"), PermissionState::Off);
        assert_eq!(
            perms.resolve_tool("any-skill", "any-tool"),
            PermissionState::Off
        );
    }

    #[test]
    fn test_skill_override_inherits_to_tools() {
        let mut skills = HashMap::new();
        skills.insert("filesystem".to_string(), PermissionState::Ask);

        let perms = SkillsPermissions {
            global: PermissionState::Allow,
            skills,
            tools: HashMap::new(),
        };

        // Skill has explicit override
        assert_eq!(perms.resolve_skill("filesystem"), PermissionState::Ask);

        // Other skill inherits from global
        assert_eq!(perms.resolve_skill("http"), PermissionState::Allow);

        // Tools on filesystem skill inherit from skill override
        assert_eq!(
            perms.resolve_tool("filesystem", "read_file"),
            PermissionState::Ask
        );
        assert_eq!(
            perms.resolve_tool("filesystem", "write_file"),
            PermissionState::Ask
        );

        // Tools on http skill inherit from global
        assert_eq!(perms.resolve_tool("http", "get"), PermissionState::Allow);
    }

    #[test]
    fn test_tool_override_takes_precedence() {
        let mut skills = HashMap::new();
        skills.insert("filesystem".to_string(), PermissionState::Allow);

        let mut tools = HashMap::new();
        tools.insert("filesystem__delete_file".to_string(), PermissionState::Ask);
        tools.insert("filesystem__format_disk".to_string(), PermissionState::Off);

        let perms = SkillsPermissions {
            global: PermissionState::Allow,
            skills,
            tools,
        };

        // Regular tool inherits from skill
        assert_eq!(
            perms.resolve_tool("filesystem", "read_file"),
            PermissionState::Allow
        );

        // delete_file has Ask override
        assert_eq!(
            perms.resolve_tool("filesystem", "delete_file"),
            PermissionState::Ask
        );

        // format_disk has Off override
        assert_eq!(
            perms.resolve_tool("filesystem", "format_disk"),
            PermissionState::Off
        );
    }

    #[test]
    fn test_mixed_inheritance_chain() {
        // Global: Allow
        // Skill "filesystem": Ask (override)
        // Skill "filesystem" tool "read_file": Allow (override, same as global but explicit)
        // Skill "filesystem" tool "delete_file": Off (override)
        // Skill "http": inherits Allow from global
        // Skill "http" tool "post": Ask (override)

        let mut skills = HashMap::new();
        skills.insert("filesystem".to_string(), PermissionState::Ask);

        let mut tools = HashMap::new();
        tools.insert("filesystem__read_file".to_string(), PermissionState::Allow);
        tools.insert("filesystem__delete_file".to_string(), PermissionState::Off);
        tools.insert("http__post".to_string(), PermissionState::Ask);

        let perms = SkillsPermissions {
            global: PermissionState::Allow,
            skills,
            tools,
        };

        // Filesystem skill has Ask override
        assert_eq!(perms.resolve_skill("filesystem"), PermissionState::Ask);
        // read_file has explicit Allow override
        assert_eq!(
            perms.resolve_tool("filesystem", "read_file"),
            PermissionState::Allow
        );
        // write_file inherits from skill (Ask)
        assert_eq!(
            perms.resolve_tool("filesystem", "write_file"),
            PermissionState::Ask
        );
        // delete_file has explicit Off override
        assert_eq!(
            perms.resolve_tool("filesystem", "delete_file"),
            PermissionState::Off
        );

        // Http skill inherits from global (Allow)
        assert_eq!(perms.resolve_skill("http"), PermissionState::Allow);
        // get inherits from skill (Allow)
        assert_eq!(perms.resolve_tool("http", "get"), PermissionState::Allow);
        // post has explicit Ask override
        assert_eq!(perms.resolve_tool("http", "post"), PermissionState::Ask);
    }
}

// =============================================================================
// Model Permission Tests
// =============================================================================

mod model_permissions {
    use super::*;

    #[test]
    fn test_global_allow_inherits_to_all() {
        let perms = ModelPermissions {
            global: PermissionState::Allow,
            providers: HashMap::new(),
            models: HashMap::new(),
        };

        assert_eq!(
            perms.resolve_provider("any-provider"),
            PermissionState::Allow
        );
        assert_eq!(
            perms.resolve_model("any-provider", "any-model"),
            PermissionState::Allow
        );
    }

    #[test]
    fn test_global_ask_inherits_to_all() {
        let perms = ModelPermissions {
            global: PermissionState::Ask,
            providers: HashMap::new(),
            models: HashMap::new(),
        };

        assert_eq!(perms.resolve_provider("any-provider"), PermissionState::Ask);
        assert_eq!(
            perms.resolve_model("any-provider", "any-model"),
            PermissionState::Ask
        );
    }

    #[test]
    fn test_global_off_inherits_to_all() {
        let perms = ModelPermissions {
            global: PermissionState::Off,
            providers: HashMap::new(),
            models: HashMap::new(),
        };

        assert_eq!(perms.resolve_provider("any-provider"), PermissionState::Off);
        assert_eq!(
            perms.resolve_model("any-provider", "any-model"),
            PermissionState::Off
        );
    }

    #[test]
    fn test_provider_override_inherits_to_models() {
        let mut providers = HashMap::new();
        providers.insert("openai".to_string(), PermissionState::Ask);

        let perms = ModelPermissions {
            global: PermissionState::Allow,
            providers,
            models: HashMap::new(),
        };

        // Provider has explicit override
        assert_eq!(perms.resolve_provider("openai"), PermissionState::Ask);

        // Other provider inherits from global
        assert_eq!(perms.resolve_provider("anthropic"), PermissionState::Allow);

        // Models on openai inherit from provider override
        assert_eq!(perms.resolve_model("openai", "gpt-4"), PermissionState::Ask);
        assert_eq!(
            perms.resolve_model("openai", "gpt-3.5-turbo"),
            PermissionState::Ask
        );

        // Models on anthropic inherit from global
        assert_eq!(
            perms.resolve_model("anthropic", "claude-3-opus"),
            PermissionState::Allow
        );
    }

    #[test]
    fn test_model_override_takes_precedence() {
        let mut providers = HashMap::new();
        providers.insert("openai".to_string(), PermissionState::Allow);

        let mut models = HashMap::new();
        models.insert("openai__gpt-4".to_string(), PermissionState::Ask);
        models.insert("openai__o1-preview".to_string(), PermissionState::Off);

        let perms = ModelPermissions {
            global: PermissionState::Allow,
            providers,
            models,
        };

        // Regular model inherits from provider
        assert_eq!(
            perms.resolve_model("openai", "gpt-3.5-turbo"),
            PermissionState::Allow
        );

        // gpt-4 has Ask override
        assert_eq!(perms.resolve_model("openai", "gpt-4"), PermissionState::Ask);

        // o1-preview has Off override
        assert_eq!(
            perms.resolve_model("openai", "o1-preview"),
            PermissionState::Off
        );
    }

    #[test]
    fn test_mixed_inheritance_chain() {
        // Global: Allow
        // Provider "openai": Ask (override)
        // Provider "openai" model "gpt-3.5-turbo": Allow (override)
        // Provider "openai" model "o1-preview": Off (override)
        // Provider "anthropic": inherits Allow from global
        // Provider "anthropic" model "claude-3-opus": Ask (override)

        let mut providers = HashMap::new();
        providers.insert("openai".to_string(), PermissionState::Ask);

        let mut models = HashMap::new();
        models.insert("openai__gpt-3.5-turbo".to_string(), PermissionState::Allow);
        models.insert("openai__o1-preview".to_string(), PermissionState::Off);
        models.insert("anthropic__claude-3-opus".to_string(), PermissionState::Ask);

        let perms = ModelPermissions {
            global: PermissionState::Allow,
            providers,
            models,
        };

        // OpenAI provider has Ask override
        assert_eq!(perms.resolve_provider("openai"), PermissionState::Ask);
        // gpt-3.5-turbo has explicit Allow override
        assert_eq!(
            perms.resolve_model("openai", "gpt-3.5-turbo"),
            PermissionState::Allow
        );
        // gpt-4 inherits from provider (Ask)
        assert_eq!(perms.resolve_model("openai", "gpt-4"), PermissionState::Ask);
        // o1-preview has explicit Off override
        assert_eq!(
            perms.resolve_model("openai", "o1-preview"),
            PermissionState::Off
        );

        // Anthropic provider inherits from global (Allow)
        assert_eq!(perms.resolve_provider("anthropic"), PermissionState::Allow);
        // claude-3-sonnet inherits from provider (Allow)
        assert_eq!(
            perms.resolve_model("anthropic", "claude-3-sonnet"),
            PermissionState::Allow
        );
        // claude-3-opus has explicit Ask override
        assert_eq!(
            perms.resolve_model("anthropic", "claude-3-opus"),
            PermissionState::Ask
        );
    }
}

// =============================================================================
// Permission State Tests
// =============================================================================

mod permission_state {
    use super::*;

    #[test]
    fn test_default_is_off() {
        assert_eq!(PermissionState::default(), PermissionState::Off);
    }

    #[test]
    fn test_is_enabled() {
        assert!(PermissionState::Allow.is_enabled());
        assert!(PermissionState::Ask.is_enabled());
        assert!(!PermissionState::Off.is_enabled());
    }

    #[test]
    fn test_equality() {
        assert_eq!(PermissionState::Allow, PermissionState::Allow);
        assert_eq!(PermissionState::Ask, PermissionState::Ask);
        assert_eq!(PermissionState::Off, PermissionState::Off);

        assert_ne!(PermissionState::Allow, PermissionState::Ask);
        assert_ne!(PermissionState::Allow, PermissionState::Off);
        assert_ne!(PermissionState::Ask, PermissionState::Off);
    }

    #[test]
    fn test_clone() {
        let state = PermissionState::Ask;
        let cloned = state.clone();
        assert_eq!(state, cloned);
    }
}

// =============================================================================
// Empty Permission Tests (edge cases)
// =============================================================================

mod empty_permissions {
    use super::*;

    #[test]
    fn test_empty_mcp_permissions_use_default() {
        let perms = McpPermissions::default();

        // Default global is Off
        assert_eq!(perms.global, PermissionState::Off);
        assert_eq!(perms.resolve_server("any"), PermissionState::Off);
        assert_eq!(perms.resolve_tool("any", "tool"), PermissionState::Off);
    }

    #[test]
    fn test_empty_skills_permissions_use_default() {
        let perms = SkillsPermissions::default();

        assert_eq!(perms.global, PermissionState::Off);
        assert_eq!(perms.resolve_skill("any"), PermissionState::Off);
        assert_eq!(perms.resolve_tool("any", "tool"), PermissionState::Off);
    }

    #[test]
    fn test_empty_model_permissions_use_default() {
        let perms = ModelPermissions::default();

        assert_eq!(perms.global, PermissionState::Off);
        assert_eq!(perms.resolve_provider("any"), PermissionState::Off);
        assert_eq!(perms.resolve_model("any", "model"), PermissionState::Off);
    }
}

// =============================================================================
// Marketplace Permission Tests
// =============================================================================

mod marketplace_permissions {
    use super::*;

    #[test]
    fn test_marketplace_permission_states() {
        // Marketplace just uses a single PermissionState
        // No inheritance needed, just test the states work
        let allow = PermissionState::Allow;
        let ask = PermissionState::Ask;
        let off = PermissionState::Off;

        assert!(allow.is_enabled());
        assert!(ask.is_enabled());
        assert!(!off.is_enabled());
    }
}
