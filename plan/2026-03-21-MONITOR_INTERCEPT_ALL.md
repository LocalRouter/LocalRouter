# Monitor Intercept All Feature — Expanded Categories + Multi-Select

## Context

Expanding the monitor intercept feature from 2 categories (LLM, MCP) to cover all interceptable approval types. Also changing from single-select to multi-select for both categories and clients, so users can intercept e.g. "LLM + Guardrails for clients A and B".

## Changes from Current Implementation

### 1. Expand `InterceptCategory` enum — `crates/lr-mcp/src/gateway/firewall.rs`

Replace the current 3-variant enum (drop `All` — represented by selecting all checkboxes):

```rust
pub enum InterceptCategory {
    Llm,          // Model permission + auto-router
    Mcp,          // MCP tool calls (non-virtual)
    Skill,        // Skill tool calls
    Marketplace,  // Marketplace install
    CodingAgent,  // Coding agent start
    Guardrails,   // Guardrail scan
    SecretScan,   // Secret scan
    Sampling,     // MCP sampling
    Elicitation,  // MCP elicitation
}
```

### 2. Change `InterceptRule` to multi-select — `firewall.rs`

```rust
pub struct InterceptRule {
    pub categories: Vec<InterceptCategory>,  // which types to intercept
    pub client_ids: Vec<String>,             // empty = all clients
}
```

### 3. Change `should_intercept` signature — `firewall.rs`

Replace `is_llm: bool` with `request_category: InterceptCategory`:

```rust
pub fn should_intercept(&self, client_id: &str, request_category: InterceptCategory) -> bool {
    // ... internal-test check ...
    let Some(rule) = guard.as_ref() else { return false };
    // Client filter: empty = all clients
    if !rule.client_ids.is_empty() && !rule.client_ids.iter().any(|id| id == client_id) {
        return false;
    }
    // Category filter
    rule.categories.contains(&request_category)
}
```

### 4. Update existing call sites to use new enum

- `chat.rs` model check: `should_intercept(&client.id, InterceptCategory::Llm)`
- `chat.rs` auto-router: `should_intercept(&client.id, InterceptCategory::Llm)`
- `gateway_tools.rs` `apply_firewall_result()`: needs category parameter (see below)
- `commands_clients.rs` re-evaluation guard: derive category from pending info flags

### 4. Parameterize `apply_firewall_result` — `gateway_tools.rs`

Add `intercept_category: InterceptCategory` parameter. Use it instead of hardcoded `false`:

```rust
async fn apply_firewall_result(
    &self, session, result, client_id, tool_name, server_or_skill_name, request,
    intercept_category: InterceptCategory,  // NEW
) -> AppResult<FirewallDecisionResult> {
    let result = if result == FirewallCheckResult::Allow
        && self.firewall_manager.should_intercept(client_id, intercept_category) { ... }
```

**Callers update:**
- `check_firewall_mcp_tool` (line 547): pass `InterceptCategory::Mcp`
- `dispatch_virtual_tool_call` (line 834): determine category from `vs.id()`:
  - `"_skills"` → `InterceptCategory::Skill`
  - `"_marketplace"` → `InterceptCategory::Marketplace`
  - `"_coding_agents"` → `InterceptCategory::CodingAgent`
  - anything else → `InterceptCategory::Mcp`

### 5. Add Guardrail intercept — `chat.rs` + `completions.rs`

**Two points per file:**

**A) Early bypass override** (chat.rs ~line 1455, completions.rs ~line 371):
```rust
if state.guardrail_approval_tracker.has_valid_bypass(&client.id)
    && !state.mcp_gateway.firewall_manager.should_intercept(&client.id, InterceptCategory::Guardrails)
{
    return Ok(None);
}
```

**B) `check_needs_approval` override** in `handle_guardrail_approval` (chat.rs ~line 1613, completions.rs similar):
```rust
let result = lr_mcp::gateway::access_control::check_needs_approval(&ctx);
let result = if result == FirewallCheckResult::Allow
    && state.mcp_gateway.firewall_manager.should_intercept(client_id, InterceptCategory::Guardrails)
{
    FirewallCheckResult::Ask
} else { result };
```

### 6. Add Secret Scan intercept — `chat.rs` + `completions.rs`

**Early bypass override** (chat.rs ~line 1785, completions.rs ~line 556):
```rust
if state.secret_scan_approval_tracker.has_valid_bypass(&client_ctx.client_id)
    && !state.mcp_gateway.firewall_manager.should_intercept(&client_ctx.client_id, InterceptCategory::SecretScan)
{
    return Ok(());
}
```

