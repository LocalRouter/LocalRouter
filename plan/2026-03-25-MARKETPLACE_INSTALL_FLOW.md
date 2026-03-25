# Plan: Marketplace Install → Actual Installation + Tool Availability

## Context

When an LLM uses the Marketplace virtual MCP server to install a new MCP server or skill, **nothing actually gets installed**. The install tool handler returns `{ "status": "approved", "config": {...} }` to the LLM but never:
- Adds the server config or downloads the skill
- Starts the server
- Grants the requesting client permissions
- Makes the new tools available in the current agentic loop
- Sends `tools/changed` notification

Additionally, the install response is too vague — it doesn't tell the LLM what tools are now available or how to use them.

**Goal**: After a marketplace install, the server/skill is actually installed, its tools appear in the current session immediately, and the LLM gets enough info (tool names, descriptions, instructions) to use them on the very next tool call.

### Design Decisions
- **Wait for server ready**: The install callback blocks until the server is initialized and we can fetch its actual tools. Slightly slower but guarantees tools work immediately.
- **Auto-grant Ask permission**: The new server is added to the client's `mcp_permissions.servers` with `Ask` state. The firewall will prompt on first tool use, adding a safety check even though the user approved the install.

---

## Implementation Steps

### Step 1: Add install callback infrastructure to `MarketplaceService`

**File**: `crates/lr-marketplace/src/lib.rs`

Add a trait + callback mechanism so the marketplace crate can trigger actual installation without depending on Tauri state directly:

```rust
/// Callback for performing actual MCP server installation.
/// Returns (server_id, server_name, tools, instructions) on success.
pub type McpInstallCallback = Arc<dyn Fn(McpInstallRequest) -> Pin<Box<dyn Future<Output = Result<McpInstallResult, String>> + Send>> + Send + Sync>;

/// Callback for performing actual skill installation.
pub type SkillInstallCallback = Arc<dyn Fn(SkillInstallRequest) -> Pin<Box<dyn Future<Output = Result<SkillInstallResult, String>> + Send>> + Send + Sync>;
```

Define request/result types:

```rust
pub struct McpInstallRequest {
    pub listing: McpServerListing,
    pub config: Value,           // User-provided config from popup
    pub client_id: String,
    pub client_name: String,
}

pub struct McpInstallResult {
    pub server_id: String,
    pub server_name: String,
    pub tools: Vec<InstalledToolInfo>,    // name + description
    pub instructions: Option<String>,     // Server's welcome/instructions text
}

pub struct SkillInstallRequest {
    pub listing: SkillListing,
    pub client_id: String,
    pub client_name: String,
}

pub struct SkillInstallResult {
    pub skill_name: String,
    pub tools: Vec<InstalledToolInfo>,
    pub instructions: Option<String>,
}

pub struct InstalledToolInfo {
    pub name: String,
    pub description: Option<String>,
}
```

Add `set_mcp_install_callback()` and `set_skill_install_callback()` methods to `MarketplaceService`.

### Step 2: Make install handlers call the callbacks and return rich responses

**File**: `crates/lr-marketplace/src/tools.rs`

Modify `handle_install_mcp_server()`:
1. After receiving user approval (Install action + config), call `mcp_install_callback(request)` to perform actual installation
2. Wait for the callback to return with the installed server info
3. Build a rich response including:
   - `status: "installed"` (not just "approved")
   - `server_id` and `server_name`
   - `tools`: array of `{ name, description }` for each tool the new server provides
   - `instructions`: the server's welcome message / usage instructions
   - `next_step`: "These tools are now available. You can call them immediately."

Similarly modify `handle_install_skill()` to use the skill install callback.

The response format becomes:
```json
{
  "status": "installed",
  "server_id": "mcp-github",
  "server_name": "GitHub",
  "tools": [
    { "name": "github_create_issue", "description": "Create a GitHub issue..." },
    { "name": "github_list_repos", "description": "List repositories..." }
  ],
  "instructions": "Use the GitHub tools to interact with repositories...",
  "message": "MCP server 'GitHub' installed and ready. The tools listed above are now available for immediate use."
}
```

This directly addresses the user's question: the install response includes the new server's tools and instructions (effectively the "welcome message"), so the LLM knows exactly what to call next.

