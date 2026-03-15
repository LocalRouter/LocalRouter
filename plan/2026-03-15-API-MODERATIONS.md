# Moderations Endpoint

## Context

OpenAI's moderations endpoint classifies text/images for content safety categories (hate, harassment, self-harm, sexual, violence, etc.). Only OpenAI natively exposes this, but it's a common API that clients expect to exist.

**Core idea**: Rather than creating a separate "native vs local" mode, we treat OpenAI's moderation models as **just another safety model family** in the existing guardrails engine — alongside Llama Guard, ShieldGemma, Nemotron, and Granite Guardian. The `/v1/moderations` endpoint is a simple enable/disable flag that exposes the guardrails engine via the standard OpenAI moderation API format.

## Endpoints

- `POST /v1/moderations` — Content safety classification (requires auth)

---

## Part 1: OpenAI Moderation as a GuardRails Safety Model

### What Changes

Add "OpenAI Moderation" as a fifth safety model family in the SafetyModelPicker, alongside the existing four. When a user has OpenAI providers configured, the picker shows `omni-moderation-latest` and `text-moderation-latest` as selectable models.

### SafetyModelPicker Changes

**New family group in `safety-model-variants.ts`:**
```typescript
{
  family: "OpenAI Moderation",
  modelType: "openai_moderation",
  variants: {
    openai: ["omni-moderation-latest", "text-moderation-latest"],
  }
}
```

**Provider filtering**: Only show entries for OpenAI-type providers (not Ollama/LM Studio). No pull needed — these are cloud models.

**Pricing display**: Show pricing badge in SafetyModelList. OpenAI moderation models are **free** ($0.00). Since models.dev doesn't currently list moderation models in their catalog, fallback to free. If models.dev adds them later, use that pricing data instead.

### New Backend Safety Model: `openai_moderation`

**New file:** `crates/lr-guardrails/src/models/openai_moderation.rs`

Unlike other safety models that use chat completions, this model calls OpenAI's dedicated `/v1/moderations` endpoint and maps the response back to the unified `SafetyVerdict` format.

```rust
pub struct OpenAIModerationModel {
    id: String,
    executor: Arc<ModerationExecutor>,  // Calls /v1/moderations instead of /v1/chat/completions
    model_name: String,                 // "omni-moderation-latest" or "text-moderation-latest"
    enabled_categories: Option<Vec<SafetyCategory>>,
}

#[async_trait]
impl SafetyModel for OpenAIModerationModel {
    fn id(&self) -> &str { &self.id }
    fn model_type_id(&self) -> &str { "openai_moderation" }
    fn display_name(&self) -> &str { "OpenAI Moderation" }
    fn inference_mode(&self) -> InferenceMode { InferenceMode::MultiCategory }

    fn supported_categories(&self) -> Vec<SafetyCategoryInfo> {
        // Returns all OpenAI moderation categories mapped to SafetyCategory
        vec![
            SafetyCategoryInfo { category: SafetyCategory::Hate, native_label: "hate".into(), .. },
            SafetyCategoryInfo { category: SafetyCategory::Harassment, native_label: "harassment".into(), .. },
            // ... all 13 OpenAI categories
        ]
    }

    async fn check(&self, input: &SafetyCheckInput) -> Result<SafetyVerdict, String> {
        // 1. Concatenate messages into input text
        // 2. POST to provider's /v1/moderations endpoint
        // 3. Map OpenAI ModerationResult → SafetyVerdict with FlaggedCategory entries
        //    - Each OpenAI category_score becomes confidence
        //    - Each flagged category maps to SafetyCategory
    }
}
```

### OpenAI → SafetyCategory Mapping (inbound, for guardrails use)

When OpenAI moderation is used as a guardrails model, its categories map INTO our unified system:

