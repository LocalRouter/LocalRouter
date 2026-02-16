# Dashboard Connection Graph Implementation Plan

## Overview

Add a dynamic connection graph to the Dashboard page showing relationships between API Keys, LLM Providers, and MCP Servers with real-time health status and active connection indicators.

## Terminology

| Code Term | Graph Label | Description |
|-----------|-------------|-------------|
| `Client` (config) | **Access Key** | Configured API key with permissions |
| Active SSE connection | **Connected App** | App actively using an Access Key |
| `ProviderInstance` | **Provider** | LLM provider (OpenAI, Ollama, etc.) |
| `McpServerConfig` | **MCP Server** | Configured MCP server |

## Location

Insert in `/src/views/dashboard/index.tsx` after the Stats Row (line ~235), before the Metrics Tabs.

## Technology

- **React Flow** (already installed: `reactflow: ^11.11.4`)
- **dagre** (already installed) for automatic left-to-right layout
- **Tauri events** for real-time updates

## Architecture

```
src/components/connection-graph/
├── ConnectionGraph.tsx      # Main component with React Flow
├── nodes/
│   ├── AccessKeyNode.tsx    # Blue node for API Keys
│   ├── ProviderNode.tsx     # Violet node for LLM Providers
│   └── McpServerNode.tsx    # Emerald node for MCP Servers
├── hooks/
│   └── useGraphData.ts      # Data fetching + event subscriptions
├── utils/
│   └── buildGraph.ts        # Transform data to nodes/edges
└── types.ts                 # TypeScript interfaces
```

## Graph Layout

```
[Access Keys]  ------>  [Providers]
     |
     +---------------->  [MCP Servers]
```

- **Left column**: Access Key nodes (blue)
- **Right-top**: Provider nodes (violet) with health dots
- **Right-bottom**: MCP Server nodes (emerald) with health dots
- Edges: Animated when connection is active

## Backend Changes

### 1. New Tauri Command: `get_active_connections`

File: `/src-tauri/src/ui/commands.rs`

```rust
#[tauri::command]
pub async fn get_active_connections(
    server_manager: State<'_, Arc<ServerManager>>,
) -> Result<Vec<String>, String> {
    // Return list of client_ids with active SSE connections
}
```

### 2. New Events for Connection Changes

File: `/src-tauri/src/server/state.rs` - Modify `SseConnectionManager`

- Add `app_handle` field to `SseConnectionManager`
- Emit `sse-connection-opened` when connection registered
- Emit `sse-connection-closed` when connection dropped

## Frontend Data Flow

1. **Initial Load**: Fetch clients, providers, MCP servers, health state, active connections
2. **Real-time Updates**:
   - `health-status-changed` → Update health dots
   - `sse-connection-opened` / `sse-connection-closed` → Update connection status
   - `config-changed` → Refetch all data

## Node Styling

| Node Type | Background | Border | Health Indicator |
|-----------|------------|--------|------------------|
| Access Key | Blue gradient | Blue | Connection icon (wifi) |
| Provider | Violet gradient | Violet | Colored dot |
| MCP Server | Emerald gradient | Emerald | Colored dot |

Health dot colors:
- Green = Healthy
- Yellow = Degraded
- Red = Unhealthy
- Gray (pulse) = Pending

## Files to Modify

1. `/src-tauri/src/server/state.rs` - Add event emission to SseConnectionManager
2. `/src-tauri/src/ui/commands.rs` - Add `get_active_connections` command
3. `/src-tauri/src/main.rs` - Register new command
4. `/src/views/dashboard/index.tsx` - Import and render ConnectionGraph

## Files to Create

1. `/src/components/connection-graph/types.ts`
2. `/src/components/connection-graph/hooks/useGraphData.ts`
3. `/src/components/connection-graph/utils/buildGraph.ts`
4. `/src/components/connection-graph/nodes/AccessKeyNode.tsx`
5. `/src/components/connection-graph/nodes/ProviderNode.tsx`
6. `/src/components/connection-graph/nodes/McpServerNode.tsx`
7. `/src/components/connection-graph/ConnectionGraph.tsx`

## Verification

1. Start the app with `cargo tauri dev`
2. Navigate to Dashboard
3. Verify graph appears between stats and metrics
4. Create a client and verify node appears
5. Configure provider access and verify edges
6. Connect an app via MCP and verify "Connected" status
7. Check health dots match sidebar health indicators
8. Disconnect app and verify status updates
