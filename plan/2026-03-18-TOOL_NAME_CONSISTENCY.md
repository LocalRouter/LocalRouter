# Plan: Consistent PascalCase Tool Names + Make All Tool Names Configurable

## Context

MCP virtual tool names are inconsistent: some use `snake_case` (`skill_read`, `resource_read`), some use `prefix__name` (`marketplace__search`), and some use PascalCase (`MemorySearch`, `AgentStart`). Only Memory, Context Management, and Coding Agents tool names are configurable; the rest are hardcoded. This plan renames all defaults to PascalCase and makes every tool name configurable from its feature's Settings tab.

### Default name changes:
| Feature | Old Default | New Default |
|---------|------------|-------------|
| Marketplace | `marketplace__search` | `MarketplaceSearch` |
| Marketplace | `marketplace__install` | `MarketplaceInstall` |
| Skills | `skill_read` | `SkillRead` |
| Skills (internal) | `skill_read_file` | `SkillReadFile` |
| Resource Read | `resource_read` | `ResourceRead` |
| Memory | `MemorySearch` / `MemoryRead` | *(no change)* |
| Context Mgmt | `IndexSearch` / `IndexRead` | *(no change)* |
| Coding Agents | `AgentStart`, etc. | *(no change)* |

---

## Phase 1: Config Changes

**File: `crates/lr-config/src/types.rs`**

### 1a. MarketplaceConfig (~line 1769)

Add fields with serde defaults:
```rust
fn default_marketplace_search_tool_name() -> String { "MarketplaceSearch".to_string() }
fn default_marketplace_install_tool_name() -> String { "MarketplaceInstall".to_string() }

// In MarketplaceConfig:
#[serde(default = "default_marketplace_search_tool_name")]
pub search_tool_name: String,
#[serde(default = "default_marketplace_install_tool_name")]
pub install_tool_name: String,
```

Update `Default` impl.

### 1b. SkillsConfig (~line 1262)

Add fields:
```rust
fn default_skill_tool_name() -> String { "SkillRead".to_string() }
fn default_skill_read_file_tool_name() -> String { "SkillReadFile".to_string() }

#[serde(default = "default_skill_tool_name")]
pub tool_name: String,
#[serde(default = "default_skill_read_file_tool_name")]
pub read_file_tool_name: String,
```

**Backward compat**: All use `#[serde(default)]` — old configs without these fields get PascalCase defaults. No migration needed since these were never configurable before.

Note: `ResourceRead` (formerly `resource_read`) is renamed but stays hardcoded — no config field or UI needed.

---

## Phase 2: Marketplace Backend

### 2a. `crates/lr-marketplace/src/lib.rs`
- Remove `TOOL_PREFIX` constant
- Change `is_marketplace_tool(name)` → `is_marketplace_tool(&self, name: &str) -> bool` on `MarketplaceService` — exact match against both configured names
- Change `is_marketplace_search_tool(name)` → `is_marketplace_search_tool(&self, name: &str) -> bool` — match against search name only
- Add accessors: `search_tool_name(&self) -> String`, `install_tool_name(&self) -> String`

### 2b. `crates/lr-marketplace/src/tools.rs`
- Remove `SEARCH` / `INSTALL` constants
- `list_tools(search_tool_name: &str, install_tool_name: &str) -> Vec<Value>` — use params in JSON
- `handle_tool_call(service, tool_name, arguments, client_id, client_name, search_tool_name, install_tool_name)` — match full names instead of stripping prefix
- Update error hint strings (lines 137, 207, 259) to use `format!("Use {} ...", install_tool_name)` / `format!("Use {} first.", search_tool_name)`

### 2c. `crates/lr-mcp/src/gateway/virtual_marketplace.rs`
- `MarketplaceSessionState`: add `search_tool_name`, `install_tool_name`
- `owns_tool()`: delegate to `self.service.is_marketplace_tool(tool_name)`
- `all_tool_names()`: return from service
- `is_tool_indexable()`: compare against service's search/install names
- `create_session_state()` / `update_session_state()`: capture from service config

### 2d. Firewall detection fix: `src/views/firewall-approval.tsx`
- Lines 118, 157 hardcode `"marketplace__install"`. Add `is_marketplace_install: bool` to the backend `ApprovalDetails` struct (set by virtual server) so frontend checks a flag, not a name.

---

## Phase 3: Skills Backend

