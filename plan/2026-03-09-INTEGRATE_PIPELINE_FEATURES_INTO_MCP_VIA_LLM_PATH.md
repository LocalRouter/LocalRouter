# Plan: Integrate Pipeline Features into MCP via LLM Path

## Context

When an LLM request arrives at `chat_completions`, several features are spawned in parallel: guardrails, prompt compression, and RouteLLM classification. The MCP via LLM path (`McpViaLlm` client mode) returns early at line 212 **before** any of these features run, bypassing:

1. **Guardrails** — no safety scanning on input
2. **Prompt compression** — no token reduction
3. **RouteLLM** — `localrouter/auto` model routing broken
4. **JSON repair** — no streaming JSON repair
5. **Streaming generation tracking** — streaming responses not recorded

The fix: move the MCP via LLM interception downstream so these features apply using the **same code** as the normal path.

## File to Modify

`crates/lr-server/src/routes/chat.rs` — all changes are in this single file.

## Changes

### 1. Move MCP via LLM interception point (lines 208-215 → after line 322)

**Current code (lines 208-215):**
```rust
// Enforce client mode: block MCP-only clients from LLM endpoints
// Also intercept MCP-via-LLM clients for agentic orchestration
if let Ok((ref client, _)) = get_client_with_strategy(&state, &auth.api_key_id) {
    check_llm_access(client)?;
    if client.client_mode == lr_config::ClientMode::McpViaLlm {
        return handle_mcp_via_llm(state, auth, client_auth, request).await;
    }
}
```

**After:** Keep `check_llm_access` at its current location (reject MCP-only clients early), but remove the McpViaLlm early return:

```rust
// Enforce client mode: block MCP-only clients from LLM endpoints
if let Ok((ref client, _)) = get_client_with_strategy(&state, &auth.api_key_id) {
    check_llm_access(client)?;
}
```

Then **after line 322** (after RouteLLM routing is injected into `provider_request`), add the MCP via LLM interception:

```rust
// MCP via LLM: intercept after compression + RouteLLM are applied
if let Ok((ref client, _)) = get_client_with_strategy(&state, &auth.api_key_id) {
    if client.client_mode == lr_config::ClientMode::McpViaLlm {
        return handle_mcp_via_llm(
            state, auth, client_auth, request, provider_request,
            guardrail_handle, compression_tokens_saved,
        ).await;
    }
}
```

**Effect:** Compression is already applied to `request.messages` (lines 267-303). RouteLLM routing is already on `provider_request.pre_computed_routing` (lines 320-322). Both features work for free with zero new code.

### 2. Update `handle_mcp_via_llm` signature (line 1539)

Add new parameters:

```rust
async fn handle_mcp_via_llm(
    state: AppState,
    auth: AuthContext,
    client_auth: Option<Extension<ClientAuthContext>>,  // was _client_auth
    request: ChatCompletionRequest,
    provider_request: ProviderCompletionRequest,  // NEW: has compression + RouteLLM
    guardrail_handle: GuardrailHandle,            // NEW: spawned guardrail scan
    compression_tokens_saved: u64,                // NEW: for cost tracking
) -> ApiResult<Response>
```

### 3. Remove redundant `convert_to_provider_request` call

Remove line 1573 (`let provider_request = convert_to_provider_request(&request)?;`) — the provider request is now passed in with RouteLLM routing already set.

### 4. Add guardrail gate (sequential mode)

At the start of `handle_mcp_via_llm`, before calling the manager, await guardrails sequentially. MCP via LLM always has side effects (tool execution), so parallel mode is never appropriate. Reuse the same `handle_guardrail_approval` function (line 1174) as the normal sequential path (lines 357-375):

```rust
// Await guardrails sequentially (MCP via LLM executes tools = side effects)
if let Some(handle) = guardrail_handle {
    let guardrail_result = handle.await.map_err(|e| {
        ApiErrorResponse::internal_error(format!("Guardrail check failed: {}", e))
    })??;
    if let Some(check_result) = guardrail_result {
        handle_guardrail_approval(
            &state,
            client_auth.as_ref().map(|e| &e.0),
            &request,
            check_result,
            "request",
        )
        .await?;
    }
}
```

### 5. Add streaming JSON repair

In the streaming path of `handle_mcp_via_llm` (after getting `chunk_stream` from the manager), set up `StreamingJsonRepairer` using the same pattern as `handle_streaming_parallel` (lines 2845-2875):

- Check if `request.response_format` is JSON and config has `json_repair.enabled`
- Create `StreamingJsonRepairer::new(schema, options)`
- In the `data_stream.map()` closure (line 1593), apply `repairer.push_content()` on each chunk's content and `repairer.finish()` on the final chunk

### 6. Add streaming generation tracking

The non-streaming path already tracks generations (lines 1749-1771). The streaming path (lines 1592-1656) does not. Add generation tracking using the same pattern as `handle_streaming` (lines 2557-2712):

- Create `generation_id`, `started_at`, `created_at` timestamps
- Add content/model/finish_reason accumulators in the `data_stream.map()` closure
- Use a `oneshot` channel to signal stream completion
- Spawn a `tokio::spawn` background task that awaits completion and calls `state.generation_tracker.record()`
- Include `compression_tokens_saved` in cost calculation

## Why This Works

- **RouteLLM propagation**: The orchestrator clones `ProviderCompletionRequest` for each loop iteration (`orchestrator.rs:192`, `orchestrator_stream.rs:243`). `pre_computed_routing` is included in the clone, so routing applies to all LLM calls in the agentic loop automatically.
- **Compression**: Applied once to the initial request messages. Subsequent loop iterations add tool results on top — this is correct behavior (tool results shouldn't be compressed away).
- **Guardrails**: Sequential mode matches the `has_side_effects` principle — MCP tool calls are real-world actions that should be gated by safety checks.

## What Does NOT Change

- `crates/lr-mcp-via-llm/` — no changes to the orchestrator, manager, or gateway client
- `crates/lr-guardrails/` — no changes
- `crates/lr-compression/` — no changes
- `crates/lr-json-repair/` — no changes

## Verification

1. `cargo test && cargo clippy` — ensure compilation and no warnings
2. Enable guardrails + send MCP via LLM request → verify guardrail scan triggers
3. Enable compression + send long MCP via LLM conversation → verify messages are compressed
4. Use `localrouter/auto` model with MCP via LLM → verify RouteLLM classification works
5. Stream MCP via LLM with `response_format: json_object` → verify JSON repair on output
6. Stream MCP via LLM → verify generation appears in dashboard/tracker
