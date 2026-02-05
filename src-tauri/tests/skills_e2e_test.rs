//! End-to-end integration test for the Skills system
//!
//! Sets up a "get-current-time" skill with a JavaScript script in a temp directory,
//! stands up the MCP gateway with skill support, and exercises all skill tool commands
//! through the gateway's JSON-RPC interface.
//!
//! Tools use per-skill namespaced names with deferred loading:
//! - `skill_{sname}_get_info` — always visible
//! - `skill_{sname}_run_{sfile}` — visible after get_info
//! - `skill_{sname}_read_{sfile}` — visible after get_info

mod mcp_tests;

use localrouter::config::AppConfig;
use localrouter::config::ConfigManager;
use localrouter::config::{PermissionState, SkillsPermissions};
use localrouter::mcp::gateway::types::DeferredLoadingState;
use localrouter::mcp::gateway::{GatewayConfig, McpGateway};
use localrouter::mcp::protocol::JsonRpcRequest;
use localrouter::mcp::McpServerManager;
use localrouter::monitoring::metrics::MetricsCollector;
use localrouter::monitoring::storage::MetricsDatabase;
use localrouter::providers::registry::ProviderRegistry;
use localrouter::router::{RateLimiterManager, Router};
use localrouter::skills::executor::ScriptExecutor;
use localrouter::skills::SkillManager;
use serde_json::json;
use std::collections::HashSet;
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

    // ScriptExecutor
    let script_executor = Arc::new(ScriptExecutor::new());

    // McpServerManager + Gateway (with async enabled for testing)
    let server_manager = Arc::new(McpServerManager::new());
    let router = create_test_router();
    let gateway = McpGateway::new(server_manager, GatewayConfig::default(), router);
    gateway.set_skill_support(skill_manager, script_executor);
    gateway.set_skills_async_enabled(true);
    let gateway = Arc::new(gateway);

    let client_id = "test-skills-client";
    let skills_permissions = {
        let mut perms = SkillsPermissions::default();
        perms
            .skills
            .insert("get-current-time".to_string(), PermissionState::Allow);
        perms
    };

    // ── Step 1: tools/list (before get_info) ──────────────────────
    // Only get_info tools should be visible initially
    let tools_list_req = JsonRpcRequest::with_id(1, "tools/list".to_string(), Some(json!({})));
    let response = gateway
        .handle_request_with_skills(
            client_id,
            vec![],
            false,
            vec![],
            skills_permissions.clone(),
            lr_config::FirewallRules::default(),
            "Test Client".to_string(),
            PermissionState::Off,
            tools_list_req,
        )
        .await
        .expect("tools/list should succeed");

    let result = response.result.expect("tools/list should have a result");
    let tools = result["tools"]
        .as_array()
        .expect("result should have tools array");

    let tool_names: Vec<&str> = tools.iter().filter_map(|t| t["name"].as_str()).collect();

    assert!(
        tool_names.contains(&"skill_get-current-time_get_info"),
        "Missing get_info tool. Found: {:?}",
        tool_names
    );
    // Run/read tools should NOT be visible before get_info
    assert!(
        !tool_names
            .iter()
            .any(|n| n.contains("_run_") || n.contains("_read_")),
        "Run/read tools should not appear before get_info. Found: {:?}",
        tool_names
    );

    // ── Step 2: skill_get-current-time_get_info ───────────────────
    let show_req = JsonRpcRequest::with_id(
        2,
        "tools/call".to_string(),
        Some(json!({
            "name": "skill_get-current-time_get_info",
            "arguments": {}
        })),
    );
    let response = gateway
        .handle_request_with_skills(
            client_id,
            vec![],
            false,
            vec![],
            skills_permissions.clone(),
            lr_config::FirewallRules::default(),
            "Test Client".to_string(),
            PermissionState::Off,
            show_req,
        )
        .await
        .expect("get_info should succeed");

    let result = response.result.expect("get_info should have a result");
    let content = result["content"]
        .as_array()
        .expect("get_info should have content array");
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
    // Verify new tool names are referenced
    assert!(
        text.contains("skill_get-current-time_run_get-time_js"),
        "get_info should reference run tool by new name"
    );
    assert!(
        text.contains("skill_get-current-time_read_notes_md"),
        "get_info should reference read tool by new name"
    );

    // ── Step 2b: tools/list (after get_info) ──────────────────────
    // Run/read tools should now be visible
    let tools_list_req2 = JsonRpcRequest::with_id(20, "tools/list".to_string(), Some(json!({})));
    let response = gateway
        .handle_request_with_skills(
            client_id,
            vec![],
            false,
            vec![],
            skills_permissions.clone(),
            lr_config::FirewallRules::default(),
            "Test Client".to_string(),
            PermissionState::Off,
            tools_list_req2,
        )
        .await
        .expect("tools/list after get_info should succeed");

    let result = response.result.expect("tools/list should have a result");
    let tools = result["tools"]
        .as_array()
        .expect("result should have tools array");
    let tool_names: Vec<&str> = tools.iter().filter_map(|t| t["name"].as_str()).collect();

    assert!(
        tool_names.contains(&"skill_get-current-time_get_info"),
        "get_info tool should still be present. Found: {:?}",
        tool_names
    );
    assert!(
        tool_names.contains(&"skill_get-current-time_run_get-time_js"),
        "Run tool should appear after get_info. Found: {:?}",
        tool_names
    );
    assert!(
        tool_names.contains(&"skill_get-current-time_read_notes_md"),
        "Read tool should appear after get_info. Found: {:?}",
        tool_names
    );
    assert!(
        tool_names.contains(&"skill_get-current-time_run_async_get-time_js"),
        "Async run tool should appear with async_enabled. Found: {:?}",
        tool_names
    );
    assert!(
        tool_names.contains(&"skill_get_async_status"),
        "Async status tool should appear with async_enabled. Found: {:?}",
        tool_names
    );

    // ── Step 3: skill_get-current-time_read_notes_md ──────────────
    let resource_req = JsonRpcRequest::with_id(
        3,
        "tools/call".to_string(),
        Some(json!({
            "name": "skill_get-current-time_read_notes_md",
            "arguments": {}
        })),
    );
    let response = gateway
        .handle_request_with_skills(
            client_id,
            vec![],
            false,
            vec![],
            skills_permissions.clone(),
            lr_config::FirewallRules::default(),
            "Test Client".to_string(),
            PermissionState::Off,
            resource_req,
        )
        .await
        .expect("read tool should succeed");

    let result = response.result.expect("read tool should have a result");
    let content = result["content"]
        .as_array()
        .expect("resource should have content array");
    let text = content[0]["text"]
        .as_str()
        .expect("content should have text");

    assert!(
        text.contains("Usage Notes"),
        "Resource content should contain 'Usage Notes'. Got: {}",
        text
    );
    assert!(
        text.contains("get-time.js"),
        "Resource content should mention the script"
    );

    // ── Step 4: skill_get-current-time_run_get-time_js (sync) ─────
    let run_sync_req = JsonRpcRequest::with_id(
        4,
        "tools/call".to_string(),
        Some(json!({
            "name": "skill_get-current-time_run_get-time_js",
            "arguments": {}
        })),
    );
    let response = gateway
        .handle_request_with_skills(
            client_id,
            vec![],
            false,
            vec![],
            skills_permissions.clone(),
            lr_config::FirewallRules::default(),
            "Test Client".to_string(),
            PermissionState::Off,
            run_sync_req,
        )
        .await
        .expect("run tool (sync) should succeed");

    let result = response.result.expect("run tool should have a result");

    assert_eq!(
        result["exit_code"].as_i64(),
        Some(0),
        "Script should exit with code 0"
    );
    assert_eq!(
        result["timed_out"].as_bool(),
        Some(false),
        "Script should not time out"
    );

    // Parse ISO date from stdout in the content text
    let content = result["content"]
        .as_array()
        .expect("run result should have content");
    let text = content[0]["text"]
        .as_str()
        .expect("content should have text");

    // Extract the ISO date line from stdout section
    let iso_date_str = text
        .lines()
        .find(|line| line.contains('T') && line.contains('Z'))
        .expect("Output should contain an ISO date string");
    let iso_date_str = iso_date_str.trim();

    let parsed = chrono::DateTime::parse_from_rfc3339(iso_date_str)
        .expect("Should parse as valid RFC3339 date");
    let now = chrono::Utc::now();
    let diff = (now - parsed.with_timezone(&chrono::Utc))
        .num_seconds()
        .abs();
    assert!(
        diff < 60,
        "Sync script date should be within 60s of now. Diff: {}s",
        diff
    );

    // ── Step 5: skill_get-current-time_run_async_get-time_js ──────
    let run_async_req = JsonRpcRequest::with_id(
        5,
        "tools/call".to_string(),
        Some(json!({
            "name": "skill_get-current-time_run_async_get-time_js",
            "arguments": {}
        })),
    );
    let response = gateway
        .handle_request_with_skills(
            client_id,
            vec![],
            false,
            vec![],
            skills_permissions.clone(),
            lr_config::FirewallRules::default(),
            "Test Client".to_string(),
            PermissionState::Off,
            run_async_req,
        )
        .await
        .expect("run_async tool should succeed");

    let result = response.result.expect("async run should have a result");

    let pid = result["pid"]
        .as_u64()
        .expect("Async result should contain a pid");
    assert!(pid > 0, "PID should be positive");

    // ── Step 6: skill_get_async_status (poll until done) ──────────
    let mut attempts = 0;
    let max_attempts = 20;
    let mut final_result = None;

    while attempts < max_attempts {
        tokio::time::sleep(std::time::Duration::from_millis(250)).await;
        attempts += 1;

        let poll_req = JsonRpcRequest::with_id(
            6,
            "tools/call".to_string(),
            Some(json!({
                "name": "skill_get_async_status",
                "arguments": {
                    "pid": pid
                }
            })),
        );
        let response = gateway
            .handle_request_with_skills(
                client_id,
                vec![],
                false,
                vec![],
                skills_permissions.clone(),
                lr_config::FirewallRules::default(),
                "Test Client".to_string(),
                PermissionState::Off,
                poll_req,
            )
            .await
            .expect("skill_get_async_status should succeed");

        let result = response.result.expect("poll should have a result");

        let running = result["running"]
            .as_bool()
            .expect("poll result should have 'running' field");

        if !running {
            final_result = Some(result);
            break;
        }
    }

    let final_result = final_result.expect("Async script should complete within polling window");

    assert_eq!(
        final_result["exit_code"].as_i64(),
        Some(0),
        "Async script should exit with code 0"
    );
    assert_eq!(
        final_result["timed_out"].as_bool(),
        Some(false),
        "Async script should not time out"
    );

    // Verify the async output also contains a valid date
    let content = final_result["content"]
        .as_array()
        .expect("poll result should have content");
    let text = content[0]["text"]
        .as_str()
        .expect("content should have text");

    let iso_date_str = text
        .lines()
        .find(|line| line.contains('T') && line.contains('Z'))
        .expect("Async output should contain an ISO date string");
    let iso_date_str = iso_date_str.trim();

    let parsed = chrono::DateTime::parse_from_rfc3339(iso_date_str)
        .expect("Should parse as valid RFC3339 date");
    let now = chrono::Utc::now();
    let diff = (now - parsed.with_timezone(&chrono::Utc))
        .num_seconds()
        .abs();
    assert!(
        diff < 60,
        "Async script date should be within 60s of now. Diff: {}s",
        diff
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

    let script_executor = Arc::new(ScriptExecutor::new());
    let server_manager = Arc::new(McpServerManager::new());
    let router = create_test_router();
    let gateway = McpGateway::new(server_manager, GatewayConfig::default(), router);
    gateway.set_skill_support(skill_manager, script_executor);

    (Arc::new(gateway), temp_dir)
}

