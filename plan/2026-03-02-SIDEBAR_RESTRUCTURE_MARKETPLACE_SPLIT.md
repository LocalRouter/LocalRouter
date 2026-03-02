# Sidebar Restructuring + Marketplace Split into Skills/MCP Pages

**Date**: 2026-03-02
**Status**: In Progress

## Goals
1. Group LLM-related items (Providers, GuardRails, Strong/Weak) under an "LLM" heading in sidebar
2. Make headings become separators when sidebar is collapsed
3. Remove standalone Marketplace page and embed into Skills and MCP pages as tabs
4. Split marketplace enable/disable into separate toggles for MCP and Skills

## Backend Changes
- Bump CONFIG_VERSION to 17
- Replace `MarketplaceConfig.enabled` with `mcp_enabled` + `skills_enabled`
- Add migration v17 copying old `enabled` to both new fields
- Add `is_mcp_enabled()`, `is_skills_enabled()` methods to MarketplaceService
- Add new Tauri commands: `marketplace_set_mcp_enabled`, `marketplace_set_skills_enabled`

## Frontend Changes
- Sidebar: merge featureNavItems into resourceNavEntries under "LLM" heading
- Sidebar: show separator line for headings when collapsed
- Remove marketplace route from App.tsx
- MCP page: add Marketplace, Settings, Try It Out tabs
- Skills page: add Marketplace, Settings tabs; remove marketplace from Add Skills dialog
- Update MarketplaceSearchPanel for per-type enabled checks
- Update TypeScript types and demo mocks
