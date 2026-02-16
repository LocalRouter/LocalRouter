# Guardrails Redesign: Per-Client Configuration

## Context

Guardrails are currently configured globally — one set of models, categories, and actions for all clients. This redesign moves guardrails to per-client configuration so each client can have different safety policies. The global settings page becomes a model management hub (download/manage models, HF token), while per-client category selection implicitly determines which models run. A new "Guardrails" tab in Try It Out replaces the test panel.

Key changes:
- No global enable/disable — guardrails are enabled per-client
- No model enable/disable — downloaded/provider models are always available; picking categories implicitly selects models
- New "Block" action silently denies without popup
- "Deny All" option in firewall popup sets categories to Block
- Test panel moves to Try It Out as a new Guardrails tab

---

## Step 1: Backend Config Types

**File: `crates/lr-config/src/types.rs`**

### 1a. Add resource properties to `SafetyModelConfig` (after line 1395)

```rust
#[serde(default)]
pub memory_mb: Option<u32>,
#[serde(default)]
pub latency_ms: Option<u32>,
#[serde(default)]
pub disk_size_mb: Option<u32>,
```

Update `default_safety_models()` with estimates:
| Model | memory_mb | latency_ms | disk_size_mb |
|-------|-----------|------------|--------------|
| Llama Guard 1B | 700 | 300 | 955 |
| Granite Guardian 2B | 1200 | 500 | 1500 |
| ShieldGemma 2B | 1200 | 400 | 1700 |
| Nemotron 8B | 5000 | 800 | 8500 |

### 1b. Add `ClientGuardrailsConfig` struct

```rust
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub struct ClientGuardrailsConfig {
    #[serde(default)]
    pub enabled: bool,
    /// Per-category actions: allow/notify/ask/block
    /// Categories are selected here; which models run is derived from which categories are selected
    #[serde(default)]
    pub category_actions: Vec<CategoryActionEntry>,
}
```

### 1c. Update `CategoryActionEntry` action values

Support `"block"` as a fourth action (in addition to allow/notify/ask).

### 1d. Update `Client` struct

Replace `guardrails_enabled: Option<bool>` (line 1694) with:
```rust
#[serde(default)]
pub guardrails: ClientGuardrailsConfig,
```

### 1e. Simplify `GuardrailsConfig` (global)

- Remove `enabled` field (no longer global toggle)
- Remove `scan_responses` (assume scan requests only for now)
- Remove `category_actions` (moved to per-client)
- Keep: `safety_models`, `hf_token`, `default_confidence_threshold`, `idle_timeout_secs`, `context_size`, `scan_requests`

### 1f. Config migration

In config migration code, handle old format:
- `guardrails_enabled: Some(true)` → `guardrails.enabled = true` with empty category_actions
- Move global `category_actions` nowhere (users reconfigure per-client)

---

## Step 2: Backend Safety Engine Changes

**File: `crates/lr-server/src/routes/chat.rs`**

### 2a. Per-client model resolution

Change `run_guardrails_scan` to:
1. Check `client.guardrails.enabled` instead of global toggle
2. From client's `category_actions`, derive which model types are needed (each category maps to model types via `SafetyCategoryInfo.supported_by`)
3. Only invoke those models from the global safety engine
4. Use client's `category_actions` for action resolution

### 2b. Handle "block" action

Before opening firewall approval popup:
- If ALL flagged categories have action "block" → immediately return 403 (no popup)
- If SOME are "block" and some "ask" → auto-deny blocked ones, show popup only for "ask" ones
- "block" categories never show in the approval popup

**File: `crates/lr-guardrails/src/engine.rs`**

Add method to run only specific model types:
```rust
pub async fn check_input_filtered(&self, request: &Value, model_types: &HashSet<String>) -> SafetyCheckResult
```

---

## Step 3: New/Modified Tauri Commands

**File: `src-tauri/src/ui/commands.rs`**

### 3a. New commands

```rust
get_client_guardrails_config(client_id: String) -> ClientGuardrailsConfig
update_client_guardrails_config(client_id: String, config_json: String) -> ()
list_available_safety_models() -> Vec<AvailableSafetyModel>
// Returns enriched list: each model with source_type (direct_download | provider),
// provider_name, ready status, resource properties
```

`list_available_safety_models` logic:
1. Get all safety models from global config
2. Get configured providers via `list_provider_instances`
3. For each model type with a matching provider model name, add a "via {provider}" entry
4. Mark direct downloads as ready if downloaded, provider entries as ready if provider is enabled

### 3b. Firewall "deny_all" / "block_category" action

Add handler: when received, update the client's `category_actions` to set all flagged categories to "block".

### 3c. Remove/deprecate `set_client_guardrails_enabled`

