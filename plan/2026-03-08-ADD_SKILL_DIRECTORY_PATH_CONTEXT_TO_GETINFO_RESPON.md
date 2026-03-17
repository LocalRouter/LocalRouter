# Plan: Add skill directory path context to `get_info` response

## Context
Skills may reference relative paths in their SKILL.md instructions body (e.g., `./scripts/deploy.sh`, `./references/api-docs.md`). When the AI calls a skill's `get_info` tool, the response lists scripts/references/assets with absolute paths, but the raw SKILL.md body is appended without any note about where the skill lives. An AI client may not realize that `./scripts/foo.sh` in the instructions maps to the absolute paths listed above.

**Goal:** Make it unambiguous to the AI where the skill directory is, and that any relative paths in the instructions are relative to that directory.

## What changes and where

All changes are in the **`get_info` tool response** — the text the AI receives when it calls `skill_X_get_info`. Nothing changes in the tool description (the short summary shown in tool listings).

### File: `crates/lr-skills/src/mcp_tools.rs` — `build_get_info_response()`

**Change 1: Add `Location` line in the metadata block**

After the metadata (name, description, version, author, tags) and before the `## Scripts` section, add a line showing the absolute path to the SKILL.md file. This gives the AI a clear anchor.

In the response it will appear as:
```
**Location:** `/Users/matus/.localrouter-dev/skills/deploy-app/SKILL.md`
```

**Change 2: Add a self-contained note at the top of `## Instructions`**

Right after the `## Instructions` heading and before the SKILL.md body content, add a note that **includes the absolute path directly** (not referencing "above"):

```
> File paths in these instructions are relative to: `/Users/matus/.localrouter-dev/skills/deploy-app/`
```

This way even if context is truncated, the AI still knows where `./scripts/foo.sh` resolves to.

### No other files need changes

- The tool description (short summary in tool listings) doesn't need the path — it's just for discovery.
- The system instructions in `virtual_skills.rs` (`build_instructions`) already mention using absolute paths with `ctx_execute_file`.

## Implementation

In `build_get_info_response()` in `crates/lr-skills/src/mcp_tools.rs`:

1. After line 178 (`text.push('\n');` after tags), insert:
   ```rust
   text.push_str(&format!("**Location:** `{}/SKILL.md`\n\n", skill_dir));
   ```

2. After line 217 (`text.push_str("## Instructions\n\n");`), insert:
   ```rust
   text.push_str(&format!(
       "> File paths in these instructions are relative to: `{}`\n\n",
       skill_dir
   ));
   ```

## Verification

1. `cargo test -p lr-skills` — ensure existing skill tests pass (may need to update snapshot assertions)
2. `cargo clippy -p lr-skills` — lint check
3. Manual: start dev, call a skill's `get_info` tool via MCP, verify the response includes the directory path and the relative-path note
