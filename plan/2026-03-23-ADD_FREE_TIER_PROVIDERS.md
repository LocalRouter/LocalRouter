# Plan: Add 7 New Free-Tier Providers

## Context

User identified 7 inference providers with free tiers that LocalRouter doesn't yet support. All are OpenAI-compatible APIs, so each can reuse the existing `OpenAICompatibleProvider` implementation with a dedicated factory for proper naming, free tier defaults, and catalog mapping.

---

## Providers to Add

| Provider | Base URL | API Key Source | Free Tier | Category |
|---|---|---|---|---|
| **GitHub Models** | `https://models.inference.ai.azure.com` | GitHub PAT | RateLimitedFree: 10-15 RPM, 50-150 RPD (varies by model tier) | ThirdParty |
| **NVIDIA NIM** | `https://integrate.api.nvidia.com/v1` | NVIDIA API key | RateLimitedFree: 40 RPM | ThirdParty |
| **Cloudflare Workers AI** | `https://api.cloudflare.com/client/v4/accounts/{account_id}/ai/v1` | Cloudflare API token | RateLimitedFree: ~10K neurons/day (unusual metric) | ThirdParty |
| **LLM7.io** | `https://api.llm7.io/v1` | Optional token | RateLimitedFree: 30 RPM (120 with token) | ThirdParty |
| **Kluster AI** | `https://api.kluster.ai/v1` | Kluster API key | RateLimitedFree: limits undocumented | ThirdParty |
| **Hugging Face** | `https://router.huggingface.co/v1` | HF token | CreditBased: $0.10/month | ThirdParty |
| **Zhipu AI** | `https://open.bigmodel.cn/api/paas/v4` | Zhipu API key | RateLimitedFree: limits undocumented | FirstParty |

---

## Per-Provider Implementation Pattern

For each provider, create/modify these locations:

### 1. Add ProviderType variant

**File:** `crates/lr-config/src/types.rs` (~line 3287, ProviderType enum)

Add new variants (with serde rename as needed):
```rust
/// GitHub Models inference
#[serde(rename = "github_models")]
GitHubModels,
/// NVIDIA NIM inference
#[serde(rename = "nvidia_nim")]
NvidiaNim,
/// Cloudflare Workers AI
#[serde(rename = "cloudflare_ai")]
CloudflareAI,
/// LLM7.io inference
#[serde(rename = "llm7")]
Llm7,
/// Kluster AI inference
#[serde(rename = "kluster_ai")]
KlusterAI,
/// Hugging Face Inference
#[serde(rename = "huggingface")]
HuggingFace,
/// Zhipu AI (GLM models)
#[serde(rename = "zhipu")]
Zhipu,
```

### 2. Add factory for each provider

**File:** `crates/lr-providers/src/factory.rs`

Each factory follows the existing pattern (e.g. `GroqProviderFactory`). Key differences per provider:

#### GitHub Models
```rust
pub struct GitHubModelsProviderFactory;
impl ProviderFactory for GitHubModelsProviderFactory {
    fn provider_type(&self) -> &str { "github_models" }
    fn display_name(&self) -> &str { "GitHub Models" }
    fn category(&self) -> ProviderCategory { ProviderCategory::ThirdParty }
    fn description(&self) -> &str { "GitHub Models free inference API" }
    fn default_free_tier(&self) -> FreeTierKind {
        FreeTierKind::RateLimitedFree {
            max_rpm: 10, max_rpd: 50, max_tpm: 0, max_tpd: 0,
            max_monthly_calls: 0, max_monthly_tokens: 0,
        }
    }
    fn free_tier_notes(&self) -> Option<&str> {
        Some("Limits vary by model tier: Low models get 15 RPM / 150 RPD, High models get 10 RPM / 50 RPD. Uses GitHub Personal Access Token for auth.")
    }
    fn setup_parameters(&self) -> Vec<SetupParameter> {
        vec![SetupParameter::required("api_key", ParameterType::ApiKey, "GitHub Personal Access Token", true)]
    }
    fn create(&self, _name: String, config: HashMap<String, String>) -> AppResult<Arc<dyn ModelProvider>> {
        // Use OpenAICompatibleProvider with base_url = https://models.inference.ai.azure.com
    }
}
```

#### NVIDIA NIM
```rust
fn default_free_tier(&self) -> FreeTierKind {
    FreeTierKind::RateLimitedFree {
        max_rpm: 40, max_rpd: 0, max_tpm: 0, max_tpd: 0,
        max_monthly_calls: 0, max_monthly_tokens: 0,
    }
}
fn free_tier_notes(&self) -> Option<&str> {
    Some("40 RPM on free tier. Access to 100+ models including Llama, Mistral, Qwen. Daily limits undocumented.")
}
```

