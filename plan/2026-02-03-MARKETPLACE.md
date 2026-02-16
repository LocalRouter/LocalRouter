# Marketplace Feature — Revised Plan

## Overview

Built-in marketplace tools injected into the MCP gateway that let AI clients search and install MCP servers and Skills. Plus a dedicated Marketplace UI page for human users to browse/install directly.

**Key design decisions (from feedback):**
- Only **4 tools**: `marketplace__search_mcp_servers`, `marketplace__install_mcp_server`, `marketplace__search_skills`, `marketplace__install_skill`
- **Not a virtual MCP server** — just built-in tools injected into the gateway tools list
- **Not visible in MCP servers list** — invisible to MCP panel
- **Visible in connection graph** — new "Marketplace" node type
- **New sidebar tab** — "Marketplace" below Skills, for human browsing/install
- **Custom install popup** — not firewall popup; delivered via same broadcast mechanism but with config form (OAuth, API key, command params)
- **Skills via API** — browse GitHub repos via Contents API, download files via raw URLs (no git clone)
- **First-visit confirmation** — Marketplace page asks user to enable on first visit

---

## 1. New Crate: `crates/lr-marketplace/`

```
crates/lr-marketplace/
├── Cargo.toml
├── src/
│   ├── lib.rs              # Module exports, constants, MarketplaceService
│   ├── types.rs            # Registry types, install request/response types
│   ├── tools.rs            # Tool definitions, tool call handler, is_marketplace_tool()
│   ├── registry.rs         # MCP Registry API client
│   ├── skill_sources.rs    # GitHub Contents API browser + raw file downloader
│   └── install.rs          # Install logic: create config, update client access, emit events
```

### Constants
```rust
pub const TOOL_PREFIX: &str = "marketplace__";
pub const MARKETPLACE_ID: &str = "marketplace";
```

### 4 Tools

| Tool | Description | Inputs | Firewall |
|------|-------------|--------|----------|
| `marketplace__search_mcp_servers` | Search MCP server registry | `query`, `limit?` | Allow |
| `marketplace__install_mcp_server` | Install MCP server into config | `name`, `transport`, `command`/`url`, `env?` | **Ask** (custom popup) |
| `marketplace__search_skills` | Browse skill sources | `query?`, `source?` | Allow |
| `marketplace__install_skill` | Download skill + add to config | `source_url`, `skill_name` | **Ask** (custom popup) |

### MarketplaceService

Central struct holding all dependencies:
```rust
pub struct MarketplaceService {
    config_manager: Arc<ConfigManager>,
    mcp_server_manager: Arc<McpServerManager>,
    skill_manager: Arc<SkillManager>,
    data_dir: PathBuf,
    http_client: reqwest::Client,
    // In-memory cache for skill source listings
    skill_cache: RwLock<HashMap<String, (Instant, Vec<SkillListing>)>>,
}
```

Methods:
- `list_tools() -> Vec<Value>` — returns JSON tool definitions for the 4 tools
- `handle_tool_call(tool_name, arguments, client_id) -> Result<Value>` — dispatches
- `is_marketplace_tool(name: &str) -> bool` — checks prefix
- `search_mcp_servers(query, limit) -> Vec<McpServerListing>`
- `search_skills(query, source) -> Vec<SkillListing>`
- `install_mcp_server(listing, client_id) -> Result<InstalledServer>`
- `install_skill(source_url, skill_name, client_id) -> Result<InstalledSkill>`

### MCP Registry Client (`registry.rs`)

Query the official MCP server registry:
- `GET https://registry.modelcontextprotocol.io/v0.1/servers?search={query}&limit={limit}&version=latest`
- Parse response: `packages[]` (npm/pypi), `remotes[]` (hosted URLs)
- Return structured `McpServerListing` with install instructions

### Skill Sources (`skill_sources.rs`)