Replace with `update_client_guardrails_config`.

---

## Step 4: TypeScript Types

**File: `src/types/tauri-commands.ts`**

```typescript
export interface ClientGuardrailsConfig {
  enabled: boolean
  category_actions: CategoryActionEntry[]
}

export interface AvailableSafetyModel {
  id: string
  label: string
  model_type: string
  source_type: "direct_download" | "provider"
  provider_name: string | null
  ready: boolean
  downloading: boolean
  memory_mb: number | null
  latency_ms: number | null
  disk_size_mb: number | null
}

// Update CategoryActionEntry action to include "block"
// Add command param types for new commands
```

Add `memory_mb`, `latency_ms`, `disk_size_mb` to `SafetyModelConfig`.

---

## Step 5: Frontend — Extract Reusable Components

Refactor `src/views/settings/guardrails-tab.tsx` (1432 lines) into smaller reusable pieces:

### 5a. `src/components/guardrails/SafetyModelPicker.tsx`

Dropdown for selecting a model to download/use. Shows model families with variants grouped:
```
Llama Guard
  ├─ Llama Guard 3 1B (~955 MB) — Direct download
  ├─ Llama Guard 3 1B — via Ollama
  ├─ Llama Guard 3 8B (~4.9 GB) — Direct download
  ...
Granite Guardian
  ├─ Granite Guardian 3.0 2B (~1.5 GB) — Direct download
  ├─ Granite Guardian 2B — via Ollama
  ...
```

Props: `onSelect(model)`, `existingModelIds`, `providers`
- Button says "Download" for direct download, "Use" for provider-based
- Indented variants under family headers

### 5b. `src/components/guardrails/SafetyModelList.tsx`

Simple list of available models with: name, source badge ("Direct download" or "via Ollama"), download progress if downloading, trash button.

Props: `models`, `downloadStatuses`, `downloadProgress`, `onRemove`
- No enable/disable toggle

### 5c. `src/components/guardrails/ResourceRequirements.tsx`

Shows aggregate resource requirements based on which models will be used.

Props: `models: AvailableSafetyModel[]` (the models that will run based on selected categories)
- Memory: sum of all model `memory_mb` values
- Latency: max of all model `latency_ms` values (parallel execution)
- Disk: sum of disk_size_mb for downloaded models

### 5d. Update `src/components/permissions/CategoryActionButton.tsx`

Add "Block" as fourth state:
- Color: red-600 (active), red-600/30 (rollup)
- Button order: Allow | Notify | Ask | Block

### 5e. Update `src/constants/safety-model-variants.ts`

Add resource properties to each variant:
```typescript
memoryMb: number
latencyMs: number
diskSizeMb: number
```

Add provider model name mappings per model type (for building "via Provider" dropdown entries).

---

## Step 6: Redesign Global Settings Page

**File: `src/views/settings/guardrails-tab.tsx`**

Simplified layout:

1. **Card: GuardRails header** — Shield icon, "GuardRails" title, `<Badge variant="outline">Experimental</Badge>`, description, link to Try It Out guardrails tab
2. **Card: Common Settings** — HF token only
3. **Card: Safety Models** — `<SafetyModelPicker>` + `<SafetyModelList>` (download/manage models, no enable/disable)
4. **Card: Memory Management** — idle timeout, context window, unload all (only shown when GGUF models downloaded)

Removed from global: enable toggle, scan toggles, category actions, test panel, resource requirements.

---

## Step 7: New Client Guardrails Tab

**New file: `src/views/clients/tabs/guardrails-tab.tsx`**

Layout:

1. **Card: GuardRails** — Shield icon, title, `<Badge>Experimental</Badge>`, enable toggle, description of what guardrails are, link to Try It Out
2. **Card: Available Models** (shown when enabled) — Read-only `<SafetyModelList>` showing all globally available models. Info text: "Models are managed in Settings. Categories you select below determine which models run."
3. **Card: Category Actions** (shown when enabled) — `<PermissionTreeSelector>` with `<CategoryActionButton>` (Allow/Notify/Ask/Block). Categories grouped by model type. Selecting categories for a model type implicitly means that model runs for this client. `__global` default action. Same 3-level hierarchy (global → model-type → category).
4. **Card: Resource Requirements** (shown when categories selected) — `<ResourceRequirements>` showing aggregate of models that will run based on selected categories. E.g., if user picks categories from Granite Guardian and Llama Guard, show combined memory of those two models, max latency of the two.

**Update `src/views/clients/client-detail.tsx`:**
- Add `guardrails` field to Client interface (type: `ClientGuardrailsConfig`)
- Add GuardRails tab trigger after Skills (always visible, not gated by clientMode)
- Add TabsContent with `<ClientGuardrailsTab>`
- Handle `activeTab === "guardrails"` fallback

