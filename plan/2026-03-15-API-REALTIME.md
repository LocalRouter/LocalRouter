# Realtime WebSocket API

## Context

The OpenAI Realtime API provides WebSocket-based real-time audio/text conversations with low latency. It supports voice activity detection, bidirectional audio streaming, and function calling during live conversations. This is the most complex new endpoint to implement.

## Endpoints

- `GET /v1/realtime` — WebSocket upgrade (query param `model=...`)

## Provider Coverage (Native OpenAI-Compatible Protocol)

| Provider | Realtime WS |
|----------|-------------|
| OpenAI | Y (`wss://api.openai.com/v1/realtime?model=...`) |
| TogetherAI | Y (`wss://api.together.ai/v1/realtime?model=...`) |
| LocalAI | Y (`ws://localhost:8080/v1/realtime?model=...`) |
| All others | N |

**Non-compatible but similar:**
- Gemini Live: Own WebSocket protocol at different URL, different event schema
- xAI Grok Voice: Own voice agent API, not OpenAI Realtime-compatible

## Translation Layer Feasibility

| Feature | Translation Feasible? | Complexity | Notes |
|---------|----------------------|------------|-------|
| Realtime API | **No (not practical)** | N/A | The Realtime API requires specialized audio processing models (voice synthesis, real-time STT, VAD). Cannot be emulated by sending text through chat completions — the entire point is sub-50ms latency bidirectional audio. Even if you chained STT → Chat → TTS, the latency would be 2-5 seconds vs 50ms native, making it useless for real-time conversation. |
| Gemini Live bridge | **Possible but complex** | Very High | Could translate between OpenAI Realtime events and Gemini Live events. Different WebSocket protocols, different event schemas, different audio formats. Would require a full protocol bridge. |
| xAI Grok Voice bridge | **Possible** | High | xAI has its own real-time voice API. Could bridge if the event schemas are similar enough. |

**Recommendation:** Native proxy only. Bridge to Gemini Live and xAI Grok Voice as future enhancements if demand exists.

## Architecture

### WebSocket Proxy Pattern
LocalRouter acts as a WebSocket proxy:
1. Client connects to `ws://localhost:3625/v1/realtime?model=...`
2. LocalRouter authenticates, resolves provider
3. LocalRouter opens upstream WebSocket to provider (`wss://api.openai.com/v1/realtime?model=...`)
4. Bridge messages bidirectionally between client and provider

This reuses patterns from existing `crates/lr-server/src/routes/mcp_ws.rs`.

### New Crate: `crates/lr-realtime/`
```
lr-realtime/
  Cargo.toml
  src/
    lib.rs              — RealtimeManager, session tracking
    types.rs            — Client events + server events (OpenAI Realtime protocol)
    session.rs          — RealtimeSession state management
    proxy.rs            — WebSocket bidirectional proxy bridge
    provider_bridge.rs  — Per-provider WebSocket connection logic
```

### Key Design Decisions

**Session management:**
- Each WebSocket connection = one realtime session
- Track active sessions for monitoring/cleanup
- Session timeout: 30 minutes (configurable)

**Authentication:**
- Bearer token in WebSocket upgrade request headers
- Same API key auth as HTTP endpoints
- Provider API key injected when opening upstream WebSocket

**Message bridging:**
- JSON text frames forwarded as-is (mostly transparent proxy)
- Binary frames for audio forwarded as-is
- LocalRouter may inspect/log certain events (session.created, response.done) for metrics
- No modification of audio data

**Provider connection:**
- Use `tokio-tungstenite` (already a dependency) for upstream WebSocket
- Connection parameters: model from query string, API key from provider config
- Reconnection: if upstream drops, notify client with error event

### Provider Trait
```rust
/// Get WebSocket URL for realtime API
fn realtime_ws_url(&self, model: &str) -> Option<String>

/// Get authentication headers for realtime WebSocket
fn realtime_auth_headers(&self) -> Option<Vec<(String, String)>>
```

These are simple methods (not async trait methods) since they just return URLs/headers.

### Provider Implementations
- `crates/lr-providers/src/openai.rs` — URL: `wss://api.openai.com/v1/realtime?model={model}`
- `crates/lr-providers/src/togetherai.rs` — URL: `wss://api.together.ai/v1/realtime?model={model}`
- `crates/lr-providers/src/localai.rs` — URL: `ws://{host}/v1/realtime?model={model}`

## Files to Modify

- **New crate:** `crates/lr-realtime/` (5 files)
- `Cargo.toml` (workspace) — add lr-realtime
- `crates/lr-server/Cargo.toml` — add lr-realtime dependency
- `crates/lr-server/src/state.rs` — add `realtime_manager: Arc<lr_realtime::RealtimeManager>`
- `crates/lr-providers/src/lib.rs` — realtime URL/auth methods
- `crates/lr-providers/src/openai.rs` — realtime_ws_url, realtime_auth_headers
- `crates/lr-providers/src/togetherai.rs` — realtime_ws_url, realtime_auth_headers
- `crates/lr-providers/src/localai.rs` — realtime_ws_url, realtime_auth_headers
- **New file:** `crates/lr-server/src/routes/realtime.rs`
- `crates/lr-server/src/routes/mod.rs` — add module
- `crates/lr-server/src/lib.rs` — register `GET /v1/realtime` WebSocket upgrade
- `crates/lr-server/src/openapi/mod.rs` — document (WebSocket endpoints are tricky in OpenAPI)

## Cross-Cutting Features Applicability

| Feature | Applies? | Notes |
|---------|----------|-------|
| **Auth (API Key)** | **Yes** | Validate Bearer token during WebSocket upgrade handshake |
| **Permission checks** | **Yes** | Client mode + provider access checked at connection time |
| **Rate limiting** | **Partial** | Check rate limits at connection time. Per-message rate limiting is impractical for real-time audio. Could track cumulative audio duration. |
| **Secret scanning** | **No** | Audio data is binary. Text messages within the session could theoretically be scanned, but latency impact is unacceptable for real-time. |
| **Guardrails** | **No** | Latency-critical — cannot add synchronous safety checks to real-time audio stream. Providers handle their own safety. |
| **Prompt compression** | **No** | Not applicable to real-time audio |
| **RouteLLM** | **No** | Realtime models are specialized (gpt-4o-realtime), no routing needed |
| **Model firewall** | **Yes** | Approve at connection time (before WebSocket upgrade completes) |
| **Token tracking** | **Yes** | Extract from `response.done` server events which include usage stats |
| **Cost calculation** | **Yes** | Realtime pricing is per-minute audio. Track from `response.done` events. |
| **Generation tracking** | **Partial** | Track session-level metadata. Each `response.done` event could get a generation ID. |
| **Metrics/logging** | **Yes** | Log connection events, session duration, total tokens/cost per session |
| **Client activity** | **Yes** | Record activity at connection time |

## Verification
1. `cargo test` — event type serialization, proxy logic
2. WebSocket test with `websocat`: connect to `ws://localhost:3625/v1/realtime?model=gpt-4o-realtime-preview`
3. Verify auth rejection for invalid tokens
4. Test with provider that doesn't support realtime — verify appropriate error during upgrade
5. Full integration: send text events, verify bidirectional message flow
6. Verify session cleanup after disconnection