No second override needed — once the scan runs and finds secrets, the popup is always shown (no `check_needs_approval` in this path).

### 7. Add Sampling intercept — `crates/lr-mcp/src/gateway/gateway.rs`

In `register_request_handlers`, capture `firewall_manager` in the closure (line ~999):
```rust
let firewall_manager = self.firewall_manager.clone();
```

At line ~1169, after computing `effective` behavior, add intercept override:
```rust
let effective = if matches!(effective, lr_config::SamplingBehavior::DirectAllow)
    && firewall_manager.should_intercept(&client_id, InterceptCategory::Sampling)
{
    lr_config::SamplingBehavior::DirectAsk
} else {
    effective
};
```

This converts `DirectAllow` → `DirectAsk` when intercept is active, forcing the sampling approval popup.

### 8. Add Elicitation intercept — `crates/lr-mcp/src/gateway/gateway.rs`

At line ~1066, after computing `effective_mode`, add intercept override:
```rust
let effective_mode = if matches!(effective_mode, lr_config::ElicitationMode::Off)
    && firewall_manager.should_intercept(&client_id, InterceptCategory::Elicitation)
{
    lr_config::ElicitationMode::Direct
} else {
    effective_mode
};
```

This converts `Off` → `Direct` when intercept is active, forcing the elicitation popup. `Passthrough` and `Direct` modes already show UI, so only `Off` needs overriding.

### 9. Update re-evaluation guard — `commands_clients.rs`

Derive category from pending info:
```rust
let category = if info.is_model_request || info.is_auto_router_request {
    InterceptCategory::Llm
} else if info.is_guardrail_request {
    InterceptCategory::Guardrails
} else if info.is_secret_scan_request {
    InterceptCategory::SecretScan
} else {
    InterceptCategory::Mcp  // covers MCP tools, skills, marketplace, coding agent
};
```

### 10. Update TypeScript types — `src/types/tauri-commands.ts`

```typescript
export type InterceptCategory = 'llm' | 'mcp' | 'skill' | 'marketplace'
  | 'coding_agent' | 'guardrails' | 'secret_scan' | 'sampling' | 'elicitation'

export interface InterceptRule {
  categories: InterceptCategory[]  // which types to intercept
  client_ids: string[]             // empty = all clients
}
```

### 11. Update frontend — `src/views/monitor/event-filters.tsx`

Replace the two `Select` dropdowns with checkbox-based multi-select using `Checkbox` from `@/components/ui/checkbox`.

**Layout inside popover:**
- "Select All" checkbox + individual category checkboxes (9 items)
- Separator
- "All Clients" checkbox + individual client checkboxes (loaded from `list_clients`)
- Start/Stop button

**State:** `categories: InterceptCategory[]` and `clientIds: string[]` (local, committed on "Start").

**"Select All" behavior:** Toggles all category checkboxes. Shown as indeterminate when partially selected. Same for "All Clients".

### 12. Update demo mock — `website/src/components/demo/TauriMockSetup.ts`

Already has mock handlers — no change needed (they return null).

### 13. Update unit tests — `firewall.rs`

Update existing tests from `is_llm: bool` to `InterceptCategory` enum. Add tests for multi-category, multi-client matching.

## Files Modified

| File | Change |
|------|--------|
| `crates/lr-mcp/src/gateway/firewall.rs` | Expand enum, change `should_intercept` signature, update tests |
| `crates/lr-mcp/src/gateway/gateway_tools.rs` | Add `intercept_category` param to `apply_firewall_result`, update callers |
| `crates/lr-mcp/src/gateway/gateway.rs` | Capture `firewall_manager` in closure, add sampling + elicitation intercept |
| `crates/lr-server/src/routes/chat.rs` | Update existing LLM intercepts to new enum, add guardrail + secret scan intercept |
| `crates/lr-server/src/routes/completions.rs` | Add guardrail + secret scan intercept |
| `src-tauri/src/ui/commands_clients.rs` | Update re-evaluation guard to use new enum |
| `src/types/tauri-commands.ts` | Expand `InterceptCategory` type |
| `src/views/monitor/event-filters.tsx` | Expand category dropdown |

## Verification

1. `cargo test` — updated unit tests
2. `cargo clippy && cargo fmt`
3. `npx tsc --noEmit`
4. Manual: test each category individually

## Mandatory Final Steps

1. **Plan review** — compare plan to implementation
2. **Test coverage review** — unit tests for all new categories
3. **Bug hunt** — review for missed intercept points
