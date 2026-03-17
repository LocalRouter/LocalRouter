# Plan: Expand "Modify Request" in firewall approval popups

## Context

The firewall approval system shows popup windows when requests need user approval. Currently only 3 of 7 popup types have the "Modify" button, and even the existing model Modify only exposes model params (temperature, max_tokens, etc.) — not the actual request content (messages). The goal is to:

1. **Add Modify to Auto Router** popups
2. **Expand Model Modify** to include messages/prompt editing and full JSON view
3. Both should support 3 editor tabs: Model + Params | Messages | Raw JSON

## Current State

| Popup Type | Has Modify? | What it captures as `full_arguments` |
|---|---|---|
| **MCP Tool** | YES (kv + JSON) | Full tool arguments JSON |
| **LLM Model** | YES (model + params only) | Only model params: model, temperature, max_tokens, etc. (**no messages**) |
| **Skill** | YES (kv + JSON) | Full skill arguments JSON |
| **Auto Router** | NO | **Nothing** (only string preview of model names) |
| Marketplace | NO (binary) | N/A |
| Free Tier Fallback | NO (binary) | N/A |
| Guardrail | NO (safety) | N/A |

### Key limitations to fix
- `chat.rs:692-701`: Model firewall only captures `{model, temperature, max_tokens, ...}` — messages are excluded
- `chat.rs:105-108`: Auto-router passes only `models_preview` string — no `full_arguments`
- `firewall.rs:315-336`: `request_auto_router_approval()` has no `full_arguments` parameter
- `chat.rs:160-210`: Edit merge is field-by-field for params only — no messages handling

---

## Implementation

### Step 1: Backend — Capture full request as `full_arguments`

**File: `crates/lr-server/src/routes/chat.rs`**

**Model firewall (lines 692-701)**: Replace manual `model_params` JSON with full request serialization:
```rust
// Replace the manual json!({}) with:
let mut full_request = serde_json::to_value(&request)
    .unwrap_or_else(|_| serde_json::json!({}));
if let Some(obj) = full_request.as_object_mut() {
    obj.remove("stream"); // not user-editable
}
```

**Auto-router (lines 94-108)**: Build structured `full_arguments` with both candidate models and request:
```rust
let auto_full_args = serde_json::json!({
    "candidate_models": auto_config.prioritized_models.iter()
        .map(|(p, m)| format!("{}/{}", p, m))
        .collect::<Vec<_>>(),
    "request": serde_json::to_value(&request)
        .map(|mut v| { v.as_object_mut().map(|o| o.remove("stream")); v })
        .unwrap_or_default(),
});
```

### Step 2: Backend — Update `request_auto_router_approval` signature

**File: `crates/lr-mcp/src/gateway/firewall.rs` (lines 315-336)**

Add `full_arguments` parameter:
```rust
pub async fn request_auto_router_approval(
    &self,
    client_id: String,
    client_name: String,
    models_preview: String,
    full_arguments: Option<serde_json::Value>,  // NEW
) -> AppResult<FirewallApprovalResponse> {
    self.request_approval_internal(
        // ... existing params ...
        full_arguments,  // was None
        None,
    ).await
}
```

### Step 3: Backend — Extend edit merge to handle messages

**File: `crates/lr-server/src/routes/chat.rs` (lines 160-210)**

Add messages handling after existing param fields:
```rust
// After seed handling (line 209), add:
if let Some(messages) = edits.get("messages") {
    if let Ok(parsed) = serde_json::from_value::<Vec<ChatMessage>>(messages.clone()) {
        request.messages = parsed;
    }
}
if let Some(v) = edits.get("stop") {
    if !v.is_null() {
        request.stop = serde_json::from_value(v.clone()).ok();
    }
}
```

### Step 4: Backend — Handle auto-router edit response

**File: `crates/lr-server/src/routes/chat.rs` (lines 118-136)**

