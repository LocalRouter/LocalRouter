# MCP Unified Gateway - Technical Documentation

**Version**: 0.1.0
**Date**: 2026-01-23
**Status**: Implementation Complete, Under Review

---

## Table of Contents

1. [Overview](#1-overview)
2. [Architecture](#2-architecture)
3. [Connection Management](#3-connection-management)
4. [Request Routing](#4-request-routing)
5. [Response Handling & Merging](#5-response-handling--merging)
6. [Session Management](#6-session-management)
7. [Notification Forwarding](#7-notification-forwarding)
8. [Deferred Loading](#8-deferred-loading)
9. [Special Features](#9-special-features)
10. [API Endpoints](#10-api-endpoints)
11. [Configuration](#11-configuration)
12. [Known Bugs & Issues](#12-known-bugs--issues)

---

## 1. Overview

The MCP Unified Gateway is a sophisticated proxy system that aggregates multiple backend MCP servers into a single client-facing interface. It enables LLM clients to interact with multiple MCP servers (filesystem, GitHub, databases, etc.) through a single endpoint while handling:

- **Request routing** - Directing requests to the appropriate backend server(s)
- **Response merging** - Combining responses from multiple servers
- **Namespace management** - Preventing tool/resource/prompt name collisions
- **Connection pooling** - Efficient transport management across backends
- **Session isolation** - Per-client caching and state management
- **Real-time notifications** - Forwarding server notifications to clients

### Design Pattern

The gateway implements a **dual-role architecture**:
- Acts as an **MCP server** to external clients (Claude, Cursor, etc.)
- Acts as an **MCP client** to backend MCP servers

---

## 2. Architecture

### Component Overview

```
┌─────────────────────────────────────────────────────────────────────┐
│                        External MCP Clients                          │
│                  (Claude Code, Cursor, etc.)                         │
└─────────────────────────────────────────────────────────────────────┘
                                    │
                         ┌──────────┼──────────┐
                         │    HTTP/SSE/WS      │
                         ▼                     ▼
┌─────────────────────────────────────────────────────────────────────┐
│                       MCP Unified Gateway                            │
│  ┌───────────────┐  ┌───────────────┐  ┌───────────────┐            │
│  │ Route Handler │  │ GatewaySession│  │     SSE       │            │
│  │   (mcp.rs)    │  │ (per-client)  │  │ ConnManager   │            │
│  └───────────────┘  └───────────────┘  └───────────────┘            │
│           │                  │                  │                    │
│           ▼                  ▼                  ▼                    │
│  ┌─────────────────────────────────────────────────────────────┐    │
│  │                      McpGateway                              │    │
│  │   ┌──────────┐  ┌──────────┐  ┌──────────┐  ┌──────────┐    │    │
│  │   │  Router  │  │  Merger  │  │ Deferred │  │Elicitation│   │    │
│  │   │          │  │          │  │ Loading  │  │  Manager │    │    │
│  │   └──────────┘  └──────────┘  └──────────┘  └──────────┘    │    │
│  └─────────────────────────────────────────────────────────────┘    │
│                              │                                       │
│                    ┌─────────┼─────────┐                            │
│                    ▼         ▼         ▼                            │
│  ┌─────────────────────────────────────────────────────────────┐    │
│  │                   McpServerManager                           │    │
│  │   ┌─────────────┐  ┌─────────────┐  ┌─────────────┐         │    │
│  │   │StdioTransport│ │ SseTransport│  │  WebSocket  │         │    │
│  │   │   (DashMap) │  │  (DashMap)  │  │ Transport   │         │    │
│  │   └─────────────┘  └─────────────┘  └─────────────┘         │    │
│  └─────────────────────────────────────────────────────────────┘    │
└─────────────────────────────────────────────────────────────────────┘
                         │         │         │
                         ▼         ▼         ▼
┌─────────────────┐ ┌─────────────┐ ┌─────────────────────┐
│   STDIO Server  │ │ SSE Server  │ │  WebSocket Server   │
│  (local process)│ │ (HTTP+SSE)  │ │     (ws://)         │
└─────────────────┘ └─────────────┘ └─────────────────────┘
```

### Key Source Files

| File | Purpose |
|------|---------|
| `src/mcp/gateway/gateway.rs` | Main gateway orchestration, request handling |
| `src/mcp/gateway/router.rs` | Broadcast logic, retry handling, result separation |
| `src/mcp/gateway/merger.rs` | Response merging (tools, resources, prompts, capabilities) |
| `src/mcp/gateway/session.rs` | Per-client session state, cache management |
| `src/mcp/gateway/types.rs` | Data structures, namespace parsing |
| `src/mcp/gateway/deferred.rs` | Deferred loading search functionality |
| `src/mcp/gateway/elicitation.rs` | User input request handling |
| `src/mcp/manager.rs` | Transport lifecycle management |
| `src/mcp/transport/sse.rs` | SSE transport implementation |
| `src/server/routes/mcp.rs` | HTTP route handlers (unified gateway SSE) |
| `src/server/routes/mcp_ws.rs` | WebSocket notification endpoint |
| `src/server/state.rs` | SseConnectionManager for client connections |

---

## 3. Connection Management

### 3.1 Transport Types

The gateway supports three transport mechanisms for connecting to backend MCP servers:

#### STDIO Transport (`transport/stdio.rs`)
- Spawns local processes with stdin/stdout communication
- Used for on-machine MCP servers (e.g., filesystem, shell tools)
- Environment variable injection for authentication
- Process lifecycle management

#### SSE Transport (`transport/sse.rs`)
- HTTP-based bidirectional communication
- **POST** requests for client→server messages
- **GET** endpoint for persistent server→client SSE stream
- Global HTTP client with connection pooling (10 idle connections/host)
- Supports inline responses OR streaming via persistent connection

#### WebSocket Transport (`transport/websocket.rs`)
- Full-duplex bidirectional communication
- Best for high-frequency notifications
- Supports streaming responses

### 3.2 Connection Flow: Client → Gateway → Backend

**Question: When a client establishes SSE to the gateway, does the gateway establish SSE to all backend servers?**

**Answer**: No. The gateway uses **lazy initialization** and **persistent connections**:

1. When a client connects, the gateway creates a `GatewaySession` with a list of `allowed_servers`
2. Backend server connections are **not** established until the first request requires them
3. Upon first use, the gateway calls `server_manager.start_server(server_id)` which:
   - Checks the transport type configured for that server
   - Establishes the appropriate transport (STDIO/SSE/WebSocket)
   - Stores the transport in a `DashMap` for reuse
4. Subsequent requests reuse existing transports

**SSE Transport Connection Details** (`SseTransport::connect()`):

```rust
// 1. Use shared HTTP client with connection pooling
let client = HTTP_CLIENT.clone();  // Global, 10 idle conns/host

// 2. Send initialize request to validate connection
let init_response = client.post(&url).json(&init_request).send().await;

// 3. Start background task for persistent SSE stream
let stream_task = tokio::spawn(sse_stream_task(...));

// 4. Return transport with:
//    - pending: HashMap<request_id, oneshot::Sender>  // Response correlation
//    - next_id: AtomicU64                            // Request ID generation
//    - notification_callback: Option<Callback>        // Notification handling
```

### 3.3 Request/Response Correlation

Each transport maintains a **pending request map** for correlating responses:

```rust
// SseTransport
pending: Arc<RwLock<HashMap<String, oneshot::Sender<JsonRpcResponse>>>>

// Request flow:
1. Generate unique request ID (atomic counter)
2. Create oneshot channel (tx, rx)
3. Insert (request_id → tx) into pending map
4. Send POST request to backend
5. Background task reads SSE stream:
   - Parse response, extract ID
   - Look up ID in pending map
   - Send response via oneshot channel
6. Waiting code receives response via rx
7. Timeout: 30 seconds

// Special case: Inline responses
// Some servers return response in POST body (not via SSE stream)
// Gateway checks POST response body first, then falls back to SSE wait
```

---

## 4. Request Routing

### 4.1 Routing Decision Tree

```
                    ┌──────────────────────┐
                    │  Incoming Request    │
                    │  (method, params)    │
                    └──────────┬───────────┘
                               │
                    ┌──────────▼───────────┐
                    │ Check: should_broadcast()│
                    └──────────┬───────────┘
                               │
          ┌────────────────────┼────────────────────┐
          │ YES                │ NO                 │
          ▼                    │                    │
┌─────────────────┐           │                    │
│ BROADCAST       │           │                    │
│ Methods:        │           │                    │
│ • initialize    │           │                    │
│ • tools/list    │           │                    │
│ • resources/list│           │                    │
│ • prompts/list  │           │                    │
│ • logging/setLevel │        │                    │
│ • ping          │           │                    │
└────────┬────────┘           │                    │
         │                    │                    │
         ▼                    │                    │
┌─────────────────┐           │      ┌────────────▼────────────┐
│ Send to ALL     │           │      │ DIRECT Methods          │
│ allowed_servers │           │      │ • tools/call            │
│ in parallel     │           │      │ • resources/read        │
│ (join_all)      │           │      │ • prompts/get           │
└────────┬────────┘           │      │ • Special handlers      │
         │                    │      └────────────┬────────────┘
         │                    │                   │
         ▼                    │                   ▼
┌─────────────────┐           │      ┌─────────────────────────┐
│ Collect results │           │      │ Extract namespace from  │
│ Merge responses │           │      │ tool/resource/prompt    │
│ Handle failures │           │      │ name: "server__name"    │
└─────────────────┘           │      └────────────┬────────────┘
                              │                   │
                              │                   ▼
                              │      ┌─────────────────────────┐
                              │      │ Route to single server  │
                              │      │ Strip namespace from    │
                              │      │ request before sending  │
                              │      └─────────────────────────┘
```

### 4.2 Namespace Convention

Tools, resources, and prompts use double-underscore namespacing: `{server_id}__{original_name}`

```rust
pub const NAMESPACE_SEPARATOR: &str = "__";

// Examples:
"filesystem__read_file"  → server_id: "filesystem", original_name: "read_file"
"github__create_issue"   → server_id: "github", original_name: "create_issue"
```

**Parsing** (`parse_namespace()`):
```rust
pub fn parse_namespace(namespaced: &str) -> Option<(String, String)> {
    let idx = namespaced.find(NAMESPACE_SEPARATOR)?;
    let server_id = &namespaced[..idx];
    let original_name = &namespaced[idx + 2..];

    if server_id.is_empty() || original_name.is_empty() {
        return None;
    }
    Some((server_id.to_string(), original_name.to_string()))
}
```

### 4.3 Broadcast Request Handling (`router.rs`)

```rust
pub async fn broadcast_request(
    server_ids: &[String],
    request: JsonRpcRequest,
    server_manager: &Arc<McpServerManager>,
    request_timeout: Duration,
    max_retries: u8,
) -> Vec<(String, AppResult<Value>)> {
    // Send to all servers in parallel using futures::future::join_all
    let futures = server_ids.iter().map(|server_id| {
        async move {
            let mut retries = 0;
            loop {
                let result = timeout(
                    request_timeout,
                    server_manager.send_request(&server_id, request.clone()),
                ).await;

                match result {
                    Ok(Ok(resp)) => return (server_id, Ok(resp.result)),
                    Ok(Err(e)) if retries < max_retries && is_retryable(&e) => {
                        retries += 1;
                        // Exponential backoff: 200ms, 400ms, 800ms... (capped at 10s)
                        let backoff = Duration::from_millis((100 * (1 << retries)).min(10_000));
                        sleep(backoff).await;
                        continue;
                    }
                    Ok(Err(e)) => return (server_id, Err(e)),
                    Err(_) => return (server_id, Err("timeout")),
                }
            }
        }
    });

    futures::future::join_all(futures).await
}
```

**Retryable Errors**:
- Timeout errors
- Connection errors
- NOT: Auth errors, method not found

### 4.4 Session-Based Mapping Lookup

When routing direct requests, the gateway verifies the tool/resource/prompt exists:

```rust
// GatewaySession stores mappings:
pub struct GatewaySession {
    // namespaced_name → (server_id, original_name)
    pub tool_mapping: HashMap<String, (String, String)>,
    pub resource_mapping: HashMap<String, (String, String)>,
    pub resource_uri_mapping: HashMap<String, (String, String)>,  // For URI-based routing
    pub prompt_mapping: HashMap<String, (String, String)>,
}

// Validation in handle_tools_call():
if !session.tool_mapping.contains_key(tool_name) {
    return JsonRpcError::tool_not_found(tool_name);
}
```

---

## 5. Response Handling & Merging

### 5.1 Initialize Response Merging (`merge_initialize_results`)

When `initialize` is broadcast to all servers, responses are merged:

```rust
pub fn merge_initialize_results(
    results: Vec<(String, InitializeResult)>,
    failures: Vec<ServerFailure>,
) -> MergedCapabilities {
    // 1. Protocol Version: Use MINIMUM (most restrictive for compatibility)
    let protocol_version = results.iter()
        .map(|(_, r)| &r.protocol_version)
        .min()
        .unwrap_or("2024-11-05");

    // 2. Capabilities: UNION (if ANY server supports, gateway supports)
    let mut merged = ServerCapabilities::default();
    for (_, result) in &results {
        if let Some(tools) = &result.capabilities.tools {
            if tools.list_changed.unwrap_or(false) {
                merged.tools.get_or_insert_default().list_changed = Some(true);
            }
        }
        // Same for resources, prompts, logging...
    }

    // 3. Server Info: Build comprehensive description
    let server_info = ServerInfo {
        name: "LocalRouter Unified Gateway",
        version: "0.1.0",
        description: build_server_description(&results, &failures),
    };

    MergedCapabilities { protocol_version, capabilities: merged, server_info, failures }
}
```

### 5.2 Tools/Resources/Prompts Merging

**Strategy**: Collect from all servers, apply namespace, sort for consistency

```rust
pub fn merge_tools(
    server_tools: Vec<(String, Vec<McpTool>)>,
    _failures: &[ServerFailure],
) -> Vec<NamespacedTool> {
    let mut merged = Vec::new();

    for (server_id, tools) in server_tools {
        for tool in tools {
            merged.push(NamespacedTool {
                name: format!("{}__{}", server_id, tool.name),
                original_name: tool.name,
                server_id: server_id.clone(),
                description: tool.description,
                input_schema: tool.input_schema,
            });
        }
    }

    // Sort by server_id, then by name for consistent ordering
    merged.sort_by(|a, b| {
        a.server_id.cmp(&b.server_id)
            .then_with(|| a.name.cmp(&b.name))
    });

    merged
}
```

### 5.3 Partial Failure Handling

The gateway supports **partial failures** - clients receive results from working servers plus error info:

```rust
// Response structure for partial failures:
{
    "tools": [ /* tools from working servers */ ],
    "_meta": {
        "partial_failure": true,
        "failures": [
            { "server_id": "github", "error": "Connection timeout" }
        ]
    }
}
```

---

## 6. Session Management

### 6.1 GatewaySession Structure

Each client gets an isolated session:

```rust
pub struct GatewaySession {
    pub client_id: String,
    pub allowed_servers: Vec<String>,

    // Mappings (populated on first list request)
    pub tool_mapping: HashMap<String, (String, String)>,
    pub resource_mapping: HashMap<String, (String, String)>,
    pub resource_uri_mapping: HashMap<String, (String, String)>,
    pub prompt_mapping: HashMap<String, (String, String)>,

    // Caches
    pub cached_tools: Option<CachedList<NamespacedTool>>,
    pub cached_resources: Option<CachedList<NamespacedResource>>,
    pub cached_prompts: Option<CachedList<NamespacedPrompt>>,

    // Deferred loading state
    pub deferred_loading: Option<DeferredLoadingState>,

    // TTL management
    pub created_at: Instant,
    pub last_activity: Instant,
    pub ttl: Duration,  // Default: 1 hour
    pub cache_ttl_manager: DynamicCacheTTL,
}
```

### 6.2 Dynamic Cache TTL

The cache TTL adapts based on invalidation frequency:

```rust
pub struct DynamicCacheTTL {
    base_ttl_seconds: u64,  // Configured value (default: 300s)
    invalidation_count: AtomicU32,
    last_reset: Instant,
}

impl DynamicCacheTTL {
    pub fn get_ttl(&self) -> Duration {
        let invalidations = self.invalidation_count.load();

        if invalidations > 20 {
            Duration::from_secs(60)   // High rate: 1 minute
        } else if invalidations > 5 {
            Duration::from_secs(120)  // Medium rate: 2 minutes
        } else {
            Duration::from_secs(self.base_ttl_seconds)  // Low rate: use config
        }

        // Counter resets hourly
    }
}
```

### 6.3 Session Lifecycle

```
1. Client authenticates with Bearer token
2. First request to gateway endpoint
3. get_or_create_session() called:
   - Check if session exists and not expired
   - If expired, remove old session
   - Create new GatewaySession
4. Register notification handlers for allowed servers
5. Session touched on every request (updates last_activity)
6. Background task cleans up expired sessions
7. All caches/mappings isolated per session
```

---

## 7. Notification Forwarding

### 7.1 Architecture

```
Backend MCP Server
        │
        │ Sends notification via STDIO/SSE
        ▼
┌─────────────────────────────────────────┐
│ Transport Layer (notification_callback) │
└─────────────────────┬───────────────────┘
                      │
                      ▼
┌─────────────────────────────────────────┐
│ McpServerManager.dispatch_notification()│
│ - Looks up handlers for server_id       │
│ - Calls each registered handler         │
└─────────────────────┬───────────────────┘
                      │
                      ▼
┌─────────────────────────────────────────┐
│ Gateway Notification Handler (global)   │
│ - One handler per server (not session)  │
│ - Handles cache invalidation events     │
│ - Forwards to broadcast channel         │
└─────────────────────┬───────────────────┘
                      │
          ┌───────────┴───────────┐
          ▼                       ▼
┌─────────────────┐   ┌─────────────────────┐
│ Cache Invalidate│   │ Broadcast Channel   │
│ for all sessions│   │ (all client WS)     │
│ using this server│  └─────────────────────┘
└─────────────────┘              │
                                 ▼
                    ┌─────────────────────────┐
                    │ WebSocket Clients       │
                    │ (filter by allowed_servers)│
                    └─────────────────────────┘
```

### 7.2 Notification Types Handled

```rust
match notification.method.as_str() {
    "notifications/tools/list_changed" => {
        // Invalidate cached_tools for ALL sessions with this server
        for session in sessions.iter() {
            if session.allowed_servers.contains(&server_id) {
                session.cached_tools = None;
            }
        }
    }
    "notifications/resources/list_changed" => {
        // Invalidate cached_resources...
    }
    "notifications/prompts/list_changed" => {
        // Invalidate cached_prompts...
    }
    _ => {
        // Forward but don't act
    }
}

// Always forward to external clients via broadcast
broadcast.send((server_id, notification));
```

### 7.3 WebSocket Client Endpoint (`/ws`)

```rust
pub async fn mcp_websocket_handler(ws: WebSocketUpgrade, ...) -> Response {
    // 1. Authenticate client
    // 2. Get allowed_servers from client config
    // 3. Subscribe to broadcast channel
    // 4. Forward matching notifications (filter by allowed_servers)

    let mut notification_rx = state.mcp_notification_broadcast.subscribe();

    loop {
        if let Ok((server_id, notification)) = notification_rx.recv().await {
            // Only forward if client has access to this server
            if allowed_servers.contains(&server_id) {
                let msg = json!({
                    "server_id": server_id,
                    "notification": notification,
                });
                sender.send(Message::Text(msg)).await;
            }
        }
    }
}
```

---

## 8. Deferred Loading

### 8.1 Purpose

When a gateway connects to many MCP servers with hundreds of tools, sending all tools in the initial `tools/list` response:
- Consumes excessive context in LLM conversations
- Increases latency
- May exceed token limits

**Deferred loading** solves this by:
1. Initially returning only a virtual "search" tool
2. Clients search for relevant tools on-demand
3. Matched tools are "activated" and appear in subsequent `tools/list` responses

### 8.2 Flow

```
1. Client sends initialize with { capabilities: { tools: { listChanged: true } } }

2. Gateway checks:
   - deferred_loading_requested (from client config)
   - client supports listChanged notifications

3. If both true, gateway:
   - Fetches full catalog from all backend servers
   - Stores in session.deferred_loading.full_catalog
   - Returns initialize with single "search" tool

4. Client calls tools/list:
   - Returns only "search" tool + any activated tools

5. Client calls tools/call with name="search", arguments={query: "file operations"}

6. Gateway:
   - Searches full_catalog using relevance scoring
   - Activates matching tools (adds to activated_tools set)
   - Returns search results with relevance scores

7. Client calls tools/list again:
   - Returns "search" + activated tools

8. Activated tools persist for session lifetime
```

### 8.3 Search Tool Definition

```rust
pub fn create_search_tool() -> NamespacedTool {
    NamespacedTool {
        name: "search",
        original_name: "search",
        server_id: "_gateway",  // Virtual gateway tool
        description: "Search for tools, resources, or prompts across all connected MCP servers...",
        input_schema: json!({
            "type": "object",
            "properties": {
                "query": { "type": "string" },
                "type": { "enum": ["tools", "resources", "prompts", "all"], "default": "all" },
                "limit": { "type": "integer", "default": 10, "max": 50 }
            },
            "required": ["query"]
        }),
    }
}
```

### 8.4 Relevance Scoring

```rust
fn calculate_relevance_score(query: &str, name: &str, description: &str) -> f32 {
    let keywords: Vec<&str> = query.to_lowercase().split_whitespace().collect();
    let mut score = 0.0;

    for keyword in &keywords {
        if name == *keyword {
            score += 5.0;          // Exact name match: highest
        } else if name.contains(keyword) {
            score += 3.0;          // Partial name match: high
        } else if description.contains(keyword) {
            score += 1.0;          // Description match: medium
        }
    }

    score / keywords.len() as f32  // Normalize
}

// Activation thresholds:
const HIGH_RELEVANCE_THRESHOLD: f32 = 0.7;  // Always activate
const LOW_RELEVANCE_THRESHOLD: f32 = 0.3;   // Activate if < MIN needed
const MIN_ACTIVATIONS: usize = 3;           // Activate at least 3
```

---

## 9. Special Features

### 9.1 Sampling Support (`sampling/createMessage`)

Enables backend MCP servers to request LLM completions through the gateway.

**Important**: Sampling is handled at the **route handler level** (`mcp.rs`), NOT in the `McpGateway` struct. The request is intercepted before reaching the gateway logic.

```rust
// In mcp.rs route handler (both unified and per-server):
match request.method.as_str() {
    "sampling/createMessage" => {
        // 1. Check if sampling enabled for this client
        if !client.mcp_sampling_enabled {
            return error("Sampling disabled");
        }

        // 2. Parse MCP sampling request format
        let sampling_req: SamplingRequest = parse(request.params);

        // 3. Convert to chat completion format
        let completion_req = convert_sampling_to_chat_request(sampling_req);

        // 4. Default model to auto-routing
        if completion_req.model.is_empty() {
            completion_req.model = "localrouter/auto";
        }

        // 5. Route through LLM router
        let completion_resp = router.complete(&client_id, completion_req).await;

        // 6. Convert back to MCP sampling response
        let sampling_resp = convert_chat_to_sampling_response(completion_resp);

        return JsonRpcResponse::success(sampling_resp);
    }
    // ... other methods go to McpGateway
}
```

**Note**: If `sampling/createMessage` reaches `McpGateway.handle_direct_request()`, it returns an error suggesting to use the individual server proxy endpoint instead. This should not happen in normal operation since the route handler intercepts it first.

### 9.2 Elicitation Support (`elicitation/requestInput`)

Enables backend MCP servers to request structured user input.

**Important**: Elicitation behavior differs between endpoints:
- **Unified gateway (`/`)**: Fully supported via `McpGateway.handle_elicitation_request()`
- **Per-server proxy (`/mcp/{server_id}`)**: Returns "not implemented" error

```rust
// ElicitationManager handles async user input requests:
pub async fn request_input(
    &self,
    server_id: String,
    request: ElicitationRequest,  // { message, schema }
    timeout_secs: Option<u64>,
) -> AppResult<ElicitationResponse> {
    // 1. Create unique request ID
    // 2. Create oneshot channel for response
    // 3. Broadcast notification to external clients via WebSocket
    // 4. Wait for response (with timeout, default 120s)
    // 5. Return response or timeout error
}

// External clients submit responses via:
POST /mcp/elicitation/respond/{request_id}
```

**Note**: The per-server proxy returns an error suggesting WebSocket infrastructure is required. This is because elicitation works best with the unified gateway where the `ElicitationManager` is properly integrated.

### 9.3 Roots Support (`roots/list`)

Returns configured filesystem roots (advisory boundaries). Handled as a **direct method** (not broadcast).

```rust
"roots/list" => {
    let roots = merge_roots(&global_roots, client.roots);
    return json!({
        "roots": roots.iter()
            .filter(|r| r.enabled)
            .map(|r| { "uri": r.uri, "name": r.name })
    });
}
```

### 9.4 Resource Subscriptions (`resources/subscribe`, `resources/unsubscribe`)

The gateway supports subscribing to resource change notifications:

```rust
// Subscribe to a resource
"resources/subscribe" => {
    // 1. Extract URI from params
    // 2. Look up server_id from resource_uri_mapping
    // 3. Check if server supports subscriptions (capabilities.resources.subscribe)
    // 4. Forward to backend server
    // 5. Track subscription in session.subscribed_resources
}

// Unsubscribe from a resource
"resources/unsubscribe" => {
    // 1. Extract URI from params
    // 2. Look up server_id from subscribed_resources
    // 3. Forward to backend server
    // 4. Remove from session tracking
}
```

**Session tracking**: Subscriptions are stored in `session.subscribed_resources: HashMap<String, String>` (uri → server_id).

### 9.5 Other Direct Methods

| Method | Behavior |
|--------|----------|
| `completion/complete` | Error: "This is a client capability. Servers request this from clients." |

---

## 10. API Endpoints

### 10.1 Unified Gateway Endpoints (MCP SSE Transport)

The gateway implements the MCP SSE transport specification:

| Method | Path | Description |
|--------|------|-------------|
| `GET` | `/` | SSE stream (if Accept: text/event-stream) or API info |
| `POST` | `/` | Unified MCP gateway - routes to all servers |
| `GET` | `/mcp/{server_id}` | SSE stream for specific server |
| `POST` | `/mcp/{server_id}` | Direct proxy to specific server |
| `POST` | `/mcp/{server_id}/stream` | Streaming proxy (SSE response) |
| `GET` | `/ws` | WebSocket for real-time notifications |
| `POST` | `/mcp/elicitation/respond/{request_id}` | Submit elicitation response |

**SSE Event Format** (MCP SSE Transport Spec):
- `endpoint` event: Data is just the endpoint path (e.g., `/` or `/mcp/filesystem`)
- `message` event: Data is raw JSON-RPC (not wrapped in envelope)

```
event: endpoint
data: /

event: message
data: {"jsonrpc":"2.0","id":1,"result":{...}}
```

### 10.2 Authentication

All endpoints require Bearer token authentication:
```
Authorization: Bearer lr-xxx...
```

Client must have appropriate `mcp_server_access` configured:
- `All` - Access to all configured MCP servers
- `Specific([...])` - Whitelist of allowed servers
- `None` - No MCP access

---

## 11. Configuration

### 11.1 Gateway Configuration

```rust
pub struct GatewayConfig {
    pub session_ttl_seconds: u64,      // Default: 3600 (1 hour)
    pub server_timeout_seconds: u64,   // Default: 10
    pub allow_partial_failures: bool,  // Default: true
    pub cache_ttl_seconds: u64,        // Default: 300 (5 minutes)
    pub max_retry_attempts: u8,        // Default: 1
}
```

### 11.2 Client Configuration (per-client)

```yaml
clients:
  - client_id: "my-client"
    mcp_server_access: "all"  # or specific: ["filesystem", "github"]
    mcp_sampling_enabled: true
    mcp_deferred_loading: true
    roots:
      - uri: "file:///home/user/projects"
        name: "Projects"
```

### 11.3 MCP Server Configuration

```yaml
mcp_servers:
  - id: "filesystem"
    name: "Filesystem Server"
    transport: "stdio"
    transport_config:
      command: "npx"
      args: ["-y", "@modelcontextprotocol/server-filesystem"]
      env: {}

  - id: "remote-api"
    name: "Remote API Server"
    transport: "sse"
    transport_config:
      url: "https://api.example.com/mcp"
      headers:
        X-API-Key: "secret"
    auth_config:
      type: "bearer_token"
      token_ref: "remote-api_bearer_token"
```

---

## 12. Known Bugs & Issues

### 12.1 ~~BUG: Elicitation Server ID Wrong~~ (FIXED)

**File**: `src/mcp/gateway/gateway.rs`

**Issue**: In `handle_elicitation_request()`, the code incorrectly used `session_read.client_id.clone()` as the `server_id`.

**Status**: FIXED - Now extracts `server_id` from request params, falling back to `"_gateway"` to indicate the request came through the unified gateway. Added documentation explaining the architectural considerations.

### 12.2 ~~BUG: Streaming Session Notification Forwarding Not Implemented~~ (FIXED)

**File**: `src/mcp/gateway/streaming.rs`

**Issue**: The `start_backend_notification_forwarding()` method was a placeholder.

**Status**: FIXED - Now registers notification handlers with the `McpServerManager` to forward notifications to the streaming session's event channel. Handler IDs are tracked in `notification_handler_ids: DashMap<String, u64>` and cleaned up when sessions close via `cleanup_handlers()`.

### 12.3 RESOLVED: Streaming Broadcast Request ID (Not a Bug)

**File**: `src/mcp/gateway/streaming.rs:289-290`

**Analysis**: The modified request ID is only used for the outgoing POST request to backend servers. Since `server_manager.send_request()` is synchronous (awaits response), the response is correlated directly without needing ID matching. The `StreamingEvent::Response` correctly uses the original `request_id`. Not a bug.

### 12.4 ~~ISSUE: Missing Initialize/Ping in Streaming Routing~~ (FIXED)

**File**: `src/mcp/gateway/streaming.rs`

**Issue**: The `parse_routing()` method only handled list methods for broadcast.

**Status**: FIXED - Added `initialize`, `ping`, and `logging/setLevel` to the broadcast method check.

### 12.5 ~~ISSUE: SSE Message Format Did Not Follow MCP Spec~~ (FIXED)

**File**: `src/server/routes/mcp.rs`

**Issue**: The SSE message events were wrapping JSON-RPC in an `SseMessage` enum, producing output like:
```
event: message
data: {"Response":{"jsonrpc":"2.0","id":1,"result":{...}}}
```

**Status**: FIXED - SSE message events now send raw JSON-RPC per the MCP SSE transport spec:
```
event: message
data: {"jsonrpc":"2.0","id":1,"result":{...}}
```

The endpoint event was already correct (sending just the path string).

### 12.6 ~~CLEANUP: Removed Redundant Streaming Session Endpoints~~ (DONE)

**Files Removed**:
- `src/server/routes/mcp_streaming.rs`
- `src/mcp/gateway/streaming.rs`

**Issue**: The session-based streaming endpoints (`/gateway/stream/*`) duplicated functionality already provided by the unified gateway's SSE support at `/` and `/mcp/{server_id}`.

**Status**: DONE - Removed redundant endpoints. The proper MCP SSE transport is now at `/` and `/mcp/{server_id}`.

### 12.7 ISSUE: SSE Transport Validation Conflates Connect with Initialize

**File**: `src/mcp/transport/sse.rs:125-175`

**Issue**: `SseTransport::connect()` sends an MCP `initialize` request as part of connection validation. This means:
1. The backend server is initialized before the client sends their `initialize`
2. Client capabilities aren't passed to the backend during actual initialization

**Impact**: Backend servers may miss client capability negotiation.

**Recommendation**: Consider separating transport connection validation from MCP initialization.

### 12.8 ~~LIMITATION: Notification Handler Cleanup~~ (FIXED)

**File**: `src/mcp/manager.rs`

**Issue**: The `on_notification()` method allowed registering handlers but there was no way to remove them.

**Status**: FIXED - Added `remove_notification_handler(server_id, handler_id)` and `clear_notification_handlers(server_id)` methods to `McpServerManager`.

---

## Appendix: Data Structures Reference

### ServerFailure
```rust
pub struct ServerFailure {
    pub server_id: String,
    pub error: String,
}
```

### NamespacedTool
```rust
pub struct NamespacedTool {
    pub name: String,           // "filesystem__read_file"
    pub original_name: String,  // "read_file" (not serialized)
    pub server_id: String,      // "filesystem" (not serialized)
    pub description: Option<String>,
    pub input_schema: Value,
}
```

### SseMessage (for client→gateway SSE)
```rust
pub enum SseMessage {
    Response(JsonRpcResponse),      // JSON-RPC response
    Notification(JsonRpcNotification), // JSON-RPC notification
    Endpoint { endpoint: String },   // Endpoint info (internal)
}
// Note: When serialized to SSE, Response and Notification send raw JSON-RPC,
// not the wrapper enum (per MCP SSE transport spec).
```

---

**Document Version**: 1.0
**Author**: Claude (automated review)
**Last Updated**: 2026-01-23
