# Improve MCP Skill Prompts: Tool Description, Welcome Message, and Catalog Indexing

## Context

The MCP skills system uses a progressive-disclosure pattern: a `SkillRead` meta-tool lets LLMs load skill instructions on demand, while a "catalog" in the welcome message lists available skills. There are three problems:

1. **Tool description is verbose** — it cross-references "welcome message", `ctx_search`, and `ResourceRead` instead of being self-contained and concise.
2. **Skill names aren't in the tool schema** — the `name` parameter accepts any string but doesn't list valid values, forcing the LLM to look elsewhere.
3. **Catalog indexing is broken** — when skills exceed 20/50 thresholds, the catalog tells the LLM to `ctx_search(source="catalog:skills")`, but skills are never indexed into the ContentStore and `ctx_search` is a hardcoded legacy name instead of the configured tool name (`IndexSearch`).

## Changes

### 1. Make tool description concise + list skill names in parameter schema

**File:** `crates/lr-skills/src/mcp_tools.rs` — `build_meta_tool()`

Change the function signature to accept the list of accessible skill names:
```rust
fn build_meta_tool(tool_name: &str, skill_names: &[&str]) -> McpTool
```

New tool definition:
- **Description:** `"Read a skill's full instructions, metadata, and file listing."`
  - Drop all references to welcome message, ctx_search, and ResourceRead.
- **`name` parameter:** List valid skill names in the description field:
  `"Skill name. Available: skill1, skill2, skill3"`
  - Don't use JSON schema `enum` — it bloats tool definitions when there are many skills and some LLM clients choke on long enums.
  - For >20 skills (when names may be compressed out of the welcome message), the description still lists all names so the LLM can always find them in the tool schema.

Update `build_skill_tools()` signature to pass accessible skill names through:
```rust
pub fn build_skill_tools(
    skill_manager: &SkillManager,
    permissions: &SkillsPermissions,
    tool_name: &str,
) -> Vec<McpTool>
```
Remove `resource_read_name` parameter (no longer used in tool description). The function already computes the accessible skills list — extract names and pass to `build_meta_tool`.

Update caller in `virtual_skills.rs:108-113` — drop the `"ResourceRead"` argument.

### 2. Keep welcome message rich + fix search tool name reference

**File:** `crates/lr-skills/src/mcp_tools.rs` — `build_skill_catalog()`

Change function signature:
```rust
pub fn build_skill_catalog(
    skill_manager: &SkillManager,
    permissions: &SkillsPermissions,
    context_management_enabled: bool,
    tool_name: &str,
    resource_read_name: &str,
    search_tool_name: &str,  // NEW — replaces hardcoded "ctx_search"
) -> Option<String>
```

Fix Phase 2/3 text to use configured search tool name:
- Phase 2: `"Use {search_tool_name}(source=\"catalog:skills\") for skill descriptions and details."`
- Phase 3: `"... and N more — use {search_tool_name}(source=\"catalog:skills\") to discover all skills"`

Phase 1 (≤20 skills) stays the same — full listing with name + description + file counts. This is the rich info in the welcome message.

Update callers in `virtual_skills.rs:220-226` to pass `search_tool_name`. Add it to `SkillsSessionState`:
```rust
pub struct SkillsSessionState {
    // ... existing fields ...
    pub search_tool_name: String,  // NEW
}
```

Wire it through `create_session_state` and `update_session_state` — get from the context management config's configured search tool name (or default `"IndexSearch"`).

### 3. Index skills into ContentStore so `catalog:skills` actually works

**File:** `crates/lr-skills/src/mcp_tools.rs` — new function:
```rust
/// Build index entries for skills (name + description + tags + file listing).
/// Returns Vec<(label, content)> where label is "catalog:skills/{name}".
pub fn build_skill_index_entries(
    skill_manager: &SkillManager,
    permissions: &SkillsPermissions,
) -> Vec<(String, String)>
```

Each entry has label `catalog:skills/{skill_name}` and content:
```
# {name}
{description}
Tags: {tags}
Files: {count}
- scripts/{file1}
- references/{file2}
```

**File:** `crates/lr-mcp/src/gateway/virtual_server.rs` — add default method to trait:
```rust
/// Provide catalog entries for FTS5 indexing.
/// Returns Vec<(label, content)> for ContentStore.index().
fn catalog_index_entries(&self, _state: &dyn VirtualSessionState) -> Vec<(String, String)> {
    Vec::new()
}
```

**File:** `crates/lr-mcp/src/gateway/virtual_skills.rs` — implement `catalog_index_entries`:
```rust
fn catalog_index_entries(&self, state: &dyn VirtualSessionState) -> Vec<(String, String)> {
    let state = state.as_any().downcast_ref::<SkillsSessionState>().expect("...");
    lr_skills::mcp_tools::build_skill_index_entries(&self.skill_manager, &state.permissions)
}
```