| OpenAI Native Category | → SafetyCategory | Notes |
|------------------------|-------------------|-------|
| `hate` | `Hate` | |
| `hate/threatening` | `Hate` | Merged with parent |
| `harassment` | `Harassment` | |
| `harassment/threatening` | `Harassment` | Merged with parent |
| `self-harm` | `SelfHarm` | |
| `self-harm/intent` | `SelfHarm` | Merged with parent |
| `self-harm/instructions` | `SelfHarm` | Merged with parent |
| `sexual` | `SexualContent` | |
| `sexual/minors` | `ChildExploitation` | Separate — good match |
| `violence` | `ViolentCrimes` | |
| `violence/graphic` | `ViolentCrimes` | Merged with parent |
| `illicit` | `IllegalActivity` | |
| `illicit/violent` | `CriminalPlanning` | |

When subcategories merge with parents, take the **max** of the two scores for confidence.

### Executor Changes

The existing `ProviderExecutor` calls `/v1/chat/completions` or `/api/generate` (Ollama). For OpenAI moderation, we need a new executor variant that calls `/v1/moderations`:

```rust
pub enum ModelExecutor {
    Provider(ProviderExecutor),           // Existing: chat completions
    Moderation(ModerationExecutor),       // New: POST /v1/moderations
}

pub struct ModerationExecutor {
    http_client: reqwest::Client,
    base_url: String,
    api_key: Option<String>,
}
```

### Engine Changes

In `SafetyEngine::from_config()`, add a match arm:
```rust
"openai_moderation" => {
    // Build ModerationExecutor instead of ProviderExecutor
    Arc::new(models::openai_moderation::OpenAIModerationModel::new(...))
}
```

### Pricing

- **models.dev catalog**: Does not currently list moderation models
- **Fallback**: Hardcode `omni-moderation-latest` and `text-moderation-latest` as free ($0.00/$0.00 per 1K tokens)
- **Future-proof**: If models.dev adds these models, the catalog lookup in `find_model("openai", "omni-moderation-latest")` will naturally pick them up
- **Display**: Add pricing badge to `SafetyModelList` component showing cost per check (free for OpenAI, free for local models)

---

## Part 2: Moderation API Endpoint

### Simple Enable/Disable

The `/v1/moderations` endpoint is controlled by a single boolean flag in the guardrails config. When enabled, it uses whatever safety models are configured in the guardrails engine.

```rust
// Added to GuardrailsConfig
pub moderation_api_enabled: bool,  // default: false
```

No mode selection needed. The endpoint always uses the safety engine. If the user wants OpenAI-native moderation, they add OpenAI Moderation as a safety model. If they want local, they add Llama Guard. They can even use both as an ensemble.

### Auth Required

All requests to `/v1/moderations` require authentication via Bearer token, same as every other endpoint. No unauthenticated requests pass through. Standard client permission checks apply.

### Request Flow

```
POST /v1/moderations { input: "some text" }
    │
    ▼
Auth + client permission check
    │
    ▼
moderation_api_enabled == false? → 501 Not Implemented
    │
    ▼
SafetyEngine::check_text(input, ScanDirection::Input)
    │
    ▼
translate SafetyCheckResult → ModerationResponse
    │
    ▼
Return ModerationResponse (OpenAI-compatible + extra categories)
```

### Category Mapping (outbound, for API response)

When producing the `/v1/moderations` response, SafetyCategory maps OUT to OpenAI format:

| SafetyCategory | → OpenAI Category | → OpenAI Subcategories | Mapped? |
|----------------|-------------------|----------------------|---------|
| `Hate` | `hate` | `hate/threatening` (same score) | Standard |
| `Harassment` | `harassment` | `harassment/threatening` (same score) | Standard |
| `SelfHarm` | `self-harm` | `self-harm/intent`, `self-harm/instructions` (same score) | Standard |
| `SexualContent` | `sexual` | — | Standard |
| `ChildExploitation` | `sexual/minors` | — | Standard |
| `ViolentCrimes` | `violence` | `violence/graphic` (same score) | Standard |
| `IllegalActivity` | `illicit` | — | Standard |
| `CriminalPlanning` | `illicit/violent` | — | Standard |
| `NonViolentCrimes` | — | — | Extra |
| `SexCrimes` | — | — | Extra |
| `Defamation` | — | — | Extra |
| `SpecializedAdvice` | — | — | Extra |
| `Privacy` | — | — | Extra |
| `IntellectualProperty` | — | — | Extra |
| `IndiscriminateWeapons` | — | — | Extra |
| `Elections` | — | — | Extra |
| `CodeInterpreterAbuse` | — | — | Extra |
| `DangerousContent` | — | — | Extra |
| `GunsIllegalWeapons` | — | — | Extra |
| `ControlledSubstances` | — | — | Extra |
| `Profanity` | — | — | Extra |
| `NeedsCaution` | — | — | Extra |
| `Manipulation` | — | — | Extra |
| `FraudDeception` | — | — | Extra |
| `Malware` | — | — | Extra |
| `HighRiskGovDecision` | — | — | Extra |
| `PoliticalMisinformation` | — | — | Extra |
| `CopyrightPlagiarism` | — | — | Extra |
| `UnauthorizedAdvice` | — | — | Extra |
| `ImmoralUnethical` | — | — | Extra |
| `SocialBias` | — | — | Extra |
| `Jailbreak` | — | — | Extra |
| `UnethicalBehavior` | — | — | Extra |

**Extra categories**: Returned directly in the `categories` and `category_scores` fields alongside the standard ones. No custom headers or extension fields. Clients that only know about standard OpenAI categories will simply ignore the extra keys. Clients aware of LocalRouter's extended categories can use them.

### Response Format

```json
{
  "id": "modr-abc123",
  "model": "localrouter-guardrails",
  "results": [
    {
      "flagged": true,
      "categories": {
        "hate": true,
        "hate/threatening": false,
        "harassment": false,
        "harassment/threatening": false,
        "self-harm": false,
        "self-harm/intent": false,
        "self-harm/instructions": false,
        "sexual": false,
        "sexual/minors": false,
        "violence": false,
        "violence/graphic": false,
        "illicit": false,
        "illicit/violent": false,
        "profanity": false,
        "jailbreak": false,
        "privacy": false
      },
      "category_scores": {
        "hate": 0.92,
        "hate/threatening": 0.0,
        "harassment": 0.05,
        "harassment/threatening": 0.0,
        "self-harm": 0.0,
        "self-harm/intent": 0.0,
        "self-harm/instructions": 0.0,
        "sexual": 0.0,
        "sexual/minors": 0.0,
        "violence": 0.0,
        "violence/graphic": 0.0,
        "illicit": 0.0,
        "illicit/violent": 0.0,
        "profanity": 0.0,
        "jailbreak": 0.0,
        "privacy": 0.0
      }
    }
  ]
}
```

Note: `categories` and `category_scores` are serialized as `HashMap<String, bool>` / `HashMap<String, f64>` rather than fixed structs, to accommodate extra categories dynamically.

### Confidence Score Handling

| Safety Model | Score Availability | Mapping |
|-------------|-------------------|---------|
| **OpenAI Moderation** | Full continuous scores (0.0-1.0) | Use `category_scores` directly |
| **Llama Guard 4** | Binary safe/unsafe per category | 1.0 if flagged, 0.0 if not |
| **ShieldGemma** | Logprobs-based confidence | Use directly |
| **Granite Guardian** | Per-category confidence | Use directly |
| **Nemotron** | Binary per category | 1.0 if flagged, 0.0 if not |

When multiple models are configured, take the **max confidence** across all model verdicts for each category.

---

## Part 3: UI — GuardRails Settings Tab

### New Card: "Moderation API Endpoint"

Added below the existing "Safety Models" card in the GuardRails settings tab:

