//! End-to-end integration test for the Coding Agents system via MCP gateway.
//!
//! Starts a real Claude Code process through the MCP gateway's `claude_code_start` tool,
//! polls `claude_code_status` until the session completes, and verifies output.
//!
//! Requires `claude` binary on PATH. Skipped automatically if not installed.

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

/// Poll `claude_code_status` until the session reaches a terminal state (done/error).
/// Returns the final status response.
async fn poll_until_done(
    gateway: &Arc<McpGateway>,
    client_id: &str,
    permissions: &CodingAgentsPermissions,
    session_id: &str,
    timeout_secs: u64,
) -> serde_json::Value {
    let timeout = std::time::Duration::from_secs(timeout_secs);
    let start_time = std::time::Instant::now();

    loop {
        tokio::time::sleep(std::time::Duration::from_secs(2)).await;

        let result = call_tool(
            gateway,
            client_id,
            permissions,
            "claude_code_status",
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

// ═══════════════════════════════════════════════════════════════════════════
// Tests
// ═══════════════════════════════════════════════════════════════════════════

/// Direct process spawn test — bypass the manager to verify claude binary produces output
#[tokio::test]
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
async fn test_coding_agents_e2e_claude_code_time_query() {
    if which::which("claude").is_err() {
        eprintln!("SKIP: `claude` binary not found on PATH");
        return;
    }

    let (gateway, permissions) = setup_gateway();
    let client_id = "test-time-query";
    initialize_gateway(&gateway, client_id, &permissions).await;

    // Start a session asking Claude Code for the time
    eprintln!("Starting Claude Code session...");
    let start_result = call_tool(
        &gateway,
        client_id,
        &permissions,
        "claude_code_start",
        json!({
            "prompt": "Tell me what time it is right now. Just respond with the current time, nothing else.",
            "permissionMode": "auto"
        }),
    )
    .await;

    let session_id = start_result["sessionId"]
        .as_str()
        .expect("start should return session_id");
    assert_eq!(
        start_result["status"].as_str().unwrap(),
        "active",
        "Session should be active"
    );
    eprintln!("Session started: {session_id}");

    // Poll until done (120s timeout — Claude Code needs to initialize)
    let final_status = poll_until_done(&gateway, client_id, &permissions, session_id, 120).await;

    eprintln!("Final status: {final_status}");
    assert_eq!(
        final_status["status"].as_str().unwrap(),
        "done",
        "Session should complete successfully. Result: {:?}, Output: {:?}",
        final_status["result"],
        final_status["recentOutput"]
    );

    // Verify we got some output
    let has_output = final_status["result"].as_str().map(|r| !r.is_empty()).unwrap_or(false)
        || final_status["recentOutput"]
            .as_array()
            .map(|a| !a.is_empty())
            .unwrap_or(false);
    assert!(has_output, "Session should produce output");

    eprintln!(
        "Result: {}",
        final_status["result"].as_str().unwrap_or("(none)")
    );
}

#[tokio::test]
async fn test_coding_agents_e2e_list_and_interrupt() {
    if which::which("claude").is_err() {
        eprintln!("SKIP: `claude` binary not found on PATH");
        return;
    }

    let (gateway, permissions) = setup_gateway();
    let client_id = "test-list-interrupt";
    initialize_gateway(&gateway, client_id, &permissions).await;

    // List sessions — should be empty
    let list_result = call_tool(
        &gateway,
        client_id,
        &permissions,
        "claude_code_list",
        json!({}),
    )
    .await;
    let sessions = list_result["sessions"].as_array().unwrap();
    assert!(sessions.is_empty(), "Should start with no sessions");

    // Start a session with a longer task so we can interrupt it
    let start_result = call_tool(
        &gateway,
        client_id,
        &permissions,
        "claude_code_start",
        json!({
            "prompt": "Write a very long essay about the history of computing. Make it at least 10000 words.",
            "permissionMode": "auto"
        }),
    )
    .await;
    let session_id = start_result["sessionId"].as_str().unwrap();
    eprintln!("Started session: {session_id}");

    // List sessions — should have one
    let list_result = call_tool(
        &gateway,
        client_id,
        &permissions,
        "claude_code_list",
        json!({}),
    )
    .await;
    let sessions = list_result["sessions"].as_array().unwrap();
    assert_eq!(sessions.len(), 1, "Should have one session");

    // Give it a moment to start running
    tokio::time::sleep(std::time::Duration::from_secs(3)).await;

    // Interrupt the session
    eprintln!("Interrupting session...");
    let interrupt_result = call_tool(
        &gateway,
        client_id,
        &permissions,
        "claude_code_interrupt",
        json!({ "sessionId": session_id }),
    )
    .await;
    eprintln!("Interrupt result: {interrupt_result}");

    // Check final status — should be interrupted or done
    tokio::time::sleep(std::time::Duration::from_secs(1)).await;
    let status_result = call_tool(
        &gateway,
        client_id,
        &permissions,
        "claude_code_status",
        json!({ "sessionId": session_id, "outputLines": 10 }),
    )
    .await;
    let status = status_result["status"].as_str().unwrap();
    assert!(
        status == "interrupted" || status == "done" || status == "error",
        "After interrupt, status should be terminal. Got: {status}"
    );
    eprintln!("Final status after interrupt: {status}");
}
