# Skill Read & Resource Read Refactor

**Date:** 2026-03-10
**Status:** In Progress

## Goal

Refactor how skills and MCP resources are exposed to the LLM in MCP via LLM:
1. Move skill/resource catalogs from tool definitions into the welcome message
2. Replace N synthetic per-resource tools with a single `resource_read` tool
3. Rename `skill_get_info` → `skill_read`, strip catalog from tool definition
4. Expose skill files (scripts, references, assets) as virtual resources readable via `resource_read`
5. Make virtual server instructions (skills listing) compressible when context management is enabled

## Architecture

### Current State

- **Skills**: Single `skill_get_info` tool has ALL skill names+descriptions in its description AND as enum values in input_schema. Blows up with many skills.
- **Resources**: Each MCP resource gets its own synthetic tool (`mcp_resource__<name>`). N resources = N tool definitions.
- **Welcome message**: Already lists tool/resource/prompt names per server. Regular server blocks are compressible. Virtual server blocks (Skills, Marketplace, etc.) are never compressed.

### New State

- **`skill_read` tool**: Generic description, accepts any string `name`. No catalog embedded.
- **`resource_read` tool**: Single tool, accepts `name` string. Resolves skill files locally or delegates to gateway.
- **Welcome message**: Contains the skill catalog (names + descriptions) in the Skills virtual server block, and resource names in regular server blocks (already there).
- **Compression**: Skills listing self-compresses when CM enabled + many skills. Resource listing already compressible via regular server blocks.

### LLM Flow

```
1. Welcome message lists:
   <skills>
   - `skill_read` (tool)
   - `resource_read` (tool)
   Available skills:
   - `data-analysis`: Analyze datasets with pandas (3 scripts, 2 refs)
   - `code-review`: Review code quality (1 script)
   Call skill_read(name) to load full instructions.
   Read skill files with resource_read(name="<skill>/<path>").
   </skills>

   <my-filesystem>
   - `filesystem__read_file` (tool)
   - `filesystem__cwd` (resource)
   </my-filesystem>

2. LLM calls skill_read(name="data-analysis") → gets SKILL.md body + file listing:
   ## Scripts (readable via resource_read)
   - data-analysis/scripts/build.sh
   - data-analysis/scripts/analyze.py
   ## References
   - data-analysis/references/api-docs.md

3. LLM reads a script: resource_read(name="data-analysis/scripts/build.sh") → file content

4. LLM reads an MCP resource: resource_read(name="filesystem__cwd") → gateway resources/read
```

### Skill File Resolution in resource_read

- Names matching `<skill_name>/<subpath>` where skill_name is a known skill → read from disk
- SKILL.md is excluded from resource_read (only returned by skill_read)
- Everything else → delegate to `gw_client.read_resource()` (existing MCP gateway path)

### Compression Tiers for Skills (virtual server self-compression)

- **No CM or ≤ 20 skills**: Full listing with name + short description + file counts
- **CM + > 20 skills**: Names only, append ctx_search hint
- **CM + > 50 skills**: Top 10 names + "... and N more — ctx_search to discover"

### Tool Compression Exclusions

Both `skill_read` and `resource_read` are marked as never-defer. Their descriptions mention using ctx_search if items are hidden.

## Files to Modify

| File | Change |
|------|--------|
| `crates/lr-skills/src/mcp_tools.rs` | Rename to `skill_read`, strip catalog from tool, change file paths to resource_read-compatible |
| `crates/lr-mcp/src/gateway/virtual_skills.rs` | Add skill catalog to `build_instructions()`, CM-aware truncation |
| `crates/lr-mcp-via-llm/src/orchestrator.rs` | Replace `inject_resource_tools` with single `resource_read` tool, add skill-file resolution |
| `crates/lr-mcp-via-llm/src/orchestrator_stream.rs` | Same as orchestrator.rs |
| `crates/lr-mcp-via-llm/src/gateway_client.rs` | Add `read_skill_file()` helper |
| `crates/lr-mcp-via-llm/src/tests.rs` | Update existing tests, add new ones |
| `crates/lr-mcp/src/gateway/gateway_tools.rs` | Exclude `skill_read`/`resource_read` from deferred loading |

## What's NOT Changing

- MCP resource listing in welcome message (already works via merger.rs server blocks)
- Gateway's `resources/read` handling (already supports name-based lookup)
- Context management compression framework for regular server blocks
- Config flags semantics (`expose_resources_as_tools` controls `resource_read` injection)
