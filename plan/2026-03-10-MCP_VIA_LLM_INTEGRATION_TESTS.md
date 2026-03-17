# MCP via LLM Integration Tests

## Context

The MCP via LLM orchestrator (`crates/lr-mcp-via-llm/`) runs an agentic loop: call LLM → inspect tool calls → execute MCP tools → re-call LLM → repeat. Existing tests (875 lines in `tests.rs`) cover unit-level concerns: tool injection, classification, sessions, streaming accumulation. What's missing are **integration tests** that exercise the full `run_agentic_loop` and `resume_after_mixed` codepaths with mocked LLM + MCP dependencies, testing the real orchestrator logic end-to-end.

## Approach

Create a new test file `crates/lr-mcp-via-llm/src/integration_tests.rs` with mock implementations of the LLM provider and MCP server, wired through real `Router` and `McpGateway` instances.

### Mocking Strategy

Both `Router` and `McpGateway` are concrete types. We mock at their internal boundaries:

1. **MockLlmProvider** — implements `ModelProvider` trait (`#[async_trait]`), returns scripted `CompletionResponse` sequences. Router routes to it via `client_id = "internal-test"` debug bypass (skips all routing config).

2. **MockMcpVirtualServer** — implements `VirtualMcpServer` trait, registered on gateway via `register_virtual_server()`. Returns scripted tool lists and results. Auto-approves all firewall checks.

## Files to Create/Modify

### New: `crates/lr-mcp-via-llm/src/integration_tests.rs`
All integration test code — mocks, helpers, test scenarios.

### Modify: `crates/lr-mcp-via-llm/src/lib.rs`
Add `#[cfg(test)] mod integration_tests;`

### Modify: `crates/lr-mcp-via-llm/Cargo.toml`
Add dev-dependencies:
```toml
[dev-dependencies]
tokio = { workspace = true, features = ["rt-multi-thread", "macros", "time"] }
async-trait = { workspace = true }
lr-monitoring = { workspace = true }
```

## Mock Types

### MockLlmProvider
```rust
struct MockLlmProvider {
    responses: Arc<Mutex<VecDeque<CompletionResponse>>>,
    requests_received: Arc<Mutex<Vec<CompletionRequest>>>,
}
```
- `name()` → `"mock"`
- `complete()` → pops next response from queue, records incoming request
- `list_models()` → `vec![ModelInfo { id: "test-model", ... }]`
- Other methods: minimal stubs

### MockProviderFactory
- `provider_type()` → `"mock"`
- `create()` → returns shared `Arc<MockLlmProvider>`

### MockMcpVirtualServer
```rust
struct MockMcpVirtualServer {
    server_id: String,
    tools: Vec<McpTool>,
    tool_results: Arc<Mutex<HashMap<String, Value>>>,
    calls_received: Arc<Mutex<Vec<(String, Value)>>>,
}
```
- `id()` → `&self.server_id`
- `owns_tool()` → checks name in tool list
- `list_tools()` → `self.tools.clone()`
- `handle_tool_call()` → records `(tool_name, arguments)` to `calls_received`, returns `Success(result)` from map
- `check_permissions()` → `Handled(FirewallDecisionResult::Proceed)`
- `is_enabled()` → `true`

Multiple mock servers can be registered on the same gateway. Each has its own `server_id`, tool list, and `calls_received` tracker. This allows tests to verify:
- Which server received which tool call
- Tools from different servers called in the same iteration
- Correct argument passthrough per server

### MockSessionState
```rust
struct MockSessionState;
// Implements VirtualSessionState with trivial as_any/clone_box
```

### TestEnv setup function
```rust
struct TestEnv {
    gateway: Arc<McpGateway>,
    router: Arc<Router>,
    client: Client,
    mock_provider: Arc<MockLlmProvider>,
    mock_servers: Vec<Arc<MockMcpVirtualServer>>,  // multiple mock servers
}

// Single-server convenience setup
async fn setup_test_env(
    llm_responses: Vec<CompletionResponse>,
    mcp_tools: Vec<McpTool>,
    tool_results: HashMap<String, Value>,
) -> TestEnv { ... }

// Multi-server setup — each entry is (server_id, tools, tool_results)
async fn setup_test_env_multi(
    llm_responses: Vec<CompletionResponse>,
    servers: Vec<(&str, Vec<McpTool>, HashMap<String, Value>)>,
) -> TestEnv { ... }
```