### Step 3: Wire up install callbacks in Tauri app initialization

**File**: `src-tauri/src/server/mod.rs` (or wherever `MarketplaceService` is constructed and state is initialized)

Create the MCP install callback closure that:
1. Creates server config from listing + user config (reuse `lr_marketplace::install::create_mcp_server_config`)
2. Stores bearer token in keychain if provided
3. Adds config to `ConfigManager` and saves
4. Adds to `McpServerManager` and starts the server
5. **Updates client permissions**: adds `server_id → Ask` in the client's `mcp_permissions.servers` (unless `global` is already enabled) — firewall will prompt on first tool use
6. **Waits for server initialization** — blocks until the server reports ready or a timeout (e.g. 30s). This ensures tools are actually available when the install response returns.
7. Fetches the server's actual tools via `McpServerManager` (tool names + descriptions)
8. Returns `McpInstallResult` with tools + instructions

Create the skill install callback closure that:
1. Downloads skill files (reuse existing logic from `marketplace_install_skill_direct`)
2. Adds path to config, triggers `SkillManager.rescan()`
3. Returns `SkillInstallResult` with the skill's tools + instructions

### Step 4: Return `SuccessWithSideEffects` for install tools

**File**: `crates/lr-mcp/src/gateway/virtual_marketplace.rs`

Change `handle_tool_call()` to detect install results and return `SuccessWithSideEffects`:

```rust
async fn handle_tool_call(...) -> VirtualToolCallResult {
    match self.service.handle_tool_call(...).await {
        Ok(result) => {
            let is_install = result.get("status").and_then(|v| v.as_str()) == Some("installed");
            if is_install {
                let new_server_id = result.get("server_id")
                    .and_then(|v| v.as_str())
                    .map(|s| s.to_string());
                VirtualToolCallResult::SuccessWithSideEffects {
                    response: json!({ "content": [{ "type": "text", "text": ... }] }),
                    invalidate_cache: true,
                    send_list_changed: true,
                    state_update: None,
                    add_allowed_servers: new_server_id.map(|id| vec![id]),
                }
            } else {
                VirtualToolCallResult::Success(...)
            }
        }
        Err(e) => VirtualToolCallResult::ToolError(e.to_string()),
    }
}
```

### Step 5: Add `add_allowed_servers` to `SuccessWithSideEffects`

**File**: `crates/lr-mcp/src/gateway/virtual_server.rs`

Add new field:
```rust
pub enum VirtualToolCallResult {
    SuccessWithSideEffects {
        response: Value,
        invalidate_cache: bool,
        send_list_changed: bool,
        state_update: Option<Box<dyn FnOnce(&mut dyn VirtualSessionState) + Send>>,
        /// New server IDs to add to the session's allowed_servers list
        add_allowed_servers: Option<Vec<String>>,
    },
    // ... existing variants
}
```

**File**: `crates/lr-mcp/src/gateway/gateway_tools.rs`

In `dispatch_virtual_tool_call()`, handle the new field after the existing `invalidate_cache`/`send_list_changed` handling:

```rust
if let Some(new_servers) = add_allowed_servers {
    let mut sw = session.write().await;
    for server_id in new_servers {
        if !sw.allowed_servers.contains(&server_id) {
            sw.allowed_servers.push(server_id);
        }
    }
}
```

Update all existing `SuccessWithSideEffects` constructors elsewhere to include `add_allowed_servers: None`.

### Step 6: Add mid-loop tool refresh in the orchestrator

**File**: `crates/lr-mcp-via-llm/src/orchestrator.rs`

After executing MCP tool calls in the agentic loop, check if any tool was a marketplace install. If so, re-fetch tools and update the request:

```rust
// After executing all MCP tool calls in an iteration:
if tools_may_have_changed {
    // Re-fetch MCP tools (cache was invalidated by SuccessWithSideEffects)
    let refreshed_tools = gw_client.list_tools().await?;
    let new_mcp_names: HashSet<String> = refreshed_tools.iter().map(|t| t.name.clone()).collect();

    if new_mcp_names != mcp_tool_names {
        // Remove old MCP tool definitions from request, inject new ones
        refresh_mcp_tools_in_request(&mut request, &mcp_tool_names, &refreshed_tools);
        mcp_tool_names = new_mcp_names;
    }
}
```

