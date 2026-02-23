# Free Tier Mode - Implementation Plan

## Date: 2026-02-23

## Goals
- Add provider-level free tier tracking with per-provider configuration
- Add per-strategy "free_tier_only" toggle that restricts routing to free resources
- Return 429 with retry-after when all free resources are exhausted
- Universal rate limit header parsing across all providers
- Provider backoff tracking to avoid hitting known-exhausted providers

## Architecture

### Core Abstraction: FreeTierKind
Discriminated union handling different free tier models:
- `None` - No free tier (OpenAI, Anthropic)
- `AlwaysFreeLocal` - Local providers (Ollama, LM Studio)
- `Subscription` - Subscription-based (GitHub Copilot)
- `RateLimitedFree` - Rate-limited access (Gemini, Groq, Cerebras, Mistral, Cohere)
- `CreditBased` - Dollar credits (OpenRouter, xAI, DeepInfra)
- `FreeModelsOnly` - Specific free models (Together AI)

### Design Principles
- Config-only provider setup (no custom per-provider handling code)
- Universal rate limit header parser for all naming conventions
- Generic handling per FreeTierKind variant (not per provider)
- Only OpenRouter gets custom API code (check_credits)

## Phases

### Phase 1: Config Types + Migration
- FreeTierKind, FreeTierResetPeriod, CreditDetection types in lr-config
- free_tier_only on Strategy, free_tier on ProviderConfig
- Config version 13 → 14

### Phase 2: Provider Changes
- default_free_tier() on ProviderFactory trait
- check_credits() on ModelProvider trait (only OpenRouter implements)
- All providers declare their FreeTierKind defaults

### Phase 3: FreeTierManager
- Universal rate limit header parser
- RateLimitTracker, CreditTracker, ProviderBackoff
- Classification logic, persistence

### Phase 4: Router Integration
- Backoff/free-tier checks in auto-routing loops
- FreeTierExhausted error → HTTP 429 with retry-after

### Phase 5: Tauri Commands
- get_free_tier_status, set_provider_free_tier, etc.

### Phase 6: Frontend Types + Demo Mock
- TypeScript types, strategy toggle, demo mock

### Phase 7: Wiring
- AppState, main.rs, background tasks

## Files Modified
- `crates/lr-config/src/types.rs` - New types + Strategy/ProviderConfig changes
- `crates/lr-config/src/migration.rs` - v14 migration
- `crates/lr-providers/src/lib.rs` - ModelProvider trait changes
- `crates/lr-providers/src/factory.rs` - ProviderFactory trait + all factories
- `crates/lr-providers/src/openrouter.rs` - check_credits() implementation
- `crates/lr-router/src/free_tier.rs` - NEW: FreeTierManager
- `crates/lr-router/src/lib.rs` - Router integration
- `crates/lr-types/src/errors.rs` - FreeTierExhausted error
- `src-tauri/src/ui/commands_free_tier.rs` - NEW: Tauri commands
- `src-tauri/src/ui/commands_clients.rs` - update_strategy changes
- `src-tauri/src/main.rs` - Registration + wiring
- `crates/lr-server/src/state.rs` - AppState changes
- `src/types/tauri-commands.ts` - Frontend types
- `website/src/components/demo/TauriMockSetup.ts` - Demo mock
