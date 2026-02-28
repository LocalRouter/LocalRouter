# AI Coding MCP - Implementation Plan

## Context

LocalRouter needs to orchestrate AI coding agents (Claude Code, Gemini CLI, Codex, etc.) as MCP tools through the Unified MCP Gateway. This enables any MCP client to spawn, interact with, and manage coding agent sessions — turning LocalRouter into a multi-agent orchestration hub.

The feature bundles into the gateway like Skills do (not as standalone MCP servers). Sessions are strictly tied to the creating client — no cross-client session visibility.

We use BloopAI/vibe-kanban's `executors` crate as a git dependency for agent process management.

## Phases

1. New Crate & Config Types
2. Core Types, Manager & Approval
3. MCP Tools & Gateway Integration
4. App Wiring & Tauri Commands
5. Frontend
6. Website Section

## Key Design Decisions

- Per-agent independent tool sets (6 tools each)
- Bidirectional streaming I/O
- Status polling + respond pattern
- Gateway-bundled (Skills pattern)
- Sessions are client-bound
