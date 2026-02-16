# Firewall Request Editing Feature

## Context

The firewall approval popup currently only supports binary Allow/Deny decisions. When a resource is set to "Ask" mode, the user sees a compact popup with request details and must approve or deny as-is. There's no way to inspect the full request or modify it before approving. This feature adds an **Edit** button between Deny and Allow that lets users view and modify the full request before sending it through.

## Overview

- Add "Edit" button to the firewall popup (between Deny and Allow)
- Store **full arguments** in the firewall session (currently only 200-char truncated preview is stored)
- Pass **edited data** back through the existing oneshot channel to the waiting request handler
- Apply edits to the request before routing it to the target server
- Resize the popup window when entering edit mode to accommodate the editor

## Design Decisions

- **Edit always counts as "Allow Once"** — no dropdown options in edit mode. Edits only apply to this single request. Simpler UX, clearer semantics.
- **Model editing includes params** — not just model name, but also temperature, max_tokens, top_p, etc. Messages are NOT shown/editable (too large, security-sensitive).

## Editable Request Types

| Type | What's Editable | Editor UI |
|------|----------------|-----------|
| **MCP Tool** | `arguments` JSON object | Key-value editor + raw JSON toggle |
| **Skill Tool** | `arguments` JSON object | Key-value editor + raw JSON toggle |
| **Model/LLM** | Model name + params (temperature, max_tokens, top_p, frequency_penalty, presence_penalty, seed) | Structured form fields |
| **MCP Prompt** | Prompt arguments | Key-value editor |
| **MCP Resource** | Resource URI | Text input |
| **Marketplace** | Not editable | Edit button hidden |

---

## Phase 1: Backend Data Plumbing

### 1.1 `crates/lr-mcp/src/gateway/firewall.rs` — Core structs

**Add `full_arguments` to `FirewallApprovalSession`:**
```rust
pub struct FirewallApprovalSession {
    // ...existing fields...
    pub full_arguments: Option<serde_json::Value>,  // NEW: complete args for edit mode
}
```

**Add `full_arguments` to `PendingApprovalInfo`:**
```rust
pub struct PendingApprovalInfo {
    // ...existing fields...
    pub full_arguments: Option<String>,  // NEW: full args as JSON string (for UI)
}
```
Note: Serialized as `String` (not `Value`) for UI transport — avoids issues with very large nested objects in Tauri serialization. The UI will parse it on demand.

**Extend `FirewallApprovalResponse`:**
```rust
pub struct FirewallApprovalResponse {
    pub action: FirewallApprovalAction,
    pub edited_arguments: Option<serde_json::Value>,  // NEW: edited tool args OR model params
}
```
For MCP/skill tools: contains the edited `arguments` JSON object.
For model requests: contains the edited model params JSON (`{"model": "...", "temperature": 0.5, ...}`).

**Update `request_approval()` / `request_approval_internal()` signatures:**
- Add param: `full_arguments: Option<serde_json::Value>`
- Store it in the session

**Update `submit_response()`:**
- Add param: `edited_arguments: Option<serde_json::Value>`
- Pass it into the `FirewallApprovalResponse` sent through the channel

**Update `list_pending()`:**
- Populate `full_arguments` from session (serialize Value to String)

### 1.2 `crates/lr-mcp/src/gateway/gateway_tools.rs` — Pass full args

In `apply_access_decision()`, `AccessDecision::Ask` branch (~line 488):
- Extract full arguments: `request.params.as_ref().and_then(|p| p.get("arguments")).cloned()`
- Pass to `request_approval()` as `full_arguments: Some(full_args)`

### 1.3 `crates/lr-server/src/routes/chat.rs` — Model requests

In `check_model_firewall_permission()`, `AccessDecision::Ask` branch (~line 495):
- Extract editable params from the `ChatCompletionRequest` as a JSON Value:
```rust
let model_params = serde_json::json!({
    "model": request.model,
    "temperature": request.temperature,
    "max_tokens": request.max_tokens,
    "max_completion_tokens": request.max_completion_tokens,
    "top_p": request.top_p,
    "frequency_penalty": request.frequency_penalty,
    "presence_penalty": request.presence_penalty,
    "seed": request.seed,
});
```
- Pass to `request_model_approval()` as `full_arguments: Some(model_params)`
- This intentionally excludes `messages`, `tools`, `stream`, `response_format` — those are not appropriate for popup editing

**Update `request_model_approval()` signature:**
- Add param: `full_arguments: Option<serde_json::Value>`
- Internally passes to `request_approval_internal()` instead of the current `String::new()`

---

## Phase 2: Backend Request Mutation

### 2.1 `crates/lr-mcp/src/gateway/gateway_tools.rs` — Apply edited arguments

**Change return type of `apply_access_decision()`** from `AppResult<Option<JsonRpcResponse>>` to `AppResult<FirewallDecisionResult>`:

