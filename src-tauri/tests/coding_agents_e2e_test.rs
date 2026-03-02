//! End-to-end integration tests for the Coding Agents system via MCP gateway.
//!
//! Starts real coding agent processes through the MCP gateway's `{prefix}_start` tool,
//! polls `{prefix}_status` until the session completes, and verifies output.
//!
//! All tests that spawn real processes are `#[ignore]` by default since they require
//! the respective agent binaries on PATH and are not run in CI.
//! Run explicitly with: `cargo test --test coding_agents_e2e_test -- --ignored`
//! Run a specific agent: `cargo test --test coding_agents_e2e_test test_e2e_claude_code -- --ignored`

mod mcp_tests;

use localrouter::config::AppConfig;
use localrouter::config::ConfigManager;
use localrouter::mcp::gateway::{GatewayConfig, McpGateway};
use localrouter::mcp::protocol::JsonRpcRequest;
use localrouter::mcp::McpServerManager;
use localrouter::monitoring::metrics::MetricsCollector;
use localrouter::monitoring::storage::MetricsDatabase;
use localrouter::providers::registry::ProviderRegistry;
use localrouter::router::{RateLimiterManager, Router};
use lr_coding_agents::manager::CodingAgentManager;
use lr_config::{CodingAgentsConfig, CodingAgentsPermissions, PermissionState};
use serde_json::json;
use std::sync::Arc;

