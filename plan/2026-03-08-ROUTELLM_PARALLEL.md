# Parallelize RouteLLM Classification in Chat Pipeline

**Date**: 2026-03-08
**Status**: Implemented

## Goal

Move RouteLLM strong/weak classification out of the router into `chat.rs` as a third parallel task alongside guardrails and compression, saving ~15-50ms per request.

## Changes

### `crates/lr-providers/src/lib.rs`
- Added `PreComputedRouting` struct (`is_strong: bool`, `win_rate: f32`)
- Added `pre_computed_routing: Option<PreComputedRouting>` field to `CompletionRequest` (serde-skipped)

### `crates/lr-router/src/lib.rs`
- Simplified `select_models_for_auto_routing()` to read pre-computed classification from the request instead of calling `predict_with_threshold()` directly

### `crates/lr-server/src/routes/chat.rs`
- Moved rate limit check before spawning parallel tasks (reject early)
- Added `spawn_routellm_classification()` helper that runs classification as a parallel tokio task
- RouteLLM handle spawned alongside guardrails and compression (only for `localrouter/auto`)
- Result awaited after compression and injected into provider request

## Flow

```
1. Check rate limits (reject early)
2. Spawn guardrails       ─┐
3. Spawn compression      ─┤ all three parallel
4. Spawn routellm          ─┘ (only if model == "localrouter/auto")
5. Await compression → apply
6. Await routellm → inject into provider request
7. Convert to provider format
8. Router reads pre_computed_routing (no classification)
```
