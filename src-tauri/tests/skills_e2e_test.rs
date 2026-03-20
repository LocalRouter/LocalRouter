//! End-to-end integration test for the Skills system
//!
//! Sets up a "get-current-time" skill with a JavaScript script in a temp directory,
//! stands up the MCP gateway with skill support, and exercises all skill tool commands
//! through the gateway's JSON-RPC interface.
//!
//! Skills use a single meta-tool pattern:
//! - `skill_read` — takes a `skill` parameter to load full instructions

mod mcp_tests;

use localrouter::config::AppConfig;
use localrouter::config::ConfigManager;
use localrouter::config::{PermissionState, SkillsPermissions};
use localrouter::mcp::gateway::{GatewayConfig, McpGateway};
use localrouter::mcp::protocol::JsonRpcRequest;
use localrouter::mcp::McpServerManager;
use localrouter::monitoring::metrics::MetricsCollector;
use localrouter::monitoring::storage::MetricsDatabase;
use localrouter::providers::registry::ProviderRegistry;
use localrouter::router::{RateLimiterManager, Router};
use localrouter::skills::SkillManager;
use serde_json::json;
use std::sync::Arc;
use tempfile::TempDir;

/// Helper to create a minimal test router (same pattern as mcp_gateway_mock_integration_tests)
fn create_test_router() -> Arc<Router> {
    let config = AppConfig::default();
    let config_manager = Arc::new(ConfigManager::new(
        config,
        std::path::PathBuf::from("/tmp/test_skills_e2e_router.yaml"),
    ));

    let provider_registry = Arc::new(ProviderRegistry::new());
    let rate_limiter = Arc::new(RateLimiterManager::new(None));

    let metrics_db_path = std::env::temp_dir().join(format!(
        "test_skills_e2e_metrics_{}.db",
        uuid::Uuid::new_v4()
    ));
    let metrics_db = Arc::new(MetricsDatabase::new(metrics_db_path).unwrap());
    let metrics_collector = Arc::new(MetricsCollector::new(metrics_db));

    Arc::new(Router::new(
        config_manager,
        provider_registry,
        rate_limiter,
        metrics_collector,
        Arc::new(lr_router::FreeTierManager::new(None)),
    ))
}

/// Create the test skill directory structure in a temp dir:
///
/// get-current-time/
/// ├── SKILL.md
/// ├── scripts/
/// │   └── get-time.js
/// └── references/
///     └── notes.md
fn create_test_skill(temp_dir: &TempDir) -> std::path::PathBuf {
    let skill_dir = temp_dir.path().join("get-current-time");
    std::fs::create_dir_all(skill_dir.join("scripts")).unwrap();
    std::fs::create_dir_all(skill_dir.join("references")).unwrap();

    // SKILL.md with YAML frontmatter
    std::fs::write(
        skill_dir.join("SKILL.md"),
        r#"---
name: get-current-time
description: "Get the current date and time"
tags:
  - time
  - utility
---
# Get Current Time

This skill provides a simple script to retrieve the current date and time
in ISO 8601 format.
"#,
    )
    .unwrap();

    // JavaScript script that prints the current time
    std::fs::write(
        skill_dir.join("scripts").join("get-time.js"),
        "console.log(new Date().toISOString());\n",
    )
    .unwrap();

    // Reference document
    std::fs::write(
        skill_dir.join("references").join("notes.md"),
        "# Usage Notes\n\nRun the get-time.js script to get the current UTC time.\n",
    )
    .unwrap();

    skill_dir
}

