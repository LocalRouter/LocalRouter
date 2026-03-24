# Plan: Update Free Tier Defaults & Provider Notes

## Context

User researched current free tier offerings from LLM providers and found:
- **Gemini** RPD is too high (250 vs actual 20 for premium models)
- **OpenRouter** uses CreditBased($0) but actually offers rate-limited free models
- Provider-specific caveats (varying limits by model, country, anti-abuse) aren't surfaced in the UI

This plan updates defaults and adds provider-specific notes to the existing `free_tier_short_text`/`free_tier_long_text` system.

---

## Step 1: Update Gemini free tier default

**File:** `crates/lr-providers/src/factory.rs:458-466` (GeminiProviderFactory)

Change:
```rust
FreeTierKind::RateLimitedFree {
    max_rpm: 10,
    max_rpd: 20,       // was 250 — use conservative Pro-tier limit
    max_tpm: 250_000,
    max_tpd: 0,
    max_monthly_calls: 0,
    max_monthly_tokens: 0,
}
```

## Step 2: Update OpenRouter free tier default

**File:** `crates/lr-providers/src/factory.rs:525-531` (OpenRouterProviderFactory)

Change from CreditBased to FreeModelsOnly:
```rust
FreeTierKind::FreeModelsOnly {
    free_model_patterns: vec![":free".to_string()],
    max_rpm: 20,
}
```

Note: OpenRouter free models have `:free` suffix in model IDs. RPD (50) is not captured in FreeModelsOnly — but the existing backoff system handles 429s adaptively, so this is acceptable. The 20 RPM is the key limit.

## Step 3: Add `free_tier_notes()` to ProviderFactory trait

**File:** `crates/lr-providers/src/factory.rs` (ProviderFactory trait, ~line 110)

Add method with default implementation:
```rust
/// Optional provider-specific notes about free tier caveats.
/// Appended to the auto-generated long description text.
fn free_tier_notes(&self) -> Option<&str> {
    None
}
```

## Step 4: Add notes for each provider with known caveats

**File:** `crates/lr-providers/src/factory.rs` (each factory impl)

| Provider | Notes |
|---|---|
| **Gemini** | "Rate limits vary significantly by model: Flash models allow up to 250 RPD while Pro models are limited to 20 RPD. Limits may also vary by region." |
| **Groq** | "Rate limits vary by model. Some models (e.g. Llama 3.3 70B) have lower daily limits (1K RPD). Token limits also vary per model." |
| **Cerebras** | "Developer tier offers 10x higher limits. Exact free tier limits are not publicly documented and may change." |
| **Mistral** | "Free tier (experiment plan) allows 1 request/second and 1 billion tokens/month. All models are accessible." |
| **Cohere** | "Trial API keys are limited to 1,000 API calls/month and 20 RPM. Contact support for production increases." |
| **OpenRouter** | "Free tier provides access to 25+ free models (model IDs ending in ':free') at 20 RPM / 50 RPD. Purchasing $10+ in credits unlocks 1,000 RPD on free models. BYOK gives 1M free requests/month." |
| **Together AI** | "Only specific models are free (currently Llama 3.3 70B Instruct Turbo Free). Rate limited to 3 RPM on free models." |
| **DeepInfra** | "$5 monthly free credits for inference. Credits reset monthly." |
| **xAI** | "$25 one-time signup credits. No recurring free tier." |
| **Perplexity** | "No free API tier. All API usage requires payment." |

## Step 5: Wire notes into ProviderTypeInfo

**File:** `crates/lr-providers/src/registry.rs`

1. Add `free_tier_notes: Option<String>` field to `ProviderTypeInfo` struct (~line 267)
2. In `list_provider_types()` (~line 341), populate from factory:
   ```rust
   free_tier_notes: factory.free_tier_notes().map(|s| s.to_string()),
   ```
3. Append notes to `free_tier_long_text` so existing UI picks them up automatically:
   ```rust
   let long_text = if let Some(notes) = factory.free_tier_notes() {
       format!("{}\n\n{}", long_text, notes)
   } else {
       long_text
   };
   ```

## Step 6: Update TypeScript types

**File:** `src/types/tauri-commands.ts`

Add to ProviderTypeInfo interface:
```typescript
freeTierNotes: string | null
```

## Step 7: Update demo mock

**File:** `website/src/components/demo/TauriMockSetup.ts`

Update mock data for OpenRouter and Gemini free tier defaults to match new values.

## Step 8: Update tests

**Files:**
- `crates/lr-providers/src/registry.rs` — update existing tests that check Gemini RPD=250
- `crates/lr-providers/src/factory.rs` — any tests referencing old defaults
- `crates/lr-router/src/free_tier.rs` — if any tests use specific Gemini/OpenRouter defaults

## Step 9: Update research doc

**File:** `plan/2026-02-28-FREE_TIER_RESEARCH.md`

Update the table to reflect new verified values (Gemini RPD=20, OpenRouter=FreeModelsOnly).

---

## Critical Files

| File | Change |
|---|---|
| `crates/lr-providers/src/factory.rs` | Gemini RPD, OpenRouter type, `free_tier_notes()` trait + impls |
| `crates/lr-providers/src/registry.rs` | `ProviderTypeInfo.free_tier_notes`, wire into long text |
| `src/types/tauri-commands.ts` | Add `freeTierNotes` field |
| `website/src/components/demo/TauriMockSetup.ts` | Update mock defaults |

## Verification

1. `cargo test -p lr-providers` — all provider tests pass
2. `cargo test -p lr-router` — free tier tests pass
3. `cargo clippy` — no warnings
4. `npx tsc --noEmit` — TypeScript compiles
5. Manual: check provider list in UI shows updated descriptions with notes

---

## Final Steps (mandatory)

1. **Plan Review** — check all steps against implementation
2. **Test Coverage Review** — ensure new `free_tier_notes()` method has tests
3. **Bug Hunt** — review for serde compat issues (new fields must have defaults)
4. **Commit** — stage only modified files