fn create_test_router() -> Arc<Router> {
    let config = AppConfig::default();
    let config_manager = Arc::new(ConfigManager::new(
        config,
        std::path::PathBuf::from("/tmp/test_coding_agents_e2e_router.yaml"),
    ));

    let provider_registry = Arc::new(ProviderRegistry::new());
    let rate_limiter = Arc::new(RateLimiterManager::new(None));

    let metrics_db_path = std::env::temp_dir().join(format!(
        "test_coding_agents_e2e_metrics_{}.db",
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

/// Helper: send a JSON-RPC request through the gateway with coding agents enabled.
/// Returns the `result` field of the JSON-RPC response.
async fn gateway_request(
    gateway: &Arc<McpGateway>,
    client_id: &str,
    permissions: &CodingAgentsPermissions,
    request: JsonRpcRequest,
) -> serde_json::Value {
    let response = gateway
        .handle_request_with_skills(
            client_id,
            vec![],
            false,
            vec![],
            lr_config::McpPermissions::default(),
            lr_config::SkillsPermissions::default(),
            "E2E Test Client".to_string(),
            PermissionState::Off,
            permissions.clone(),
            request,
        )
        .await
        .expect("Gateway request should succeed");

    if let Some(ref err) = response.error {
        panic!(
            "JSON-RPC error: code={}, message={}, data={:?}",
            err.code, err.message, err.data
        );
    }

    response
        .result
        .expect("Response should have a result (not an error)")
}

fn setup_gateway() -> (Arc<McpGateway>, CodingAgentsPermissions) {
    let coding_agents_config = CodingAgentsConfig {
        agents: vec![],
        default_working_directory: Some(std::env::temp_dir().to_string_lossy().to_string()),
        max_concurrent_sessions: 5,
        output_buffer_size: 1000,
    };
    let manager = Arc::new(CodingAgentManager::new(coding_agents_config));

    let server_manager = Arc::new(McpServerManager::new());
    let router = create_test_router();
    let gateway = McpGateway::new(server_manager, GatewayConfig::default(), router);
    gateway.set_coding_agent_support(manager);

    let permissions = CodingAgentsPermissions {
        global: PermissionState::Allow,
        agents: Default::default(),
    };

    (Arc::new(gateway), permissions)
}

async fn initialize_gateway(
    gateway: &Arc<McpGateway>,
    client_id: &str,
    permissions: &CodingAgentsPermissions,
) {
    let init_req = JsonRpcRequest::with_id(
        1,
        "initialize".to_string(),
        Some(json!({
            "protocolVersion": "2024-11-05",
            "capabilities": {},
            "clientInfo": { "name": "e2e-test", "version": "1.0.0" }
        })),
    );
    let init_result = gateway_request(gateway, client_id, permissions, init_req).await;
    assert!(
        init_result.get("protocolVersion").is_some(),
        "Initialize should return protocolVersion, got: {init_result}"
    );
}

/// Call a coding agent tool and return the raw result JSON.
/// The gateway returns tool results directly as JSON (not wrapped in MCP content array).
async fn call_tool(
    gateway: &Arc<McpGateway>,
    client_id: &str,
    permissions: &CodingAgentsPermissions,
    tool_name: &str,
    arguments: serde_json::Value,
) -> serde_json::Value {
    let req = JsonRpcRequest::with_id(
        10,
        "tools/call".to_string(),
        Some(json!({
            "name": tool_name,
            "arguments": arguments
        })),
    );
    gateway_request(gateway, client_id, permissions, req).await
}

/// Poll `{tool_prefix}_status` until the session reaches a terminal state (done/error).
/// Returns the final status response.
async fn poll_until_done(
    gateway: &Arc<McpGateway>,
    client_id: &str,
    permissions: &CodingAgentsPermissions,
    tool_prefix: &str,
    session_id: &str,
    timeout_secs: u64,
) -> serde_json::Value {
    let status_tool = format!("{tool_prefix}_status");
    let timeout = std::time::Duration::from_secs(timeout_secs);
    let start_time = std::time::Instant::now();

    loop {
        tokio::time::sleep(std::time::Duration::from_secs(2)).await;

        let result = call_tool(
            gateway,
            client_id,
            permissions,
            &status_tool,
            json!({ "sessionId": session_id, "outputLines": 100 }),
        )
        .await;

        let status = result["status"].as_str().unwrap_or("unknown");
        eprintln!("  poll: status={status}");

        if let Some(output) = result["recentOutput"].as_array() {
            for line in output.iter().rev().take(3).collect::<Vec<_>>().into_iter().rev() {
                if let Some(l) = line.as_str() {
                    if !l.is_empty() {
                        eprintln!("    | {}", &l[..l.len().min(120)]);
                    }
                }
            }
        }

        match status {
            "done" | "error" => return result,
            _ => {}
        }

        assert!(
            start_time.elapsed() < timeout,
            "Timed out after {timeout_secs}s waiting for session to complete"
        );
    }
}

/// Run a full E2E time query test for any coding agent.
/// Starts a session, polls until completion, and verifies output.
async fn run_agent_e2e_time_query(binary_name: &str, tool_prefix: &str, display_name: &str) {
    if which::which(binary_name).is_err() {
        eprintln!("SKIP: `{binary_name}` binary not found on PATH");
        return;
    }

    let (gateway, permissions) = setup_gateway();
    let client_id = &format!("test-{tool_prefix}-time");
    initialize_gateway(&gateway, client_id, &permissions).await;

    let start_tool = format!("{tool_prefix}_start");

    eprintln!("Starting {display_name} session...");
    let start_result = call_tool(
        &gateway,
        client_id,
        &permissions,
        &start_tool,
        json!({
            "prompt": "Tell me what time it is right now. Just respond with the current time, nothing else.",
            "permissionMode": "auto"
        }),
    )
    .await;

    let session_id = start_result["sessionId"]
        .as_str()
        .unwrap_or_else(|| panic!("{display_name} start should return sessionId, got: {start_result}"));
    assert_eq!(
        start_result["status"].as_str().unwrap(),
        "active",
        "{display_name} session should be active"
    );
    eprintln!("{display_name} session started: {session_id}");

    // Poll until done (120s timeout)
    let final_status = poll_until_done(&gateway, client_id, &permissions, tool_prefix, session_id, 120).await;

    eprintln!("{display_name} final status: {final_status}");
    assert_eq!(
        final_status["status"].as_str().unwrap(),
        "done",
        "{display_name} session should complete successfully. Result: {:?}, Output: {:?}",
        final_status["result"],
        final_status["recentOutput"]
    );

    let has_output = final_status["result"].as_str().map(|r| !r.is_empty()).unwrap_or(false)
        || final_status["recentOutput"]
            .as_array()
            .map(|a| !a.is_empty())
            .unwrap_or(false);
    assert!(has_output, "{display_name} session should produce output");

    eprintln!(
        "{display_name} result: {}",
        final_status["result"].as_str().unwrap_or("(none)")
    );
}

/// Run a list + interrupt test for any coding agent.
async fn run_agent_e2e_list_and_interrupt(binary_name: &str, tool_prefix: &str, display_name: &str) {
    if which::which(binary_name).is_err() {
        eprintln!("SKIP: `{binary_name}` binary not found on PATH");
        return;
    }

    let (gateway, permissions) = setup_gateway();
    let client_id = &format!("test-{tool_prefix}-interrupt");
    initialize_gateway(&gateway, client_id, &permissions).await;

    let start_tool = format!("{tool_prefix}_start");
    let list_tool = format!("{tool_prefix}_list");
    let interrupt_tool = format!("{tool_prefix}_interrupt");
    let status_tool = format!("{tool_prefix}_status");

    // List sessions — should be empty
    let list_result = call_tool(&gateway, client_id, &permissions, &list_tool, json!({})).await;
    let sessions = list_result["sessions"].as_array().unwrap();
    assert!(sessions.is_empty(), "{display_name}: should start with no sessions");

    // Start a session with a longer task so we can interrupt it
    let start_result = call_tool(
        &gateway,
        client_id,
        &permissions,
        &start_tool,
        json!({
            "prompt": "Write a very long essay about the history of computing. Make it at least 10000 words.",
            "permissionMode": "auto"
        }),
    )
    .await;
    let session_id = start_result["sessionId"].as_str().unwrap();
    eprintln!("{display_name} session started: {session_id}");

    // List sessions — should have one
    let list_result = call_tool(&gateway, client_id, &permissions, &list_tool, json!({})).await;
    let sessions = list_result["sessions"].as_array().unwrap();
    assert_eq!(sessions.len(), 1, "{display_name}: should have one session");

    // Give it a moment to start running
    tokio::time::sleep(std::time::Duration::from_secs(3)).await;

    // Interrupt the session
    eprintln!("Interrupting {display_name} session...");
    let interrupt_result = call_tool(
        &gateway,
        client_id,
        &permissions,
        &interrupt_tool,
        json!({ "sessionId": session_id }),
    )
    .await;
    eprintln!("{display_name} interrupt result: {interrupt_result}");

    // Check final status
    tokio::time::sleep(std::time::Duration::from_secs(1)).await;
    let status_result = call_tool(
        &gateway,
        client_id,
        &permissions,
        &status_tool,
        json!({ "sessionId": session_id, "outputLines": 10 }),
    )
    .await;
    let status = status_result["status"].as_str().unwrap();
    assert!(
        status == "interrupted" || status == "done" || status == "error",
        "{display_name}: after interrupt, status should be terminal. Got: {status}"
    );
    eprintln!("{display_name} final status after interrupt: {status}");
}

// ═══════════════════════════════════════════════════════════════════════════
// Tests
// ═══════════════════════════════════════════════════════════════════════════

/// Direct process spawn test — bypass the manager to verify claude binary produces output
#[tokio::test]
#[ignore]
async fn test_coding_agents_e2e_raw_claude_process() {
    if which::which("claude").is_err() {
        eprintln!("SKIP: `claude` binary not found on PATH");
        return;
    }

    use tokio::io::AsyncBufReadExt;
    use std::process::Stdio;

    let mut cmd = tokio::process::Command::new("claude");
    cmd.current_dir(std::env::temp_dir());
    cmd.arg("-p").arg("What is 2+2? Just say the number.");
    cmd.arg("--output-format").arg("stream-json");
    cmd.arg("--dangerously-skip-permissions");
    cmd.env_remove("CLAUDECODE");
    cmd.env_remove("CLAUDE_CODE_ENTRYPOINT");
    cmd.env_remove("CLAUDE_CODE_SESSION_ACCESS_TOKEN");
    cmd.stdin(Stdio::null());
    cmd.stdout(Stdio::piped());
    cmd.stderr(Stdio::null());

    let mut child = cmd.spawn().expect("Failed to spawn claude");
    let stdout = child.stdout.take().expect("stdout should be available");
    let reader = tokio::io::BufReader::new(stdout);
    let mut lines = reader.lines();

    let mut output_lines = Vec::new();
    let timeout = tokio::time::timeout(std::time::Duration::from_secs(60), async {
        while let Ok(Some(line)) = lines.next_line().await {
            eprintln!("  raw | {}", &line[..line.len().min(100)]);
            output_lines.push(line.clone());
            // Stop after result line
            if line.contains("\"type\":\"result\"") {
                break;
            }
        }
    });

    timeout.await.expect("Raw claude process should complete within 60s");

    assert!(!output_lines.is_empty(), "Should have received output lines");
    assert!(
        output_lines.iter().any(|l| l.contains("\"type\":\"result\"")),
        "Should have received a result line"
    );
    eprintln!("Got {} output lines from raw claude process", output_lines.len());

    child.kill().await.ok();
}

#[tokio::test]
#[ignore]
async fn test_coding_agents_e2e_tools_list() {
    if which::which("claude").is_err() {
        eprintln!("SKIP: `claude` binary not found on PATH");
        return;
    }

    let (gateway, permissions) = setup_gateway();
    let client_id = "test-tools-list";
    initialize_gateway(&gateway, client_id, &permissions).await;

    // tools/list should include all 6 claude_code_* tools
    let list_req = JsonRpcRequest::with_id(2, "tools/list".to_string(), Some(json!({})));
    let list_result = gateway_request(&gateway, client_id, &permissions, list_req).await;
    let tools = list_result["tools"]
        .as_array()
        .expect("tools/list should return tools array");
    let tool_names: Vec<&str> = tools.iter().filter_map(|t| t["name"].as_str()).collect();

    let expected = [
        "claude_code_start",
        "claude_code_say",
        "claude_code_status",
        "claude_code_respond",
        "claude_code_interrupt",
        "claude_code_list",
    ];
    for name in &expected {
        assert!(
            tool_names.contains(name),
            "Missing tool: {name}. Found: {tool_names:?}"
        );
    }
}

#[tokio::test]
#[ignore]
async fn test_e2e_claude_code_time_query() {
    run_agent_e2e_time_query("claude", "claude_code", "Claude Code").await;
}

#[tokio::test]
#[ignore]
async fn test_e2e_claude_code_list_and_interrupt() {
    run_agent_e2e_list_and_interrupt("claude", "claude_code", "Claude Code").await;
}

// ═══════════════════════════════════════════════════════════════════════════
// Per-agent E2E tests
// ═══════════════════════════════════════════════════════════════════════════
// Each agent has two tests: time_query (start → poll → done) and
// list_and_interrupt (list → start → list → interrupt → verify).
// All are #[ignore] — run with: cargo test --test coding_agents_e2e_test -- --ignored

// --- Gemini CLI ---

#[tokio::test]
#[ignore]
async fn test_e2e_gemini_cli_time_query() {
    run_agent_e2e_time_query("gemini", "gemini_cli", "Gemini CLI").await;
}

#[tokio::test]
#[ignore]
async fn test_e2e_gemini_cli_list_and_interrupt() {
    run_agent_e2e_list_and_interrupt("gemini", "gemini_cli", "Gemini CLI").await;
}

// --- Codex ---

#[tokio::test]
#[ignore]
async fn test_e2e_codex_time_query() {
    run_agent_e2e_time_query("codex", "codex", "Codex").await;
}

#[tokio::test]
#[ignore]
async fn test_e2e_codex_list_and_interrupt() {
    run_agent_e2e_list_and_interrupt("codex", "codex", "Codex").await;
}

// --- Amp ---

#[tokio::test]
#[ignore]
async fn test_e2e_amp_time_query() {
    run_agent_e2e_time_query("amp", "amp", "Amp").await;
}

#[tokio::test]
#[ignore]
async fn test_e2e_amp_list_and_interrupt() {
    run_agent_e2e_list_and_interrupt("amp", "amp", "Amp").await;
}

// --- Aider ---

#[tokio::test]
#[ignore]
async fn test_e2e_aider_time_query() {
    run_agent_e2e_time_query("aider", "aider", "Aider").await;
}

#[tokio::test]
#[ignore]
async fn test_e2e_aider_list_and_interrupt() {
    run_agent_e2e_list_and_interrupt("aider", "aider", "Aider").await;
}

// --- Cursor ---

#[tokio::test]
#[ignore]
async fn test_e2e_cursor_time_query() {
    run_agent_e2e_time_query("cursor", "cursor", "Cursor").await;
}

#[tokio::test]
#[ignore]
async fn test_e2e_cursor_list_and_interrupt() {
    run_agent_e2e_list_and_interrupt("cursor", "cursor", "Cursor").await;
}

// --- Opencode ---

#[tokio::test]
#[ignore]
async fn test_e2e_opencode_time_query() {
    run_agent_e2e_time_query("opencode", "opencode", "Opencode").await;
}

#[tokio::test]
#[ignore]
async fn test_e2e_opencode_list_and_interrupt() {
    run_agent_e2e_list_and_interrupt("opencode", "opencode", "Opencode").await;
}

// --- Qwen Code ---

#[tokio::test]
#[ignore]
async fn test_e2e_qwen_code_time_query() {
    run_agent_e2e_time_query("qwen", "qwen_code", "Qwen Code").await;
}

#[tokio::test]
#[ignore]
async fn test_e2e_qwen_code_list_and_interrupt() {
    run_agent_e2e_list_and_interrupt("qwen", "qwen_code", "Qwen Code").await;
}

// --- Copilot ---

#[tokio::test]
#[ignore]
async fn test_e2e_copilot_time_query() {
    run_agent_e2e_time_query("copilot", "copilot", "Copilot").await;
}

#[tokio::test]
#[ignore]
async fn test_e2e_copilot_list_and_interrupt() {
    run_agent_e2e_list_and_interrupt("copilot", "copilot", "Copilot").await;
}

// --- Droid ---

#[tokio::test]
#[ignore]
async fn test_e2e_droid_time_query() {
    run_agent_e2e_time_query("droid", "droid", "Droid").await;
}

#[tokio::test]
#[ignore]
async fn test_e2e_droid_list_and_interrupt() {
    run_agent_e2e_list_and_interrupt("droid", "droid", "Droid").await;
}
