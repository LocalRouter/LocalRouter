# Plan: Skills E2E Integration Test

## Summary

Create an end-to-end integration test that sets up a "get current time" skill with a JavaScript script, stands up the MCP gateway, and exercises all skill tool commands through the gateway's JSON-RPC interface.

---

## New file

`src-tauri/tests/skills_e2e_test.rs`

## Test Skill Structure (created in temp dir)

```
get-current-time/
├── SKILL.md
│   frontmatter: name: get-current-time, description: "Get the current date and time", tags: [time, utility]
│   body: markdown notes about the skill
├── scripts/
│   └── get-time.js       # console.log(new Date().toISOString())
└── references/
    └── notes.md          # Usage notes
```

## Component Setup

Reuse `create_test_router()` pattern from `src-tauri/tests/mcp_gateway_mock_integration_tests.rs`:
1. `SkillManager::new()` → `initial_scan(&[], &[skill_dir_path])`
2. `ScriptExecutor::new()`
3. `McpServerManager::new()`
4. `create_test_router()` → `Router`
5. `McpGateway::new(server_manager, GatewayConfig::default(), router)` → `set_skill_support(skill_manager, script_executor)`

## Test Cases (single `#[tokio::test]` with sequential steps)

1. **tools/list** — `handle_request_with_skills(client_id, [], false, [], ["get-current-time"], tools_list_request)` → verify response contains tools: `show-skill_get-current-time`, `get-skill-resource`, `run-skill-script`, `get-skill-script-run`

2. **show-skill** — Call `tools/call` with `name: "show-skill_get-current-time"` → verify response text contains skill description, file listings, body content

3. **get-skill-resource** — Call `tools/call` with `name: "get-skill-resource"`, args `{skill_name: "get-current-time", resource: "references/notes.md"}` → verify returns notes.md content

4. **run-skill-script (sync)** — Call `tools/call` with `name: "run-skill-script"`, args `{skill_name: "get-current-time", script: "scripts/get-time.js"}` → verify exit_code=0, parse ISO date from stdout, assert within ±60s of now

5. **run-skill-script (async)** — Call `tools/call` with `name: "run-skill-script"`, args `{..., async: true}` → verify response has `pid` field

6. **get-skill-script-run** — Call `tools/call` with `name: "get-skill-script-run"`, args `{pid: <from step 5>}`. Poll with short sleep until `running: false`. Verify exit_code=0 and date output within ±60s of now

## Key Implementation Details

- Uses `handle_request_with_skills()` directly (no HTTP server)
- JSON-RPC requests built via `JsonRpcRequest::with_id()`
- Responses parsed as `serde_json::Value` to check tool lists and call results
- `chrono::Utc::now()` for date comparison (already a dependency)
- Requires `node` in PATH (CI should have it)
- Single test function to avoid repeated setup and ensure sequential ordering

## Verification

```bash
cargo test --test skills_e2e_test -- --nocapture
```