#### Cloudflare Workers AI
```rust
fn default_free_tier(&self) -> FreeTierKind {
    // Cloudflare uses "neurons" not requests — approximate as RPD
    FreeTierKind::RateLimitedFree {
        max_rpm: 0, max_rpd: 0, max_tpm: 0, max_tpd: 0,
        max_monthly_calls: 0, max_monthly_tokens: 0,
    }
}
fn free_tier_notes(&self) -> Option<&str> {
    Some("10,000 neurons/day free allowance. Neuron cost varies by model and input size. Requires Cloudflare account ID in base URL.")
}
```
Note: Cloudflare's "neurons" metric doesn't map cleanly to RPM/RPD. Use `None`-like limits and rely on backoff. Requires `account_id` setup parameter.

#### LLM7.io
```rust
fn default_free_tier(&self) -> FreeTierKind {
    FreeTierKind::RateLimitedFree {
        max_rpm: 30, max_rpd: 0, max_tpm: 0, max_tpd: 0,
        max_monthly_calls: 0, max_monthly_tokens: 0,
    }
}
fn free_tier_notes(&self) -> Option<&str> {
    Some("30 RPM without token, 120 RPM with token. Access to DeepSeek R1, Qwen2.5 Coder, and 27+ more models.")
}
```

#### Kluster AI
```rust
fn default_free_tier(&self) -> FreeTierKind {
    FreeTierKind::RateLimitedFree {
        max_rpm: 30, max_rpd: 0, max_tpm: 0, max_tpd: 0,
        max_monthly_calls: 0, max_monthly_tokens: 0,
    }
}
fn free_tier_notes(&self) -> Option<&str> {
    Some("Free tier limits are undocumented. Supports DeepSeek-R1, Llama 4 Maverick, Qwen3-235B.")
}
```

#### Hugging Face
```rust
fn default_free_tier(&self) -> FreeTierKind {
    FreeTierKind::CreditBased {
        budget_usd: 0.10,
        reset_period: lr_config::FreeTierResetPeriod::Monthly,
        detection: lr_config::CreditDetection::LocalOnly,
    }
}
fn free_tier_notes(&self) -> Option<&str> {
    Some("$0.10/month free credits for all users. PRO users get $2/month. No markup — provider costs passed through directly. Uses HF User Access Token.")
}
```

#### Zhipu AI
```rust
fn default_free_tier(&self) -> FreeTierKind {
    FreeTierKind::RateLimitedFree {
        max_rpm: 0, max_rpd: 0, max_tpm: 0, max_tpd: 0,
        max_monthly_calls: 0, max_monthly_tokens: 0,
    }
}
fn free_tier_notes(&self) -> Option<&str> {
    Some("Free tier limits are undocumented. Supports GLM-4.7-Flash, GLM-4.5-Flash, GLM-4.6V-Flash. Chinese-language focused provider.")
}
```

### 3. Register factories in main.rs

**File:** `src-tauri/src/main.rs` (~line 296)

```rust
provider_registry.register_factory(Arc::new(GitHubModelsProviderFactory));
provider_registry.register_factory(Arc::new(NvidiaNimProviderFactory));
provider_registry.register_factory(Arc::new(CloudflareAIProviderFactory));
provider_registry.register_factory(Arc::new(Llm7ProviderFactory));
provider_registry.register_factory(Arc::new(KlusterAIProviderFactory));
provider_registry.register_factory(Arc::new(HuggingFaceProviderFactory));
provider_registry.register_factory(Arc::new(ZhipuProviderFactory));
```

### 4. Update TypeScript types

**File:** `src/types/tauri-commands.ts`

Add new provider type string literals to the ProviderType union.

### 5. Update demo mock

**File:** `website/src/components/demo/TauriMockSetup.ts`

Add mock entries for new providers.

### 6. Add catalog mappings (where applicable)

Override `catalog_provider_id()` for providers that have models.dev catalog entries. For new/niche providers, return `None`.

---

## Implementation Notes

- **All 7 use OpenAI-compatible API** — reuse `OpenAICompatibleProvider` internally
- **Cloudflare requires account_id** — needs a setup parameter and URL template
- **Zhipu may need custom headers** — verify if standard OpenAI-compatible auth works
- **GitHub Models uses PAT** — standard Bearer token auth should work
- **Config migration** — adding new ProviderType variants is safe (existing configs just won't have them)

## Critical Files

| File | Change |
|---|---|
| `crates/lr-config/src/types.rs` | 7 new ProviderType variants |
| `crates/lr-providers/src/factory.rs` | 7 new factory structs |
| `src-tauri/src/main.rs` | Register 7 new factories |
| `src/types/tauri-commands.ts` | Update ProviderType union |
| `website/src/components/demo/TauriMockSetup.ts` | Mock entries |

## Verification

1. `cargo test -p lr-config` — ProviderType serde roundtrip
2. `cargo test -p lr-providers` — factory tests
3. `cargo clippy` — no warnings
4. `npx tsc --noEmit` — TypeScript compiles
5. Manual: add each provider in setup wizard, verify it appears correctly

---

## Final Steps (mandatory)

1. **Plan Review** — check all 7 providers implemented correctly
2. **Test Coverage Review** — each factory has at least a creation test
3. **Bug Hunt** — serde renames, URL construction, auth header format
4. **Commit** — stage only modified files
