# MCP Protocol Feature Gaps — Audit & Fix Plan

**Date**: 2026-03-19
**Status**: In Progress

## Summary

Redesign sampling/elicitation from per-client to global config, add passthrough/direct modes with client_mode compatibility, implement missing protocol features (SSE server→client requests, completion/complete, notifications/cancelled, progress namespacing, resources/templates/list, pagination).

## Steps

1. New global MCP config types (SamplingMode, ElicitationMode, McpGatewaySettings)
2. Wire global settings into gateway (GatewayConfig, register_request_handlers, capability filtering)
3. Add SamplingApprovalManager + PassthroughManager to gateway
4. Global settings UI (MCP Settings tab, Tauri commands)
5. SSE transport server→client request support
6. Fix completion/complete handling
7. Forward notifications/cancelled
8. Progress token namespacing
9. resources/templates/list
10. Exhaust upstream pagination

See full plan in Claude Code conversation transcript.
