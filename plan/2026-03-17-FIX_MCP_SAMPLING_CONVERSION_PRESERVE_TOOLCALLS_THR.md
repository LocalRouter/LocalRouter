# Fix MCP Sampling Conversion: Preserve tool_calls Through Pipeline

## Context

When a client (e.g., opencode) sends a multi-turn conversation with tool use through LocalRouter's MCP `sampling/createMessage` path, tool-related fields are silently stripped. This produces invalid OpenAI-protocol requests:

1. Assistant messages lose `tool_calls` array
2. Tool role messages lose `tool_call_id` (providers reject this)
3. LLM response tool_calls are silently discarded
4. `finish_reason: "tool_calls"` is mapped to `"end_turn"`, losing the signal

**Root cause**: `SamplingMessage`, `SamplingRequest`, and `SamplingResponse` lack optional fields for tool_calls/tool_call_id/tools. The conversion functions in `sampling.rs` hardcode these to `None`.

## Approach

Add optional tool fields to the MCP protocol types and pass them through the conversion functions. All new fields are `Option` with `skip_serializing_if` and `default`, so this is fully backward-compatible. Reuse existing `lr_providers` types (`ToolCall`, `Tool`, `ToolChoice`) — no new type definitions needed.

## Changes

### 1. `crates/lr-mcp/src/protocol.rs` — Add fields to protocol types

**SamplingMessage** (line 456): Add 3 optional fields:
```rust
#[serde(rename = "toolCalls", alias = "tool_calls", skip_serializing_if = "Option::is_none", default)]
pub tool_calls: Option<Vec<ToolCall>>,

#[serde(rename = "toolCallId", alias = "tool_call_id", skip_serializing_if = "Option::is_none", default)]
pub tool_call_id: Option<String>,

#[serde(skip_serializing_if = "Option::is_none", default)]
pub name: Option<String>,
```

**SamplingRequest** (line 512): Add 2 optional fields after `metadata`:
```rust
#[serde(skip_serializing_if = "Option::is_none", default)]
pub tools: Option<Vec<Tool>>,

#[serde(rename = "toolChoice", alias = "tool_choice", skip_serializing_if = "Option::is_none", default)]
pub tool_choice: Option<ToolChoice>,
```

**SamplingResponse** (line 543): Add 1 optional field:
```rust
#[serde(rename = "toolCalls", alias = "tool_calls", skip_serializing_if = "Option::is_none", default)]
pub tool_calls: Option<Vec<ToolCall>>,
```

**Import**: Add `use lr_providers::{Tool, ToolCall, ToolChoice};` at top.

### 2. `crates/lr-mcp/src/gateway/sampling.rs` — Update conversion functions

**`convert_sampling_message_to_chat()`** (line 85-91): Pass through instead of hardcoding `None`:
```rust
tool_calls: msg.tool_calls,
tool_call_id: msg.tool_call_id,
name: msg.name,
```

**`convert_sampling_to_chat_request()`** (lines 53-54): Pass through tools:
```rust
tools: sampling_req.tools,
tool_choice: sampling_req.tool_choice,
```

**`convert_chat_to_sampling_response()`**:
- Line 121: Change `"tool_calls" => "end_turn"` to `"tool_calls" => "tool_calls"`
- Lines 127-132: Add `tool_calls: choice.message.tool_calls.clone()` to the response struct

### 3. Update all struct literal call sites (add `None` for new fields)

**15 `SamplingMessage` literals** — add `tool_calls: None, tool_call_id: None, name: None`:
- `sampling.rs`: lines 143, 166, 187, 213, 231, 258, 264, 270, 312
- `protocol.rs`: lines 739, 760, 809
- `sampling_approval.rs`: line 449
- `commands.rs`: line 4484

**6 `SamplingRequest` literals** — add `tools: None, tool_choice: None`:
- `sampling.rs`: lines 142, 165, 256, 311
- `protocol.rs`: line 808
- `sampling_approval.rs`: line 448
- `commands.rs`: line 4483

**1 `SamplingResponse` literal** — add `tool_calls: None`:
- `protocol.rs`: line 842

**1 `SamplingResponse` construction** — add `tool_calls`:
- `sampling.rs`: line 127

### 4. Update tests

**Convert bug-documenting tests → fix-verification tests** in `sampling.rs`:
- `test_convert_sampling_message_sets_tool_calls_to_none` → verify tool_calls ARE preserved
- `test_convert_sampling_message_sets_tool_call_id_to_none` → verify tool_call_id/name ARE preserved
- `test_convert_sampling_request_strips_tool_calls_from_multi_turn_conversation` → verify full multi-turn roundtrip works
- `test_convert_sampling_request_does_not_forward_tools_definition` → verify tools/tool_choice ARE forwarded
- `test_convert_response_discards_tool_calls_from_llm` → verify tool_calls ARE preserved in response
- `test_convert_response_maps_finish_reasons_correctly` → update "tool_calls" expectation from "end_turn" to "tool_calls"

**Add new tests** in `protocol.rs`:
- Round-trip serialization of SamplingMessage with tool_calls (camelCase output)
- Snake_case alias deserialization (`tool_calls` → works like `toolCalls`)
- SamplingMessage with tool_call_id round-trip

## Verification

```bash
cargo test -p lr-mcp && cargo clippy -p lr-mcp && cargo fmt --check -p lr-mcp
cargo test -p lr-server   # ensure no breakage in routes
cargo build               # full build check
```