Add `refresh_mcp_tools_in_request()` helper:
```rust
fn refresh_mcp_tools_in_request(
    request: &mut CompletionRequest,
    old_mcp_names: &HashSet<String>,
    new_mcp_tools: &[McpTool],
) {
    if let Some(ref mut tools) = request.tools {
        tools.retain(|t| !old_mcp_names.contains(&t.function.name));
    }
    inject_mcp_tools(request, new_mcp_tools);
}
```

**Detection**: A tool call is a marketplace install if the tool name matches the marketplace install tool name. The `MarketplaceService` exposes `is_marketplace_install_tool()` — we can pass the install tool name through to the orchestrator, or simply check if the tool result contains `"status": "installed"`.

### Step 7: Same tool refresh for streaming orchestrator

**File**: `crates/lr-mcp-via-llm/src/orchestrator_stream.rs`

Apply the same tool refresh logic as Step 6. The streaming orchestrator has a similar loop structure — after MCP tool execution, check for tools_changed and refresh if needed.

### Step 8: Re-build gateway instructions after install (system prompt update)

**File**: `crates/lr-mcp-via-llm/src/orchestrator.rs`

After tool refresh (Step 6), also re-build and update gateway instructions so the system message reflects the new server:

```rust
if tools_may_have_changed {
    // Re-initialize to get updated instructions
    let new_instructions = gw_client.rebuild_instructions().await?;
    if let Some(ref instructions) = new_instructions {
        session.write().gateway_instructions = Some(instructions.clone());
        // Update the system message in the request template
        update_server_instructions(&mut request, instructions);
    }
}
```

This requires adding a `rebuild_instructions()` method to `GatewayClient` that calls `build_gateway_instructions()` for the current session state. This is the proper way to update the "welcome message" mid-conversation — the system message gets updated and the LLM sees the full picture including the new server's section on the next iteration.

**Note**: This combined with the rich install response gives the LLM two signals:
1. The install tool response tells it what tools are available (immediate)
2. The updated system message has the full server catalog (persistent across turns)

---

## Critical Files to Modify

| File | Change |
|------|--------|
| `crates/lr-marketplace/src/lib.rs` | Add install callback types and setters |
| `crates/lr-marketplace/src/tools.rs` | Call install callbacks, return rich responses |
| `crates/lr-marketplace/src/types.rs` | Add `McpInstallRequest/Result`, `SkillInstallRequest/Result`, `InstalledToolInfo` |
| `crates/lr-mcp/src/gateway/virtual_server.rs` | Add `add_allowed_servers` to `SuccessWithSideEffects` |
| `crates/lr-mcp/src/gateway/virtual_marketplace.rs` | Return `SuccessWithSideEffects` for installs |
| `crates/lr-mcp/src/gateway/gateway_tools.rs` | Handle `add_allowed_servers` in dispatch |
| `crates/lr-mcp-via-llm/src/orchestrator.rs` | Mid-loop tool refresh + instructions update |
| `crates/lr-mcp-via-llm/src/orchestrator_stream.rs` | Same tool refresh for streaming |
| `crates/lr-mcp-via-llm/src/gateway_client.rs` | Add `rebuild_instructions()` method |
| `src-tauri/src/server/mod.rs` | Wire up install callbacks with ConfigManager/McpServerManager/SkillManager |

Other files with minor updates (add `add_allowed_servers: None` to existing `SuccessWithSideEffects`):
- Any virtual server returning `SuccessWithSideEffects` (grep for it)

---

## Verification

1. **Unit tests**: Test install callback invocation, rich response format, `refresh_mcp_tools_in_request` helper
2. **Integration test**: Search → Install → verify server config added, server started, client permissions updated, tools available in next iteration
3. **Manual test**:
   - Start dev mode (`cargo tauri dev`)
   - Create a client with MCP-via-LLM mode + marketplace enabled
   - Send a chat: "Search the marketplace for a filesystem MCP server and install it"
   - Verify: install popup appears, after approval the LLM's next response uses the newly installed server's tools
   - Verify: `tools/changed` notification is sent (check monitor events)
   - Verify: gateway instructions are updated to include the new server
4. **Edge cases**: Install timeout, install cancelled, server fails to start, skill download fails
