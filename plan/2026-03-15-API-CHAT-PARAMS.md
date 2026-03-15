# Chat Completions Parameter Improvements

## Context

The existing `/v1/chat/completions` endpoint is missing several parameters that OpenAI has added over the past year. These are incremental additions to existing code — no new endpoints, no new crates.

## Missing Request Parameters

| Parameter | Type | Description | Provider Support | Priority |
|-----------|------|-------------|-----------------|----------|
| `n` | `Option<u32>` | Number of completions to generate | OpenAI, some others | Medium |
| `logit_bias` | `Option<HashMap<String, f32>>` | Modify token likelihoods | OpenAI, some others | Low |
| `parallel_tool_calls` | `Option<bool>` | Allow concurrent function calling | OpenAI | Medium |
| `service_tier` | `Option<String>` | Latency tier selection ("auto", "default") | OpenAI only | Low |
| `store` | `Option<bool>` | Store for distillation/evaluation | OpenAI only | Low |
| `metadata` | `Option<HashMap<String, String>>` | Developer-defined tags | OpenAI only | Low |
| `modalities` | `Option<Vec<String>>` | Output types: ["text"], ["text", "audio"] | OpenAI (audio models) | Medium |
| `audio` | `Option<AudioOutputConfig>` | Voice, format for audio output | OpenAI (audio models) | Medium |
| `prediction` | `Option<Prediction>` | Predicted output for faster generation | OpenAI | Low |
| `reasoning_effort` | `Option<String>` | low/medium/high for reasoning models | OpenAI, Anthropic | Medium |

## Missing Response Fields

| Field | Type | Description | Priority |
|-------|------|-------------|----------|
| `system_fingerprint` | `Option<String>` | Model version identifier | Medium |
| `service_tier` | `Option<String>` | Tier used for request | Low |
| `usage` in final streaming chunk | `Usage` | Token counts in last SSE event | High |

## Translation Layer Feasibility

| Parameter | Translation Feasible? | Complexity | Notes |
|-----------|----------------------|------------|-------|
| `n` | **Yes (Low)** | Low | Pass through to providers that support it. For others, make N sequential calls and merge responses. |
| `logit_bias` | **No** | N/A | Provider-specific token IDs — cannot translate across tokenizers |
| `parallel_tool_calls` | **Yes (Low)** | Low | Pass through to supporting providers. Others ignore. |
| `service_tier` | **No** | N/A | OpenAI infrastructure concept, no equivalent elsewhere |
| `store` | **No** | N/A | OpenAI infrastructure concept |
| `metadata` | **Yes (trivial)** | Trivial | Store locally, pass through where supported |
| `modalities`/`audio` | **No** | N/A | Requires audio-capable models |
| `prediction` | **No** | N/A | Provider-specific optimization |
| `reasoning_effort` | **Yes (Medium)** | Medium | Map to Anthropic's budget_tokens, Gemini's thinking_level. Already partially handled by feature adapters. |
| `system_fingerprint` | **Yes (trivial)** | Trivial | Generate locally if provider doesn't return one |
| `usage` in streaming | **Yes (Medium)** | Medium | Accumulate token counts during stream, emit in final chunk. May already be partially implemented. |

**Recommendation:** Pass-through approach — add parameters to request types, forward to providers that support them, ignore for others. Translation only for `n` (sequential calls) and `usage` in streaming (accumulate locally).

## Files to Modify

### Request Types
- `crates/lr-providers/src/lib.rs` — Add fields to `CompletionRequest`
- `crates/lr-server/src/types.rs` — Add fields to `ChatCompletionRequest`

### Response Types
- `crates/lr-providers/src/lib.rs` — Add `system_fingerprint`, `service_tier` to `CompletionResponse`
- `crates/lr-server/src/types.rs` — Add to `ChatCompletionResponse`, `ChatCompletionChunk`

### Streaming
- `crates/lr-server/src/routes/chat.rs` — Emit `usage` in final streaming chunk (check if already done)

### Provider Forwarding
- `crates/lr-providers/src/openai.rs` — Forward new parameters in request body
- `crates/lr-providers/src/groq.rs` — Forward supported parameters
- Other providers: no changes (unsupported params are `Option<>` and skipped via `#[serde(skip_serializing_if = "Option::is_none")]`)

### OpenAPI
- `crates/lr-server/src/openapi/mod.rs` — Update schema documentation

## Cross-Cutting Features

No changes to cross-cutting features. These are parameter additions to existing endpoints that already have the full feature pipeline.

## Verification
1. `cargo test` — serialization round-trips for new fields
2. `cargo clippy && cargo fmt`
3. Test with `n=2`: verify multiple choices returned
4. Test streaming: verify `usage` appears in final chunk
5. Verify `/openapi.json` reflects new parameters
6. Test unknown parameters are ignored (backward compatibility)