```
┌─────────────────────────────────────────────┐
│ Moderation API Endpoint                      │
│                                              │
│ Expose your configured safety models via     │
│ the /v1/moderations endpoint. Clients can    │
│ call this endpoint to classify content using │
│ the standard OpenAI moderation API format.   │
│                                              │
│ Enabled: [toggle]          Auth required     │
│                                              │
│ ── Category Mapping ───────────────────────  │
│                                              │
│ │ Safety Category       │ OpenAI Category    │
│ │───────────────────────│────────────────────│
│ │ Hate                  │ hate               │
│ │ Harassment            │ harassment         │
│ │ Self-Harm             │ self-harm          │
│ │ Sexual Content        │ sexual             │
│ │ Child Exploitation    │ sexual/minors      │
│ │ Violent Crimes        │ violence           │
│ │ Illegal Activity      │ illicit            │
│ │ Criminal Planning     │ illicit/violent     │
│ │───────────────────────│────────────────────│
│ │ Non-Violent Crimes    │ (extra)            │
│ │ Privacy               │ (extra)            │
│ │ Profanity             │ (extra)            │
│ │ Jailbreak             │ (extra)            │
│ │ ... +N more           │ (extra)            │
│                                              │
│ Standard categories follow the OpenAI        │
│ moderation response format. Extra categories │
│ are detected by your safety models but not   │
│ part of the official OpenAI spec — they are  │
│ returned alongside the standard ones.        │
└─────────────────────────────────────────────┘
```

The table is informational — no configuration needed. It shows users which categories map to OpenAI's format and which are extras unique to LocalRouter.

Collapsible or scrollable for the extras section to avoid taking too much vertical space.

---

## Feasibility Assessment

### High Feasibility — Minimal New Code

| Component | Effort | Details |
|-----------|--------|---------|
| **OpenAI Moderation safety model** | Medium | New model type + ModerationExecutor (~200 lines). Well-defined API, straightforward mapping. |
| **SafetyModelPicker changes** | Small | Add family group + OpenAI provider filter (~30 lines in constants, ~20 lines in picker) |
| **Pricing fallback** | Trivial | Hardcode free for moderation models (~10 lines) |
| **Translation layer** | Small | `SafetyCheckResult` → `ModerationResponse` (~100 lines, pure mapping) |
| **Route handler** | Small | Follow embeddings pattern (~100 lines) |
| **Config flag** | Trivial | One bool field + migration |
| **UI card** | Small | Toggle + static mapping table (~100 lines) |

**Total estimate**: ~560 lines of new code, spread across 8-10 files. No new architectural patterns.

### What We Get For Free

1. **Multi-model ensemble** — User can combine OpenAI + Llama Guard for higher coverage
2. **Category action system** — Per-category Allow/Notify/Ask/Block still works with OpenAI moderation
3. **Per-client overrides** — Client-specific category action overrides apply
4. **Confidence thresholds** — Configurable globally and per-model
5. **Parallel scanning** — OpenAI moderation runs in parallel with other safety models
6. **Try It Out panel** — Existing test panel works with OpenAI moderation model
7. **Model management UI** — Add/remove/view in SafetyModelList

---

## Value Assessment

| Dimension | Rating | Rationale |
|-----------|--------|-----------|
| **Privacy** | Very High | Local models keep content on-device. Users choose their own tradeoff. |
| **Flexibility** | Very High | Mix cloud (OpenAI) and local (Llama Guard) models as safety ensemble. |
| **API compat** | High | Standard `/v1/moderations` format for any client expecting it. |
| **Cost** | High | OpenAI moderation is free. Local models are free. |
| **Differentiation** | Very High | No other gateway offers local moderation OR mixed ensemble moderation. |
| **Leverage** | Very High | ~560 lines of new code for significant new functionality. |

---

## Files to Modify

### Backend — New Safety Model

| File | Change |
|------|--------|
| `crates/lr-guardrails/src/models/openai_moderation.rs` | **New.** OpenAI Moderation safety model impl |
| `crates/lr-guardrails/src/models/mod.rs` | Add `pub mod openai_moderation` |
| `crates/lr-guardrails/src/executor.rs` | Add `ModerationExecutor` variant to `ModelExecutor` enum |
| `crates/lr-guardrails/src/engine.rs` | Add `"openai_moderation"` match arm in `from_config()` |

