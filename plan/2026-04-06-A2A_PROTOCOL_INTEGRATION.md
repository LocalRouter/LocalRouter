# A2A Protocol Integration Plan

## Context

LocalRouter currently supports LLM routing and MCP (Model Context Protocol) as its two main protocol pillars. The A2A (Agent-to-Agent) protocol is a new open standard (v1.0.0, under Linux Foundation/LFAI) for inter-agent communication. Adding A2A support makes LocalRouter a unified gateway for all three agent communication paradigms: LLM inference, MCP tools/resources, and A2A agent collaboration.

**Goal**: Full A2A support — both as a client (connecting to external A2A agents) and as a server (exposing a unified A2A agent that routes to downstream agents). Plus marketplace, discovery, UI, monitoring, and bridging (A2A via MCP).

---

## Phase 1: Foundation — Protocol Types & Config

**Goal**: Rust data types for the A2A protocol and configuration storage for A2A agents.

### 1.1 New Crate: `crates/lr-a2a/`

Create a new crate mirroring `crates/lr-mcp/` structure:

```
crates/lr-a2a/
├── Cargo.toml
└── src/
    ├── lib.rs
    ├── protocol.rs      # A2A protocol types
    ├── agent_card.rs     # Agent Card types & parsing
    ├── discovery.rs      # Well-known discovery client
    ├── client.rs         # JSON-RPC HTTP client (Phase 2)
    ├── gateway/          # Gateway (Phase 3)
    └── manager.rs        # Agent lifecycle management (Phase 2)
```

### 1.2 Protocol Types (`protocol.rs`)

All A2A protocol structs derived from the spec's protobuf definitions, serialized as JSON with camelCase field names:

**Core types:**
- `Task` — id, context_id, status, artifacts, history, metadata
- `TaskState` enum — Submitted, Working, Completed, Failed, Canceled, InputRequired, Rejected, AuthRequired
- `TaskStatus` — state, message, timestamp
- `Message` — message_id, context_id, task_id, role (User/Agent), parts, metadata, extensions, reference_task_ids
- `Part` — enum: Text(String), Raw(bytes), Url(String), Data(serde_json::Value) + metadata, filename, media_type
- `Artifact` — artifact_id, name, description, parts, metadata, extensions
- `Role` enum — User, Agent

**Streaming types:**
- `TaskStatusUpdateEvent` — task_id, context_id, status, metadata
- `TaskArtifactUpdateEvent` — task_id, context_id, artifact, append, last_chunk, metadata
- `StreamResponse` — enum: Task, Message, StatusUpdate, ArtifactUpdate
- `SendMessageResponse` — enum: Task, Message

**Request types:**
- `SendMessageRequest` — tenant, message, configuration, metadata
- `SendMessageConfiguration` — accepted_output_modes, push_notification_config, history_length, return_immediately
- `GetTaskRequest` — tenant, id, history_length
- `ListTasksRequest` — tenant, context_id, status, page_size, page_token, history_length, status_timestamp_after, include_artifacts
- `ListTasksResponse` — tasks, next_page_token, page_size, total_size
- `CancelTaskRequest` — tenant, id, metadata
- `SubscribeToTaskRequest` — tenant, id

**JSON-RPC wrapper types:**
- `A2aJsonRpcRequest` — jsonrpc, id, method (PascalCase), params
- `A2aJsonRpcResponse` — jsonrpc, id, result/error
- `A2aJsonRpcError` — code, message, data

**Error codes** (constants):
- `-32001` TaskNotFound through `-32009` VersionNotSupported

### 1.3 Agent Card Types (`agent_card.rs`)

- `AgentCard` — name, description, supported_interfaces, provider, version, capabilities, security_schemes, security_requirements, default_input_modes, default_output_modes, skills, icon_url
- `AgentInterface` — url, protocol_binding (JSONRPC/GRPC/HTTP+JSON), tenant, protocol_version
- `AgentCapabilities` — streaming, push_notifications, extensions, extended_agent_card
- `AgentProvider` — url, organization
- `AgentSkill` — id, name, description, tags, examples, input_modes, output_modes, security_requirements
- `SecurityScheme` — enum: ApiKey, HttpAuth, OAuth2, OpenIdConnect, MutualTls (each with their sub-types)
- `SecurityRequirement` — schemes map (name → scopes)
- `AgentCardSignature` — protected, signature, header

### 1.4 Config Types

Add to `crates/lr-config/src/types.rs`:

```rust
pub struct A2aAgentConfig {
    pub id: String,                           // UUID
    pub name: String,                         // Human-readable name
    pub url: String,                          // Base URL for discovery
    pub endpoint_url: Option<String>,         // JSON-RPC endpoint (from agent card)
    pub agent_card: Option<AgentCard>,        // Cached agent card
    pub auth_config: Option<McpAuthConfig>,   // Reuse MCP auth system
    pub enabled: bool,
    pub created_at: DateTime<Utc>,
}
```