```rust
pub(crate) enum FirewallDecisionResult {
    Proceed,
    ProceedWithEdits { edited_arguments: Option<serde_json::Value> },
    Blocked(JsonRpcResponse),
}
```

In the `AllowOnce` / `AllowSession` / `AllowPermanent` / `Allow1Hour` match arms, return `ProceedWithEdits` if `response.edited_arguments.is_some()`, otherwise `Proceed`.

**In `handle_tools_call()` (~line 280-308):**
After the firewall check, match on the result:
```rust
match firewall_result {
    FirewallDecisionResult::Blocked(resp) => return Ok(resp),
    FirewallDecisionResult::ProceedWithEdits { edited_arguments: Some(new_args) } => {
        // Apply edits to request before routing
        if let Some(params) = transformed_request.params.as_mut() {
            if let Some(obj) = params.as_object_mut() {
                obj.insert("arguments".to_string(), new_args);
            }
        }
    }
    _ => {} // Proceed unchanged
}
```

Same for skill tool calls.

### 2.2 `crates/lr-server/src/routes/chat.rs` — Apply edited model + params

**Change return type of `check_model_firewall_permission()`** from `ApiResult<()>` to `ApiResult<Option<serde_json::Value>>`:
- `Ok(None)` = proceed with original request
- `Ok(Some(edits))` = JSON object with edited params to apply

In `chat_completions()` (~line 95), after calling `check_model_firewall_permission()`:
```rust
if let Some(edits) = firewall_edits {
    // Apply each edited field back to the request
    if let Some(model) = edits.get("model").and_then(|v| v.as_str()) {
        request.model = model.to_string();
    }
    if let Some(temp) = edits.get("temperature").and_then(|v| v.as_f64()) {
        request.temperature = Some(temp as f32);
    }
    if let Some(max) = edits.get("max_tokens").and_then(|v| v.as_u64()) {
        request.max_tokens = Some(max as u32);
    }
    // ... same pattern for top_p, frequency_penalty, presence_penalty, seed, max_completion_tokens
    // Null values in the edits JSON clear the field (set to None)
}
```

The edited data comes from `response.edited_arguments` (which contains the full JSON the user edited in the popup). For model requests, `edited_model` field is not needed separately — the model name is part of the edited arguments JSON.

---

## Phase 3: Tauri Commands

### 3.1 `src-tauri/src/ui/commands_clients.rs`

**Update `submit_firewall_approval`** — add optional param:
```rust
pub async fn submit_firewall_approval(
    // ...existing params...
    request_id: String,
    action: FirewallApprovalAction,
    edited_arguments: Option<String>,  // NEW: JSON string of edited args or model params
    // ...existing state params...
) -> Result<(), String>
```
Parse `edited_arguments` from String → `serde_json::Value` before passing to `submit_response()`.

**Add new command `get_firewall_full_arguments`:**
```rust
#[tauri::command]
pub async fn get_firewall_full_arguments(
    request_id: String,
    state: State<'_, Arc<lr_server::state::AppState>>,
) -> Result<Option<String>, String>
```
Looks up request in pending sessions, returns the `full_arguments` JSON string. This lazy-loads full args only when Edit is clicked (keeps the common Allow/Deny path fast).

### 3.2 `src-tauri/src/main.rs`

Register `get_firewall_full_arguments` in the invoke handler list (~line 1201).

### 3.3 `src-tauri/src/ui/commands.rs`

Already does `pub use commands_clients::*` — new command auto-exported.

---

## Phase 4: TypeScript Types

### 4.1 `src/types/tauri-commands.ts`

Update `SubmitFirewallApprovalParams`:
```typescript
export interface SubmitFirewallApprovalParams {
  requestId: string
  action: FirewallApprovalAction
  editedArguments?: string | null   // JSON string of edited args or model params
}
```

Add:
```typescript
export interface GetFirewallFullArgumentsParams {
  requestId: string
}
```

---

## Phase 5: Frontend UI

### 5.1 `src/views/firewall-approval.tsx`

**New state variables:**
```typescript
const [editMode, setEditMode] = useState(false)
const [fullArguments, setFullArguments] = useState<string | null>(null)
const [editedJson, setEditedJson] = useState<string>('')
const [editedModel, setEditedModel] = useState<string>('')
const [jsonValid, setJsonValid] = useState(true)
const [editorMode, setEditorMode] = useState<'kv' | 'raw'>('kv')
```

**Edit button** — placed between the Deny and Allow split-buttons:
- Hidden for `requestType === 'marketplace'`
- Shows a pencil/edit icon + "Edit" label
- Styled as a neutral/outline button (not destructive, not green)
- On click: calls `enterEditMode()`

