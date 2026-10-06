# Skill script locations in SkillRead

## Problem
An agent using the unified gateway could read a script-based skill (e.g.
`ticket-monitor`) but not run it: SkillRead listed files only as
`SkillRead(name, path)` references, gave no directory, and SKILL.md bodies
hardcoding Claude Code's `.claude/skills/<skill>/` paths pointed nowhere.
Agents had to search the disk for the script.

## Root cause
`2026-03-08-ADD_SKILL_DIRECTORY_PATH_CONTEXT_TO_GETINFO_RESPON` added a
`**Location:**` line, absolute file paths and a "paths are relative to" note.
Commit 5d85189c (MCP-via-LLM refactor, skill_get_info → skill_read) replaced
them with virtual `<skill>/<path>` references — right for MCP via LLM (the
model has no shell) but the same builder serves direct gateway clients, which
lost the paths too.

## Change
- `lr_skills::mcp_tools::SkillPathStyle { Disk, Virtual }`. The skills
  virtual server picks `Virtual` for MCP-via-LLM sessions (`McpMode::ViaLlm`,
  set on the gateway's synthetic client from the client mode) and `Disk`
  otherwise.
- Disk: SkillRead shows `**Location:**`, absolute paths for scripts,
  references and assets ("run scripts from their absolute path"), and a note
  that relative paths in the instructions resolve against the skill
  directory. The SkillRead tool description, the skills catalog in gateway
  instructions, the `catalog:skills` IndexSearch entries and the `path="."`
  listing mention locations too.
- Both styles resolve `{{SKILL_DIR}}` and Claude Code install paths
  (`.claude/skills/<name>/` with `~/`, `$HOME/`, `${HOME}/`, `./` or bare
  prefixes): absolute for Disk, skill-relative for Virtual.
- File reads (`SkillRead(name, path)`) stay byte-exact.
- Read hints are prefilled with a real file instead of `path="..."`: each
  SkillRead section uses its first file, and the skills catalog adds an
  `e.g. SkillRead(name="<first skill with files>", path="<its first file>")`.

## Tests
`lr-skills`: Disk vs Virtual responses for a ticket-monitor-like skill
(location, absolute paths, legacy path and placeholder rewriting), byte-exact
file reads, listings, tool description, catalog and index entries.
`mcp_gateway_stability_tests`: the gateway serves disk paths to direct MCP
clients and virtual paths to MCP-via-LLM sessions.
