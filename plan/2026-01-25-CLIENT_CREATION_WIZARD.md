# Client Creation Wizard Implementation Plan

## Overview

Create a multi-step wizard flow for client creation that guides users through:
1. Naming the client
2. Selecting models (with inline provider setup if needed)
3. Selecting MCP servers (with inline MCP setup if needed)
4. Viewing credentials (API key + OAuth)

## Trigger Points

1. **Client List Page**: "Create Client" button opens wizard
2. **First-Run**: App startup with `clients.length === 0` → auto-open wizard
3. **Dashboard**: Add "Create Client" button → opens wizard
4. **System Tray**: "Quick Create & Copy API Key" (simplified, no wizard)

---

## Reusable Components (Already Built)

| Component | Location | Reuse In |
|-----------|----------|----------|
| `AllowedModelsSelector` | `src/components/strategy/AllowedModelsSelector.tsx` | Step 2 - Model selection |
| MCP server checkbox list | `src/views/clients/tabs/mcp-tab.tsx` | Step 3 - Extract as component |
| Credentials display | `src/views/clients/tabs/config-tab.tsx` | Step 4 - Extract as component |
| `ProviderForm` | `src/components/ProviderForm.tsx` | Step 2 - Inline add provider |
| `McpServerTemplates` | `src/components/mcp/McpServerTemplates.tsx` | Step 3 - Inline add MCP |
| MCP manual form | `src/views/resources/mcp-servers-panel.tsx` | Step 3 - Inline add MCP |

---

## Implementation Plan

### Phase 1: Extract Reusable Components

**1.1 Extract `McpServerSelector` from `mcp-tab.tsx`**

File: `src/components/mcp/McpServerSelector.tsx`

```typescript
interface McpServerSelectorProps {
  servers: McpServer[]
  accessMode: 'none' | 'all' | 'specific'
  selectedServers: string[]
  onChange: (mode: 'none' | 'all' | 'specific', servers: string[]) => void
  loading?: boolean
  disabled?: boolean
}
```

Extract the checkbox list UI (lines 225-293 in mcp-tab.tsx) into a standalone component.

**1.2 Extract `CredentialsDisplay` from `config-tab.tsx`**

File: `src/components/client/CredentialsDisplay.tsx`

```typescript
interface CredentialsDisplayProps {
  clientId: string
  secret: string | null
  loadingSecret: boolean
  showWarning?: boolean  // "Save now, shown once" warning
}
```

Extract the credentials card (lines 197-375 in config-tab.tsx) without the name editing or rotate functionality.

### Phase 2: Create Wizard Component

**2.1 Wizard Container**

File: `src/components/wizard/ClientCreationWizard.tsx`

```typescript
interface ClientCreationWizardProps {
  open: boolean
  onOpenChange: (open: boolean) => void
  onComplete: (clientId: string) => void
}
```

- Multi-step modal dialog
- Progress indicator (Step X of 4)
- Back/Next/Skip navigation
- State managed via useState

**2.2 Wizard State**

```typescript
interface WizardState {
  // Step 1
  clientName: string

  // Step 2 - Models
  allowedModels: AllowedModelsSelection

  // Step 3 - MCP
  mcpAccessMode: 'none' | 'all' | 'specific'
  selectedMcpServers: string[]

  // After creation
  clientId?: string
  clientSecret?: string
}
```

### Phase 3: Implement Wizard Steps

**Step 1: Name Your Client** (`src/components/wizard/steps/StepName.tsx`)

- Title: "Create New Client"
- Simple Input field for name
- Placeholder examples
- Validation: non-empty

**Step 2: Select Models** (`src/components/wizard/steps/StepModels.tsx`)

- Reuse `AllowedModelsSelector` component
- Default: `selected_all: true`
- Empty state: Show "Add Provider" inline form
- Inline form uses `ProviderForm` component (from providers-panel.tsx pattern)

**Step 3: Select MCP Servers** (`src/components/wizard/steps/StepMcp.tsx`)

- Reuse new `McpServerSelector` component
- Default: No servers selected (`mode: 'none'`)
- "Skip" button prominent (MCP is optional)
- Empty state: Show "Add MCP Server" inline form
- Inline form: Tabs for Templates/Manual (from mcp-servers-panel.tsx)

**Step 4: Credentials** (`src/components/wizard/steps/StepCredentials.tsx`)

- Reuse new `CredentialsDisplay` component
- Show warning: "Save these credentials now"
- "Copy API Key" primary button
- "Done" button closes wizard

### Phase 4: Backend Integration

**4.1 New Tauri Command** (optional optimization)

The wizard can use existing commands sequentially:
1. `create_client(name)` → returns `(secret, ClientInfo)`
2. `update_strategy_allowed_models(strategyId, allowedModels)`
3. `set_client_mcp_access(clientId, mode, servers)`

Or create a single atomic command:

File: `src-tauri/src/ui/commands.rs`

```rust
#[tauri::command]
pub async fn create_client_with_config(
    name: String,
    allowed_models: Option<AllowedModelsSelection>,
    mcp_access_mode: String,
    mcp_servers: Option<Vec<String>>,
    // ... state params
) -> Result<(String, ClientInfo), String>
```

### Phase 5: System Tray Changes

File: `src-tauri/src/ui/tray.rs`