### Backend — Moderation Endpoint

| File | Change |
|------|--------|
| `crates/lr-server/src/routes/moderations.rs` | **New.** Route handler |
| `crates/lr-server/src/routes/mod.rs` | Add module |
| `crates/lr-server/src/lib.rs` | Register `POST /v1/moderations` + `/moderations` |
| `crates/lr-server/src/openapi/mod.rs` | Register types and path |

### Backend — Types & Config

| File | Change |
|------|--------|
| `crates/lr-config/src/types.rs` | Add `moderation_api_enabled: bool` to `GuardrailsConfig` |
| `crates/lr-config/src/migration.rs` | Add default for new field |
| `crates/lr-server/src/types.rs` | Add `ModerationRequest`, `ModerationResponse`, `ModerationResult` |

### Frontend — Safety Model Picker

| File | Change |
|------|--------|
| `src/constants/safety-model-variants.ts` | Add OpenAI Moderation family + model variants |
| `src/components/guardrails/SafetyModelPicker.tsx` | Support OpenAI-type providers (no pull, cloud model) |
| `src/components/guardrails/SafetyModelList.tsx` | Show pricing badge per model |

### Frontend — Guardrails Tab

| File | Change |
|------|--------|
| `src/views/settings/guardrails-tab.tsx` | Add "Moderation API Endpoint" card with toggle + mapping table |
| `src/types/tauri-commands.ts` | Update `GuardrailsConfig` type with `moderation_api_enabled` |

### Demo Mock

| File | Change |
|------|--------|
| `website/src/components/demo/TauriMockSetup.ts` | Update GuardrailsConfig mock |

---

## Cross-Cutting Features

| Feature | Applies? | Notes |
|---------|----------|-------|
| **Auth (API Key)** | **Yes** | Required. Standard Bearer token. No unauthenticated access. |
| **Permission checks** | **Yes** | Client must be enabled with LLM mode. |
| **Rate limiting** | **Yes** | Estimate tokens from input text length |
| **Secret scanning** | **No** | Input is content TO BE scanned for safety |
| **Guardrails** | **N/A** | The endpoint IS the guardrails |
| **Prompt compression** | **No** | Not applicable |
| **RouteLLM** | **No** | Single-purpose endpoint |
| **Model firewall** | **No** | Read-only classification |
| **Token tracking** | **Yes** | Track estimated input tokens |
| **Cost calculation** | **Yes** | Free for OpenAI and local models, but track for completeness |
| **Generation tracking** | **Yes** | Assign generation ID |
| **Metrics/logging** | **Yes** | Standard metrics |
| **Client activity** | **Yes** | Record activity |

---

## Implementation Order

1. **OpenAI Moderation safety model** — `ModerationExecutor` + `OpenAIModerationModel` + engine integration
2. **SafetyModelPicker** — Add family, provider filter, pricing display
3. **Config** — `moderation_api_enabled` field + migration
4. **Translation layer** — `SafetyCheckResult` → `ModerationResponse` with category mapping
5. **Route handler** — `moderations.rs` with enable check + auth
6. **UI** — Moderation API card with toggle + mapping table
7. **Tests** — Model check, translation accuracy, auth enforcement, enable/disable

## Verification

1. `cargo test` — OpenAI moderation model mapping, translation layer, handler
2. Add OpenAI Moderation via picker with an OpenAI provider → model appears in list, shows "Free"
3. Enable moderation API → `curl -X POST localhost:3625/v1/moderations -H "Authorization: Bearer ..." -d '{"input":"I hate you"}'` → returns flagged categories
4. Without auth → 401
5. With `moderation_api_enabled: false` → 501
6. With local models only (no OpenAI) → still works, uses Llama Guard etc.
7. Verify `/openapi.json` includes moderations path
8. Test multi-input: `{"input": ["text1", "text2"]}` → two results
9. Verify extra categories appear in response alongside standard ones