Browse GitHub repos via Contents API (no git clone):
- Parse repo URL → `{owner}/{repo}`, extract branch + path
- `GET https://api.github.com/repos/{owner}/{repo}/contents/{path}?ref={branch}` → list subdirs
- For each subdir, check if `SKILL.md` exists via Contents API
- Cache results in-memory with 5-min TTL
- Download skill files via `https://raw.githubusercontent.com/{owner}/{repo}/{branch}/{path}/{skill}/SKILL.md`
- For multi-file skills: enumerate via Contents API, download each file via raw URL
- Save to `{data_dir}/marketplace-skills/{label}/{skill_name}/`

### Install Logic (`install.rs`)

**MCP Server Install:**
1. Create `McpServerConfig` (id=uuid, name, transport, transport_config)
2. Add to `config.mcp_servers` via ConfigManager
3. Add to McpServerManager
4. Auto-grant: add server ID to requesting client's `mcp_server_access`
5. Set firewall: `server_rules[new_server_id] = Ask`
6. Emit `mcp-servers-changed` event

**Skill Install:**
1. Download skill files to `{data_dir}/marketplace-skills/{label}/{skill_name}/`
2. Add path to `config.skills.paths`
3. Trigger skill rescan
4. Auto-grant: add skill name to requesting client's `skills_access`
5. Set firewall: `skill_rules[skill_name] = Ask`
6. Emit `skills-changed` event

---

## 2. Config Changes

**File:** `crates/lr-config/src/types.rs`

Add to `AppConfig`:
```rust
#[serde(default)]
pub marketplace: MarketplaceConfig,
```

New types:
```rust
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct MarketplaceConfig {
    /// Whether marketplace is enabled globally
    #[serde(default = "default_true")]
    pub enabled: bool,

    /// MCP server registry URL
    #[serde(default = "default_registry_url")]
    pub registry_url: String,

    /// Skill source repos to browse
    #[serde(default = "default_skill_sources")]
    pub skill_sources: Vec<MarketplaceSkillSource>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct MarketplaceSkillSource {
    pub repo_url: String,
    #[serde(default = "default_main")]
    pub branch: String,
    pub path: String,
    pub label: String,
}
```

Add to `Client`:
```rust
/// Whether this client has marketplace access (search + install tools)
/// When this is set to true, global marketplace.enabled is also auto-set to true
#[serde(default)]
pub marketplace_enabled: bool,
```

**Enable behavior:**
- `config.marketplace.enabled` — global flag, controls whether marketplace is initialized at all
- `client.marketplace_enabled` — per-client flag, controls whether this client gets the 4 tools
- When any client's `marketplace_enabled` is set to `true`, the global `config.marketplace.enabled` is automatically set to `true` (triggers marketplace initialization)
- First-visit confirmation in UI sets the global flag when accepted

Default skill sources (ship with multiple, verify during implementation):
1. `https://github.com/anthropics/skills` → root or `skills/` (label: "Anthropic")
2. `https://github.com/travisvn/awesome-claude-skills` → `skills/` (label: "Awesome Claude Skills")
3. Additional community repos TBD during implementation (verify they exist and have SKILL.md format)

---

## 3. Custom Install Popup System (AI-triggered)

**Pattern:** Same delivery mechanism as firewall popups (broadcast notification → open Tauri WebviewWindow), but with a different UI that shows a config form.

**Note:** This popup is only for AI-triggered installs. Human users in the Marketplace page get an inline config form/dialog within the page itself.

### Backend: `MarketplaceInstallManager`

**File:** `crates/lr-marketplace/src/install_popup.rs`

Similar to `FirewallManager`:
```rust
pub struct MarketplaceInstallManager {
    pending: DashMap<String, PendingInstall>,
    broadcast: Option<Arc<tokio::sync::broadcast::Sender<(String, JsonRpcNotification)>>>,
    timeout_seconds: u64,
}

pub struct PendingInstall {
    pub request_id: String,
    pub install_type: InstallType, // McpServer or Skill
    pub listing: Value,            // Full listing data for the popup to display
    pub client_id: String,
    pub client_name: String,
    pub response_sender: Option<oneshot::Sender<InstallResponse>>,
    pub created_at: Instant,
    pub timeout_seconds: u64,
}

pub enum InstallType {
    McpServer,
    Skill,
}

pub struct InstallResponse {
    pub action: InstallAction,
    pub config: Option<Value>, // User-provided config (transport, env, OAuth, etc.)
}

pub enum InstallAction {
    Install,  // Proceed with config from popup
    Cancel,
}
```