**5.1 Rename menu item** (line ~368):
```rust
// Before
"➕ Create & copy API Key"
// After
"Quick Create & Copy API Key"
```

**5.2 Update handler** `handle_create_and_copy_api_key()` (lines 774-809):

```rust
async fn handle_create_and_copy_api_key<R: Runtime>(app: &AppHandle<R>) -> tauri::Result<()> {
    // 1. Create client with name "App" (existing)
    let (client_id, secret, config) = client_manager.create_client("App".to_string())?;

    // 2. NEW: Update strategy to allow ALL models
    let strategy = config_manager.get_strategy(&config.strategy_id)?;
    strategy.allowed_models = AllowedModelsSelection {
        selected_all: true,
        selected_providers: vec![],
        selected_models: vec![],
    };
    // Clear auto provider and weak model
    strategy.prioritized_models = vec![];
    strategy.weak_model = None;

    // 3. NEW: Set MCP access to none (already default, but be explicit)
    // config.mcp_server_access = McpServerAccess::None;

    // 4. Save and copy (existing)
    config_manager.save().await?;
    copy_to_clipboard(&secret)?;

    Ok(())
}
```

### Phase 6: First-Run Detection

The wizard should automatically open on first app launch. Track this via a simple flag in the config.

**Backend**: Add flag to AppConfig and Tauri commands.

File: `src-tauri/src/config/mod.rs`

```rust
pub struct AppConfig {
    // ... existing fields ...

    /// Whether the first-install wizard has been shown
    #[serde(default)]
    pub setup_wizard_shown: bool,
}
```

File: `src-tauri/src/ui/commands.rs`

```rust
#[tauri::command]
pub async fn get_setup_wizard_shown(
    config_manager: State<'_, ConfigManager>,
) -> Result<bool, String> {
    Ok(config_manager.config().setup_wizard_shown)
}

#[tauri::command]
pub async fn set_setup_wizard_shown(
    config_manager: State<'_, ConfigManager>,
) -> Result<(), String> {
    config_manager.update(|cfg| {
        cfg.setup_wizard_shown = true;
    })?;
    config_manager.save().await.map_err(|e| e.to_string())
}
```

**Frontend**: Check flag on app startup.

File: `src/App.tsx`

```typescript
function App() {
  const [showWizard, setShowWizard] = useState(false)
  const [ready, setReady] = useState(false)

  useEffect(() => {
    const checkSetupWizard = async () => {
      const shown = await invoke<boolean>('get_setup_wizard_shown')
      if (!shown) {
        setShowWizard(true)
      }
      setReady(true)
    }
    checkSetupWizard()
  }, [])

  const handleWizardComplete = async (clientId: string) => {
    await invoke('set_setup_wizard_shown')
    setShowWizard(false)
    handleViewChange('clients', `${clientId}/config`)
  }

  return (
    <>
      <AppShell ...>
        {ready && renderView()}
      </AppShell>

      <ClientCreationWizard
        open={showWizard}
        onOpenChange={setShowWizard}
        onComplete={handleWizardComplete}
      />
    </>
  )
}
```

### Phase 7: Dashboard Integration

File: `src/views/dashboard/index.tsx`

Add a "Create Client" button that is **always visible** in the dashboard (not just when empty). This provides quick access to creating new clients from the main view.

---

## File Changes Summary

### New Files
- `src/components/wizard/ClientCreationWizard.tsx`
- `src/components/wizard/steps/StepName.tsx`
- `src/components/wizard/steps/StepModels.tsx`
- `src/components/wizard/steps/StepMcp.tsx`
- `src/components/wizard/steps/StepCredentials.tsx`
- `src/components/mcp/McpServerSelector.tsx`
- `src/components/client/CredentialsDisplay.tsx`

### Modified Files
- `src/App.tsx` - First-run detection, wizard integration
- `src/views/clients/index.tsx` - Replace create dialog with wizard
- `src/views/clients/tabs/mcp-tab.tsx` - Use extracted McpServerSelector
- `src/views/clients/tabs/config-tab.tsx` - Use extracted CredentialsDisplay
- `src/views/dashboard/index.tsx` - Add "Create Client" button (always visible)
- `src-tauri/src/config/mod.rs` - Add `setup_wizard_shown` flag to AppConfig
- `src-tauri/src/ui/tray.rs` - Rename + update quick create behavior
- `src-tauri/src/ui/commands.rs` - Add `get_setup_wizard_shown`, `set_setup_wizard_shown` commands

---

## Verification Plan

1. **Wizard Flow**:
   - Create client → verify name persists
   - Select models → verify strategy updated
   - Select MCP → verify access mode set
   - Copy credentials → verify clipboard

2. **First-Run** (true first launch):
   - Fresh install (no config file) → launch app → wizard auto-opens
   - Complete wizard → `first_run_complete` flag set
   - Restart app → wizard does NOT auto-open
   - Delete clients but keep config → restart app → wizard does NOT auto-open (not first-run)

3. **Quick Create (Tray)**:
   - Click "Quick Create & Copy API Key"
   - Verify client created with ALL models
   - Verify no prioritized models, no weak model
   - Verify MCP access = none
   - Verify API key copied to clipboard

4. **Dashboard**:
   - "Create Client" button always visible
   - Click → wizard opens

5. **Clients List**:
   - Click "Create Client" → wizard opens
   - Complete wizard → navigates to new client
