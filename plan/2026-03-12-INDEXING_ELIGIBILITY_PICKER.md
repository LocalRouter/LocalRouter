# Indexing Eligibility Picker + Configurable Tool Names

**Date**: 2026-03-12
**Status**: In Progress

## Overview

Adds fine-grained control over what gets indexed into FTS5:
1. Unified MCP Gateway indexing — GLOBAL picker for gateway tools
2. Client Tools indexing — per-client picker for MCP via LLM clients
3. Configurable tool names — IndexSearch/IndexRead instead of ctx_search/ctx_read
4. Client tool response indexing in MCP via LLM mode
5. Tool preview table in global settings

## Implementation Phases

1. Config types (IndexingState, GatewayIndexingPermissions, ClientToolsIndexingPermissions)
2. Configurable tool names (replace hardcoded constants)
3. Known client tool lists
4. Virtual server tool eligibility metadata
5. Backend filtering for gateway tools
6. Client tool response indexing in MCP via LLM
7. Tauri commands
8. Global UI (tool names, preview, threshold, gateway picker, client default)
9. Per-client client tools picker
10. Demo mocks & tests

## Key Design Decisions

- Session snapshotting: all config captured at session creation, active sessions unaffected by changes
- Gateway indexing is GLOBAL only — applies to all clients
- Client tools indexing has global default + per-client overrides
- Virtual server non-indexable tools shown disabled in picker
- Tool names default to IndexSearch/IndexRead (breaking change for existing sessions)
- CONFIG_VERSION bump to 21 (no-op migration)