#[tokio::test]
async fn test_skills_e2e_all_tool_commands() {
    // ── Setup ──────────────────────────────────────────────────────
    let temp_dir = TempDir::new().unwrap();
    let _skill_dir = create_test_skill(&temp_dir);

    // SkillManager: discover the test skill
    let skill_manager = Arc::new(SkillManager::new());
    skill_manager.initial_scan(
        &[temp_dir
            .path()
            .join("get-current-time")
            .to_string_lossy()
            .to_string()],
        &[],
    );

    // Verify skill was discovered
    let skills = skill_manager.list();
    assert_eq!(skills.len(), 1, "Expected exactly one skill");
    assert_eq!(skills[0].name, "get-current-time");

    // McpServerManager + Gateway
    let server_manager = Arc::new(McpServerManager::new());
    let router = create_test_router();
    let gateway = McpGateway::new(server_manager, GatewayConfig::default(), router);
    gateway.register_virtual_server(Arc::new(
        lr_mcp::gateway::virtual_skills::SkillsVirtualServer::new(
            skill_manager,
            lr_config::ContextManagementConfig::default(),
            lr_config::SkillsConfig::default(),
        ),
    ));
    let gateway = Arc::new(gateway);

    let client_id = "test-skills-client";
    let skills_permissions = {
        let mut perms = SkillsPermissions::default();
        perms
            .skills
            .insert("get-current-time".to_string(), PermissionState::Allow);
        perms
    };

    // ── Step 1: tools/list ──────────────────────────────────────────
    // Should contain the single `skill_read` meta-tool
    let tools_list_req = JsonRpcRequest::with_id(1, "tools/list".to_string(), Some(json!({})));
    let response = gateway
        .handle_request_with_skills(
            client_id,
            None,
            vec![],
            vec![],
            lr_config::McpPermissions::default(),
            skills_permissions.clone(),
            "Test Client".to_string(),
            PermissionState::Off,
            lr_config::PermissionState::Off,
            None,
            None,
            lr_config::PermissionState::default(), // mcp_sampling_permission
            lr_config::PermissionState::default(), // mcp_elicitation_permission
            None,                                  // memory_enabled
            lr_config::ClientMode::default(),
            tools_list_req,
            None, // monitor_session_id
        )
        .await
        .expect("tools/list should succeed");

    let result = response.result.expect("tools/list should have a result");
    let tools = result["tools"]
        .as_array()
        .expect("result should have tools array");

    let tool_names: Vec<&str> = tools.iter().filter_map(|t| t["name"].as_str()).collect();

    assert!(
        tool_names.contains(&"SkillRead"),
        "Missing skill_read meta-tool. Found: {:?}",
        tool_names
    );

    // Verify the meta-tool exists and accepts a "name" parameter
    let skill_tool = tools
        .iter()
        .find(|t| t["name"].as_str() == Some("SkillRead"))
        .expect("skill_read tool should exist");
    let schema = &skill_tool["inputSchema"];
    assert!(
        schema["properties"]["name"]["type"].as_str() == Some("string"),
        "skill_read should have a 'name' string parameter. Schema: {:?}",
        schema
    );

    // ── Step 2: skill_read with name parameter ─────────────────
    let show_req = JsonRpcRequest::with_id(
        2,
        "tools/call".to_string(),
        Some(json!({
            "name": "SkillRead",
            "arguments": { "name": "get-current-time" }
        })),
    );
    let response = gateway
        .handle_request_with_skills(
            client_id,
            None,
            vec![],
            vec![],
            lr_config::McpPermissions::default(),
            skills_permissions.clone(),
            "Test Client".to_string(),
            PermissionState::Off,
            lr_config::PermissionState::Off,
            None,
            None,
            lr_config::PermissionState::default(), // mcp_sampling_permission
            lr_config::PermissionState::default(), // mcp_elicitation_permission
            None,                                  // memory_enabled
            lr_config::ClientMode::default(),
            show_req,
            None, // monitor_session_id
        )
        .await
        .expect("skill_read should succeed");

    let result = response.result.expect("skill_read should have a result");
    let content = result["content"]
        .as_array()
        .expect("skill_read should have content array");
    let text = content[0]["text"]
        .as_str()
        .expect("content should have text");

    assert!(
        text.contains("get-current-time"),
        "get_info response should contain skill name"
    );
    assert!(
        text.contains("Get the current date and time"),
        "get_info response should contain description"
    );
    assert!(
        text.contains("scripts/get-time.js"),
        "get_info response should list scripts"
    );
    assert!(
        text.contains("references/notes.md"),
        "get_info response should list references"
    );
    assert!(
        text.contains("ResourceRead"),
        "get_info should reference ResourceRead for reading scripts"
    );

    // ── Step 3: tools/list unchanged after get_info ─────────────────
    let tools_list_req2 = JsonRpcRequest::with_id(20, "tools/list".to_string(), Some(json!({})));
    let response = gateway
        .handle_request_with_skills(
            client_id,
            None,
            vec![],
            vec![],
            lr_config::McpPermissions::default(),
            skills_permissions.clone(),
            "Test Client".to_string(),
            PermissionState::Off,
            lr_config::PermissionState::Off,
            None,
            None,
            lr_config::PermissionState::default(), // mcp_sampling_permission
            lr_config::PermissionState::default(), // mcp_elicitation_permission
            None,                                  // memory_enabled
            lr_config::ClientMode::default(),
            tools_list_req2,
            None, // monitor_session_id
        )
        .await
        .expect("tools/list after get_info should succeed");

    let result = response.result.expect("tools/list should have a result");
    let tools = result["tools"]
        .as_array()
        .expect("result should have tools array");
    let tool_names: Vec<&str> = tools.iter().filter_map(|t| t["name"].as_str()).collect();

    assert!(
        tool_names.contains(&"SkillRead"),
        "skill_read should still be present. Found: {:?}",
        tool_names
    );
}

