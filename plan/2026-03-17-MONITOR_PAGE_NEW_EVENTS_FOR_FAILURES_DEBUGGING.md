# Monitor Page: New Events for Failures & Debugging

## Context
The monitor page currently emits events for LLM request/response/error and MCP tool/resource/prompt operations. Many other failure conditions throughout the codebase are only logged (or silently returned as HTTP errors) but not surfaced as monitor events. Adding these would make the monitor page a comprehensive debugging tool.

## Current State
- **Fully emitted**: `LlmRequest`, `LlmRequestTransformed`, `LlmResponse`, `LlmError`, `McpToolCall`, `McpToolResponse`, `McpResourceRead`, `McpResourceResponse`, `McpPromptGet`, `McpPromptResponse`
- **Defined in types but NOT emitted yet**: `GuardrailRequest/Response`, `SecretScanRequest/Response`, `McpElicitation*`, `McpSampling*`, `RouteLlm*`, `RoutingDecision`, `PromptCompression`, `FirewallDecision`, `SseConnection`
- **Not defined at all**: Many auth/access/connection failures below

---

## Candidate Events (pick and choose)

### Category: Authentication & Access Control (`auth` category)

| # | Event | HTTP Code | Trigger | Source |
|---|-------|-----------|---------|--------|
| A1 | **Auth: Missing header** | 401 | Request to protected endpoint has no `Authorization` header | `middleware/auth_layer.rs:110-116` |
| A2 | **Auth: Invalid header format** | 401 | `Authorization` header isn't `Bearer <token>` | `middleware/auth_layer.rs:120-126` |
| A3 | **Auth: Invalid API key** | 401 | Bearer token doesn't match any client | `middleware/auth_layer.rs` + `client_auth.rs` |
| A4 | **Auth: Client not found** | 401 | Client ID from token lookup doesn't exist in config | `routes/helpers.rs:48, 92, 135` |
| A5 | **Auth: Client disabled** | 403 | Client exists but `enabled: false` | `routes/helpers.rs:52, 96, 138` |
| A6 | **Access: MCP-only client hit LLM endpoint** | 403 | Client mode is `McpOnly`, tried `/v1/chat/completions` etc. | `routes/helpers.rs:147-152` |
| A7 | **Access: LLM-only client hit MCP endpoint** | 403 | Client mode is `LlmOnly`, tried `/mcp/*` | `routes/helpers.rs:159-161` |
| A8 | **Access: MCP-via-LLM client hit direct MCP** | 403 | Client mode is `McpViaLlm`, tried direct MCP access | `routes/helpers.rs:162-164` |
| A9 | **Access: Client can't use model** | 403 | Strategy doesn't allow this provider/model | `routes/embeddings.rs:454-457`, `routes/models.rs:181` |
| A10 | **Access: Model not found** | 404 | Requested model doesn't exist | `routes/models.rs:197` |

### Category: Rate Limiting & Quotas (`rate_limit` category)

| # | Event | HTTP Code | Trigger | Source |
|---|-------|-----------|---------|--------|
| R1 | **Rate limit exceeded** | 429 | Client exceeded request quota | `middleware/error.rs:120` via rate limiter |
| R2 | **OAuth token rate limit** | 429 | >10 token requests/min per client | `routes/oauth.rs:232-241` |
| R3 | **Free tier exhausted** | 402 | All free-tier providers at capacity | `middleware/error.rs:121-123` |
| R4 | **Free tier fallback available** | 402 | Free tier exhausted but paid fallback exists | `middleware/error.rs:124-130` |

### Category: Request Validation (`validation` category)

| # | Event | HTTP Code | Trigger | Source |
|---|-------|-----------|---------|--------|
| V1 | **Invalid request: missing model** | 400 | No `model` field in request | `routes/embeddings.rs:286`, `routes/audio.rs:134` |
| V2 | **Invalid request: empty input** | 400 | Empty input for embeddings | `routes/embeddings.rs:294-306` |
| V3 | **Invalid request: bad params** | 400 | Invalid encoding_format, dimensions, temperature, etc. | `routes/embeddings.rs:316-327`, `routes/audio.rs:112-148` |
| V4 | **Invalid multipart data** | 400 | Audio endpoint received malformed multipart | `routes/audio.rs:71` |
| V5 | **Invalid image request** | 400 | Image model format invalid or provider not found | `routes/images.rs:69-83` |

