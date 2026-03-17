# Plan: "Clients Using This Feature" Section on Optimize Feature Pages

## Context

Each Optimize feature (JSON Repair, Compression, GuardRails, Secret Scanning, Catalog Compression, Context Management) has a global on/off toggle and per-client overrides (`Option<bool>` or similar). Feature detail pages currently show configuration but not which clients are affected. This plan adds a read-only section to each feature's Info tab showing which clients effectively have the feature enabled, with links to the client's Optimize tab.

## Approach

**One new backend command + one shared frontend component**, integrated into 6 feature pages.

---

## Step 1: Backend — New Tauri Command

**File:** `src-tauri/src/ui/commands_clients.rs`

Add struct and command:

```rust
#[derive(Debug, Clone, Serialize)]
pub struct ClientFeatureStatus {
    pub client_id: String,
    pub client_name: String,
    /// Whether the feature is effectively active for this client
    pub active: bool,
    /// "override" if per-client setting exists, "global" if inherited
    pub source: String,
}

#[tauri::command]
pub async fn get_feature_clients_status(
    feature: String,
    state: tauri::State<'_, crate::state::AppState>,
) -> Result<Vec<ClientFeatureStatus>, String>
```

Resolution logic per feature (all data is on `Client` struct + `AppConfig`):

| Feature | Global default | Per-client override | Active when |
|---------|---------------|-------------------|-------------|
| `json_repair` | `config.json_repair.enabled` | `client.json_repair.enabled: Option<bool>` | `override.unwrap_or(global)` |
| `prompt_compression` | `config.prompt_compression.enabled` | `client.prompt_compression.enabled: Option<bool>` | `override.unwrap_or(global)` |
| `guardrails` | `config.guardrails.scan_requests` | `client.guardrails.category_actions: Option<Vec<...>>` | Reuse existing `guardrails_active` logic (effective actions have any non-"allow") |
| `secret_scanning` | `config.secret_scanning.action` | `client.secret_scanning.action: Option<SecretScanAction>` | effective action != `Off` |
| `catalog_compression` | `config.context_management.catalog_compression` | `client.catalog_compression_enabled: Option<bool>` | Reuse `client.is_catalog_compression_enabled()` |
| `context_management` | derived from `config.context_management` | `client.context_management_enabled: Option<bool>` | Reuse `client.is_context_management_enabled()` |

Source: `"override"` when the per-client field is `Some(...)`, else `"global"`.

Filter out test clients (same as `list_clients`).

**File:** `src-tauri/src/main.rs`
Register `commands_clients::get_feature_clients_status` in `invoke_handler`.

---

## Step 2: TypeScript Types

**File:** `src/types/tauri-commands.ts`

```typescript
/** Rust: src-tauri/src/ui/commands_clients.rs - ClientFeatureStatus */
export interface ClientFeatureStatus {
  client_id: string
  client_name: string
  active: boolean
  source: 'override' | 'global'
}

/** Params for get_feature_clients_status */
export interface GetFeatureClientsStatusParams {
  feature: 'json_repair' | 'prompt_compression' | 'guardrails' | 'secret_scanning' | 'catalog_compression' | 'context_management'
}
```

---

## Step 3: Shared Frontend Component

**New file:** `src/components/shared/FeatureClientsCard.tsx`

Props:
```typescript
interface FeatureClientsCardProps {
  feature: GetFeatureClientsStatusParams['feature']
  onNavigateToClient?: (view: string, subTab?: string | null) => void
}
```

Behavior:
- On mount + on `clients-changed` / `config-changed` events: call `invoke<ClientFeatureStatus[]>('get_feature_clients_status', { feature })`
- Render a `Card` titled "Clients"
- Show a compact read-only list/table of clients:
  - **Client name** (clickable → navigates to client Optimize tab)
  - **Status badge**: green "Active" or muted "Inactive"
  - **Source badge**: small "Override" pill if source is `"override"`, nothing for "global" (inherited is the default, no need to call it out)
- If no clients exist: show "No clients configured" with muted text
- Navigation: `onNavigateToClient?.("clients", `${clientId}|optimize`)`
  (Uses existing `ClientsView.parseSubTab` format: `"clientId|tab"`)

---

## Step 4: Integrate into Feature Pages

Add `<FeatureClientsCard>` to the **Info tab** of each feature page, after the main config card(s):

| File | Feature prop |
|------|-------------|
| `src/views/json-repair/index.tsx` | `"json_repair"` |
| `src/views/compression/index.tsx` | `"prompt_compression"` |
| `src/views/guardrails/index.tsx` | `"guardrails"` |
| `src/views/secret-scanning/index.tsx` | `"secret_scanning"` |
| `src/views/catalog-compression/index.tsx` | `"catalog_compression"` |
| `src/views/response-rag/index.tsx` | `"context_management"` |

Each page already has `onTabChange` prop — pass it as `onNavigateToClient`.

---

## Step 5: Demo Mock

**File:** `website/src/components/demo/TauriMockSetup.ts`

```typescript
'get_feature_clients_status': (): ClientFeatureStatus[] => [
  { client_id: 'demo-1', client_name: 'Claude Code', active: true, source: 'global' },
  { client_id: 'demo-2', client_name: 'Cursor', active: true, source: 'override' },
],
```

---

## Files to Modify

| File | Change |
|------|--------|
| `src-tauri/src/ui/commands_clients.rs` | Add `ClientFeatureStatus` struct + `get_feature_clients_status` command |
| `src-tauri/src/main.rs` | Register new command |
| `src/types/tauri-commands.ts` | Add `ClientFeatureStatus` + `GetFeatureClientsStatusParams` |
| `src/components/shared/FeatureClientsCard.tsx` | **New** — shared component |
| `src/views/json-repair/index.tsx` | Add `<FeatureClientsCard>` to Info tab |
| `src/views/compression/index.tsx` | Add `<FeatureClientsCard>` to Info tab |
| `src/views/guardrails/index.tsx` | Add `<FeatureClientsCard>` to Info tab |
| `src/views/secret-scanning/index.tsx` | Add `<FeatureClientsCard>` to Info tab |
| `src/views/catalog-compression/index.tsx` | Add `<FeatureClientsCard>` to Info tab |
| `src/views/response-rag/index.tsx` | Add `<FeatureClientsCard>` to Info tab |
| `website/src/components/demo/TauriMockSetup.ts` | Add mock handler |

## Existing Code to Reuse

- `client.is_catalog_compression_enabled(&config.context_management)` — `crates/lr-config/src/types.rs:3415`
- `client.is_context_management_enabled(&config.context_management)` — `crates/lr-config/src/types.rs:3403`
- `guardrails_active` computation — `src-tauri/src/ui/commands_clients.rs:96`
- `ClientsView.parseSubTab()` — `src/views/clients/index.tsx:112` (navigation format: `"clientId|tab"`)
- Card/Badge UI components from `@/components/ui/`

## Verification

1. `cargo test && cargo clippy && cargo fmt`
2. `npx tsc --noEmit`
3. Open each feature page's Info tab → confirm "Clients" card renders
4. Toggle a global feature off → confirm client statuses update
5. Add a per-client override → confirm "Override" badge appears
6. Click a client name → confirm it navigates to client Optimize tab