/// Helper to set up a gateway with NO skills
async fn setup_gateway_without_skills() -> Arc<McpGateway> {
    let server_manager = Arc::new(McpServerManager::new());
    let router = create_test_router();
    let gateway = McpGateway::new(server_manager, GatewayConfig::default(), router);
    // Intentionally NOT calling set_skill_support
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
            vec![],
            false,
            vec![],
            SkillsPermissions::default(),
            lr_config::FirewallRules::default(),
            "Test Client".to_string(),
            PermissionState::Off,
            req,
        )
        .await
        .expect("tools/list should succeed");

    let tool_names = extract_tool_names(&response);

    // No skill tools should be present
    assert!(
        !tool_names.iter().any(|n| n.starts_with("skill_")),
        "skill_ tools should not appear without skills. Found: {:?}",
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
            vec![],
            false,
            vec![],
            skills_permissions.clone(),
            lr_config::FirewallRules::default(),
            "Test Client".to_string(),
            PermissionState::Off,
            req,
        )
        .await
        .expect("first tools/list should succeed");

    let names1 = extract_tool_names(&response1);
    assert!(
        names1.contains(&"skill_get-current-time_get_info".to_string()),
        "First call should include get_info tool. Found: {:?}",
        names1
    );

    // Second call: should hit cache but still include skill tools
    let req2 = JsonRpcRequest::with_id(2, "tools/list".to_string(), Some(json!({})));
    let response2 = gateway
        .handle_request_with_skills(
            client_id,
            vec![],
            false,
            vec![],
            skills_permissions.clone(),
            lr_config::FirewallRules::default(),
            "Test Client".to_string(),
            PermissionState::Off,
            req2,
        )
        .await
        .expect("second tools/list should succeed");

    let names2 = extract_tool_names(&response2);
    assert!(
        names2.contains(&"skill_get-current-time_get_info".to_string()),
        "Cached tools/list must still include get_info tool. Found: {:?}",
        names2
    );
}

