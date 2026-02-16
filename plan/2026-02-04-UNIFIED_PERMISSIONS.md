# Unified Permission Control System

## Overview

Replace checkbox-based access control + separate firewall tab with a unified **Allow/Ask/Off** hierarchical permission system across MCPs, Skills, Models, and Marketplace.

**Key Concept**: Merge access control and firewall into single state:
- **Allow** = enabled, no approval needed
- **Ask** = enabled, requires approval popup
- **Off** = disabled (not accessible)

---

## Behavior Specification

### Inheritance Rules
1. Parent permission cascades to all children unless child has explicit override
2. When parent changes to match all children, child overrides can be collapsed (inherited)
3. When toggling Off→Allow/Ask, children restore their previous Ask states if they had overrides
4. Visual: inherited values shown slightly dimmed, explicit overrides shown full opacity

### Tool Visibility vs Execution
When a parent (skill/MCP server) is Allow/Ask but a child tool is Off:
- **Tool is still visible** to MCP clients (appears in `tools/list`)
- **Execution returns error** ("permission denied" / "tool not allowed")
- This allows skills/MCPs to reference the tool in their logic while blocking actual execution
- Only when the entire parent is Off does the tool become invisible

### Tree Hierarchies
- **Skills**: All Skills → Individual Skills → Skill Tools (collapsed, shown when skill is Allow/Ask)
- **MCPs**: All MCPs → Individual Servers → Tools/Resources/Prompts (collapsed, shown when server is Allow/Ask)
- **Models**: All Models → Providers → Individual Models (only in "allowed" mode)
- **Marketplace**: Single Allow/Ask/Off toggle

### Special Exceptions

#### Deferred Loading Tool
The MCP deferred loading tool (`tools/listChanged`) should **never** be Ask - always executable when enabled. This is handled at the backend level, not configurable per-client.

#### Marketplace Tools
- **Search tools**: Never Ask - always allowed when marketplace is enabled
- **Install tools**: Follow the normal Allow/Ask firewall rules
- Backend enforces this distinction, UI doesn't show search tools in the tree

#### Marketplace Allow Warning
When user sets Marketplace to "Allow", display an amber warning:
> "Warning: Allowing marketplace grants access to install any item. Only enable if you trust the configured marketplace sources."

### Model "Ask" Behavior
When a model has "Ask" permission, an approval popup appears before the request proceeds:
- Same popup style as MCP tools
- **Options differ from MCP**: Allow Once / Allow for 1 Hour / Allow Permanently
- Note: "Allow for Session" doesn't apply to models (requests are stateless)
- Triggered when a request would use that specific model

---

## Data Model Changes

### File: `crates/lr-config/src/types.rs`

#### 1. New `PermissionState` Enum
```rust
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum PermissionState {
    Allow,
    Ask,
    #[default]
    Off,
}
```

#### 2. New Permission Structs
```rust
pub struct McpPermissions {
    pub global: PermissionState,                    // All MCPs
    pub servers: HashMap<String, PermissionState>, // server_id -> state
    pub tools: HashMap<String, PermissionState>,   // server_id__tool_name -> state
    pub resources: HashMap<String, PermissionState>,
    pub prompts: HashMap<String, PermissionState>,
}

pub struct SkillsPermissions {
    pub global: PermissionState,
    pub skills: HashMap<String, PermissionState>,  // skill_name -> state
    pub tools: HashMap<String, PermissionState>,   // skill_name__tool_name -> state
}

pub struct ModelPermissions {
    pub global: PermissionState,
    pub providers: HashMap<String, PermissionState>,
    pub models: HashMap<String, PermissionState>,  // provider__model_id -> state
}
```

#### 3. Update `Client` Struct
Add new fields (keep old for migration):
```rust
pub mcp_permissions: McpPermissions,
pub skills_permissions: SkillsPermissions,
pub model_permissions: ModelPermissions,
pub marketplace_permission: PermissionState,
```

---

## Backend Commands