**`enterEditMode()` function:**
1. Fetch full arguments: `invoke<string | null>('get_firewall_full_arguments', { requestId })`
2. Set state: `fullArguments`, `editedJson`, `editMode = true`
3. For model requests: set `editedModel = details.tool_name`
4. Resize window: `getCurrentWebviewWindow().setSize(new LogicalSize(500, 520))`
5. Re-center: `getCurrentWebviewWindow().center()`

**`exitEditMode()` function:**
1. Reset edit state
2. Resize back: `setSize(new LogicalSize(400, 320))` + `center()`

**Edit mode view** (replaces details grid when `editMode === true`):

For **MCP tool / skill / prompt** requests:
- Toggle between "Fields" and "JSON" modes (two small tabs at top of editor area)
- **Fields mode**: Each argument key rendered as a label + textarea pair in a scrollable grid. Keys are read-only, values are editable. Uses existing `Textarea` from `src/components/ui/textarea.tsx`.
- **JSON mode**: Single monospace `<textarea>` with full JSON, editable. Real-time validation indicator (border turns red on invalid JSON).
- Both modes stay in sync — switching mode serializes/deserializes

For **model** requests — structured form with labeled fields:
- **Model**: text input, pre-filled with current model name
- **Temperature**: number input (0-2, step 0.1), pre-filled or empty if not set
- **Max Tokens**: number input, pre-filled or empty
- **Top P**: number input (0-1, step 0.1), pre-filled or empty
- **Frequency Penalty**: number input (-2 to 2, step 0.1)
- **Presence Penalty**: number input (-2 to 2, step 0.1)
- **Seed**: number input (integer)
- Provider shown as read-only info above the fields
- Empty/cleared fields = null (remove from request)

For **resource** requests:
- Text input for URI

**Modified action bar in edit mode:**
- Left: "Cancel" text button (exits edit mode, discards edits)
- Right: Single "Allow with Edits" green button (always `allow_once`)
- No dropdown options — edits only apply to this single request

**Modified `handleAction()`:**
```typescript
// In edit mode, always use allow_once
const params: any = { requestId: details.request_id, action: 'allow_once' }
if (editMode) {
  // Send edited data — for tools/skills/prompts, it's the edited JSON string
  // For models, it's also JSON but with model params structure
  if (editedJson !== fullArguments) {
    params.editedArguments = editedJson
  }
}
await invoke("submit_firewall_approval", params)
```

### 5.2 Window Sizing

Current: 400x320, `resizable(false)`. The `set-size` capability is already enabled.
Edit mode: Programmatically resize to 500x520 (wider + taller for the editor).
Exit edit: Resize back to 400x320.

---

## Phase 6: Demo Mock & Debug

### 6.1 `website/src/components/demo/TauriMockSetup.ts`

Add mock for `get_firewall_full_arguments`:
```typescript
'get_firewall_full_arguments': () => JSON.stringify({ path: '/tmp/test.txt', content: 'hello world' })
```

Update `submit_firewall_approval` mock to accept/ignore new optional params.

### 6.2 `src-tauri/src/ui/commands.rs` — `debug_trigger_firewall_popup`

Update the debug command to include `full_arguments` in the test session so Edit mode can be tested during development.

---

## Verification

1. **`cargo test`** — all existing firewall tests pass (backwards-compatible; edited_arguments defaults to None)
2. **`cargo clippy`** — no warnings
3. **`npx tsc --noEmit`** — TypeScript types check out
4. **Manual testing with `debug_trigger_firewall_popup`:**
   - Open popup → verify Edit button appears for tool/skill/model types
   - Click Edit → window resizes, full arguments load
   - Modify arguments → click "Allow with Edits" → verify edited args reach backend (check logs)
   - Click Back → window returns to compact size
   - Marketplace popup → verify Edit button is hidden
5. **End-to-end with real MCP tool:**
   - Set an MCP tool to "Ask" mode
   - Trigger it via API
   - Edit the arguments in the popup
   - Verify the server receives the edited arguments (not the originals)
6. **Model request:**
   - Set a model to "Ask"
   - Edit the model name in the popup
   - Verify the request routes to the edited model

## Critical Files

| File | Changes |
|------|---------|
| `crates/lr-mcp/src/gateway/firewall.rs` | `full_arguments` in session/info, `edited_*` in response, updated `submit_response()` |
| `crates/lr-mcp/src/gateway/gateway_tools.rs` | `FirewallDecisionResult` enum, pass full args, apply edits before routing |
| `crates/lr-server/src/routes/chat.rs` | Return edited model from permission check, apply to request |
| `src-tauri/src/ui/commands_clients.rs` | New `get_firewall_full_arguments`, updated `submit_firewall_approval` |
| `src-tauri/src/main.rs` | Register new command |
| `src/views/firewall-approval.tsx` | Edit mode UI, window resize, JSON editor, model editor |
| `src/types/tauri-commands.ts` | Updated params types |
| `website/src/components/demo/TauriMockSetup.ts` | Mock for new command |