/// Test: Skill get_info tools must be present even when deferred loading is enabled
#[tokio::test]
async fn test_skill_tools_present_with_deferred_loading() {
    let (gateway, _temp_dir) = setup_gateway_with_skill().await;

    let client_id = "deferred-test-client";
    let skills_permissions = {
        let mut perms = SkillsPermissions::default();
        perms
            .skills
            .insert("get-current-time".to_string(), PermissionState::Allow);
        perms
    };

    // First call to create the session and set allowed_skills
    let req = JsonRpcRequest::with_id(1, "tools/list".to_string(), Some(json!({})));
    gateway
        .handle_request_with_skills(
            client_id,
            vec![],
            false,
            vec![],
            skills_permissions.clone(),
            lr_config::FirewallRules::default(),
            "Test Client".to_string(),
            PermissionState::Off,
            req,
        )
        .await
        .expect("initial tools/list should succeed");

    // Manually enable deferred loading on the session
    let session = gateway
        .get_session(client_id)
        .expect("session should exist after first request");
    {
        let mut session_write = session.write().await;
        session_write.deferred_loading = Some(DeferredLoadingState {
            enabled: true,
            resources_deferred: false,
            prompts_deferred: false,
            activated_tools: HashSet::new(),
            full_catalog: vec![],
            activated_resources: HashSet::new(),
            full_resource_catalog: vec![],
            activated_prompts: HashSet::new(),
            full_prompt_catalog: vec![],
        });
    }

    // Now call tools/list again — deferred loading path should still include skill tools
    let req2 = JsonRpcRequest::with_id(2, "tools/list".to_string(), Some(json!({})));
    let response = gateway
        .handle_request_with_skills(
            client_id,
            vec![],
            false,
            vec![],
            skills_permissions.clone(),
            lr_config::FirewallRules::default(),
            "Test Client".to_string(),
            PermissionState::Off,
            req2,
        )
        .await
        .expect("deferred tools/list should succeed");

    let tool_names = extract_tool_names(&response);

    assert!(
        tool_names.contains(&"skill_get-current-time_get_info".to_string()),
        "Deferred loading must still include get_info tool. Found: {:?}",
        tool_names
    );
}

