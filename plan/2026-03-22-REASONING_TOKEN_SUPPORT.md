# Reasoning Token Support - Audit & Fix Plan

## Context

Reasoning tokens (OpenAI o1/o3, Anthropic extended thinking, Gemini thinking) are partially supported but have significant gaps in propagation, pricing, and display. The OpenAI-compatible API response correctly passes through `completion_tokens_details` (including `reasoning_tokens` and `thinking_tokens`) from providers — so API consumers see them. However, the **internal systems** (monitoring, logging, pricing, UI) do NOT track, price, or display them properly.

### models.dev Format
models.dev provides reasoning pricing in `cost.reasoning` (per million tokens), e.g.:
```json
{ "cost": { "input": 2, "output": 12, "reasoning": 12, "cache_read": 0.4 } }
```
The scraper parses this into `Cost.reasoning: Option<f64>`, but it is **discarded during code generation**.

---

## Findings (Gaps)

### 1. Catalog: Reasoning pricing discarded at build time
- `crates/lr-catalog/buildtools/models.rs:121` — `Cost.reasoning: Option<f64>` is parsed
- `crates/lr-catalog/buildtools/models.rs:135-154` — NO `reasoning_cost_per_token()` helper
- `crates/lr-catalog/buildtools/codegen.rs:111-117` — Pricing template OMITS reasoning
- `crates/lr-catalog/src/types.rs:60-73` — `CatalogPricing` has NO `reasoning_per_token` field
- `crates/lr-catalog/src/types.rs:108-136` — `calculate_cost*()` methods ignore reasoning

### 2. PricingInfo: No reasoning cost
- `crates/lr-providers/src/lib.rs:969-976` — `PricingInfo` only has `input_cost_per_1k` + `output_cost_per_1k`

### 3. Cost calculation ignores reasoning tokens
- `crates/lr-router/src/lib.rs:272-280` — `calculate_cost()` uses only input + output
- `crates/lr-server/src/types.rs:780-789` — `CostDetails` has `prompt_cost`, `completion_cost`, `total_cost` only

### 4. Monitor events: No reasoning tokens
- `crates/lr-monitor/src/types.rs:170-212` — `LlmCall` data has `input_tokens`, `output_tokens`, `total_tokens` but NO `reasoning_tokens`
- `crates/lr-server/src/routes/monitor_helpers.rs:173-223` — `complete_llm_call()` doesn't receive reasoning tokens
- Frontend `src/views/monitor/event-detail.tsx:366-371` has UI code for `data.reasoning_tokens` but the backend **never provides this field**

### 5. Access logging: No reasoning tokens
- `crates/lr-monitoring/src/logger.rs:21-61` — `AccessLogEntry` lacks reasoning_tokens

### 6. Frontend display gaps
- Monitor event detail: has dead code for reasoning display (backend never sends it)
- Try-it-out chat panel `MessageMetadata`: no reasoning fields
- Dashboard aggregate stats: no reasoning tracking
- Charts/metrics: no reasoning metrics

### 7. What DOES work
- `CompletionTokensDetails` struct exists with `reasoning_tokens` + `thinking_tokens` fields
- API responses correctly propagate `completion_tokens_details` to external consumers
- `reasoning_effort` parameter is passed through to providers
- Catalog tracks `capabilities.reasoning: bool` correctly

---

## Fix Plan

### Step 1: Add reasoning pricing to catalog

**`crates/lr-catalog/buildtools/models.rs`**
- Add `reasoning_cost_per_token()` method to `ModelsDevModel`

**`crates/lr-catalog/src/types.rs`**
- Add `reasoning_per_token: Option<f64>` to `CatalogPricing`
- Add `reasoning_cost_per_1k()`, `reasoning_cost_per_1m()` helper methods
- Update `calculate_cost()` → add `calculate_cost_with_reasoning()` method that accepts reasoning token count

**`crates/lr-catalog/buildtools/codegen.rs`**
- Add `reasoning_per_token` to the pricing template (lines 111-117)
- Extract `model.model.reasoning_cost_per_token()` in generation

### Step 2: Add reasoning cost to PricingInfo