### Category: Provider Errors (`provider` category)

| # | Event | HTTP Code | Trigger | Source |
|---|-------|-----------|---------|--------|
| P1 | **Provider error** | 502 | Upstream provider returned error | `middleware/error.rs:117-118` |
| P2 | **Provider unreachable** | 502 | Provider timeout or connection refused | `router/mod.rs` `RouterError::Unreachable` |
| P3 | **Provider context length exceeded** | 502 | Request exceeded model's token limit | `router/mod.rs` `RouterError::ContextLengthExceeded` |
| P4 | **Provider policy violation** | 502 | Provider rejected content | `router/mod.rs` `RouterError::PolicyViolation` |
| P5 | **Provider rate limited** | 429 | Upstream provider returned 429 | `router/mod.rs` `RouterError::RateLimited` |
| P6 | **Embedding provider error** | 502 | Provider error during embedding call | `routes/embeddings.rs:165` |
| P7 | **Image generation error** | 502 | Provider error during image generation | `routes/images.rs:101-121` |

### Category: MCP Server Health (`mcp_server` category)

| # | Event | Trigger | Source |
|---|-------|---------|--------|
| M1 | **MCP server connection failed** | STDIO/HTTP/WS transport failed to connect | `lr-mcp/src/manager.rs` |
| M2 | **MCP server disconnected** | Running server lost connection unexpectedly | `lr-mcp/src/transport/stdio.rs:252-298` |
| M3 | **MCP server health changed** | Server went Healthy→Unhealthy or vice versa | `lr-mcp/src/manager.rs` health checks |
| M4 | **MCP server stop failed** | Error stopping a server | `lr-mcp/src/manager.rs:1429` |
| M5 | **MCP server PATH resolution failed** | macOS couldn't resolve server binary path | `lr-mcp/src/manager.rs:71-104` |
| M6 | **MCP gateway critical error** | Unrecoverable gateway error | `lr-mcp/src/gateway/gateway.rs:542` |

### Category: MCP Operations (already partially emitted)

| # | Event | Trigger | Source |
|---|-------|---------|--------|
| M7 | **MCP tool not found** | Tool name doesn't exist in session mapping | `lr-mcp/src/gateway/gateway_tools.rs:214-222` |
| M8 | **MCP broadcast all-servers-failed** | All MCP servers failed to respond | `lr-mcp/src/gateway/gateway_tools.rs:142-151` |
| M9 | **MCP resource not found** | Resource URI not in mapping | `lr-mcp/src/gateway/gateway_resources.rs:189-195` |
| M10 | **MCP prompt not found** | Prompt name not in mapping | `lr-mcp/src/gateway/gateway_prompts.rs:165-170` |

### Category: MCP Auth & OAuth (`mcp_auth` category)

| # | Event | Trigger | Source |
|---|-------|---------|--------|
| O1 | **OAuth client secret retrieval failed** | Keychain lookup failed for MCP OAuth | `lr-mcp/src/manager.rs:551, 756` |
| O2 | **OAuth browser token failed** | OAuth browser flow token retrieval failed | `lr-mcp/src/manager.rs:636` |
| O3 | **OAuth credential validation failed** | `/oauth/token` credential check failed | `routes/oauth.rs:256-259` |
| O4 | **OAuth token generation failed** | Internal error generating JWT/token | `routes/oauth.rs:289` |

### Category: Security (types exist, not yet emitted)

| # | Event | Trigger | Source |
|---|-------|---------|--------|
| S1 | **Guardrail request check** | Input sent to safety model for checking | `routes/chat.rs`, `routes/completions.rs` |
| S2 | **Guardrail flagged** | Safety model flagged content | guardrail pipeline |
| S3 | **Guardrail response check** | Output checked by safety model | guardrail pipeline |
| S4 | **Secret scan initiated** | Request scanned for secrets | `routes/chat.rs`, `routes/completions.rs` |
| S5 | **Secret scan found secrets** | Secrets detected in request/response | secret scanner |
| S6 | **Firewall decision** | User approved/denied tool/model access | firewall approval flow |