Add to `AppConfig`:
```rust
pub a2a_agents: Vec<A2aAgentConfig>,
```

### 1.5 Client Capabilities (replacing ClientMode)

Replace the `ClientMode` enum with a flags-based struct:

```rust
pub struct ClientCapabilities {
    pub llm_enabled: bool,       // LLM routing access
    pub mcp_enabled: bool,       // MCP proxy access
    pub a2a_enabled: bool,       // A2A agent access
    pub mcp_via_llm: bool,       // Inject MCP tools into LLM (sub-toggle)
    pub a2a_via_mcp: bool,       // Expose A2A as MCP tools (sub-toggle)
}
```

**Migration**: Map old `ClientMode` enum values:
- `Both` → `{llm: true, mcp: true, a2a: false, ...}`
- `LlmOnly` → `{llm: true, mcp: false, a2a: false, ...}`
- `McpOnly` → `{llm: false, mcp: true, a2a: false, ...}`
- `McpViaLlm` → `{llm: true, mcp: true, a2a: false, mcp_via_llm: true, ...}`

Keep old enum as `#[serde(alias)]` for backward compat deserialization.

### 1.6 A2A Permissions

Add to `Client` struct:
```rust
pub a2a_permissions: A2aPermissions,
```

```rust
pub struct A2aPermissions {
    pub global: PermissionState,                          // All agents
    pub agents: HashMap<String, PermissionState>,         // agent_id -> state
}
```

Resolution: agent-level → global fallback. No per-skill granularity.

### Critical Files
- `crates/lr-a2a/src/protocol.rs` (new)
- `crates/lr-a2a/src/agent_card.rs` (new)
- `crates/lr-config/src/types.rs` (modify: add A2aAgentConfig, ClientCapabilities, A2aPermissions)
- `crates/lr-config/src/migration.rs` (modify: ClientMode → ClientCapabilities migration)
- `Cargo.toml` workspace (add lr-a2a)

---

## Phase 2: A2A Client — Connecting to External Agents

**Goal**: HTTP client for A2A JSON-RPC, well-known discovery, health checks, and Tauri commands.

### 2.1 Well-Known Discovery (`discovery.rs`)

```rust
pub async fn discover_agent(base_url: &str) -> Result<AgentCard, A2aError> {
    // GET {base_url}/.well-known/agent-card.json
    // Parse response as AgentCard
    // Validate required fields
    // Extract JSON-RPC endpoint from supported_interfaces (prefer JSONRPC binding)
}
```

Discovery is the ONLY way to add agents — no manual form. User enters a URL, we fetch the agent card. If `/.well-known/agent-card.json` is not found, return an error.

### 2.2 A2A JSON-RPC Client (`client.rs`)

HTTP client using `reqwest` for A2A JSON-RPC 2.0:

```rust
pub struct A2aClient {
    http_client: reqwest::Client,
    endpoint_url: String,        // From agent card's supported_interfaces
    auth: Option<AuthConfig>,
}
```

**Methods:**
- `send_message(req: SendMessageRequest) -> Result<SendMessageResponse>` — POST JSON-RPC with method `SendMessage`
- `send_streaming_message(req: SendMessageRequest) -> Result<impl Stream<Item = StreamResponse>>` — POST JSON-RPC, parse SSE response
- `get_task(req: GetTaskRequest) -> Result<Task>`
- `list_tasks(req: ListTasksRequest) -> Result<ListTasksResponse>`
- `cancel_task(req: CancelTaskRequest) -> Result<Task>`
- `subscribe_to_task(req: SubscribeToTaskRequest) -> Result<impl Stream<Item = StreamResponse>>`

**HTTP details:**
- All requests: `POST {endpoint_url}`, `Content-Type: application/json`
- Headers: `A2A-Version: 1.0`, auth headers from config
- Streaming responses: Parse `text/event-stream` response, yield `StreamResponse` objects from `data:` lines
- Errors: Parse JSON-RPC error format, map A2A error codes to typed errors

### 2.3 Agent Manager (`manager.rs`)

```rust
pub struct A2aAgentManager {
    agents: HashMap<String, A2aAgentState>,  // agent_id -> state
    clients: HashMap<String, A2aClient>,      // agent_id -> client
}

pub struct A2aAgentState {
    pub config: A2aAgentConfig,
    pub health: A2aAgentHealth,
    pub agent_card: Option<AgentCard>,       // Cached card
    pub last_card_fetch: Option<DateTime<Utc>>,
}
```

**Health checks:**
- Periodic `GET /.well-known/agent-card.json` fetch
- If responds → healthy (also detects capability changes)
- If fails → unhealthy
- Emit `a2a-health-check` Tauri events (mirror MCP pattern)

### 2.4 Tauri Commands (`src-tauri/src/ui/commands_a2a.rs`)