**`crates/lr-providers/src/lib.rs`**
- Add `reasoning_cost_per_1k: Option<f64>` to `PricingInfo`
- Update all provider `get_model_pricing()` implementations to include reasoning pricing from catalog

### Step 3: Update cost calculation

**`crates/lr-router/src/lib.rs`**
- Update `calculate_cost()` to accept optional reasoning tokens + pricing
- When `reasoning_cost_per_1k` is present and reasoning tokens are reported, use the reasoning rate; otherwise fall back to output token rate

**`crates/lr-server/src/types.rs`**
- Add optional `reasoning_cost: Option<f64>` to `CostDetails`

**`crates/lr-server/src/routes/chat.rs`** (and completions.rs)
- Extract reasoning tokens from `completion_tokens_details` when calculating cost
- Pass reasoning tokens to cost calculation

### Step 4: Add reasoning tokens to monitor events

**`crates/lr-monitor/src/types.rs`**
- Add `reasoning_tokens: Option<u64>` to `MonitorEventData::LlmCall`

**`crates/lr-server/src/routes/monitor_helpers.rs`**
- Add `reasoning_tokens: Option<u64>` parameter to `complete_llm_call()`
- Set the field in the event data

**Call sites in `crates/lr-server/src/routes/chat.rs`** (and completions.rs)
- Extract reasoning tokens from `completion_tokens_details` when calling `complete_llm_call()`

### Step 5: Add reasoning tokens to access logging

**`crates/lr-monitoring/src/logger.rs`**
- Add `reasoning_tokens: Option<u64>` to `AccessLogEntry`
- Update `success()` and `log_success()` to accept reasoning tokens

### Step 6: Frontend - Monitor event detail (already has display code)

**`src/views/monitor/event-detail.tsx`**
- The existing code at lines 366-371 will now work since backend provides `reasoning_tokens`
- No change needed (unless we want to also show cost breakdown)

### Step 7: Frontend - Try-it-out chat panel

**`src/views/try-it-out/llm-tab/chat-panel.tsx`**
- Add `reasoningTokens?: number` to `MessageMetadata`
- Display reasoning tokens in the token breakdown when present: `{promptTokens} + {completionTokens} ({reasoningTokens} reasoning) = {totalTokens} tokens`

### Step 8: Plan review, test coverage review, bug hunt

1. **Plan Review**: Review plan against implementation for missed items
2. **Test Coverage Review**: Add tests for reasoning cost calculation, catalog pricing with reasoning
3. **Bug Hunt**: Check edge cases — models with no reasoning pricing, zero reasoning tokens, backward compatibility of access log format

---

## Verification

1. `cargo test && cargo clippy && cargo fmt` — all pass
2. Check generated catalog code includes reasoning pricing for models that have it
3. Run `cargo tauri dev`, make a request to a reasoning model (e.g. o1), verify:
   - Monitor event shows reasoning tokens
   - Cost includes reasoning token pricing
   - API response includes `completion_tokens_details`
4. Check access log entries include reasoning_tokens
5. Try-it-out chat shows reasoning tokens in metadata

---

## Files to Modify

| File | Change |
|------|--------|
| `crates/lr-catalog/buildtools/models.rs` | Add `reasoning_cost_per_token()` |
| `crates/lr-catalog/buildtools/codegen.rs` | Include reasoning pricing in template |
| `crates/lr-catalog/src/types.rs` | Add reasoning field + helpers to `CatalogPricing` |
| `crates/lr-providers/src/lib.rs` | Add reasoning to `PricingInfo` |
| `crates/lr-router/src/lib.rs` | Update `calculate_cost()` for reasoning |
| `crates/lr-server/src/types.rs` | Add reasoning to `CostDetails` |
| `crates/lr-server/src/routes/chat.rs` | Extract & use reasoning tokens in cost calc |
| `crates/lr-server/src/routes/completions.rs` | Same as chat.rs |
| `crates/lr-monitor/src/types.rs` | Add reasoning_tokens to LlmCall |
| `crates/lr-server/src/routes/monitor_helpers.rs` | Pass reasoning tokens through |
| `crates/lr-monitoring/src/logger.rs` | Add reasoning_tokens to AccessLogEntry |
| `src/views/try-it-out/llm-tab/chat-panel.tsx` | Show reasoning tokens in metadata |
