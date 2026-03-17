# Inject Unified MCP Gateway Instructions into LLM Prompt

## Context

MCP servers return `instructions` and `serverInfo.description` during initialization. The gateway already builds a comprehensive unified instructions document via `build_gateway_instructions()` (in `merger.rs`) that describes all servers, their tools, resources, prompts, descriptions, and instructions in a structured format. This is returned in the `initialize` JSON-RPC response's `instructions` field.

Currently, the MCP via LLM orchestrator discards this — the `GatewayClient::initialize()` returns `()`. We need to extract the unified instructions and inject them as a **system message** before the first non-system message in the request, so the LLM knows how to use the MCP tools.

## Changes

### 1. `crates/lr-mcp-via-llm/src/gateway_client.rs` — Return instructions from initialize

Change `initialize()` return type from `Result<(), McpViaLlmError>` to `Result<Option<String>, McpViaLlmError>`.

Extract `instructions` from the response JSON result:
```rust
let instructions = response.result
    .as_ref()
    .and_then(|r| r.get("instructions"))
    .and_then(|v| v.as_str())
    .map(|s| s.to_string());
// ... send initialized notification ...
Ok(instructions)
```

### 2. `crates/lr-mcp-via-llm/src/orchestrator.rs` — Add injection function + call it

Add `inject_server_instructions()`:
```rust
pub(crate) fn inject_server_instructions(request: &mut CompletionRequest, instructions: &str) {
    // Find first non-system message
    let insert_idx = request.messages.iter()
        .position(|m| m.role != "system")
        .unwrap_or(request.messages.len());

    request.messages.insert(insert_idx, ChatMessage {
        role: "system".to_string(),
        content: ChatMessageContent::Text(instructions.to_string()),
        tool_calls: None,
        tool_call_id: None,
        name: None,
    });
}
```

Call it in `run_agentic_loop()` after initialization (~line 109-112):
```rust
if !gateway_initialized {
    let instructions = gw_client.initialize().await?;
    session.write().gateway_initialized = true;
    if let Some(instructions) = instructions {
        inject_server_instructions(&mut request, &instructions);
    }
}
```

### 3. `crates/lr-mcp-via-llm/src/orchestrator_stream.rs` — Same call in streaming path

Same pattern after `gw_client.initialize()` (~line 63-66):
```rust
if !gateway_initialized {
    let instructions = gw_client.initialize().await?;
    session.write().gateway_initialized = true;
    if let Some(instructions) = instructions {
        orchestrator::inject_server_instructions(&mut request, &instructions);
    }
}
```

### 4. `crates/lr-mcp-via-llm/src/tests.rs` — Unit test

Add test for `inject_server_instructions()`:
- Verify it inserts system message after existing system messages but before user message
- Verify the content matches the instructions string

### 5. No config flag needed

Always inject — this is core MCP functionality, not optional. The gateway already returns `None` when there are no servers, so no injection happens in that case.

## Files Modified

| File | Change |
|------|--------|
| `crates/lr-mcp-via-llm/src/gateway_client.rs` | `initialize()` returns `Option<String>` |
| `crates/lr-mcp-via-llm/src/orchestrator.rs` | Add `inject_server_instructions()`, call after init |
| `crates/lr-mcp-via-llm/src/orchestrator_stream.rs` | Call `inject_server_instructions()` after init |
| `crates/lr-mcp-via-llm/src/tests.rs` | Unit test for injection |

## Verification

1. `cargo test -p lr-mcp-via-llm` — tests pass
2. `cargo clippy` — no warnings
3. `cargo build` — compiles