**File:** `crates/lr-mcp/src/gateway/gateway.rs` — in the CM indexing block (after line ~2300), add:
```rust
// Index virtual server catalog entries (skills, etc.)
for (vs_id, vs) in &self.virtual_servers {
    if let Some(state) = session_read.virtual_server_state.get(vs_id.as_str()) {
        for (label, content) in vs.catalog_index_entries(state.as_ref()) {
            store_idx.index(&label, &content)?;
        }
    }
}
```

Also register catalog source labels for activation tracking (after line ~2355):
```rust
// Register virtual catalog sources
for (vs_id, vs) in &self.virtual_servers {
    if let Some(state) = session_write.virtual_server_state.get(vs_id.as_str()) {
        for (label, _) in vs.catalog_index_entries(state.as_ref()) {
            cm_state.catalog_sources.insert(label, CatalogItemType::Skill);
        }
    }
}
```

### 4. Add `CatalogItemType::Skill` variant

**File:** `crates/lr-mcp/src/gateway/context_mode.rs`

Add variant:
```rust
pub enum CatalogItemType {
    Tool,
    Resource,
    Prompt,
    ServerWelcome,
    Skill,  // NEW
}
```

Update `CTX_SEARCH_SOURCE_GUIDE` to document `catalog:skills`:
```
  source="catalog:skills"               — search all skill descriptions and metadata
  source="catalog:skills/MySkill"       — find a specific skill's details
```

Update all match arms on `CatalogItemType` to handle `Skill` — it should NOT trigger tool/resource/prompt activation (skills are accessed via `SkillRead`, not directly).

### 5. How it looks at each stage

**With ≤20 skills (Phase 1 — no compression):**

Tool schema:
```json
{
  "name": "SkillRead",
  "description": "Read a skill's full instructions, metadata, and file listing.",
  "inputSchema": {
    "properties": {
      "name": {
        "type": "string",
        "description": "Skill name. Available: CodeReview, DataPipeline, Deployment"
      }
    }
  }
}
```

Welcome message:
```
<skills>
- `SkillRead` (tool)

Available skills:
- `CodeReview`: Analyzes code for bugs and improvements (5 files)
- `DataPipeline`: Process and transform data files (3 files)
- `Deployment`: Deploy to staging and production (2 files)
Call SkillRead(name) to load full instructions.
Read skill files with ResourceRead(name="<skill>/<path>").
</skills>
```

**With 21-50 skills (Phase 2 — names only in welcome):**

Tool schema: Same as above (still lists all names in parameter description).

Welcome message:
```
<skills>
- `SkillRead` (tool)

Available skills:
- `CodeReview`
- `DataPipeline`
- ... (all names, no descriptions)
Use IndexSearch(source="catalog:skills") for skill descriptions and details.
Call SkillRead(name) to load full instructions.
Read skill files with ResourceRead(name="<skill>/<path>").
</skills>
```

ContentStore indexed: Each skill at `catalog:skills/{name}` with full description, tags, file listing.

**With >50 skills (Phase 3 — top 10 only in welcome):**

Tool schema: Same (still lists all names — this is the LLM's complete reference).

Welcome message:
```
<skills>
- `SkillRead` (tool)

Available skills:
- `CodeReview`
- `DataPipeline`
- ... and 48 more — use IndexSearch(source="catalog:skills") to discover all skills
Call SkillRead(name) to load full instructions.
</skills>
```

ContentStore indexed: Same as Phase 2.

## Files to modify

1. `crates/lr-skills/src/mcp_tools.rs` — Core: tool def, catalog, new index builder
2. `crates/lr-mcp/src/gateway/virtual_skills.rs` — Wiring: session state, call sites, catalog_index_entries impl
3. `crates/lr-mcp/src/gateway/virtual_server.rs` — Trait: add `catalog_index_entries` default method
4. `crates/lr-mcp/src/gateway/context_mode.rs` — `CatalogItemType::Skill`, source guide, match arms
5. `crates/lr-mcp/src/gateway/gateway.rs` — Indexing: add virtual catalog entries + source registration

## Verification

1. `cargo test -p lr-skills` — unit tests for build_meta_tool, build_skill_catalog, build_skill_index_entries
2. `cargo test -p lr-mcp` — gateway tests, context_mode tests, merger tests
3. `cargo clippy` — no warnings
4. Manual: connect with an MCP client, verify:
   - SkillRead tool description is concise
   - Skill names appear in the `name` parameter description
   - Welcome message shows full skill descriptions
   - With context management on: `IndexSearch(source="catalog:skills")` returns skill entries

## Mandatory final steps

1. **Plan Review**: Compare each file change against this plan
2. **Test Coverage Review**: Ensure new `build_skill_index_entries`, modified `build_meta_tool`, gateway indexing path all have tests
3. **Bug Hunt**: Check edge cases — empty skill list, skills with no description, very long skill lists (>50), context management disabled