/// Helper to set up a gateway with a single "get-current-time" skill
async fn setup_gateway_with_skill() -> (Arc<McpGateway>, TempDir) {
    let temp_dir = TempDir::new().unwrap();
    let _skill_dir = create_test_skill(&temp_dir);

    let skill_manager = Arc::new(SkillManager::new());
    skill_manager.initial_scan(
        &[temp_dir
            .path()
            .join("get-current-time")
            .to_string_lossy()
            .to_string()],
        &[],
    );

    let server_manager = Arc::new(McpServerManager::new());
    let router = create_test_router();
    let gateway = McpGateway::new(server_manager, GatewayConfig::default(), router);
    gateway.register_virtual_server(Arc::new(
        lr_mcp::gateway::virtual_skills::SkillsVirtualServer::new(
            skill_manager,
            lr_config::ContextManagementConfig::default(),
            lr_config::SkillsConfig::default(),
        ),
    ));

    (Arc::new(gateway), temp_dir)
}

/// Helper to set up a gateway with NO skills
async fn setup_gateway_without_skills() -> Arc<McpGateway> {
    let server_manager = Arc::new(McpServerManager::new());
    let router = create_test_router();
    let gateway = McpGateway::new(server_manager, GatewayConfig::default(), router);
    // No virtual servers registered — no skills
    Arc::new(gateway)
}

/// Helper to extract tool names from a tools/list response
fn extract_tool_names(response: &localrouter::mcp::protocol::JsonRpcResponse) -> Vec<String> {
    let result = response.result.as_ref().expect("should have result");
    let tools = result["tools"].as_array().expect("should have tools array");
    tools
        .iter()
        .filter_map(|t| t["name"].as_str().map(|s| s.to_string()))
        .collect()
}

/// Test: When there are no skills, skill tools must be completely absent from tools/list
#[tokio::test]
async fn test_no_skill_tools_when_no_skills_configured() {
    let gateway = setup_gateway_without_skills().await;

    let client_id = "no-skills-client";

    // Call tools/list with empty allowed_skills
    let req = JsonRpcRequest::with_id(1, "tools/list".to_string(), Some(json!({})));
    let response = gateway
        .handle_request_with_skills(
            client_id,
            None,
            vec![],
            vec![],
            lr_config::McpPermissions::default(),
            SkillsPermissions::default(),
            "Test Client".to_string(),
            PermissionState::Off,
            lr_config::PermissionState::Off,
            None,
            None,
            lr_config::PermissionState::default(), // mcp_sampling_permission
            lr_config::PermissionState::default(), // mcp_elicitation_permission
            None,                                  // memory_enabled
            lr_config::ClientMode::default(),
            req,
            None, // monitor_session_id
        )
        .await
        .expect("tools/list should succeed");

    let tool_names = extract_tool_names(&response);

    // No skill tools should be present
    assert!(
        !tool_names.iter().any(|n| n.starts_with("Skill")),
        "Skill tools should not appear without skills. Found: {:?}",
        tool_names
    );
}

/// Test: Skill tools must survive the tools/list cache (second call must still include them)
#[tokio::test]
async fn test_skill_tools_present_after_cache_hit() {
    let (gateway, _temp_dir) = setup_gateway_with_skill().await;

    let client_id = "cache-test-client";
    let skills_permissions = {
        let mut perms = SkillsPermissions::default();
        perms
            .skills
            .insert("get-current-time".to_string(), PermissionState::Allow);
        perms
    };

    // First call: populates cache
    let req = JsonRpcRequest::with_id(1, "tools/list".to_string(), Some(json!({})));
    let response1 = gateway
        .handle_request_with_skills(
            client_id,
            None,
            vec![],
            vec![],
            lr_config::McpPermissions::default(),
            skills_permissions.clone(),
            "Test Client".to_string(),
            PermissionState::Off,
            lr_config::PermissionState::Off,
            None,
            None,
            lr_config::PermissionState::default(), // mcp_sampling_permission
            lr_config::PermissionState::default(), // mcp_elicitation_permission
            None,                                  // memory_enabled
            lr_config::ClientMode::default(),
            req,
            None, // monitor_session_id
        )
        .await
        .expect("first tools/list should succeed");

    let names1 = extract_tool_names(&response1);
    assert!(
        names1.contains(&"SkillRead".to_string()),
        "First call should include skill_read tool. Found: {:?}",
        names1
    );

    // Second call: should hit cache but still include skill tools
    let req2 = JsonRpcRequest::with_id(2, "tools/list".to_string(), Some(json!({})));
    let response2 = gateway
        .handle_request_with_skills(
            client_id,
            None,
            vec![],
            vec![],
            lr_config::McpPermissions::default(),
            skills_permissions.clone(),
            "Test Client".to_string(),
            PermissionState::Off,
            lr_config::PermissionState::Off,
            None,
            None,
            lr_config::PermissionState::default(), // mcp_sampling_permission
            lr_config::PermissionState::default(), // mcp_elicitation_permission
            None,                                  // memory_enabled
            req2,
            None, // monitor_session_id
        )
        .await
        .expect("second tools/list should succeed");

    let names2 = extract_tool_names(&response2);
    assert!(
        names2.contains(&"SkillRead".to_string()),
        "Cached tools/list must still include skill_read tool. Found: {:?}",
        names2
    );
}
