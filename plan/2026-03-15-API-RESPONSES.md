# Responses API

## Context

The Responses API (`POST /v1/responses`) is OpenAI's new primary API surface, replacing the Assistants API (deprecated Aug 2026). It's an agent-oriented API with built-in tools, richer streaming events, and conversation state. Groq and xAI already support it natively. Many developer tools are building against it.

## Endpoints

- `POST /v1/responses` — Create a response (streaming or non-streaming)

Future (can be added later):
- `GET /v1/responses/:response_id` — Get a stored response
- `DELETE /v1/responses/:response_id` — Delete a stored response
- `POST /v1/responses/:response_id/cancel` — Cancel in-progress response

## Provider Coverage (Native)

| Provider | Responses API |
|----------|--------------|
| OpenAI | Y |
| Groq | Y |
| xAI | Y |
| All others | N |

## Translation Layer Feasibility

| Feature | Translation Feasible? | Complexity | Notes |
|---------|----------------------|------------|-------|
| Responses API (basic) | **Yes (High feasibility)** | Medium | The Responses API is essentially a higher-level wrapper around chat completions. A translation layer can: (1) convert Responses input format to chat messages, (2) send through existing chat completions router, (3) wrap the response in Responses API format. This means ALL chat-capable providers could support the Responses API through LocalRouter. |
| Built-in tools (web_search) | **Yes (Medium)** | Medium-High | Can delegate to MCP servers that provide web search. LocalRouter already has MCP integration. |
| Built-in tools (code_interpreter) | **No** | N/A | Requires sandboxed code execution environment. Not feasible to translate. |
| Built-in tools (file_search) | **Possible** | High | Would require local vector store + RAG pipeline. Defer. |
| Response state/chaining | **Yes** | Medium | Store responses locally, allow `previous_response_id` to build conversation chains. |
| WebSocket mode | **Partial** | High | Could implement locally for translated providers, but mainly useful for native providers. Defer. |

**Recommendation:** Translation layer is the primary implementation strategy. Since the Responses API maps to chat completions, we can support ALL providers through translation, while also supporting native proxy for OpenAI/Groq/xAI.

## Architecture

### Two Modes

1. **Native proxy** (OpenAI, Groq, xAI) — Forward request directly to provider's `/v1/responses` endpoint. Preserves built-in tools, full streaming event fidelity, and provider-specific features.

2. **Translation layer** (all other providers) — Convert Responses → Chat Completions → Responses:
   - Parse `input` (string, messages array, or items) → convert to `messages[]` for chat completions
   - Map `instructions` → system message
   - Map `tools` (function definitions) → chat completions `tools` parameter
   - Send through existing `router.stream_complete()` or `router.complete()`
   - Convert `ChatCompletionResponse` → `ResponseObject` with proper structure
   - For streaming: convert `ChatCompletionChunk` SSE events → Responses SSE events

### Streaming Event Translation

Chat completions SSE → Responses SSE mapping:
```
(start)                    → response.created
                          → response.output_item.added (message item)
                          → response.content_part.added (text part)
chunk.choices[0].delta     → response.output_text.delta
chunk.choices[0].delta.tool_calls → response.function_call_arguments.delta
(finish_reason: stop)      → response.output_text.done
                          → response.output_item.done
                          → response.completed
```

### New Crate: `crates/lr-responses/`
```
lr-responses/
  Cargo.toml
  src/
    lib.rs          — ResponsesManager, public API
    types.rs        — ResponseObject, ResponseInput, OutputItem, etc.
    native.rs       — Native proxy for OpenAI/Groq/xAI
    translate.rs    — Translation layer (Responses ↔ Chat Completions)
    streaming.rs    — SSE event type conversion
    storage.rs      — Optional response persistence for previous_response_id
```

### Provider Trait
For native proxy, add to `ModelProvider`:
```rust
async fn create_response(&self, request: ResponseCreateRequest) -> AppResult<ResponseObject>
async fn stream_response(&self, request: ResponseCreateRequest) -> AppResult<Pin<Box<dyn Stream<Item = AppResult<ResponseStreamEvent>> + Send>>>
```