Flow (AI-triggered):
1. AI calls `marketplace__install_mcp_server` with listing data
2. Gateway intercepts → creates `PendingInstall` → broadcasts notification
3. Frontend SSE listener catches notification → opens custom Tauri popup window
4. Popup window shows: listing info + config form (command, URL, env vars, OAuth)
5. User fills in config + clicks Install (or Cancel)
6. Frontend calls Tauri command `marketplace_install_respond(request_id, action, config)`
7. Backend receives response → performs actual install → returns result to AI

### Frontend: Install Popup Window (AI-triggered only)

**New file:** `src/views/marketplace/install-popup.tsx`

Rendered in a separate Tauri window (like firewall popup). Detects `window.__MARKETPLACE_INSTALL_REQUEST_ID__` to know which request to display.

**For MCP Server installs**, the popup shows:
- Server name, description, homepage link
- Transport selector (Stdio / HTTP+SSE)
- For Stdio: command input, env vars editor
- For HTTP+SSE: URL input, headers editor
- Auth section: None / Bearer Token / OAuth (with "Login" button that opens browser)
- Install / Cancel buttons

**For Skill installs**, the popup shows:
- Skill name, description, source repo
- Confirmation text ("This will download skill files from GitHub")
- Install / Cancel buttons

### Tauri Commands for Install Popup

**File:** `src-tauri/src/ui/commands_marketplace.rs`

```rust
#[tauri::command]
pub async fn marketplace_install_respond(request_id: String, action: String, config: Option<Value>) -> Result<(), String>

#[tauri::command]
pub async fn marketplace_get_pending_install(request_id: String) -> Result<PendingInstallInfo, String>
```

### SSE Notification for Popup

When a pending install is created, broadcast a notification:
```json
{
  "method": "notifications/marketplace/install_request",
  "params": {
    "request_id": "...",
    "install_type": "mcp_server",
    "listing": { ... }
  }
}
```

The SSE handler in `src-tauri/src/server/routes/mcp_sse.rs` (or the Tauri event listener) catches this and opens the popup window.

---

## 4. Gateway Integration

**File:** `crates/lr-mcp/src/gateway/gateway.rs`

Add field:
```rust
pub(crate) marketplace_service: OnceLock<Arc<MarketplaceService>>,
```

Add setter:
```rust
pub fn set_marketplace_service(&self, service: Arc<MarketplaceService>) {
    let _ = self.marketplace_service.set(service);
}
```

**File:** `crates/lr-mcp/src/gateway/gateway_tools.rs`

### Tools list — `handle_tools_list`

Add marketplace tools after skill tools:
```rust
// After append_skill_tools:
self.append_marketplace_tools(&mut tools, &marketplace_enabled);
```

New method:
```rust
fn append_marketplace_tools(&self, tools: &mut Vec<Value>, marketplace_enabled: &bool) {
    if !marketplace_enabled { return; }
    if let Some(service) = self.marketplace_service.get() {
        tools.extend(service.list_tools());
    }
}
```

### Tool call — `handle_tools_call`

Add marketplace check before skill tool check (line ~212):
```rust
// Check if it's a marketplace tool
if lr_marketplace::is_marketplace_tool(&tool_name) {
    // Firewall check (install tools are Ask, search tools are Allow)
    if let Some(denied) = self.check_firewall_mcp_tool(
        &session, &tool_name, lr_marketplace::MARKETPLACE_ID, &request
    ).await? {
        return Ok(denied);
    }
    return self.handle_marketplace_tool_call(session, &tool_name, request).await;
}
```

### Session changes

Add `marketplace_enabled: bool` to `GatewaySession`. Set it from client config in `handle_request_with_skills`.

---

## 5. Frontend: Marketplace Sidebar Tab + Page

