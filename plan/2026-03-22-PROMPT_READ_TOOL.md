# Replace Eager Prompt Injection with Lazy PromptRead Tool

## Context

The MCP via LLM orchestrator currently has two config flags (`inject_prompts`, `expose_resources_as_tools`) that control prompt and resource injection. The `inject_prompts` flag causes all no-argument prompts to be eagerly fetched via `prompts/get` on **every** LLM request, creating noisy `McpPromptGet` monitor events. Parameterized prompts become synthetic `mcp_prompt__*` tools.

This change:
1. Replaces eager prompt injection with a single lazy **PromptRead** tool (matching the SkillRead pattern)
2. Improves **ResourceRead** with fuzzy matching and consistent error messages
3. Removes both config flags — tools are always injected
4. Extracts duplicated fuzzy matching into `lr-types`

Both tools stay at the orchestrator level (not virtual servers) because they are **proxy tools** that route to upstream MCP servers — different from self-contained virtual servers like Skills/Memory/Marketplace. The orchestrator already has all the data via `gw_client`.

---

## Phase 1: Extract Shared Fuzzy Matching into `lr-types`

Two duplicate implementations exist:
- `crates/lr-skills/src/fuzzy.rs` — 4-layer: exact → case-insensitive → normalized → Levenshtein (richer)
- `crates/lr-context/src/fuzzy.rs` — simpler: just Levenshtein correction

**Move the richer implementation to `crates/lr-types/src/fuzzy.rs`** (lr-types is already a shared dependency):

