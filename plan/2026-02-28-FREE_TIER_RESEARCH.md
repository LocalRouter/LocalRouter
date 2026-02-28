# Free Tier Research — LLM Providers (Feb 2026)

## Goal
Verify `default_free_tier()` values in `crates/lr-providers/src/factory.rs` and add human-readable description texts.

## Provider Free Tier Summary

| Provider | FreeTierKind | Verified | Notes |
|---|---|---|---|
| **OpenAI** | `None` | Yes | No free API tier |
| **Anthropic** | `None` | Yes | No free API tier |
| **Gemini** | `RateLimitedFree` (10 RPM, 250 RPD, 250K TPM) | Yes | Google AI Studio free tier |
| **Groq** | `RateLimitedFree` (30 RPM, 14.4K RPD, 6K TPM, 500K TPD) | Yes | Free tier for all models |
| **Cerebras** | `RateLimitedFree` (30 RPM, 14.4K RPD, 60K TPM, 1M TPD) | Yes | Free inference tier |
| **Mistral** | `RateLimitedFree` (60 RPM, 500K TPM, 1B monthly tokens) | Yes | Free "experiment" tier |
| **Cohere** | `RateLimitedFree` (20 RPM, 100K TPM, 1K monthly calls) | Yes | Trial API key tier |
| **OpenRouter** | `CreditBased` ($0, never) | Yes | Free models available at $0 cost |
| **Perplexity** | `CreditBased` ($5/mo) | Yes | Monthly free credits for new accounts |
| **DeepInfra** | `CreditBased` ($5/mo) | Yes | Monthly free credits |
| **xAI** | `CreditBased` ($25, one-time) | Yes | One-time signup credits |
| **Together AI** | `FreeModelsOnly` (Llama-3.3-70B, 3 RPM) | Yes | Specific free models only |
| **GitHub Copilot** | `Subscription` | Yes | Included in GitHub Copilot subscription |
| **ChatGPT Plus** | `Subscription` | Yes | Included in ChatGPT Plus subscription |
| **Ollama** | `AlwaysFreeLocal` | Yes | Local inference |
| **LM Studio** | `AlwaysFreeLocal` | Yes | Local inference |
| **Jan** | `AlwaysFreeLocal` | Yes | Local inference |
| **GPT4All** | `AlwaysFreeLocal` | Yes | Local inference |
| **LocalAI** | `AlwaysFreeLocal` | Yes | Local inference |
| **llama.cpp** | `AlwaysFreeLocal` | Yes | Local inference |
| **OpenAI Compatible** | `AlwaysFreeLocal` | Yes | Assumed local/self-hosted |

## Changes Required
No changes to `default_free_tier()` values needed — all match current reality.

## Implementation
- Add `format_number()` and `free_tier_description_texts()` to `registry.rs`
- Extend `ProviderTypeInfo` with `default_free_tier`, `free_tier_short_text`, `free_tier_long_text`
- Display texts on provider cards (wizard), free tier tab, and provider list
