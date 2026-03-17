# Plan: Expand OpenAI-Compatible API Surface

## Overview

Expanding LocalRouter from ~30% to ~80% coverage of the OpenAI API surface. Plans are split into 7 independent work streams that can be implemented in parallel.

**Reference saved:** `plan/2026-03-15-PROVIDER-ENDPOINT-COVERAGE.md` — full provider support matrix

## Implement First

| # | Plan File | Description | Est. LOC |
|---|-----------|-------------|----------|
| 0 | `plan/2026-03-15-FEATURE-SUPPORT-MATRIX.md` | Feature support matrix UI (provider Info tab + Optimize page) | ~800 |

This must be implemented first so that all subsequent API additions automatically appear as `NotImplemented` → `Supported` in the UI.

## Independent Work Streams (implement after #0)

| # | Plan File | Endpoints | Est. LOC | New Crate? |
|---|-----------|-----------|----------|------------|
| 1 | `plan/2026-03-15-API-AUDIO.md` | `/v1/audio/transcriptions`, `/v1/audio/translations`, `/v1/audio/speech` | ~1500 | No |
| 2 | `plan/2026-03-15-API-MODERATIONS.md` | `/v1/moderations` | ~400 | No |
| 3 | `plan/2026-03-15-API-FILES-BATCHES.md` | `/v1/files/*`, `/v1/batches/*` | ~2000 | `lr-files`, `lr-batches` |
| 4 | `plan/2026-03-15-API-RESPONSES.md` | `/v1/responses` | ~2500 | `lr-responses` |
| 5 | `plan/2026-03-15-API-REALTIME.md` | `WSS /v1/realtime` | ~3000 | `lr-realtime` |
| 6 | `plan/2026-03-15-API-CHAT-PARAMS.md` | Existing `/v1/chat/completions` improvements | ~300 | No |

**All 6 are independent** — can be implemented in parallel by separate engineers/agents.
**Exception:** Within plan #3, Files must be done before Batches.

## Translation Layer Summary

| Endpoint | Translation? | Strategy |
|----------|-------------|----------|
| Audio (STT/TTS) | **No** | Native only — requires specialized audio models |
| Moderations | **Possible (Medium)** | Could route to safety models via chat completions. Defer. |
| Files | **N/A** | Local storage only |
| Batches | **Yes (High)** | Process JSONL locally → all providers supported |
| Responses API | **Yes (High)** | Convert to chat completions → all providers supported |
| Realtime | **No** | Native only — latency requirements |
| Chat params | **Varies** | Mostly pass-through |

## Cross-Cutting Features Summary

Each plan has a detailed feature applicability table. High-level:

- **Full pipeline**: Responses API (translation mode), Batches (translated mode)
- **Standard pipeline** (auth + permissions + rate limiting + metrics): Audio, Moderations, Files, Batches (native)
- **Connection-time only**: Realtime WebSocket
- **Always apply**: Auth, permissions, rate limiting, metrics, client activity
- **Never apply to new endpoints**: Prompt compression (chat only), RouteLLM (auto model only), JSON repair (JSON format only)

## Shared Modifications

All plans touch these files (coordinate to minimize merge conflicts):
- `crates/lr-providers/src/lib.rs` — trait methods + types + Capability enum
- `crates/lr-server/src/lib.rs` — route registration in `build_app()`
- `crates/lr-server/src/routes/mod.rs` — module declarations
- `crates/lr-server/src/openapi/mod.rs` — OpenAPI registration
- `crates/lr-router/src/lib.rs` — dispatch methods

Strategy: each plan adds to the END of registrations; use separate branches, merge sequentially.
