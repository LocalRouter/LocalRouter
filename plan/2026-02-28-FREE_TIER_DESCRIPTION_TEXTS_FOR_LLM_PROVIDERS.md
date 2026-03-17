# Free Tier Description Texts for LLM Providers

## Context

Users adding LLM providers need to quickly understand which providers offer free usage. Currently, free tier data exists as structured enums (`FreeTierKind`) but there's no human-readable text exposed to the UI. We need short and long text descriptions generated from the existing free tier config, displayed on provider cards (wizard), the free tier tab, and other relevant locations.

Additionally, we need to save the free tier research to `./plan/` and verify the current `default_free_tier()` values match reality.

## Plan

### Step 0: Save Research Document
- Save the free tier research findings to `./plan/2026-02-28-FREE_TIER_RESEARCH.md`

### Step 1: Verify & Update Free Tier Defaults
Check each provider's `default_free_tier()` in `crates/lr-providers/src/factory.rs` against current reality. Based on research:
- **OpenAI**: `None` — correct (no free API tier)
- **Anthropic**: `None` — correct (no free API tier)
- **All local providers**: `AlwaysFreeLocal` — correct
- **Gemini** (10 RPM, 250 RPD, 250K TPM): verify, may need update post-Dec 2025 cuts
- **Groq, Cerebras, Mistral, Cohere**: verify limits match current offerings
- **OpenRouter** ($0, ProviderApi): correct
- **Perplexity** ($5/mo), **DeepInfra** ($5/mo), **xAI** ($25 one-time): verify
- **Together AI** (Llama-3.3-70B free, 3 RPM): correct
- **GitHub Copilot, ChatGPT Plus**: `Subscription` — correct

No changes expected unless research shows limit changes.

### Step 2: Backend — Add Text Generation Function
**File: `crates/lr-providers/src/registry.rs`**

Add `format_number(n: u64) -> String` helper (e.g., 250000→"250K", 1B→"1B").

Add `free_tier_description_texts(kind: &FreeTierKind) -> (String, String)` that pattern-matches on `FreeTierKind` to produce `(short_text, long_text)`:

| Kind | Short Text | Long Text |
|------|-----------|-----------|
| `None` | *(empty)* | No free tier available. All API usage is billed. |
| `AlwaysFreeLocal` | Free — runs locally | Runs entirely on your machine. No API costs, no rate limits. |
| `Subscription` | Included in subscription | Included in your existing subscription at no additional cost. |
| `RateLimitedFree` | Free tier: {top 2 limits} | Free access within rate limits: {all limits}. Router auto-skips when exhausted. |
| `CreditBased` ($0) | Free models available | Some models available for free via provider API. |
| `CreditBased` ($X) | $X/mo free credits | $X in free credits that reset monthly/never. Router auto-skips when exhausted. |
| `FreeModelsOnly` | Free models: {rpm} RPM | Specific models available for free. Rate-limited to {rpm} req/min. |

### Step 3: Backend — Extend ProviderTypeInfo Struct
**File: `crates/lr-providers/src/registry.rs`**

Add three fields to `ProviderTypeInfo`:
```rust
pub default_free_tier: lr_config::FreeTierKind,
pub free_tier_short_text: String,
pub free_tier_long_text: String,
```

Update `list_provider_types()` to call `factory.default_free_tier()` and `free_tier_description_texts()` to populate these.

### Step 4: Backend — Unit Tests
**File: `crates/lr-providers/src/registry.rs`**

Add tests for `format_number` and `free_tier_description_texts` covering all variants.

### Step 5: TypeScript — Update Types
**File: `src/types/tauri-commands.ts`** — Add 3 fields to `ProviderTypeInfo` interface:
```typescript
default_free_tier: FreeTierKind
free_tier_short_text: string
free_tier_long_text: string
```

**File: `src/components/ProviderForm.tsx`** — Add optional fields to local `ProviderType` interface:
```typescript
free_tier_short_text?: string
free_tier_long_text?: string
```

### Step 6: UI — Provider Cards in Add Provider Wizard
**File: `src/views/resources/providers-panel.tsx`** (~line 1548-1556, `ProviderButton` component)

Add `free_tier_short_text` below the description, conditionally rendered (skip if empty). Style: small green text, `text-[11px] text-green-600 dark:text-green-400 font-medium`.

### Step 7: UI — Free Tier Tab
**File: `src/views/resources/providers-panel.tsx`** (~line 970-982, Free Tier tab)

Add `free_tier_long_text` as an info paragraph at top of the Configuration Card, between CardDescription and the Type badge. Look up `providerTypes` to find the matching type info. Style: `text-sm text-muted-foreground`.

### Step 8: UI — Other Display Locations
Check and add where sensible:
- **Wizard StepModels** (`src/components/wizard/steps/StepModels.tsx`): If provider dropdown exists, show short text next to provider name
- **Provider list sidebar** in providers-panel.tsx: Show short text badge next to each provider name in the left panel list

### Step 9: Mock Data
**File: `website/src/components/demo/mockData.ts`** — Add `default_free_tier`, `free_tier_short_text`, `free_tier_long_text` to all entries in `providerTypes` array.

### Step 10: Verify
- `cargo test && cargo clippy && cargo fmt`
- `npx tsc --noEmit`

## Files to Modify
1. `crates/lr-providers/src/registry.rs` — Core: struct, text gen function, tests
2. `crates/lr-providers/src/factory.rs` — Only if free tier defaults need updating
3. `src/types/tauri-commands.ts` — TypeScript type
4. `src/components/ProviderForm.tsx` — Local ProviderType interface
5. `src/views/resources/providers-panel.tsx` — Provider cards + free tier tab + provider list
6. `src/components/wizard/steps/StepModels.tsx` — Wizard provider dropdown (if applicable)
7. `website/src/components/demo/mockData.ts` — Mock data
8. `plan/2026-02-28-FREE_TIER_RESEARCH.md` — Research document (new file)