Construction chain:
1. `ConfigManager::new(AppConfig::default(), PathBuf::from("/tmp/lr-test.yaml"))`
2. `ProviderRegistry::new()` → register `MockProviderFactory` → `create_provider("mock", "mock", {})`
3. `RateLimiterManager::new(None)`
4. `MetricsDatabase::new(temp_path)` → `MetricsCollector::new(db)`
5. `Router::new(config_manager, registry, rate_limiter, metrics, FreeTierManager::new(None))`
6. `McpServerManager::new_for_test()` → `McpGateway::new(manager, GatewayConfig::default(), router_arc)`
7. For each mock server: `gateway.register_virtual_server(mock_mcp_server)`
8. `Client::new_with_strategy(...)` with `id = "internal-test"`, `client_mode = ClientMode::McpViaLlm`

### Helper functions
- `make_response(text, tool_calls)` → `CompletionResponse`
- `make_tool_call(id, name, arguments)` → `ToolCall`
- `make_mcp_tool(name, description)` → `McpTool`
- `make_request(user_message)` → `CompletionRequest` with `model: "mock/test-model"`
- `make_config()` → `McpViaLlmConfig` with defaults

## Test Scenarios

### Module: `agentic_loop_tests`

| # | Test | Description |
|---|------|-------------|
| 1 | `passthrough_no_mcp_tools` | Empty tool list → LLM called once → response passed through |
| 2 | `single_mcp_tool_iteration` | LLM calls 1 MCP tool → executed → LLM called again → final text |
| 3 | `multiple_mcp_tools_in_one_turn` | LLM calls 3 MCP tools simultaneously → all executed → next iteration |
| 4 | `client_only_tools_returned_directly` | LLM calls only client tools → returned without MCP execution |
| 5 | `tool_injection_verified_in_request` | Assert MCP tools appear in the request sent to LLM |

### Module: `mcp_server_verification_tests`

| # | Test | Description |
|---|------|-------------|
| 6 | `verify_mcp_server_receives_tool_call` | Single server: assert `calls_received` contains exact tool name + arguments after loop |
| 7 | `multiple_mcp_servers_different_tools` | 2 mock servers (filesystem, database) with different tools. LLM calls one tool from each. Assert each server's `calls_received` has exactly 1 entry for the correct tool |
| 8 | `mcp_server_call_order_matches_llm_response` | LLM returns 3 tool calls in order A, B, C. Assert `calls_received` records them in the same order |
| 9 | `mcp_server_receives_correct_arguments` | LLM passes `{"path": "/tmp/test", "encoding": "utf-8"}` as arguments. Assert the mock server received those exact arguments |

### Module: `mixed_tool_tests`

| # | Test | Description |
|---|------|-------------|
| 10 | `mixed_mcp_and_client_tools` | Both MCP + client tool calls → PendingMixed returned, MCP in background. Assert mock server's `calls_received` has MCP tool entries |
| 11 | `resume_after_mixed_execution` | Full flow: PendingMixed → await background → resume with client results → complete. Verify mock servers received all expected calls |

### Module: `guardrail_tests`

| # | Test | Description |
|---|------|-------------|
| 12 | `guardrail_pass` | Gate resolves Ok → response returned normally |
| 13 | `guardrail_deny` | Gate resolves Err → GuardrailDenied error, no tool execution. Assert mock server `calls_received` is empty |
| 14 | `guardrail_checked_once_across_iterations` | Multi-iteration: guardrail gate consumed after first LLM call |

### Module: `error_handling_tests`

