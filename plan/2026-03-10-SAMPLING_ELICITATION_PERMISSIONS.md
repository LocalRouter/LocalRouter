# Per-Client Sampling & Elicitation Permission Settings

**Date:** 2026-03-10
**Status:** In Progress

## Summary
Implement unified Allow/Ask/Off permission system for MCP sampling and elicitation features,
with behavior that varies by client mode, approval/form popups, capability negotiation gating,
and comprehensive tests.

## Key Changes
- Replace boolean `mcp_sampling_enabled` with `PermissionState` enum (Allow/Ask/Off)
- Add `mcp_elicitation_permission` field
- Config migration for existing settings
- Capability negotiation gating
- Sampling approval popup window
- Elicitation form popup window
- Runtime enforcement in request handlers
- Debug panel triggers
- ~28 test cases

## Implementation Order
1. Config & Data Model
2. Tauri Commands
3. Settings Tab UI
4. SamplingApprovalManager
5. Popup Windows
6. Capability Negotiation
7. Runtime Request Handling
8. Debug Panel
9. Tests
