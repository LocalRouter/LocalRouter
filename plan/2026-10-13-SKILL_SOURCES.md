# Skill sources: per-source client permissions and Skills page revamp

## Problem

- The client MCP tab's "Skills Permissions" card lists every skill tool in a
  read-only `McpToolDisplay` block above the real permission tree. With many
  skills it is a wall of clickable-looking chips that do nothing.
- The permission tree ("All Skills") has no search and no way to grant a whole
  skill source (a configured path such as `~/.claude/skills`) at once.
- The Skills page is a flat list; there is no way to browse or manage skills by
  the source they came from.

## Design

### Permission model (`lr-config::SkillsPermissions`)

Add `sources: HashMap<String, PermissionState>` keyed by the configured skill
source path (`SkillDefinition::source_path`, which is the exact string in
`skills.paths`). Resolution becomes tool -> skill -> source -> global.

`#[serde(default, skip_serializing_if = "HashMap::is_empty")]` keeps existing
configs loading unchanged (no sources = old behaviour) and leaves configs
without source overrides byte-identical on save. No migration step is needed.

Resolution methods take the skill's source path:
`resolve_skill(source, skill)`, `resolve_tool(source, skill, tool)`,
`has_any_enabled_for_skill(source, skill)`. `has_any_access` also counts
source grants.

Callers updated: `lr-skills::mcp_tools` (has the `SkillDefinition`),
`access_control::FirewallCheckContext::SkillTool` (new `skill_source` field),
`virtual_skills::check_permissions` and `reevaluate_pending_approvals`
(look the source up in `SkillManager`), tray toggle.

### Tauri commands

- `set_client_skills_permission`: new `source` level.
- `clear_client_skills_child_permissions`: new optional `source_path`; clears
  the skill and tool overrides of every skill in that source. Global clear also
  clears sources.
- New `list_skill_sources` -> `SkillSourceInfo { path, label, kind, exists,
  skill_count }` with kind `user | marketplace | directory | archive`.
- New `set_skills_enabled(skill_names, enabled)` for enabling/disabling a whole
  source in one config save (`set_skill_enabled` delegates to it).

Marketplace installs are configured as one source per skill, so both the client
tree and the Skills page show them as a single display-only "Marketplace" group
of skills (no source-level button; skill-level permissions apply).

### Frontend

- Client MCP tab: remove the tool listing block; tree becomes
  All Skills -> Source -> Skill -> Tool with a search box
  (`PermissionTreeSelector` gains an opt-in `searchable` prop).
- Connection graph uses the source-aware resolution.
- Skills page: sidebar grouped by source (collapsible, counts, search across
  name/description/tags/source), source detail panel (path, kind, open folder,
  enable/disable all, remove source), overview panel when nothing is selected,
  skill detail keeps Info / Try It Out / Settings.
- Types in `src/types/tauri-commands.ts`, demo mocks in `TauriMockSetup.ts`.

## Status (2026-10-13)

Implemented. Verified: clippy (stable 1.99) clean, fmt clean, tsc (app +
website) clean, targeted tests for lr-config, lr-skills, lr-mcp and the
localrouter lib / permission_inheritance / skills_e2e / gateway stability
suites pass. UI checked in the website demo (Skills page overview, source
panel, skill detail; client MCP tab tree with search and source toggles).

## Final steps

1. Plan review against implementation.
2. Test coverage review (resolution order, serde back-compat, has_any_access).
3. Bug hunt.
4. clippy / fmt / targeted tests / tsc, commit and push.