### File: `src-tauri/src/ui/commands_clients.rs`

#### New Commands
1. `set_client_mcp_permission(client_id, level, key, state)`
   - level: "global" | "server" | "tool" | "resource" | "prompt"
2. `set_client_skills_permission(client_id, level, key, state)`
   - level: "global" | "skill" | "tool"
3. `set_client_model_permission(client_id, level, key, state)`
   - level: "global" | "provider" | "model"
4. `set_client_marketplace_permission(client_id, state)`

### File: `src-tauri/src/ui/commands_mcp.rs`

#### New Command for MCP Capabilities
```rust
get_mcp_server_capabilities(server_id) -> McpServerCapabilities {
    tools: Vec<{name, description}>,
    resources: Vec<{uri, name, description}>,
    prompts: Vec<{name, description}>,
}
```

### File: `src-tauri/src/ui/commands_skills.rs`

#### New Command for Skill Tools
```rust
get_skill_tools(skill_name) -> Vec<SkillToolInfo> {
    name: String,
    description: Option<String>,
}
```
Returns the list of tools exposed by a skill (parsed from skill.md or manifest).

### File: `crates/lr-mcp/src/gateway/firewall.rs`

#### Update `FirewallApprovalAction`
Add `AllowPermanent` variant that updates client permissions when selected.

### Model Firewall Integration

#### File: `src-tauri/src/server/` (request handling)
When routing a request in "allowed" mode:
1. Resolve model permission from `client.model_permissions`
2. If `Ask`, check time-based approvals first (see below)
3. If no valid approval, trigger approval popup
4. Approval details include: client name, model name, provider
5. Actions: allow_once, allow_1_hour, allow_permanent

#### Time-Based Model Approvals
Store in-memory map: `(client_id, provider__model_id) -> expiry_timestamp`
- `allow_1_hour`: Set expiry to now + 1 hour
- Check on each request; if expired, remove and prompt again
- Cleared on app restart (in-memory only, not persisted)

---

## React Components

### New Directory: `src/components/permissions/`

#### 1. `PermissionStateButton.tsx`
Adapts existing `FirewallPolicySelector` pattern:
- Three states: Allow (emerald) / Ask (amber) / Off (gray)
- `size="sm" | "md"` support
- `inherited?: boolean` prop for dimmed display

#### 2. `PermissionTreeSelector.tsx`
Core reusable component:
- Props: `nodes: TreeNode[]`, `permissions: Record<string, PermissionState>`, `onPermissionChange`
- Sticky "All" row at top
- Collapsible children with lazy loading
- Indentation: `12 + depth * 16` px (matches existing file tree)
- ChevronDown/ChevronRight for expand state

#### 3. Wrapper Components
- `McpPermissionTree.tsx` - loads servers, lazy-loads tools/resources/prompts on expand
- `SkillsPermissionTree.tsx` - loads skills from backend
- `ModelsPermissionTree.tsx` - shows models from strategy (only when not "all")

---

## UI Tab Changes

### File: `src/views/clients/tabs/mcp-tab.tsx`
Replace checkbox list with `McpPermissionTree`:
- Shows servers with Allow/Ask/Off
- Expand server (only when Allow/Ask) → shows Tools, Resources, Prompts groups (collapsed)
- Expand group → shows individual items with Allow/Ask/Off
- Keep Deferred Loading toggle as separate section
- Move Marketplace access here with Allow/Ask/Off
- Show amber warning when Marketplace is set to "Allow"

### File: `src/views/clients/tabs/skills-tab.tsx`
Replace checkbox list with `SkillsPermissionTree`:
- Shows "All Skills" with Allow/Ask/Off
- Individual skills indented with Allow/Ask/Off
- Expand skill (only when Allow/Ask) → shows skill tools (collapsed by default)

### File: `src/views/clients/tabs/models-tab.tsx`
Add new section when strategy uses "allowed" mode (not "all models"):
- `ModelsPermissionTree` showing providers → models
- Only visible when `strategy.routing_mode === "allowed"`