/// Test: Run/read tools should NOT be available before get_info is called
#[tokio::test]
async fn test_run_tool_blocked_before_get_info() {
    let (gateway, _temp_dir) = setup_gateway_with_skill().await;

    let client_id = "gate-test-client";
    let skills_permissions = {
        let mut perms = SkillsPermissions::default();
        perms
            .skills
            .insert("get-current-time".to_string(), PermissionState::Allow);
        perms
    };

    // Create session first
    let req = JsonRpcRequest::with_id(1, "tools/list".to_string(), Some(json!({})));
    gateway
        .handle_request_with_skills(
            client_id,
            vec![],
            false,
            vec![],
            skills_permissions.clone(),
            lr_config::FirewallRules::default(),
            "Test Client".to_string(),
            PermissionState::Off,
            req,
        )
        .await
        .expect("tools/list should succeed");

    // Try to call run tool before get_info
    let run_req = JsonRpcRequest::with_id(
        2,
        "tools/call".to_string(),
        Some(json!({
            "name": "skill_get-current-time_run_get-time_js",
            "arguments": {}
        })),
    );
    let response = gateway
        .handle_request_with_skills(
            client_id,
            vec![],
            false,
            vec![],
            skills_permissions.clone(),
            lr_config::FirewallRules::default(),
            "Test Client".to_string(),
            PermissionState::Off,
            run_req,
        )
        .await
        .expect("run tool call should return response (not panic)");

    let result = response.result.expect("should have result");
    // Should be an error response
    assert_eq!(
        result["isError"].as_bool(),
        Some(true),
        "Calling run tool before get_info should return an error. Got: {:?}",
        result
    );
    let error_text = result["content"][0]["text"].as_str().unwrap_or("");
    assert!(
        error_text.contains("get_info first"),
        "Error should mention calling get_info first. Got: {}",
        error_text
    );
}