### 3a. `crates/lr-skills/src/mcp_tools.rs`
- Change constant defaults: `SKILL_META_TOOL_NAME = "SkillRead"`, `SKILL_READ_FILE_TOOL_NAME = "SkillReadFile"` (keep constants as fallback defaults)
- `build_meta_tool(tool_name: &str, resource_read_name: &str)` — accept names as params
- `build_skill_tools(manager, permissions, tool_name)` — pass configured name
- `build_skill_catalog(manager, permissions, ctx_enabled, tool_name, resource_read_name)` — use configured names in output text
- `is_skill_tool(name, configured_tool_name, configured_rfile_name)` — compare against both configured names
- `handle_skill_tool_call(tool_name, args, manager, permissions, configured_name)` — use configured name for matching
- Update description text referencing `resource_read` and `skill_read` to use configured names

### 3b. `crates/lr-mcp/src/gateway/virtual_skills.rs`
- `SkillsVirtualServer`: add `skills_config: RwLock<SkillsConfig>` field + `update_skills_config()` method
- `SkillsSessionState`: add `tool_name`, `read_file_tool_name`
- `owns_tool()`: compare against both configured names
- `list_tools()`: pass configured tool name
- `handle_tool_call()`: match against configured names
- `build_instructions()`: pass configured names to catalog builder
- `all_tool_names()`: return configured `tool_name`
- `is_tool_indexable()`: compare against configured tool name
- `create_session_state()` / `update_session_state()`: capture from skills config

### 3c. Orchestrator references
- `crates/lr-mcp-via-llm/src/orchestrator_stream.rs` (line 854): pass configured `read_file_tool_name` instead of hardcoded `"skill_read_file"`
- `crates/lr-mcp-via-llm/src/gateway_client.rs` (line 374): same
- These need access to `SkillsConfig` tool names — pass through `McpViaLlmManager` or session context

---

## Phase 4: Resource Read Rename (Hardcoded, Not Configurable)

### 4a. `crates/lr-mcp-via-llm/src/orchestrator.rs`
- Change `RESOURCE_READ_TOOL_NAME` constant value: `"resource_read"` → `"ResourceRead"`
- No config field needed — stays hardcoded as a constant
- All usages already reference the constant, so they update automatically

### 4b. `crates/lr-mcp-via-llm/src/orchestrator_stream.rs`
- Already uses `orchestrator::RESOURCE_READ_TOOL_NAME` — updates automatically

### 4c. Update description text in `inject_resource_read_tool()` and skill mcp_tools that reference `resource_read` by name

---

## Phase 5: Tauri Commands

**File: `src-tauri/src/ui/commands.rs` (or commands_marketplace.rs)**

### 5a. `update_marketplace_tool_names`
```rust
pub async fn update_marketplace_tool_names(
    search_tool_name: Option<String>,
    install_tool_name: Option<String>,
    config_manager, marketplace_vs
) -> Result<(), String>
```
Validate non-empty, update `config.marketplace`, save, propagate to virtual server.

### 5b. `update_skills_tool_names`
```rust
pub async fn update_skills_tool_names(
    tool_name: Option<String>,
    read_file_tool_name: Option<String>,
    config_manager, skills_vs
) -> Result<(), String>
```

### 5c. `update_coding_agents_tool_prefix`
Check if this already exists. If not, add it. Pattern: validate, update `config.coding_agents.tool_prefix`, save, propagate.

### 5d. Register in `src-tauri/src/main.rs`

---

## Phase 6: TypeScript Types

**File: `src/types/tauri-commands.ts`**

- `MarketplaceConfig`: add `search_tool_name: string`, `install_tool_name: string`
- `SkillsConfig`: add `tool_name: string`, `read_file_tool_name: string`
- Add param interfaces for new commands

---

## Phase 7: Frontend UI

### 7a. Marketplace Settings Tab (`src/views/marketplace/index.tsx`)
Add "Tool Names" Card after existing settings cards:
- Input for "Search tool" (default `MarketplaceSearch`)
- Input for "Install tool" (default `MarketplaceInstall`)
- `onBlur` save pattern (same as catalog-compression)
- Note: "Changes apply to new sessions only"

### 7b. Skills Settings Tab (`src/views/skills/index.tsx`)
The per-skill Settings tab (~line 687) currently only has Danger Zone. Add "Tool Names" Card above Danger Zone:
- Input for "Skill read tool" (default `SkillRead`)
- Input for "File read tool" (default `SkillReadFile`)
- Note: "Global setting — applies to all skills. Changes apply to new sessions only."