### Sidebar

**File:** `src/components/layout/sidebar.tsx`

- Add `'marketplace'` to `View` type
- Add entry to `mainNavItems` after `skills`:
  ```ts
  { id: 'marketplace', icon: StoreIcon, label: 'Marketplace', shortcut: '...' }
  ```

### App.tsx

Add case:
```ts
case 'marketplace': return <MarketplaceView activeSubTab={activeSubTab} onTabChange={handleViewChange} />
```

### MarketplaceView

**New file:** `src/views/marketplace/index.tsx`

Two sections via Tabs: "MCP Servers" and "Skills"

**MCP Servers tab:**
- Search input → calls Tauri command `marketplace_search_mcp_servers(query)`
- Results list with name, description, packages info
- "Install" button → opens **inline config dialog** (not popup window)
- Inline dialog: same config form as the AI popup (transport, command, URL, auth), rendered within the Marketplace page using AlertDialog/Sheet
- On submit, calls Tauri command `marketplace_install_mcp_server_direct(config)` which does the actual install (no pending flow)

**Skills tab:**
- Browse by source (dropdown of configured sources)
- Shows skills from selected source
- "Install" button → opens **inline confirmation dialog**
- On confirm, calls Tauri command `marketplace_install_skill_direct(source_url, skill_name)`

**First-visit confirmation:**
- Check `config.marketplace.enabled`
- If not enabled, show overlay: "Enable Marketplace? This will allow browsing MCP server registries and skill repositories over the internet."
- On accept, set `config.marketplace.enabled = true`

### Tauri Commands for Marketplace Page

**File:** `src-tauri/src/ui/commands_marketplace.rs`

```rust
// Search (used by both UI and can be called by AI tools internally)
#[tauri::command]
pub async fn marketplace_search_mcp_servers(query: String, limit: Option<u32>) -> Result<Vec<McpServerListing>, String>

#[tauri::command]
pub async fn marketplace_search_skills(query: Option<String>, source: Option<String>) -> Result<Vec<SkillListing>, String>

// Direct install (for human UI, bypasses pending popup flow)
#[tauri::command]
pub async fn marketplace_install_mcp_server_direct(config: McpInstallConfig) -> Result<InstalledServer, String>

#[tauri::command]
pub async fn marketplace_install_skill_direct(source_url: String, skill_name: String) -> Result<InstalledSkill, String>

// Config management
#[tauri::command]
pub async fn marketplace_get_config() -> Result<MarketplaceConfig, String>

#[tauri::command]
pub async fn marketplace_set_enabled(enabled: bool) -> Result<(), String>

#[tauri::command]
pub async fn marketplace_list_skill_sources() -> Result<Vec<MarketplaceSkillSource>, String>

#[tauri::command]
pub async fn marketplace_add_skill_source(source: MarketplaceSkillSource) -> Result<(), String>

#[tauri::command]
pub async fn marketplace_remove_skill_source(repo_url: String) -> Result<(), String>
```

---

## 6. Connection Graph

**File:** `src/components/connection-graph/types.ts`

Add new node type:
```ts
type GraphNodeType = 'accessKey' | 'provider' | 'mcpServer' | 'skill' | 'marketplace'
```

**File:** `src/components/connection-graph/nodes/MarketplaceNode.tsx`

New node component — distinct color (e.g., pink/magenta gradient), store/shop icon.

**File:** `src/components/connection-graph/utils/buildGraph.ts`

- If a client has `marketplace_enabled`, add a "Marketplace" target node
- Draw edge from client to marketplace node
- Click navigates to `marketplace` view

**File:** `src/components/connection-graph/hooks/useGraphData.ts`

- Include `marketplace_enabled` in client data from `list_clients`

---

## 7. Client Config UI

**File:** `src/views/clients/tabs/mcp-tab.tsx` (or similar)

Add a toggle/checkbox: "Enable Marketplace" alongside the MCP server access configuration. When toggled, updates `client.marketplace_enabled`.

---

## Files Modified (existing)