**Agent CRUD:**
- `list_a2a_agents()` → `Vec<A2aAgentInfo>`
- `discover_a2a_agent(url: String)` → `A2aAgentDiscoveryResult` (fetches card, returns preview)
- `add_a2a_agent(url: String)` → `A2aAgentInfo` (discover + save to config)
- `delete_a2a_agent(agent_id: String)`
- `update_a2a_agent_name(agent_id: String, name: String)`
- `toggle_a2a_agent_enabled(agent_id: String, enabled: bool)`

**Health:**
- `get_a2a_agent_health(agent_id: String)` → `A2aAgentHealth`
- `get_all_a2a_agent_health()` → `HashMap<String, A2aAgentHealth>`
- `start_a2a_health_checks()`
- `check_single_a2a_health(agent_id: String)`
- `refresh_a2a_agent_card(agent_id: String)` → `AgentCard` (force re-fetch)

**Operations:**
- `a2a_send_message(agent_id: String, message: String, task_id: Option<String>, context_id: Option<String>)` → `A2aSendMessageResult`
- `a2a_get_task(agent_id: String, task_id: String)` → `Task`
- `a2a_list_tasks(agent_id: Option<String>)` → `Vec<Task>`
- `a2a_cancel_task(agent_id: String, task_id: String)` → `Task`

### Critical Files
- `crates/lr-a2a/src/discovery.rs` (new)
- `crates/lr-a2a/src/client.rs` (new)
- `crates/lr-a2a/src/manager.rs` (new)
- `src-tauri/src/ui/commands_a2a.rs` (new)
- `src-tauri/src/ui/mod.rs` (modify: register commands)
- `src/types/tauri-commands.ts` (modify: add A2A types)
- `website/src/components/demo/TauriMockSetup.ts` (modify: add mocks)

---

## Phase 3: A2A Gateway — Unified Routing Agent

**Goal**: A gateway that merges multiple downstream A2A agents into a single unified A2A interface with message-based routing.

### 3.1 Gateway Architecture (`crates/lr-a2a/src/gateway/`)

```
gateway/
├── mod.rs
├── gateway.rs          # Core A2aGateway
├── router.rs           # Message routing ("To: agent-name")
├── task_tracker.rs     # Track tasks per downstream agent
├── card_builder.rs     # Build unified Agent Card
└── session.rs          # Per-session state
```

### 3.2 Core Gateway (`gateway.rs`)

```rust
pub struct A2aGateway {
    agent_manager: Arc<A2aAgentManager>,
    task_tracker: Arc<TaskTracker>,
    config: A2aGatewayConfig,
}
```

**Request flow:**
1. Receive `SendMessageRequest`
2. Parse message for "To: agent-slug" prefix (first line or metadata)
3. If no target specified or ambiguous → return error task with `TASK_STATE_INPUT_REQUIRED` and a message listing all available agents with their full agent cards
4. Route to target agent's A2A client
5. Track the task: map gateway task_id → (downstream_agent_id, downstream_task_id)
6. Return response (task/message)

### 3.3 Message Router (`router.rs`)

```rust
pub struct MessageRouter;

impl MessageRouter {
    /// Parse target agent from message.
    /// Format: First text part starts with "To: <agent-slug>\n" or "To: <agent-slug> "
    /// Returns (agent_slug, cleaned_message_without_prefix)
    pub fn parse_target(message: &Message) -> Option<(String, Message)>;

    /// Build error message listing all available agents
    pub fn build_disambiguation_message(agents: &[AgentCard]) -> Message;
}
```

**Routing rules:**
- Always require explicit "To: agent-name" target
- Agent name matched against agent slug (derived from config name, lowercased, hyphenated)
- If target not found → error with available agents list
- If no target specified → error with full agent cards of all available agents
- The "To:" prefix is stripped before forwarding to downstream agent

### 3.4 Task Tracker (`task_tracker.rs`)

```rust
pub struct TaskTracker {
    /// Map: gateway_task_id -> TaskMapping
    tasks: DashMap<String, TaskMapping>,
}

pub struct TaskMapping {
    pub gateway_task_id: String,
    pub downstream_agent_id: String,
    pub downstream_task_id: String,
    pub context_id: Option<String>,
    pub created_at: DateTime<Utc>,
}
```

When the gateway proxies a `SendMessage`:
1. Receive downstream task response with downstream `task_id`
2. Generate a gateway-level `task_id` (or reuse if follow-up)
3. Store mapping
4. Return task to caller with gateway `task_id`

When `GetTask`/`CancelTask` is called with gateway task_id:
1. Look up mapping
2. Forward to correct downstream agent with downstream task_id
3. Return result with gateway task_id

### 3.5 Unified Agent Card Builder (`card_builder.rs`)

Build a combined `AgentCard` from all enabled downstream agents:

```rust
pub fn build_unified_card(
    agents: &[A2aAgentConfig],
    agent_cards: &HashMap<String, AgentCard>,
    server_config: &ServerConfig,
) -> AgentCard {
    AgentCard {
        name: "LocalRouter A2A Gateway",
        description: "Unified gateway routing to multiple A2A agents",
        supported_interfaces: vec![
            AgentInterface {
                url: format!("http://localhost:{}/a2a", port),
                protocol_binding: "JSONRPC",
                protocol_version: "1.0",
            }
        ],
        capabilities: AgentCapabilities {
            streaming: true,  // if any downstream supports it
            push_notifications: false,  // not initially
            extended_agent_card: false,
        },
        skills: collect_all_skills(agents, agent_cards),
        // Skills listed as-is from each downstream agent, NO namespacing
        // Each skill's description includes "[agent-name] " prefix for attribution
        default_input_modes: vec!["text/plain", "application/json"],
        default_output_modes: union_of_all_output_modes(agent_cards),
        ..
    }
}
```

**Skill attribution**: Each skill's description is prefixed with `[agent-slug]` so callers know which agent it belongs to. Skill IDs are NOT namespaced (per design decision). If duplicate skill IDs exist across agents, append `-{agent-slug}` suffix only where needed.

### 3.6 Session Management (`session.rs`)

```rust
pub struct A2aGatewaySession {
    pub session_id: String,
    pub client_id: String,
    pub allowed_agents: Vec<String>,   // Agent IDs this client can access
    pub task_mappings: HashMap<String, TaskMapping>,
}
```

### Critical Files
- `crates/lr-a2a/src/gateway/gateway.rs` (new)
- `crates/lr-a2a/src/gateway/router.rs` (new)
- `crates/lr-a2a/src/gateway/task_tracker.rs` (new)
- `crates/lr-a2a/src/gateway/card_builder.rs` (new)
- `crates/lr-a2a/src/gateway/session.rs` (new)

---

## Phase 4: A2A Server — Exposing LocalRouter as an Agent

**Goal**: Serve A2A JSON-RPC endpoints so external callers can interact with LocalRouter as an A2A agent.

### 4.1 Endpoint Registration

Add A2A routes in `crates/lr-server/src/lib.rs`:

**Dedicated `/a2a` prefix (primary):**
- `POST /a2a` → `a2a_rpc_handler()` — JSON-RPC 2.0 dispatcher
- `GET /.well-known/agent-card.json` → `agent_card_handler()` — Public agent card

**Root sharing with MCP (secondary):**
- `POST /` — Modify existing handler to detect A2A vs MCP by method name:
  - PascalCase methods (`SendMessage`, `GetTask`, etc.) → route to A2A handler
  - lowercase/slash methods (`initialize`, `tools/list`, etc.) → route to MCP handler

**No GET / conflict**: MCP uses GET / for SSE connections. A2A streaming is per-request (POST returns SSE), so no conflict.

### 4.2 JSON-RPC Dispatcher (`crates/lr-server/src/routes/a2a.rs`)

```rust
pub async fn a2a_rpc_handler(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(request): Json<A2aJsonRpcRequest>,
) -> Response {
    // Validate A2A-Version header (default to "1.0")
    // Dispatch by method:
    match request.method.as_str() {
        "SendMessage" => handle_send_message(state, request).await,
        "SendStreamingMessage" => handle_send_streaming_message(state, request).await,
        "GetTask" => handle_get_task(state, request).await,
        "ListTasks" => handle_list_tasks(state, request).await,
        "CancelTask" => handle_cancel_task(state, request).await,
        "SubscribeToTask" => handle_subscribe_to_task(state, request).await,
        "GetExtendedAgentCard" => handle_get_extended_card(state, request).await,
        _ => json_rpc_error(-32601, "Method not found"),
    }
}
```

**Streaming responses** (`SendStreamingMessage`, `SubscribeToTask`):
- Return `Content-Type: text/event-stream`
- Forward SSE events from downstream agent through the gateway
- Each `data:` line is a JSON-RPC response wrapping a `StreamResponse`

### 4.3 Agent Card Endpoint

```rust
pub async fn agent_card_handler(State(state): State<AppState>) -> Json<AgentCard> {
    // Build unified card from all enabled downstream agents
    // No auth required (public endpoint per spec)
}
```

Served at `GET /.well-known/agent-card.json` (root level, no prefix).

### 4.4 Auth for A2A Server

A2A server endpoints use the same `client_auth_middleware` as MCP:
- Callers authenticate with client credentials (OAuth token or client secret)
- The client's `a2a_permissions` determine which downstream agents they can access
- Agent card endpoint is public (no auth)

### 4.5 Root POST Multiplexer

Modify the existing `POST /` handler to detect protocol:

```rust
pub async fn unified_post_handler(/* ... */) -> Response {
    // Parse JSON body
    let value: serde_json::Value = /* ... */;

    // Check if it's a JSON-RPC request with a method field
    if let Some(method) = value.get("method").and_then(|m| m.as_str()) {
        // A2A methods are PascalCase: SendMessage, GetTask, etc.
        if method.chars().next().map(|c| c.is_uppercase()).unwrap_or(false) {
            return a2a_rpc_handler(state, value).await;
        }
        // MCP methods are lowercase/slash: initialize, tools/list, etc.
        return mcp_gateway_handler(state, value).await;
    }

    // If it has result/error, it's a JSON-RPC response (MCP passthrough)
    return mcp_response_handler(state, value).await;
}
```

### Critical Files
- `crates/lr-server/src/routes/a2a.rs` (new)
- `crates/lr-server/src/routes/mod.rs` (modify: add a2a module)
- `crates/lr-server/src/lib.rs` (modify: register routes, add multiplexer)
- `crates/lr-server/src/routes/mcp.rs` (modify: extract POST body parsing for sharing)

---

## Phase 5: A2A via MCP Bridge

**Goal**: Expose A2A agent capabilities as MCP tools so MCP-only clients can interact with A2A agents.

### 5.1 Virtual MCP Server for A2A (`crates/lr-mcp/src/gateway/virtual_a2a.rs`)

Create a new virtual MCP server (like virtual_skills, virtual_marketplace) that exposes A2A operations as MCP tools:

**Tools exposed:**

1. **`a2a_list_agents`**
   - Description: "List all available A2A agents and their capabilities"
   - Input: `{}` (no params)
   - Output: JSON array of agent summaries (name, slug, description, skills)

2. **`a2a_send_message`**
   - Description: "Send a message to an A2A agent. Use 'agent' to specify target."
   - Input: `{ agent: string, message: string, task_id?: string, context_id?: string }`
   - Output: Task result (status, artifacts, messages)
   - Behavior: Calls `SendMessage` on target agent. Blocks until terminal state or `INPUT_REQUIRED`. Returns full task.

3. **`a2a_get_task`**
   - Description: "Get the status and result of an A2A task"
   - Input: `{ task_id: string }`
   - Output: Task object with status and artifacts

4. **`a2a_list_tasks`**
   - Description: "List A2A tasks, optionally filtered by agent or context"
   - Input: `{ agent?: string, context_id?: string }`
   - Output: Array of task summaries

5. **`a2a_cancel_task`**
   - Description: "Cancel a running A2A task"
   - Input: `{ task_id: string }`
   - Output: Updated task with canceled status

**Multi-turn flow via MCP:**
1. Client calls `a2a_send_message` → agent returns `INPUT_REQUIRED`
2. Tool returns the task with status and the agent's question
3. Client calls `a2a_send_message` again with same `task_id` and the answer
4. Agent completes → tool returns final result

### 5.2 Registration

Register `VirtualA2aMcpServer` in the MCP gateway alongside existing virtual servers (skills, marketplace, coding agents, memory). Only active when client has `a2a_via_mcp: true` in their capabilities.

### 5.3 Tool Descriptions

Tool descriptions include dynamic content from available A2A agents:
```
a2a_send_message: Send a message to an A2A agent.

Available agents:
- route-planner: Optimizes travel routes with traffic awareness
- translator: Multi-language translation with context preservation
- code-reviewer: Automated code review and suggestions

Use the 'agent' parameter to specify the target agent slug.
```

### Critical Files
- `crates/lr-mcp/src/gateway/virtual_a2a.rs` (new)
- `crates/lr-mcp/src/gateway/mod.rs` (modify: register virtual server)
- `crates/lr-mcp/src/gateway/gateway.rs` (modify: include A2A virtual server in session)

---

## Phase 6: Frontend UI

**Goal**: Full UI for A2A agent management, client configuration, and visualization.

### 6.1 Sidebar Navigation

Add new top-level section "A2A Agents" in `src/components/layout/sidebar.tsx`:
- Position: After "MCP Servers" section
- Icon: Consistent with other sections
- Dynamic children: List of configured A2A agents (like MCP servers show dynamic children)

Update `View` type to include `a2a-agents` view.

### 6.2 A2A Agents View (`src/views/a2a-agents/`)

```
src/views/a2a-agents/
├── index.tsx              # Main view with agent list
├── agent-detail.tsx       # Agent detail panel (card info, skills, health)
├── add-agent-dialog.tsx   # URL discovery dialog
└── a2a-settings-panel.tsx # Gateway-level A2A settings
```

**Agent List:**
- Shows all configured A2A agents with health status indicators (green/yellow/red)
- Agent name, description (from card), skill count, capability badges (streaming, push)
- Enable/disable toggle per agent
- Add agent button → opens discovery dialog

**Add Agent Dialog:**
- Single URL input field
- "Discover" button → fetches `/.well-known/agent-card.json`
- Shows agent card preview (name, description, provider, skills list, capabilities)
- "Add" button to save