### Category: Routing (types exist, not yet emitted)

| # | Event | Trigger | Source |
|---|-------|---------|--------|
| RT1 | **RouteLLM classification** | RouteLLM classified request as strong/weak tier | routellm pipeline |
| RT2 | **Routing decision** | Final model routing decision (auto_router, routellm, direct) | chat route |
| RT3 | **Strategy not found** | Client's strategy_id doesn't match any strategy | `routes/helpers.rs:103-108` |

### Category: MCP Sampling & Elicitation (types exist, not yet emitted)

| # | Event | Trigger | Source |
|---|-------|---------|--------|
| E1 | **MCP sampling request** | MCP server requested LLM sampling | `lr-mcp/src/gateway/sampling_approval.rs` |
| E2 | **MCP sampling approved/rejected** | User approved or rejected sampling | sampling approval flow |
| E3 | **MCP sampling broadcast failed** | Failed to broadcast sampling request to UI | `lr-mcp/src/gateway/sampling_approval.rs:139` |
| E4 | **MCP elicitation request** | MCP server requested user input | `lr-mcp/src/gateway/elicitation.rs` |
| E5 | **MCP elicitation timeout** | User didn't respond within 120s | elicitation flow |
| E6 | **MCP elicitation broadcast failed** | Failed to broadcast elicitation to UI | `lr-mcp/src/gateway/elicitation.rs:141` |

### Category: Connection & Transport (`connection` category)

| # | Event | Trigger | Source |
|---|-------|---------|--------|
| C1 | **SSE connection opened** | New MCP SSE client connected | MCP SSE handler |
| C2 | **SSE connection closed** | MCP SSE client disconnected | MCP SSE handler |
| C3 | **WebSocket upgrade failed** | WS handshake failed | `routes/mcp_ws.rs` |
| C4 | **STDIO bridge: config not found** | Bridge can't find config file | `lr-mcp/src/bridge/stdio_bridge.rs:56-61` |
| C5 | **STDIO bridge: client not found** | Bridge client lookup failed | `lr-mcp/src/bridge/stdio_bridge.rs:69` |
| C6 | **STDIO bridge: no MCP servers** | Client has no MCP servers configured | `lr-mcp/src/bridge/stdio_bridge.rs:70-76` |
| C7 | **STDIO bridge: server unavailable** | HTTP server not reachable from bridge | `lr-mcp/src/bridge/stdio_bridge.rs:112` |

### Category: Internal Errors (`internal` category)

| # | Event | HTTP Code | Trigger | Source |
|---|-------|-----------|---------|--------|
| I1 | **Storage error** | 500 | Database/file storage failure | `middleware/error.rs:136-138` |
| I2 | **Serialization error** | 500 | JSON serialization/deserialization failed | `middleware/error.rs:144-146` |
| I3 | **Crypto error** | 500 | Encryption/decryption failure | `middleware/error.rs:147-149` |
| I4 | **Config error** | 400 | Configuration validation failed | `middleware/error.rs:114-116` |
| I5 | **IO error** | 500 | File system or network IO failure | `middleware/error.rs:143` |

### Category: Moderation (`moderation` category)

| # | Event | HTTP Code | Trigger | Source |
|---|-------|-----------|---------|--------|
| MD1 | **Moderation endpoint disabled** | 503 | `/v1/moderations` called but feature disabled | `routes/moderations.rs:69` |
| MD2 | **No safety models configured** | 503 | Moderation called but no safety models | `routes/moderations.rs:80` |
| MD3 | **No safety models loaded** | 503 | Safety models configured but none loaded | `routes/moderations.rs:88` |

---

## Implementation Approach

Once you select which events to implement, the pattern for each is:

1. **If new event type needed**: Add variant to `MonitorEventType` enum + `MonitorEventData` enum in `crates/lr-monitor/src/types.rs`
2. **Add emit helper** (if needed) in `crates/lr-server/src/routes/monitor_helpers.rs`
3. **Add emit call** at the error site (route handler, middleware, or MCP gateway)
4. **Frontend**: Add event rendering in monitor page components

For events that already have types defined (S1-S6, RT1-RT2, E1-E6, C1-C2), only steps 3-4 are needed.