### File: `src/views/clients/tabs/firewall-tab.tsx`
**DELETE** - no longer needed

### File: `src/views/clients/client-detail.tsx`
Remove Firewall tab from tabs array.

---

## Approval Popup Changes

### File: `src/views/firewall-approval.tsx`

Replace three buttons with:
```
[Deny]  [Allow Once]  [Allow ▾]
                        ├─ Allow for Session (MCP/Skills only)
                        ├─ Allow for 1 Hour (Models only)
                        └─ Allow Permanently
```

Use `DropdownMenu` from Radix UI for the Allow button dropdown.

**Context-aware options**:
- MCP/Skills: "Allow for Session" (tied to MCP session lifetime)
- Models: "Allow for 1 Hour" (time-based, since requests are stateless)

"Allow Permanently" calls new action that:
1. Submits `allow_permanent` to backend
2. Backend updates client's permissions to set that item to Allow
3. Saves config and emits clients-changed event

Backend needs to track model time-based approvals (client_id + model_id + expiry timestamp).

---

## Migration

### File: `crates/lr-config/src/migration.rs`

#### Version 6 Migration
Convert existing fields to new permission system:

1. **MCP Migration**:
   - `mcp_server_access::All` → `mcp_permissions.global = Allow`
   - `mcp_server_access::None` → `mcp_permissions.global = Off`
   - `mcp_server_access::Specific(ids)` → set each server to Allow, global to Off
   - Apply `firewall.server_rules` and `firewall.tool_rules` as Ask overrides

2. **Skills Migration**:
   - Same pattern as MCP
   - Apply `firewall.skill_rules` and `firewall.skill_tool_rules`

3. **Marketplace Migration**:
   - `marketplace_enabled: true` → `marketplace_permission = Allow`
   - `marketplace_enabled: false` → `marketplace_permission = Off`

4. **Models Migration**:
   - Initialize `model_permissions` with defaults (no firewall)

---

## Files to Modify

### Backend (Rust)
1. `crates/lr-config/src/types.rs` - Add PermissionState, *Permissions structs
2. `crates/lr-config/src/migration.rs` - Add v6 migration
3. `src-tauri/src/ui/commands_clients.rs` - Add permission commands, update ClientInfo
4. `src-tauri/src/ui/commands_mcp.rs` - Add get_mcp_server_capabilities
5. `src-tauri/src/ui/commands_skills.rs` - Add get_skill_tools
6. `crates/lr-mcp/src/gateway/firewall.rs` - Add AllowPermanent, Allow1Hour actions
7. `src-tauri/src/server/` - Model permission checks and time-based approval tracking

### Frontend (React/TypeScript)
1. **Create** `src/components/permissions/` directory with:
   - `index.ts`
   - `PermissionStateButton.tsx`
   - `PermissionTreeSelector.tsx`
   - `McpPermissionTree.tsx`
   - `SkillsPermissionTree.tsx`
   - `ModelsPermissionTree.tsx`
   - `types.ts`
2. **Update** `src/views/clients/tabs/mcp-tab.tsx`
3. **Update** `src/views/clients/tabs/skills-tab.tsx`
4. **Update** `src/views/clients/tabs/models-tab.tsx`
5. **Update** `src/views/clients/client-detail.tsx` - remove Firewall tab
6. **Update** `src/views/firewall-approval.tsx` - add dropdown
7. **Delete** `src/views/clients/tabs/firewall-tab.tsx`

---

## Verification

1. **Unit Tests**: Permission resolution logic in Rust
2. **Migration Test**: Create test config with old format, verify migration
3. **Manual Testing**:
   - Create client, set MCPs to Allow, expand server, set one tool to Ask
   - Trigger that tool → approval popup appears
   - Click "Allow Permanently" → tool permission updates to Allow
   - Toggle "All MCPs" to Off → all servers show as Off
   - Toggle back to Allow → tool retains its Allow state
4. **E2E**: Run `cargo test && cargo clippy`