| File | Change |
|------|--------|
| `crates/lr-config/src/types.rs` | Add `MarketplaceConfig`, `MarketplaceSkillSource`, `marketplace` field on `AppConfig`, `marketplace_enabled` on `Client` |
| `crates/lr-mcp/src/gateway/gateway.rs` | Add `marketplace_service: OnceLock`, setter |
| `crates/lr-mcp/src/gateway/gateway_tools.rs` | `append_marketplace_tools`, marketplace tool call interception in `handle_tools_call` |
| `crates/lr-mcp/src/gateway/session.rs` | Add `marketplace_enabled: bool` field |
| `src-tauri/src/main.rs` | Create and wire `MarketplaceService` |
| `src/components/layout/sidebar.tsx` | Add `'marketplace'` to `View`, add nav item |
| `src/App.tsx` | Add `MarketplaceView` case |
| `src/components/connection-graph/types.ts` | Add `'marketplace'` node type |
| `src/components/connection-graph/utils/buildGraph.ts` | Add marketplace nodes/edges |
| `src/components/connection-graph/hooks/useGraphData.ts` | Include marketplace data |
| `src/views/clients/` | Add marketplace toggle in client config |
| `Cargo.toml` (workspace) | Add `lr-marketplace` to workspace members |
| `src-tauri/Cargo.toml` | Add `lr-marketplace` dependency |

## Files Created (new)

| File | Purpose |
|------|---------|
| `crates/lr-marketplace/Cargo.toml` | Crate manifest |
| `crates/lr-marketplace/src/lib.rs` | Exports, constants, `MarketplaceService` |
| `crates/lr-marketplace/src/types.rs` | `McpServerListing`, `SkillListing`, install types |
| `crates/lr-marketplace/src/tools.rs` | Tool JSON definitions, `handle_tool_call` dispatch |
| `crates/lr-marketplace/src/registry.rs` | MCP Registry HTTP client |
| `crates/lr-marketplace/src/skill_sources.rs` | GitHub Contents API browser + raw downloader |
| `crates/lr-marketplace/src/install.rs` | Install logic (create config, update client, emit events) |
| `crates/lr-marketplace/src/install_popup.rs` | `MarketplaceInstallManager` (pending install → popup → response) |
| `src-tauri/src/ui/commands_marketplace.rs` | Tauri commands for marketplace |
| `src/views/marketplace/index.tsx` | Marketplace page (MCP servers + Skills tabs) |
| `src/views/marketplace/install-popup.tsx` | Install popup window component |
| `src/components/connection-graph/nodes/MarketplaceNode.tsx` | Graph node component |

---

## Implementation Order

1. `lr-marketplace` crate: types, constants, `MarketplaceService` with stub methods
2. Config: `MarketplaceConfig` + `marketplace_enabled` on `Client`
3. Gateway integration: wire marketplace tools into `gateway_tools.rs`
4. MCP Registry client (`registry.rs`)
5. Skill sources browser (`skill_sources.rs`)
6. Install logic (`install.rs`) — actual config creation + client updates
7. Install popup system (`install_popup.rs` + Tauri commands)
8. Frontend: Marketplace page (`src/views/marketplace/`)
9. Frontend: Install popup window
10. Frontend: Connection graph marketplace node
11. Frontend: Client config marketplace toggle
12. App initialization wiring (`main.rs`)
13. Tests + verification

## Verification
1. `cargo test && cargo clippy && cargo fmt` passes
2. `cargo tauri dev` — Marketplace tab appears in sidebar
3. Marketplace page shows first-visit confirmation → enable → shows search UI
4. Search MCP servers → results from registry.modelcontextprotocol.io
5. Search skills → results from configured GitHub sources
6. Install MCP server → custom popup with config form → fill in → server appears in MCP list
7. Install skill → custom popup → confirm → skill appears in Skills list
8. Client with `marketplace_enabled` → `tools/list` includes 4 marketplace tools
9. AI calls `marketplace__install_mcp_server` → popup appears → approve → server installed
10. Connection graph shows Marketplace node connected to clients that have it enabled