1. Create `crates/lr-types/src/fuzzy.rs`:
   - Move `MatchKind`, `levenshtein()`, `max_edit_distance()`, `normalize_name()` (renamed from `normalize_skill_name`), `find_best_match()` from `lr-skills`
   - Add `find_best_correction()` wrapper (matching `lr-context`'s API) that delegates to `find_best_match`
   - Make all functions `pub`
   - Move tests from both crates

2. Update `crates/lr-types/src/lib.rs` — add `pub mod fuzzy;`

3. Update `crates/lr-skills/src/fuzzy.rs` — replace with re-export from `lr_types::fuzzy`

4. Update `crates/lr-context/src/fuzzy.rs` — replace with delegation to `lr_types::fuzzy`

5. Update `Cargo.toml` deps if needed (lr-skills and lr-context should already depend on lr-types)

**Files:**
- Create: `crates/lr-types/src/fuzzy.rs`
- Modify: `crates/lr-types/src/lib.rs`, `crates/lr-skills/src/fuzzy.rs`, `crates/lr-context/src/fuzzy.rs`

---

## Phase 2: Implement `PromptRead` Tool (Non-Streaming Orchestrator)

**File: `crates/lr-mcp-via-llm/src/orchestrator.rs`**

### 2a. Add constant and tool builder

```rust
pub(crate) const PROMPT_READ_TOOL_NAME: &str = "PromptRead";
```

New function `inject_prompt_read_tool(request, prompts)`:
- Follow `build_meta_tool()` pattern from `crates/lr-skills/src/mcp_tools.rs`
- Tool schema:
  ```json
  {
    "name": "PromptRead",
    "description": "Get an MCP prompt by name. Prompt names and descriptions are listed in MCP server sections of the welcome message.",
    "parameters": {
      "type": "object",
      "properties": {
        "name": {
          "type": "string",
          "description": "Prompt name. Available: server__prompt1, server__prompt2, ..."
        },
        "arguments": {
          "type": "object",
          "description": "Optional arguments for parameterized prompts"
        }
      },
      "required": ["name"],
      "additionalProperties": false
    }
  }
  ```
- The `name` description lists all available prompt names (from `list_prompts()` results) — unless context management compression is active, in which case reference IndexSearch instead
- Only inject the tool if there are prompts available (same as SkillRead with empty skills)

### 2b. Add `execute_prompt_read()` function

This tool serves as both **documentation and execution** — it validates arguments locally before making network calls.

1. **Fuzzy match** the name against known prompts (using `lr_types::fuzzy::find_best_match`)
2. **No match** → return error listing available prompt names (like `not_found_error()` in skills)
3. **Match found, prompt has arguments**:
   - **Validate locally first** using the prompt's argument schema:
     - Check all required arguments are present
     - Check no unknown argument names are provided
   - If validation fails → return error with full argument docs (no network call):
     ```
     Prompt 'X' requires arguments:
     - arg1 (required): description
     - arg2: description
     Call PromptRead(name="X", arguments={"arg1": "...", "arg2": "..."})
     ```
   - If arguments valid → call `gw_client.get_prompt(name, arguments)` and return content
4. **Match found, no-arg prompt** → call `gw_client.get_prompt(name, {})` and return content directly
5. **Fuzzy match** → prepend correction note (like skills: `"Note: No prompt named 'X' was found. Showing prompt 'Y' instead."`)

### 2c. Update orchestrator injection (lines ~186-244)

Replace:
```rust
// Old: config-gated resource_read + prompt injection
if config.expose_resources_as_tools { ... }
if config.inject_prompts { ... }
```

With:
```rust
// Always inject ResourceRead
inject_resource_read_tool(&mut request);
mcp_tool_names.insert(RESOURCE_READ_TOOL_NAME.to_string());

// Always inject PromptRead (if prompts available)
let prompts = gw_client.list_prompts().await.unwrap_or_default();
if !prompts.is_empty() {
    inject_prompt_read_tool(&mut request, &prompts);
    mcp_tool_names.insert(PROMPT_READ_TOOL_NAME.to_string());
}
```

Note: `list_prompts()` is still called once at init, but `get_prompt()` is no longer called eagerly — only when the LLM invokes the tool.

### 2d. Update tool dispatch (lines ~607-653)

Replace the `prompt_tools.get()` branch with `PromptRead` handling:
```rust
} else if tool_name == PROMPT_READ_TOOL_NAME {
    execute_prompt_read(&gw_client, &arguments, &prompts).await
} else {
    // Regular MCP tool
    ...
}
```

Remove `prompt_tools` HashMap entirely.

### 2e. Remove dead functions

- Remove `inject_prompt_tools()`
- Remove `inject_prompt_messages()`

### 2f. Update transformation event metadata

Replace:
```rust
if config.expose_resources_as_tools { transformations.push("mcp_resource_read_tool") }
if config.inject_prompts { transformations.push("mcp_prompt_injection") }
```
With:
```rust
transformations.push("mcp_resource_read_tool".to_string());
if !prompts.is_empty() {
    transformations.push("mcp_prompt_read_tool".to_string());
}
```

---

## Phase 3: Mirror Changes in Streaming Orchestrator

**File: `crates/lr-mcp-via-llm/src/orchestrator_stream.rs`**

Mirror all Phase 2 changes:
- Remove config-gated injection (lines ~126-173), replace with always-inject pattern
- Store `prompts` vec for tool dispatch
- Update tool dispatch (line ~626-654) to use `execute_prompt_read` instead of `prompt_tools` HashMap
- Remove `prompt_tools` HashMap
- Update transformation metadata
- Note: streaming orchestrator uses `execute_prompt_get_background()` and `execute_resource_read_background()` — update or replace the prompt background function

---

## Phase 4: Improve `ResourceRead` Consistency

Make ResourceRead consistent with SkillRead and the new PromptRead.

### 4a. Add `list_resources()` to GatewayClient

**File: `crates/lr-mcp-via-llm/src/gateway_client.rs`**

Add method to call `resources/list` JSON-RPC, returning resource names + URIs. Similar to existing `list_prompts()`.

### 4b. List resource names in tool parameter description

**File: `crates/lr-mcp-via-llm/src/orchestrator.rs`**

Update `inject_resource_read_tool()` to accept an optional list of resource names and include them in the `name` parameter description (like SkillRead: `"Resource name or skill file path. Available: server__res1, server__res2, ..."`). Keep the skill file path documentation too.

Resource names come from the welcome message's per-server listings (already known). If not available at tool injection time, the description omits the "Available:" list — the tool still works.

### 4c. Add fuzzy matching to `execute_resource_read()`

Update `execute_resource_read()` (lines 1165-1198):

Currently: exact MCP resource name → skill file fallback → error.

New flow (skill file fallback removed in Phase 5f — ResourceRead handles MCP resources only):
1. Try exact MCP resource read via `gw_client.read_resource(name)`
2. If not found, call `gw_client.list_resources()` (lazy, only on miss)
3. Try **fuzzy match** against resource names (using `lr_types::fuzzy::find_best_match`)
4. If fuzzy match found, read with corrected name + prepend correction note
5. If all fail, return error **listing available resource names** (consistent with SkillRead's `not_found_error`)

### 4d. Not-found error consistency

All name-based tools should follow the same error pattern:
```
Resource 'X' not found. Available resources: server__file1, server__file2
```
(Matching SkillRead's `not_found_error()` pattern)

---

## Phase 5: Merge SkillReadFile into SkillRead

Currently there are two separate skill tools:
- **SkillRead** — visible to LLM, `name` param → returns full skill instructions + file listing
- **SkillReadFile** — internal/hidden tool (not listed to LLM), `skill` + `path` params → returns a specific skill file. Called by ResourceRead when the name matches `<skill>/<path>` pattern.

The LLM can't call SkillReadFile directly — it has to go through ResourceRead with a `<skill>/<path>` pattern, which is indirect. Merge the file-reading capability into SkillRead by adding an optional `path` parameter.

### 5a. Update tool schema

**File: `crates/lr-skills/src/mcp_tools.rs`** — `build_meta_tool()`

Add optional `path` parameter:
```json
{
  "name": "SkillRead",
  "description": "Read a skill's full instructions, metadata, and file listing. Pass 'path' to read a specific skill file.",
  "input_schema": {
    "type": "object",
    "properties": {
      "name": {
        "type": "string",
        "description": "Skill name. Available: ..."
      },
      "path": {
        "type": "string",
        "description": "Optional: relative file path within the skill (e.g. 'scripts/run.sh'). Omit to get full instructions."
      }
    },
    "required": ["name"],
    "additionalProperties": false
  }
}
```

### 5b. Update `handle_skill_tool_call()`

**File: `crates/lr-skills/src/mcp_tools.rs`**

When `path` is provided:
- Delegate to existing `read_skill_file()` logic (already has fuzzy matching, permission checks)
- Return file content as `SkillToolResult::Response`

When `path` is omitted:
- Current behavior (return full skill instructions + metadata + file listing)

### 5c. Update skill catalog text

**File: `crates/lr-skills/src/mcp_tools.rs`** — `build_skill_catalog()`

Currently says:
```
Call SkillRead(name) to load full instructions.
Read skill files with ResourceRead(name="<skill>/<path>").
```

Change to:
```
Call SkillRead(name) to load full instructions.
Read skill files with SkillRead(name, path="<relative-path>").
```

### 5d. Update skill_read response

**File: `crates/lr-skills/src/mcp_tools.rs`** — `build_skill_read_response()`

Currently file listings say `Read with ResourceRead(name="...")`. Update to reference SkillRead with path:
```
Read with SkillRead(name="my-skill", path="scripts/run.sh").
```

### 5e. Remove SkillReadFile

- Remove `SKILL_READ_FILE_TOOL_NAME` constant
- Remove the `SkillReadFile` branch from `virtual_skills.rs` `handle_tool_call()`
- Remove `read_file_tool_name` from `SkillsConfig` and `SkillsSessionState`
- Remove `owns_tool` check for `read_file_tool_name`
- Remove config field `read_file_tool_name` from `crates/lr-config/src/types.rs` `SkillsConfig`

### 5f. Update ResourceRead skill file fallback

**File: `crates/lr-mcp-via-llm/src/orchestrator.rs`** — `execute_resource_read()`

The `<skill>/<path>` pattern in ResourceRead currently calls `gw_client.read_skill_file()` which invokes SkillReadFile. After SkillReadFile is removed:
- Option A: ResourceRead calls SkillRead with `path` parameter instead (routes through gateway as a regular tool call)
- Option B: Remove the skill file fallback from ResourceRead entirely — users should use `SkillRead(name, path)` directly

Option B is cleaner: ResourceRead handles MCP resources only, SkillRead handles skill files. No cross-tool fallback.

### 5g. Update gateway_client

**File: `crates/lr-mcp-via-llm/src/gateway_client.rs`**

Remove `read_skill_file()` method (was calling SkillReadFile internally).

---

## Note: FTS5 Indexing Already Covered

No additional indexing work is needed. The existing context management infrastructure handles everything:
- **Prompt/resource definitions** are already indexed at session init (`mcp/<slug>/prompt/<name>`, `mcp/<slug>/resource/<name>`) via `format_prompt_as_markdown()` / `format_resource_as_markdown()` in merger.rs
- **Skill definitions** are indexed via `build_skill_index_entries()` at `catalog:skills/<name>`
- **Tool outputs** (PromptRead, ResourceRead, SkillRead responses) are automatically compressed/indexed via `compress_client_tool_response()` if they exceed the threshold (~8KB)
- **Catalog compression** already defers prompts and resources in Phase 2 of the compression plan

The only requirement: tool descriptions should reference IndexSearch when context management is active (already covered in Phase 2a and 4b).

---

## Phase 6: Remove Config Fields (McpViaLlmConfig)

**File: `crates/lr-config/src/types.rs`**

1. Remove from `McpViaLlmConfig`:
   - `expose_resources_as_tools: bool`
   - `inject_prompts: bool`
2. Remove default helper functions if now unused
3. Old configs with these fields will be silently ignored by serde (no `deny_unknown_fields`)

**File: `crates/lr-config/src/migration.rs`**
- Add migration to bump config version (fields just get dropped)

**Files to update references:**
- `crates/lr-mcp-via-llm/src/orchestrator.rs` — remove `config.inject_prompts` / `config.expose_resources_as_tools` references
- `crates/lr-mcp-via-llm/src/orchestrator_stream.rs` — same
- `crates/lr-mcp-via-llm/src/tests.rs` — remove fields from test configs
- `crates/lr-mcp-via-llm/src/integration_tests.rs` — same

---

## Phase 7: Tests

### Update existing tests
- Remove tests for `inject_prompt_tools()`, `inject_prompt_messages()` (deleted functions)
- Remove `inject_prompts` / `expose_resources_as_tools` from test config construction
- Remove/update tests for SkillReadFile (merged into SkillRead)

### Add new tests

**PromptRead:**
- `prompt_read_tool_schema()` — verifies tool has correct name, parameters, and lists prompt names in description
- `prompt_read_no_arg_prompt()` — fetches and returns content directly
- `prompt_read_parameterized_missing_args()` — returns error with argument docs
- `prompt_read_parameterized_with_args()` — passes arguments through to `get_prompt()`
- `prompt_read_fuzzy_match()` — fuzzy match resolves + prepend correction note
- `prompt_read_not_found()` — lists available prompt names in error

**ResourceRead:**
- `resource_read_fuzzy_match()` — fuzzy match on resource names
- `resource_read_not_found_lists_names()` — error includes available resource names

**SkillRead (merged):**
- `skill_read_with_path()` — returns specific file content when `path` param provided
- `skill_read_without_path()` — returns full instructions (existing behavior)
- `skill_read_path_blocks_skill_md()` — SKILL.md still blocked via path param

**Shared:**
- Fuzzy matching tests in `lr-types`

---

## Phase 8: Mandatory Final Steps

1. **Plan Review** — verify all code paths referencing removed config fields, deleted functions, and SkillReadFile are updated
2. **Test Coverage Review** — ensure all new functions have unit tests
3. **Bug Hunt** — check edge cases:
   - `list_prompts()` failure → log warning, don't inject PromptRead tool
   - Empty prompts → don't inject tool
   - Very short prompt names (1-2 chars) and aggressive fuzzy matching
   - ResourceRead without skill file fallback (removed in Phase 5f)
   - SkillRead path parameter with fuzzy-matched skill name

---

## Verification

1. `cargo test -p lr-types` — fuzzy matching tests pass
2. `cargo test -p lr-skills` — skill tests pass (SkillReadFile merged into SkillRead)
3. `cargo test -p lr-context` — context tests still pass
4. `cargo test -p lr-mcp` — gateway tests pass (SkillReadFile removed from virtual_skills)
5. `cargo test -p lr-mcp-via-llm` — all new and existing tests pass
6. `cargo test && cargo clippy && cargo fmt` — full suite
7. Manual: use Try it Out client in MCP via LLM mode:
   - Verify no `McpPromptGet` events on initial chat message
   - Ask LLM to use a prompt → LLM calls `PromptRead` → single `McpPromptGet` event
   - Verify prompt and resource names appear in tool parameter descriptions
   - Test fuzzy matching by misspelling a prompt/resource name
   - Test `SkillRead(name, path="...")` reads a skill file

---

## Key Files

| File | Changes |
|------|---------|
| `crates/lr-types/src/fuzzy.rs` | **New** — shared fuzzy matching |
| `crates/lr-types/src/lib.rs` | Add `pub mod fuzzy` |
| `crates/lr-skills/src/fuzzy.rs` | Replace with re-export from lr-types |
| `crates/lr-context/src/fuzzy.rs` | Replace with delegation to lr-types |
| `crates/lr-skills/src/mcp_tools.rs` | Merge SkillReadFile into SkillRead (add `path` param) |
| `crates/lr-mcp/src/gateway/virtual_skills.rs` | Remove SkillReadFile branch, update owns_tool |
| `crates/lr-mcp-via-llm/src/orchestrator.rs` | PromptRead tool, fuzzy ResourceRead, remove old injection, remove skill file fallback |
| `crates/lr-mcp-via-llm/src/orchestrator_stream.rs` | Mirror orchestrator changes |
| `crates/lr-mcp-via-llm/src/gateway_client.rs` | Add `list_resources()`, remove `read_skill_file()` |
| `crates/lr-config/src/types.rs` | Remove 2 McpViaLlmConfig fields, remove `read_file_tool_name` from SkillsConfig |
| `crates/lr-config/src/migration.rs` | Config version bump |
| `crates/lr-mcp-via-llm/src/tests.rs` | Update + new tests |