**Agent Detail Panel:**
- Agent Card info (name, description, provider, version)
- Skills list with descriptions and tags
- Supported interfaces (protocol bindings)
- Security requirements
- Capabilities (streaming, push notifications)
- Health status and last check time
- "Refresh Card" button
- Delete agent

### 6.3 Client Configuration Updates

**Client Mode Selector** (`src/components/client/ClientModeSelector.tsx`):
Replace the 4-option mode selector with toggles:

```
Client Capabilities:
┌─────────────────────────────────┐
│ [✓] LLM Access                  │
│ [✓] MCP Access                  │
│   [✓] MCP via LLM              │
│ [✓] A2A Access                  │
│   [✓] A2A via MCP              │
└─────────────────────────────────┘
```

- Sub-toggles only visible when parent is enabled
- "MCP via LLM" requires both LLM and MCP enabled
- "A2A via MCP" requires both A2A and MCP enabled
- At least one of LLM/MCP/A2A must be enabled

**New Client Tab: A2A** (`src/views/clients/tabs/a2a-tab.tsx`):
- A2A permission tree (global + per-agent toggles)
- Simple allow/ask/off per agent
- Shows agent names with health indicators

**Client Detail** (`src/views/clients/client-detail.tsx`):
- Add "a2a" tab alongside existing tabs
- Show A2A tab when client has `a2a_enabled: true`

### 6.4 Connection Graph Updates

**New node type**: `A2aAgentNode` (`src/components/connection-graph/nodes/A2aAgentNode.tsx`)
- Similar to McpServerNode
- Shows agent name, health status, skill count
- Click navigates to agent detail

**Graph layout** (`src/components/connection-graph/utils/buildGraph.ts`):
- Add A2A agents as a new column (right side, alongside MCP servers)
- Or below MCP servers in the same column
- Edges from clients to A2A agents based on permissions
- A2A endpoint node (like MCP endpoint node) in the router group

**Node types** update in `src/components/connection-graph/types.ts`:
- Add `A2aAgentNode` type
- Add `a2a` variant to `EndpointNode`

### 6.5 Dashboard Updates (`src/views/dashboard/`)

Add third tab: **A2A** (alongside LLM and MCP):
- Scope selector: global, per-client, per-agent
- Metrics: message count, task count, success/failure rates, latency
- Active tasks display

### 6.6 Monitor Updates (`src/views/monitor/`)

Add A2A event types:
- A2A Message Sent/Received
- A2A Task Created/Completed/Failed/Canceled
- A2A Agent Health Changed
- A2A Discovery events

### 6.7 Try-it-out Panel

Add A2A tab to Try-it-out view (`src/views/try-it-out/`):

```
src/views/try-it-out/a2a-tab/
├── index.tsx              # A2A testing interface
├── agent-selector.tsx     # Dropdown to pick target agent
├── message-panel.tsx      # Send messages, view responses
└── task-panel.tsx         # View task history, status
```

- Agent selector dropdown
- Message input (text area)
- Send button (blocking) / Stream button
- Response display with task status, artifacts
- Task history panel
- Multi-turn: if INPUT_REQUIRED, show follow-up input

### 6.8 TypeScript Types

Add to `src/types/tauri-commands.ts`:

```typescript
// A2A types
export interface A2aAgentInfo {
  id: string;
  name: string;
  url: string;
  endpoint_url: string | null;
  enabled: boolean;
  agent_card: AgentCard | null;
  created_at: string;
}

export interface AgentCard {
  name: string;
  description: string;
  supported_interfaces: AgentInterface[];
  provider: AgentProvider | null;
  version: string;
  capabilities: AgentCapabilities;
  skills: AgentSkill[];
  icon_url: string | null;
  // ... other fields
}

export interface AgentSkill {
  id: string;
  name: string;
  description: string;
  tags: string[];
  examples: string[];
}

export interface A2aAgentHealth {
  status: 'healthy' | 'unhealthy' | 'unknown';
  last_check: string | null;
  error: string | null;
  latency_ms: number | null;
}

export interface A2aPermissions {
  global: PermissionState;
  agents: Record<string, PermissionState>;
}

// Client capabilities (replacing ClientMode)
export interface ClientCapabilities {
  llm_enabled: boolean;
  mcp_enabled: boolean;
  a2a_enabled: boolean;
  mcp_via_llm: boolean;
  a2a_via_mcp: boolean;
}

// A2A operation types
export interface A2aSendMessageResult {
  task: A2aTask | null;
  message: A2aMessage | null;
}

export interface A2aTask {
  id: string;
  context_id: string | null;
  status: A2aTaskStatus;
  artifacts: A2aArtifact[];
  history: A2aMessage[];
}

// ... etc
```