After auto-router approval, check for user edits:
```rust
// In the Allow match arm (line 124-131):
if let Some(ref edits) = response.edited_arguments {
    // Check for request-level edits (nested under "request" key)
    if let Some(req_edits) = edits.get("request") {
        // Apply all field edits (model, messages, params)
        apply_request_edits(&mut request, req_edits);
    }
    // Check if user selected a specific model (skip auto-routing)
    let selected_model = edits.get("request")
        .and_then(|r| r.get("model"))
        .and_then(|v| v.as_str());
    if let Some(model) = selected_model {
        if model != "localrouter/auto" {
            request.model = model.to_string();
            // Don't set to localrouter/auto — bypass auto-routing
        } else {
            request.model = "localrouter/auto".to_string();
        }
    } else {
        request.model = "localrouter/auto".to_string();
    }
} else {
    request.model = "localrouter/auto".to_string();
}
```

Extract the existing edit-merge code (lines 160-210) into an `apply_request_edits()` helper function so it can be reused by both the model firewall and auto-router flows.

### Step 5: Frontend — Enable Modify for auto_router

**File: `src/components/shared/FirewallApprovalCard.tsx` (line 201)**

Remove `auto_router` from exclusion:
```typescript
// Before:
const canEdit = requestType !== "marketplace" && requestType !== "guardrail"
  && requestType !== "free_tier_fallback" && requestType !== "auto_router"
// After:
const canEdit = requestType !== "marketplace" && requestType !== "guardrail"
  && requestType !== "free_tier_fallback"
```

### Step 6: Frontend — Three-tab editor for model & auto-router

**File: `src/views/firewall-approval.tsx`**

**New state:**
```typescript
const [editorMode, setEditorMode] = useState<"params" | "messages" | "raw">("params")
const [editedMessages, setEditedMessages] = useState<{role: string; content: string}[]>([])
```

**Tab bar** (replaces current "Fields" / "JSON" two-tab for model/auto-router):
```
[ Model + Params ]  [ Messages ({n}) ]  [ JSON ]
```
- For MCP tool/skill: Keep existing 2-tab layout (Fields | JSON) — no Messages tab
- For model/auto-router: Show 3-tab layout

**Messages tab UI:**
- List of messages, each with: role dropdown (system/user/assistant/tool) + content textarea
- "Add Message" button at bottom
- Remove button per message
- Auto-size textareas based on content

**Params tab for auto-router:**
- Model dropdown (candidate models from `full_arguments.candidate_models` in a priority group + all allowed models below)
- Option to keep "Auto" (default)
- Same param fields as existing model editor (temperature, max_tokens, etc.)

**Raw JSON tab:**
- Full request body as editable JSON textarea (existing `renderJsonEditor` pattern)

**`enterEditMode` updates:**
- Parse messages from `full_arguments.messages` (model) or `full_arguments.request.messages` (auto-router)
- For auto-router: extract `candidate_models` array for the model dropdown priority group

**`handleAction` updates:**
- Build edited data from all tabs (merge params + messages + any raw JSON edits)
- For auto-router: wrap under `{ request: {...} }` structure
- For model: send flat `{ model, messages, temperature, ... }`

**Window resizing:**
- Params tab: 500×520 (current)
- Messages tab: 560×600 (larger for message list)
- Raw JSON tab: 560×600

### Step 7: Website demo mock update

**File: `website/src/components/demo/TauriMockSetup.ts`**

Update `get_firewall_full_arguments` mock to return full request body with messages for model popup types.

---

## Files to modify

| File | Changes |
|------|---------|
| `crates/lr-server/src/routes/chat.rs` | Capture full request; extract `apply_request_edits()`; handle auto-router edits; add messages merge |
| `crates/lr-mcp/src/gateway/firewall.rs` | Add `full_arguments` param to `request_auto_router_approval` |
| `src/components/shared/FirewallApprovalCard.tsx` | Remove `auto_router` from `canEdit` exclusion |
| `src/views/firewall-approval.tsx` | 3-tab editor; messages editor; auto-router model picker; window resizing |
| `website/src/components/demo/TauriMockSetup.ts` | Update mock for richer full_arguments |

## Verification

1. `cargo test && cargo clippy` — backend compiles and passes
2. `npx tsc --noEmit` — frontend types check
3. Test via debug popup trigger (SamplePopupButton):
   - `auto_router` popup → Modify button appears → 3-tab editor works
   - `llm_model` popup → Modify → Messages tab shows request messages
   - Edit messages → Allow with Edits → verify edits apply to request
   - Raw JSON tab → edit → verify valid JSON enforcement
   - Window resizes correctly between tabs
   - Back button returns to normal view
