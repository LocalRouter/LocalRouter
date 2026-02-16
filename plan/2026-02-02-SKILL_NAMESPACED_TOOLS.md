# Refactor: Per-Skill Namespaced MCP Tools with Deferred Loading

## Summary

Replace the current shared skill tools (`show-skill_{name}`, `get-skill-resource`, `run-skill-script`, `get-skill-script-run`) with per-skill namespaced tools and deferred loading. Only `get_info` tools are initially visible; run/read tools appear after the client calls `get_info` for each skill.

## New Tool Naming

| Current | New (per skill per file) |
|---------|--------------------------|
| `show-skill_{name}` | `skill_{sname}_get_info` |
| `run-skill-script(skill_name, script)` | `skill_{sname}_run_{sfile}` |
| `get-skill-resource(skill_name, resource)` | `skill_{sname}_read_{sfile}` |
| `run-skill-script(async=true)` | `skill_{sname}_run_async_{sfile}` (when async_enabled) |
| `get-skill-script-run(pid)` | `skill_get_async_status` (when async_enabled) |

Where `sname`/`sfile` = sanitized to `[a-z0-9_-]`, consecutive underscores collapsed.

## Files to Modify

### 1. `crates/lr-skills/src/types.rs` — Add sanitization helpers
- `pub fn sanitize_name(input: &str) -> String` — lowercase, replace non-`[a-z0-9_-]` with `_`, collapse consecutive `_`, trim edges
- `pub fn sanitize_tool_segment(file_path: &str) -> String` — strip directory prefix (`scripts/`, `references/`, `assets/`), then sanitize (dots become underscores, so `build.sh` → `build_sh`)

### 2. `crates/lr-config/src/lib.rs` — Add async config
- Add `pub async_enabled: bool` (default false via `#[serde(default)]`) to `SkillsConfig` at line 724

### 3. `crates/lr-mcp/src/gateway/session.rs` — Track info-loaded skills
- Add field: `pub skills_info_loaded: HashSet<String>` (init empty)
- Add methods: `mark_skill_info_loaded(&mut self, name: &str)`, `is_skill_info_loaded(&self, name: &str) -> bool`

### 4. `crates/lr-skills/src/mcp_tools.rs` — Complete rewrite

**New tool builders:**
- `build_get_info_tool(skill)` → `skill_{sname}_get_info` (no input params)
- `build_run_tool(skill, script)` → `skill_{sname}_run_{sfile}` (input: optional command, timeout, tail)
- `build_run_async_tool(skill, script)` → `skill_{sname}_run_async_{sfile}` (same inputs)
- `build_read_tool(skill, resource)` → `skill_{sname}_read_{sfile}` (no input params)
- `build_get_async_status_tool()` → `skill_get_async_status` (input: pid, tail)

**New `build_skill_tools` signature:**
```rust
pub fn build_skill_tools(
    skill_manager: &SkillManager,
    access: &SkillsAccess,
    info_loaded: &HashSet<String>,
    async_enabled: bool,
) -> Vec<McpTool>
```
- Always include `get_info` tools for all allowed skills
- Only include run/read/run_async tools for skills in `info_loaded`
- Include `skill_get_async_status` only when `async_enabled` and any skill loaded

**New return type for `handle_skill_tool_call`:**
```rust
pub enum SkillToolResult {
    Response(serde_json::Value),
    InfoLoaded { skill_name: String, response: serde_json::Value },
}
```
- `InfoLoaded` signals gateway to update session and invalidate tools cache

**Tool name parsing:**
```rust
enum SkillToolParsed {
    GetInfo { skill_name: String },
    Run { skill_name: String, script_file: String },
    RunAsync { skill_name: String, script_file: String },
    Read { skill_name: String, resource_file: String },
    GetAsyncStatus,
}
```
- Parse by iterating allowed skills, matching `skill_{sanitized_name}_` prefix
- Reverse-map sanitized file name by comparing against skill's scripts/references/assets
- For run/read: check `info_loaded`, return error if not loaded

**`build_show_skill_response` (now `build_get_info_response`):**
- Update tool name references to new naming convention
- List available run/read tools by their exact new names

### 5. `crates/lr-mcp/src/gateway/gateway.rs` — Gateway integration

**`is_skill_tool`:** Simplify to `tool_name.starts_with("skill_")`

**`append_skill_tools`:** New signature passes `info_loaded: &HashSet<String>` and `async_enabled: bool`. Update all 3 call sites (lines 1179, 1200, 1243) to read from session.

**`handle_skill_tool_call`:** After receiving `SkillToolResult::InfoLoaded`:
1. Write-lock session, call `mark_skill_info_loaded`
2. Invalidate tools cache
3. (Optional) Send `notifications/tools/list_changed`

### 6. `crates/lr-mcp/src/gateway/types.rs` — Config plumbing
- Add `pub skills_async_enabled: bool` to gateway config (or store on gateway struct)

### 7. Wire config → gateway construction
- Pass `SkillsConfig::async_enabled` through to wherever gateway reads it

## Implementation Order

1. `lr-config/src/lib.rs` — `async_enabled` field (trivial)
2. `lr-skills/src/types.rs` — sanitize helpers + tests
3. `lr-mcp/src/gateway/session.rs` — info_loaded field
4. `lr-skills/src/mcp_tools.rs` — full rewrite (biggest change)
5. `lr-mcp/src/gateway/gateway.rs` — update all skill integration points
6. Config wiring (types.rs + construction site)
7. Tests

## Edge Cases

- **Name collisions**: Two skills sanitizing to same name → log warning, last wins
- **Reverse mapping**: O(n*m) lookup but negligible for typical skill/file counts
- **Cache invalidation on get_info**: Full cache invalidation (existing pattern)

## Verification

1. `cargo test -p lr-skills` — unit tests for sanitization, tool generation, parsing, gate enforcement
2. `cargo test -p lr-mcp` — session tests for info_loaded tracking
3. `cargo clippy && cargo fmt` — lint pass
4. Manual test: start dev server, connect MCP client, verify:
   - `tools/list` returns only `get_info` tools for skills
   - Calling `run` tool before `get_info` returns error
   - After calling `get_info`, `tools/list` returns expanded tools
   - Run and read tools work correctly
   - With `async_enabled: true`, async tools appear