### 7c. Coding Agents Settings Tab (`src/views/coding-agents/index.tsx`)
Add "Tool Prefix" Card after Concurrency card (~line 1043):
- Input for "Tool prefix" (default `Agent`)
- Description showing derived names: "AgentStart, AgentSay, AgentStatus, AgentList"
- `onBlur` save

*(No UI needed for ResourceRead — hardcoded rename only)*

---

## Phase 8: Demo/Mock Updates

**File: `website/src/components/demo/TauriMockSetup.ts`**

- Update all tool name references: `skill_read` → `SkillRead`, `marketplace__search` → `MarketplaceSearch`, `marketplace__install` → `MarketplaceInstall`
- Add `search_tool_name`/`install_tool_name` to marketplace config mock
- Add `tool_name`/`read_file_tool_name` to skills config mock
- Add mock handlers for new commands

---

## Phase 9: Website/Docs

- `website/src/pages/Home.tsx`: Update `MemoryRecall` references if needed
- `website/src/components/docs/MarketplaceInstallDemo.tsx`: Update `toolName="marketplace__install"` → `"MarketplaceInstall"`

---

## Phase 10: Test Updates

- `crates/lr-marketplace/src/tools.rs` tests: Update tool name assertions
- `src-tauri/tests/skills_e2e_test.rs`: Update `"skill_read"` → `"SkillRead"` everywhere
- `crates/lr-mcp-via-llm/src/tests.rs`: Update `"resource_read"` → `"ResourceRead"`
- `crates/lr-mcp/src/gateway/context_mode.rs` tests: no changes (already PascalCase)
- `crates/lr-mcp/src/gateway/merger.rs` tests: Update marketplace tool names
- `crates/lr-coding-agents/src/mcp_tools.rs` tests: no changes (already PascalCase)

---

## Phase 11: Mandatory Final Steps

1. **Plan Review**: Check every change against plan — look for missed hardcoded strings
2. **Test Coverage Review**: Ensure config-driven tool name paths have test coverage
3. **Bug Hunt**: Check for tool name collisions, empty-string edge cases, session state consistency

---

## Verification

1. `cargo test` — all tests pass with updated default names
2. `cargo clippy` — no warnings
3. `npx tsc --noEmit` — TypeScript types match
4. Manual: Start dev mode, check each feature's Settings tab shows tool name config
5. Manual: Change a tool name in Settings, start new MCP session, verify the tool appears with new name
6. Manual: Verify firewall approval popup still works for marketplace install
7. Manual: Check demo/website mock renders correctly

---

## Critical Files Summary

| File | Changes |
|------|---------|
| `crates/lr-config/src/types.rs` | Add tool name fields to MarketplaceConfig, SkillsConfig, McpViaLlmConfig |
| `crates/lr-marketplace/src/lib.rs` | Remove TOOL_PREFIX, add config-driven methods |
| `crates/lr-marketplace/src/tools.rs` | Accept tool names as params, remove hardcoded names |
| `crates/lr-mcp/src/gateway/virtual_marketplace.rs` | Config-driven tool names in session state |
| `crates/lr-skills/src/mcp_tools.rs` | Accept tool names as params |
| `crates/lr-mcp/src/gateway/virtual_skills.rs` | Config-driven tool names, add SkillsConfig |
| `crates/lr-mcp-via-llm/src/orchestrator.rs` | Rename RESOURCE_READ_TOOL_NAME constant |
| `crates/lr-mcp-via-llm/src/orchestrator_stream.rs` | Config-driven skill_read_file name (resource_read auto-updates via constant) |
| `crates/lr-mcp-via-llm/src/gateway_client.rs` | Config-driven skill_read_file name |
| `src-tauri/src/ui/commands.rs` | New Tauri commands for tool name updates |
| `src-tauri/src/main.rs` | Register new commands |
| `src/types/tauri-commands.ts` | TypeScript type updates |
| `src/views/marketplace/index.tsx` | Tool Names card in Settings tab |
| `src/views/skills/index.tsx` | Tool Names card in Settings tab |
| `src/views/coding-agents/index.tsx` | Tool Prefix card in Settings tab |
| `src/views/firewall-approval.tsx` | Use flag instead of hardcoded name |
| `website/src/components/demo/TauriMockSetup.ts` | Mock updates |