### Critical Files
- `src/views/a2a-agents/` (new directory, multiple files)
- `src/views/clients/tabs/a2a-tab.tsx` (new)
- `src/components/client/ClientModeSelector.tsx` (rewrite)
- `src/components/layout/sidebar.tsx` (modify)
- `src/components/connection-graph/nodes/A2aAgentNode.tsx` (new)
- `src/components/connection-graph/utils/buildGraph.ts` (modify)
- `src/components/connection-graph/types.ts` (modify)
- `src/views/dashboard/index.tsx` (modify)
- `src/views/monitor/` (modify)
- `src/views/try-it-out/a2a-tab/` (new)
- `src/types/tauri-commands.ts` (modify)
- `website/src/components/demo/TauriMockSetup.ts` (modify)

---

## Phase 7: Marketplace, Templates & Discovery

**Goal**: Full marketplace for A2A agents with curated templates, registry integration, and discovery.

### 7.1 Marketplace Integration

Add A2A section to marketplace view (`src/views/marketplace/`):
- New tab: "A2A Agents" alongside existing MCP servers and Skills tabs
- Search A2A agents from registries
- One-click install (discover + add)

### 7.2 A2A Agent Templates

Curated list of popular/notable A2A agent types (based on ecosystem research):

**Template categories:**
- **Cloud Platform Agents**: Google ADK agents, Azure AI Foundry, AWS Bedrock
- **Framework Agents**: LangGraph, CrewAI, AG2/AutoGen, Semantic Kernel
- **Enterprise Agents**: ServiceNow AI Agents, Salesforce Agentforce
- **Utility Agents**: Translation, Code Review, Content Generation, Data Analysis
- **Sample Agents**: Official a2a-samples agents (Hello World, Currency, Image Gen, etc.)

Each template:
```typescript
interface A2aAgentTemplate {
  id: string;
  name: string;
  description: string;
  category: string;
  icon: string;
  default_url: string;           // Well-known URL to discover
  provider: string;
  tags: string[];
  documentation_url?: string;
  setup_instructions?: string;   // How to deploy/configure the agent
}
```

### 7.3 Registry Integration

Support querying known A2A registries for agent discovery:

**Primary registries:**
- [a2aregistry.org](https://a2aregistry.org) — Community-driven, has API + health checks
- [a2a-registry.org](https://www.a2a-registry.org) — Global registry with browse + API

**Config for registries:**
```rust
pub struct A2aRegistryConfig {
    pub url: String,
    pub enabled: bool,
    pub name: String,
}
```

Default registries pre-configured but user can add custom ones.

### 7.4 Discovery Flow

1. **From URL**: User enters any URL → try `/.well-known/agent-card.json` → show card preview → add
2. **From Marketplace Search**: Search registries → show results → click to discover → add
3. **From Template**: Pick template → auto-fill URL → discover → add

All three flows converge at the same discovery endpoint. If well-known fails, show error — no manual fallback.

### 7.5 A2A Settings

Global A2A settings panel (like MCP settings panel):
```
A2A Gateway Settings:
- Enable A2A Server (expose as agent): [toggle]
- Server port (if different from main): [input]
- Registries: [list with add/remove]
- Default auth for outgoing: [config]
```

### Critical Files
- `src/views/marketplace/index.tsx` (modify: add A2A tab)
- `src/views/marketplace/a2a-tab.tsx` (new)
- `src/components/a2a/A2aAgentTemplates.tsx` (new)
- `crates/lr-a2a/src/registry.rs` (new: registry client)
- `crates/lr-config/src/types.rs` (modify: add A2aRegistryConfig, marketplace settings)

---

## Phase 8: Monitoring, OpenAPI & Polish

**Goal**: Complete integration with monitoring, OpenAPI spec, and cross-cutting polish.

### 8.1 Monitoring Integration

Add A2A events to the monitoring system (`src-tauri/src/monitoring/`):

**Event types:**
- `A2aMessageSent` — agent, message preview, timestamp
- `A2aMessageReceived` — agent, response type, artifacts count
- `A2aTaskCreated` — agent, task_id, context_id
- `A2aTaskStateChanged` — task_id, old_state, new_state
- `A2aTaskCompleted` — task_id, duration, artifact count
- `A2aTaskFailed` — task_id, error
- `A2aDiscovery` — url, success/failure, agent name
- `A2aHealthChanged` — agent, old_status, new_status

### 8.2 OpenAPI Updates

Add A2A endpoints to OpenAPI spec (`src-tauri/src/server/openapi/`):
- `POST /a2a` — JSON-RPC dispatcher
- `GET /.well-known/agent-card.json` — Agent Card
- Document JSON-RPC request/response schemas
- Add A2A types to schema registry

### 8.3 Config Migration

Write migration in `crates/lr-config/src/migration.rs`:
- `ClientMode` enum → `ClientCapabilities` flags struct
- Add `a2a_agents: []` to config
- Add `a2a_permissions` to client defaults
- Preserve backward compat with serde aliases

### 8.4 STDIO Bridge for A2A

Extend the MCP STDIO bridge (`crates/lr-mcp/src/bridge/`) or create new A2A bridge:
- External clients (like Claude Desktop) could connect via STDIO
- Bridge forwards JSON-RPC to LocalRouter's A2A endpoint
- Detect MCP vs A2A by method casing in the STDIO stream

### 8.5 Testing

- Unit tests for all protocol types (serialization roundtrips)
- Unit tests for message router (target parsing)
- Unit tests for task tracker
- Integration tests for A2A client against mock A2A server
- Integration tests for gateway routing
- Integration tests for A2A via MCP bridge
- Frontend component tests

---

## Endpoint Summary

| Endpoint | Method | Protocol | Auth | Description |
|----------|--------|----------|------|-------------|
| `GET /.well-known/agent-card.json` | GET | A2A | Public | Unified Agent Card |
| `POST /a2a` | POST | A2A | Client Auth | JSON-RPC dispatcher |
| `POST /` | POST | MCP or A2A | Client Auth | Multiplexed by method casing |
| `GET /` | GET | MCP | Client Auth | SSE connection (MCP only) |
| `POST /mcp` | POST | MCP | Client Auth | MCP JSON-RPC (unchanged) |
| `POST /v1/*` | POST | LLM | API Key | LLM endpoints (unchanged) |

---

## Data Flow Diagrams

### Client → A2A Agent (via LocalRouter)

```
Client (Claude/Cursor/etc)
    │
    ├─ LLM ──→ POST /v1/chat/completions ──→ Provider routing
    │
    ├─ MCP ──→ POST / (JSON-RPC) ──→ MCP Gateway ──→ MCP Servers
    │
    └─ A2A ──→ POST /a2a (JSON-RPC) ──→ A2A Gateway ──→ External A2A Agents
         │                                    │
         │  "To: route-planner"               ├─→ Agent A (route-planner)
         │  "Optimize route from X to Y"      ├─→ Agent B (translator)
         │                                    └─→ Agent C (code-reviewer)
         │
         └─ A2A via MCP ──→ MCP Gateway
              │  tool: a2a_send_message
              │  args: {agent: "route-planner", message: "..."}
              └──→ A2A Gateway ──→ External A2A Agents
```

### External Caller → LocalRouter A2A Server

```
External A2A Caller
    │
    ├─ GET /.well-known/agent-card.json ──→ Unified card (all downstream agents)
    │
    └─ POST /a2a
         │  SendMessage: "To: route-planner\nOptimize..."
         │
         └──→ A2A Gateway ──→ route-planner agent
              │  (tracks task mapping)
              └──→ Response back to caller
```

---

## Verification Plan

### Phase 1 Verification
- `cargo check` — all types compile
- `cargo test --package lr-a2a` — serialization roundtrip tests
- `cargo test --package lr-config` — config migration tests

### Phase 2 Verification
- Mock A2A server (simple HTTP server returning canned responses)
- Test discovery: `discover_a2a_agent("http://localhost:9999")` fetches card
- Test client: send_message, get_task, cancel_task against mock
- Tauri commands work from frontend dev tools

### Phase 3 Verification
- Gateway routes messages to correct downstream agent
- Task tracker maintains correct mappings
- "To: agent-name" parsing handles edge cases
- Missing target returns full agent cards

### Phase 4 Verification
- `POST /a2a` handles all JSON-RPC methods
- `GET /.well-known/agent-card.json` returns unified card
- Root `POST /` correctly routes MCP vs A2A by method casing
- Client auth works for A2A endpoints

### Phase 5 Verification
- MCP client sees `a2a_*` tools when `a2a_via_mcp` enabled
- `a2a_send_message` tool routes to correct agent
- Multi-turn (INPUT_REQUIRED) works via repeated tool calls
- Tools hidden when `a2a_via_mcp` disabled

### Phase 6 Verification
- `cargo tauri dev` — navigate to A2A Agents view
- Add agent via URL discovery
- Agent appears in sidebar with health status
- Client config shows A2A tab with permissions
- Connection graph shows A2A agent nodes
- Dashboard A2A tab shows metrics
- Try-it-out A2A panel sends messages and shows responses

### Phase 7 Verification
- Marketplace A2A tab loads
- Templates display correctly
- Registry search returns results
- Install flow: search → discover → add → appears in agent list

### Phase 8 Verification
- Monitor shows A2A events
- OpenAPI spec includes A2A endpoints
- `cargo test && cargo clippy` passes
- Config migration preserves existing data

### End-to-End Test
1. Start LocalRouter (`cargo tauri dev`)
2. Add an A2A agent via URL discovery in UI
3. Create a client with all three capabilities enabled (LLM + MCP + A2A)
4. In Try-it-out, send a message to the A2A agent
5. Verify response appears with task status
6. Connect via MCP, use `a2a_send_message` tool
7. Verify A2A via MCP bridge works
8. Check dashboard for A2A metrics
9. Check monitor for A2A events
10. Verify connection graph shows the A2A agent
