# Fix Marketplace "Via MCP" Tools & UI Text When MCP/Skills Are Individually Disabled

## Context

The Marketplace has two independent toggles: `mcp_enabled` and `skills_enabled`. When one or both are disabled, tool descriptions exposed to MCP clients and UI text hardcode references to both "MCP servers and skills" without adapting. This is confusing — if only Skills is enabled, the MCP tool still says "Search the marketplace for available MCP servers and/or skills."

**User expectation**: If both disabled → marketplace MCP tools don't show up (already works). If one disabled → tool descriptions and UI text adapt to only reference the enabled feature.

## What Already Works

- Virtual MCP server `list_tools()` returns empty vec when both disabled (via `service.is_enabled()`)
- `build_instructions()` returns None when both disabled
- Browse tab shows DisabledOverlay when both disabled, disables individual filter buttons
- Search/install handlers properly gate on `is_mcp_enabled()` / `is_skills_enabled()` individually

## Fixes

### 1. Make `list_tools()` config-aware — `crates/lr-marketplace/src/tools.rs`

Add a helper function for the noun phrase:

```rust
fn feature_label(mcp: bool, skills: bool) -> &'static str {
    match (mcp, skills) {
        (true, true) => "MCP servers and skills",
        (true, false) => "MCP servers",
        (false, true) => "skills",
        (false, false) => "marketplace items",
    }
}
```

Change `list_tools()` signature to `list_tools(mcp_enabled: bool, skills_enabled: bool)`:
- **Search tool description**: Use `feature_label()` — e.g. "Search the marketplace for available {label}."
- **Search tool `type` enum**: Build dynamically — both: `["mcp", "skill", "all"]`, MCP-only: `["mcp"]`, skills-only: `["skill"]`
- **Install tool description**: Adapt similarly — "Install an MCP server" / "Install a skill" / "Install an MCP server or skill"
- **Install tool `type` enum**: Both: `["mcp", "skill"]`, MCP-only: `["mcp"]`, skills-only: `["skill"]`

Also adapt the search result **hint text** (line 136-137) to reference only enabled types.

Update existing tests and add new tests for `(true, false)` and `(false, true)` permutations.

### 2. Bridge config in `MarketplaceService::list_tools()` — `crates/lr-marketplace/src/lib.rs`

Change line 195-197 to read config and pass booleans:
```rust
pub fn list_tools(&self) -> Vec<Value> {
    let cfg = self.config.read();
    tools::list_tools(cfg.mcp_enabled, cfg.skills_enabled)
}
```

### 3. Dynamic `build_instructions()` content — `crates/lr-mcp/src/gateway/virtual_marketplace.rs`

Line 166 — adapt instruction text based on which features are enabled:
- Both: "Use marketplace tools to discover and install new MCP servers and skills."
- MCP-only: "Use marketplace tools to discover and install new MCP servers."
- Skills-only: "Use marketplace tools to discover and install new skills."

The `is_enabled()` guard on lines 160-161 already returns None when both disabled.

### 4. Wire config into `get_marketplace_tool_definitions` — `src-tauri/src/ui/commands_marketplace.rs`

Add `config_manager: State<'_, ConfigManager>` parameter to the command (line 709). Read marketplace config and pass booleans to `tools::list_tools()`. The imports (`ConfigManager`, `State`) are already present in the file.

### 5. Frontend text adaptation — `src/views/marketplace/index.tsx`

Add a `featureLabel(mcpEnabled, skillsEnabled)` helper (mirrors backend logic).

Update:
- **Line 486**: Page description — "Browse and install {label} from online registries and sources."
- **Line 560**: Search placeholder for "all" filter — "Search {label}..."
- **Lines 766-768**: Via MCP tab — "MCP clients can search for and install {label} directly through tool calls"
- **Lines 172-185**: Add `config?.mcp_enabled, config?.skills_enabled` to the tool-loading `useEffect` dependency array, so tool definitions reload when config changes

### 6. Client marketplace tab — `src/views/clients/tabs/marketplace-tab.tsx`

Line 64 — change to generic phrasing that works regardless of config:
"Allow this client to search and install from the marketplace."
(This component doesn't have marketplace config and shouldn't need it — it controls client permission, not marketplace features.)

### 7. Wizard step — `src/components/wizard/steps/StepExtensions.tsx`

Line 273 — same approach: "Search and install from the marketplace"

### 8. Demo mock — `website/src/components/demo/TauriMockSetup.ts`

Line 2250 — update mock `get_marketplace_tool_definitions` response description to match the "both enabled" text from the backend (the demo config always has both enabled).

### 9. Update merger test fixtures — `crates/lr-mcp/src/gateway/merger.rs`

Lines 726, 777, 2674 — these are test fixtures that construct `VirtualInstructions` directly. Keep them as-is since they represent the "both enabled" scenario. If instructions text changes format, update to match.

## Files NOT Modified (with rationale)

| File | Reason |
|------|--------|
| `src/components/client/HowToConnect.tsx:436,530` | Describes MCP proxy/gateway capability, not marketplace discovery |
| `src/views/try-it-out/mcp-tab/index.tsx:621,647` | Describes MCP testing modes, not marketplace |

## Verification

1. `cargo test -p lr-marketplace` — verify new test permutations pass
2. `cargo test -p lr-mcp` — verify merger tests still pass
3. `cargo clippy` — no warnings
4. `npx tsc --noEmit` — frontend types check
5. Manual: Enable only MCP marketplace → Via MCP tab shows tools referencing only "MCP servers"
6. Manual: Enable only Skills marketplace → Via MCP tab shows tools referencing only "skills"
7. Manual: Disable both → Via MCP tab shows tools from Tauri command (page description adapts), virtual server returns no tools to MCP clients
8. Manual: Enable both → current behavior preserved

## Final Steps (per CLAUDE.md)

1. **Plan Review**: Verify all changes match the plan
2. **Test Coverage Review**: Ensure all `(mcp_enabled, skills_enabled)` permutations are tested
3. **Bug Hunt**: Check for race conditions between config change and tool listing (not an issue — `handle_tool_call` already validates individually)