### Provider Implementations (Native)
- `crates/lr-providers/src/openai.rs`
- `crates/lr-providers/src/groq.rs`
- `crates/lr-providers/src/xai.rs`

### Phased Implementation

**Phase A: Basic non-streaming (translation layer)**
- Parse Responses input → messages
- Route through existing chat completions
- Return ResponseObject format
- Works with ALL providers

**Phase B: Streaming (translation layer)**
- Convert chat completion chunks to Responses SSE events
- Emit proper event types: `response.created`, `response.output_text.delta`, `response.completed`

**Phase C: Native proxy**
- Forward to OpenAI/Groq/xAI natively
- Preserve built-in tools and full event fidelity

**Phase D: Function/tool calling**
- Map Responses tool definitions to chat completions tools
- Handle `function_call` output items in response

**Phase E: Built-in tools (web_search)**
- Integrate with MCP servers for web search
- Translate web_search tool results into Responses format

**Phase F: State management**
- Store responses locally
- Support `previous_response_id` for conversation chaining
- Store in `{config_dir}/responses/`

## Files to Modify

- **New crate:** `crates/lr-responses/` (6 files)
- `Cargo.toml` (workspace) — add lr-responses
- `crates/lr-server/Cargo.toml` — add lr-responses dependency
- `crates/lr-server/src/state.rs` — add `responses_manager: Arc<lr_responses::ResponsesManager>`
- `crates/lr-providers/src/lib.rs` — response trait methods + types
- `crates/lr-providers/src/openai.rs` — native create_response, stream_response
- `crates/lr-providers/src/groq.rs` — native
- `crates/lr-providers/src/xai.rs` — native
- **New file:** `crates/lr-server/src/routes/responses.rs`
- `crates/lr-server/src/routes/mod.rs` — add module
- `crates/lr-server/src/lib.rs` — register `POST /v1/responses` + `/responses`
- `crates/lr-server/src/openapi/mod.rs` — register

## Cross-Cutting Features Applicability

### Native Proxy Mode
When proxying to OpenAI/Groq/xAI natively, LocalRouter is a pass-through. Cross-cutting features apply at the proxy level:

| Feature | Applies? | Notes |
|---------|----------|-------|
| **Auth (API Key)** | **Yes** | Standard |
| **Permission checks** | **Yes** | Client mode + provider access |
| **Rate limiting** | **Yes** | Estimate tokens from input |
| **Secret scanning** | **Yes** | Scan input text content |
| **Guardrails** | **Yes** | Scan input messages for safety |
| **Prompt compression** | **No** | Would modify the request — risky with native pass-through |
| **RouteLLM** | **No** | Responses API specifies model explicitly |
| **Model firewall** | **Yes** | Approve model usage |
| **Token tracking** | **Yes** | Track from provider response |
| **Cost calculation** | **Yes** | Use provider pricing |
| **Generation tracking** | **Yes** | Assign generation ID |
| **Metrics/logging** | **Yes** | Standard |

### Translation Layer Mode
When translating to chat completions, the underlying chat completions request goes through the FULL existing pipeline:

| Feature | Applies? | Notes |
|---------|----------|-------|
| All features | **Yes** | The translated request goes through `router.complete()` / `router.stream_complete()` which applies the full chat completions pipeline: auth, permissions, rate limiting, secret scanning, guardrails, compression, RouteLLM, firewall, token tracking, cost calculation, generation tracking, metrics. |

This is the key advantage of the translation layer — all existing features work automatically.

## Verification
1. `cargo test` — Responses types, translation logic, streaming conversion
2. Non-streaming: `curl -X POST localhost:3625/v1/responses -d '{"model":"gpt-4o","input":"Hello"}'`
3. Streaming: `curl -N localhost:3625/v1/responses -d '{"model":"gpt-4o","input":"Hello","stream":true}'`
4. Test with non-native provider (e.g., Anthropic) — verify translation layer works
5. Test with native provider (OpenAI) — verify native proxy
6. Verify `/openapi.json` includes responses path