| # | Test | Description |
|---|------|-------------|
| 15 | `max_iterations_limit` | LLM always returns tools → MaxIterations error after limit. Assert mock server received calls for all iterations |
| 16 | `tool_error_fed_back_to_llm` | Mock server returns `ToolError("disk full")` → error message as tool result → LLM retries |
| 17 | `malformed_tool_arguments` | Invalid JSON args → parse error fed to LLM → loop continues. Assert mock server `calls_received` is empty (never reached) |

### Module: `metadata_tests`

| # | Test | Description |
|---|------|-------------|
| 18 | `token_aggregation` | 2 iterations → usage tokens summed across iterations |
| 19 | `mcp_via_llm_extension_metadata` | Assert `extensions.mcp_via_llm` contains iterations, tools_called, tokens |

### Module: `session_tests`

| # | Test | Description |
|---|------|-------------|
| 20 | `session_history_tracking` | Multi-iteration → session history includes all messages |
| 21 | `gateway_initialized_flag` | Session's `gateway_initialized` set to true after first call |

## Key Implementation Details

### Router "internal-test" bypass
`crates/lr-router/src/lib.rs:1420-1432` — in debug builds, `client_id == "internal-test"` bypasses all routing config. Model must be `"provider/model"` format (e.g., `"mock/test-model"`). Router calls `execute_request("internal-test", "mock", "test-model", request)` which calls `registry.get_provider("mock")` → `provider.complete(request)`.

### Virtual server tool routing
`crates/lr-mcp/src/gateway/gateway_tools.rs:200-211` — `handle_tools_call` checks `vs.owns_tool(&tool_name)` for each virtual server. If matched, dispatches to `dispatch_virtual_tool_call()` which calls `vs.check_permissions()` then `vs.handle_tool_call()`.

### Virtual server tool listing
`crates/lr-mcp/src/gateway/gateway_tools.rs:58` — `handle_tools_list` calls `append_virtual_server_tools()` which iterates virtual servers and calls `vs.list_tools(state)`. Virtual server state is created per-session via `vs.create_session_state(client)`.

### Firewall auto-approve
When `check_permissions()` returns `VirtualFirewallResult::Handled(FirewallDecisionResult::Proceed)`, the gateway skips the firewall popup flow entirely.

## Reusable Code References

- `Client::new_with_strategy()` — `crates/lr-config/src/types.rs:3064`
- `McpServerManager::new_for_test()` — `crates/lr-mcp/src/manager.rs`
- `Router::new()` — `crates/lr-router/src/lib.rs:312`
- `McpGateway::new()` — `crates/lr-mcp/src/gateway/gateway.rs:65`
- `VirtualMcpServer` trait — `crates/lr-mcp/src/gateway/virtual_server.rs:20`
- `VirtualSessionState` trait — `crates/lr-mcp/src/gateway/virtual_server.rs:80`
- `ModelProvider` trait — `crates/lr-providers/src/lib.rs:61`
- `ProviderFactory` trait — `crates/lr-providers/src/factory.rs:54`
- `ProviderRegistry::register_factory()` — `crates/lr-providers/src/registry.rs:310`
- `ProviderRegistry::create_provider()` — `crates/lr-providers/src/registry.rs:372`
- `run_agentic_loop()` — `crates/lr-mcp-via-llm/src/orchestrator.rs:83`
- `resume_after_mixed()` — `crates/lr-mcp-via-llm/src/orchestrator.rs:558`
- Existing test helpers in `crates/lr-mcp-via-llm/src/tests.rs` (msg, tc, make_request, make_mcp_tool patterns)

## Verification

```bash
# Run all mcp-via-llm tests (unit + integration)
cargo test -p lr-mcp-via-llm

# Run only integration tests
cargo test -p lr-mcp-via-llm integration_tests

# Run a specific test
cargo test -p lr-mcp-via-llm integration_tests::agentic_loop_tests::single_mcp_tool_iteration

# Ensure no clippy warnings
cargo clippy -p lr-mcp-via-llm

# Verify compilation of full workspace still passes
cargo check
```