---

## Step 8: Try It Out — Guardrails Tab

**New file: `src/views/try-it-out/guardrails-tab/index.tsx`**

Add a third tab to Try It Out alongside LLM and MCP.

Mode selector (similar to other tabs):
- **Client** — Pick a client, run safety check using that client's guardrails config (models + category actions)
- **All Models** — Run all available models (downloaded + provider), show raw verdicts
- **Specific Model** — Pick one model, run only that model

UI:
- Mode dropdown / radio selector at top
- Client selector (when client mode)
- Model selector (when specific model mode)
- Quick test buttons (Jailbreak, Violence, Self-harm, Safe)
- Text input + Run button
- Results table: Model | Verdict | Flagged Categories | Confidence | Duration
- Actions Required section (only in client mode, uses client's category actions)
- Raw output expandable
- Summary row

Move the test panel logic from `guardrails-tab.tsx` into this component.

**Update `src/views/try-it-out/index.tsx`:**
- Add `<TabsTrigger value="guardrails">GuardRails</TabsTrigger>`
- Add `<TabsContent>` with `<GuardrailsTab>`
- Support init path `"guardrails/init/client/{clientId}"`

---

## Step 9: Firewall Approval — Deny All / Block

**File: `src/components/shared/FirewallApprovalCard.tsx`**

Add to Deny dropdown (after "Disable Client"):
```tsx
{isGuardrailRequest && (
  <DropdownMenuItem onClick={() => onAction?.("block_categories")}>
    Block Categories
  </DropdownMenuItem>
)}
```

**File: `src/views/firewall-approval.tsx`**

Handle `"block_categories"` action:
- Call `update_client_guardrails_config` to set all flagged categories to "block" in the client's config
- Then deny the current request

**Backend**: Handle in firewall approval response — when action is "block_categories", update client guardrails and return deny.

---

## Step 10: Demo Mocks

**File: `website/src/components/demo/TauriMockSetup.ts`**

Add mocks for:
- `get_client_guardrails_config` → demo ClientGuardrailsConfig
- `update_client_guardrails_config` → no-op
- `list_available_safety_models` → demo list with direct + provider entries

---

## Implementation Order

1. Backend config types (Step 1)
2. Config migration (Step 1f)
3. Safety engine per-client filtering + block action (Step 2)
4. New Tauri commands (Step 3)
5. TypeScript types (Step 4)
6. CategoryActionButton — add "block" state (Step 5d)
7. Extract reusable components (Step 5a-c, 5e)
8. Redesign global settings page (Step 6)
9. New client guardrails tab (Step 7)
10. Try It Out guardrails tab (Step 8)
11. Firewall approval changes (Step 9)
12. Demo mocks (Step 10)

---

## Verification

1. `cargo test && cargo clippy && cargo fmt` — all pass
2. `npx tsc --noEmit` — no type errors
3. `cargo tauri dev` — app launches, settings page shows simplified guardrails
4. Create a client → GuardRails tab appears after Skills
5. Enable guardrails for client → select categories → resource requirements update
6. Download a model in global settings → appears in client's available models
7. Try It Out → GuardRails tab → test with client mode and all-models mode
8. Trigger a guardrail violation → popup shows → "Block Categories" in deny menu works
9. After blocking, same violation is silently denied (no popup)
10. Demo website mock data renders correctly

## Critical Files

- `crates/lr-config/src/types.rs` — Config types, Client struct, GuardrailsConfig
- `crates/lr-server/src/routes/chat.rs` — Safety scan logic, block handling
- `crates/lr-guardrails/src/engine.rs` — Filtered model execution
- `src-tauri/src/ui/commands.rs` — Tauri commands
- `src/views/settings/guardrails-tab.tsx` — Global settings (major refactor)
- `src/views/clients/client-detail.tsx` — Add guardrails tab
- `src/views/clients/tabs/guardrails-tab.tsx` — New client tab
- `src/views/try-it-out/index.tsx` — Add guardrails tab
- `src/views/try-it-out/guardrails-tab/index.tsx` — New test tab
- `src/components/permissions/CategoryActionButton.tsx` — Add "block" state
- `src/components/guardrails/SafetyModelPicker.tsx` — New reusable picker
- `src/components/guardrails/SafetyModelList.tsx` — New reusable list
- `src/components/guardrails/ResourceRequirements.tsx` — New reusable component
- `src/components/shared/FirewallApprovalCard.tsx` — Deny all / block
- `src/types/tauri-commands.ts` — TypeScript types
- `src/constants/safety-model-variants.ts` — Resource properties
- `website/src/components/demo/TauriMockSetup.ts` — Demo mocks
