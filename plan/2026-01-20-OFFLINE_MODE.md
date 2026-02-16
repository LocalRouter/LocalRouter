# Offline Mode Implementation Plan

## Overview
Add an "Offline Mode" feature that allows users to disable all external providers while keeping local providers active. This will be accessible from:
1. System tray menu (with toggle)
2. UI configuration tab
3. YAML configuration file

## Requirements
- Global `offline_mode` boolean setting (terminology: "Online Mode" vs "Offline Mode", never "disabled offline")
- Per-provider `is_local` flag for OpenAI-compatible providers
- Block external provider requests when offline mode is enabled
- Allow local providers (Ollama, LM Studio, local OpenAI-compatible) to continue working
- UI toggle in Preferences tab + system tray toggle with visual feedback (checkmark)
- Smart auto-detection of local providers from URL
- "Is Local" checkbox that auto-updates while typing URL, but "sticks" once user touches it
- After provider creation, checkbox no longer auto-updates from URL changes
- In-place Router update when offline mode toggles (no recreation)
- Persist state across app restarts
- Default state: **Online Mode** (offline_mode: false)

## Implementation Steps

### 1. Config Schema Changes

**File**: `src-tauri/src/config/mod.rs`

**Changes**:
1. Add `offline_mode` field to `ServerConfig` struct (around line 193):
   ```rust
   pub struct ServerConfig {
       pub host: String,
       pub port: u16,
       #[serde(default = "default_true")]
       pub enable_cors: bool,
       #[serde(default)]  // Defaults to false
       pub offline_mode: bool,
   }
   ```

2. Update `Default` implementation for `ServerConfig` (around line 1130):
   ```rust
   impl Default for ServerConfig {
       fn default() -> Self {
           Self {
               host: "127.0.0.1".to_string(),
               port: 3625,
               enable_cors: false,
               offline_mode: false,  // Default: Online Mode (all providers available)
           }
       }
   }
   ```

**For OpenAI-compatible providers** (no code changes needed):
- The `is_local` flag will be stored in `provider_config` JSON
- Example YAML:
  ```yaml
  providers:
    - name: "LocalAI"
      provider_type: "openai_compatible"
      enabled: true
      provider_config:
        base_url: "http://localhost:8080/v1"
        is_local: true
  ```

### 2. Router Enforcement Logic

**File**: `src-tauri/src/router/mod.rs`

**Changes**:

1. Add offline mode field to `Router` struct (around line 88):
   ```rust
   pub struct Router {
       provider_registry: Arc<ProviderRegistry>,
       offline_mode_enabled: bool,  // ADD THIS
       // ... existing fields
   }
   ```

2. Update `Router::new()` constructor to accept offline mode (around line 103):
   ```rust
   pub fn new(
       provider_registry: Arc<ProviderRegistry>,
       offline_mode_enabled: bool,  // ADD THIS
   ) -> Self {
       Self {
           provider_registry,
           offline_mode_enabled,
           // ... existing fields
       }
   }
   ```

3. Add helper methods to check if provider is local and to update offline mode:
   ```rust
   impl Router {
       /// Determines if a provider is local (doesn't require internet)
       fn is_provider_local(&self, provider_name: &str) -> AppResult<bool> {
           let provider_instance = self
               .provider_registry
               .get_provider(provider_name)
               .ok_or_else(|| {
                   AppError::Router(format!("Provider '{}' not found", provider_name))
               })?;

           let provider_type = &provider_instance.provider_type;

           // Hard-coded local provider types
           if matches!(provider_type.as_str(), "ollama" | "lmstudio") {
               return Ok(true);
           }

           // For OpenAI-compatible, check is_local flag in config
           if provider_type == "openai_compatible" {
               if let Some(is_local) = provider_instance.config.get("is_local") {
                   if let Ok(is_local_bool) = is_local.parse::<bool>() {
                       return Ok(is_local_bool);
                   }
               }
               // Fallback: check if base_url is localhost (for backward compat)
               if let Some(base_url) = provider_instance.config.get("base_url") {
                   let is_localhost = base_url.contains("localhost")
                       || base_url.contains("127.0.0.1")
                       || base_url.contains("::1");
                   return Ok(is_localhost);
               }
           }

           // All other providers are external
           Ok(false)
       }

       /// Updates offline mode setting without recreating Router
       pub fn set_offline_mode(&mut self, enabled: bool) {
           self.offline_mode_enabled = enabled;
           info!("Router offline mode updated: {}", if enabled { "Offline" } else { "Online" });
       }
   }
   ```

4. Add offline mode check in `Router::execute_request()` (after line 355):
   ```rust
   // After getting provider instance
   let provider_instance = self
       .provider_registry
       .get_provider(provider)
       .ok_or_else(|| {
           AppError::Router(format!("Provider '{}' not found or disabled", provider))
       })?;

   // ADD OFFLINE MODE CHECK
   if self.offline_mode_enabled && !self.is_provider_local(provider)? {
       return Err(AppError::Router(format!(
           "Offline mode enabled: cannot use external provider '{}'",
           provider
       )));
   }

   // Continue with health check...
   ```

5. Add offline mode check in `Router::complete_with_auto_routing()` (inside loop at line 506):
   ```rust
   for (idx, (provider, model)) in auto_config.prioritized_models.iter().enumerate() {
       // ADD OFFLINE MODE CHECK
       if self.offline_mode_enabled {
           match self.is_provider_local(provider) {
               Ok(false) => {
                   warn!("Offline mode enabled, skipping external provider {}", provider);
                   continue;  // Skip to next model
               }
               Err(e) => {
                   warn!("Error checking provider locality: {}", e);
                   continue;
               }
               Ok(true) => {
                   // Local provider, proceed
               }
           }
       }

       // Continue with existing rate limit checks...
   ```

6. Update Router initialization in `main.rs` to pass offline mode:
   ```rust
   let router = Router::new(
       provider_registry.clone(),
       config.server.offline_mode,  // Pass from config
   );
   ```

### 3. System Tray Menu Integration

**File**: `src-tauri/src/ui/tray.rs`

**Changes**:

1. Add menu item in `build_tray_menu()` (after line 494, after toggle_server):
   ```rust
   // Add offline mode toggle
   let offline_text = if let Some(config_manager) = app.try_state::<ConfigManager>() {
       let config = config_manager.get();
       if config.server.offline_mode {
           "✓ Offline Mode"
       } else {
           "Offline Mode"
       }
   } else {
       "Offline Mode"
   };
   menu_builder = menu_builder.text("toggle_offline_mode", offline_text);
   ```

2. Add DUPLICATE menu item in `build_tray_menu_from_handle()` (after line 852):
   ```rust
   // Same code as above
   ```

3. Add event handler in `on_menu_event` (after line 54):
   ```rust
   "toggle_offline_mode" => {
       info!("Toggle offline mode requested from tray");
       let app_clone = app.clone();
       tauri::async_runtime::spawn(async move {
           if let Err(e) = handle_toggle_offline_mode(&app_clone).await {
               error!("Failed to toggle offline mode: {}", e);
           }
       });
   }
   ```

4. Add handler function (after line 941):
   ```rust
   async fn handle_toggle_offline_mode<R: Runtime>(app: &AppHandle<R>) -> tauri::Result<()> {
       info!("Toggling offline mode");

       let config_manager = app.state::<ConfigManager>();

       // Toggle the offline mode setting
       config_manager.update(|cfg| {
           cfg.server.offline_mode = !cfg.server.offline_mode;
       }).map_err(|e| tauri::Error::Anyhow(e.into()))?;

       // Save to disk
       config_manager
           .save()
           .await
           .map_err(|e| tauri::Error::Anyhow(e.into()))?;

       let new_state = config_manager.get().server.offline_mode;
       info!("Offline mode set to: {}", new_state);

       // Emit event for UI
       let _ = app.emit("offline-mode-changed", new_state);

       // Rebuild menu to update checkmark
       rebuild_tray_menu(app)?;

       Ok(())
   }
   ```

### 4. UI Configuration Tab (Preferences)

**Files**:
- `src-tauri/src/ui/mod.rs` (Tauri commands)
- `src/components/tabs/PreferencesTab.tsx` (or wherever preferences are)

**Changes**:

1. Add Tauri command to get/set offline mode:
   ```rust
   #[tauri::command]
   pub async fn get_offline_mode(
       config_manager: tauri::State<'_, ConfigManager>,
   ) -> Result<bool, String> {
       Ok(config_manager.get().server.offline_mode)
   }

   #[tauri::command]
   pub async fn set_offline_mode(
       config_manager: tauri::State<'_, ConfigManager>,
       router: tauri::State<'_, Arc<tokio::sync::Mutex<Router>>>,
       enabled: bool,
       app: tauri::AppHandle,
   ) -> Result<(), String> {
       // Update config
       config_manager.update(|cfg| {
           cfg.server.offline_mode = enabled;
       }).map_err(|e| e.to_string())?;

       config_manager.save().await.map_err(|e| e.to_string())?;

       // Update Router in-place (no recreation)
       let mut router_guard = router.lock().await;
       router_guard.set_offline_mode(enabled);
       drop(router_guard);

       // Emit event
       let _ = app.emit("offline-mode-changed", enabled);

       // Rebuild tray menu
       crate::ui::tray::rebuild_tray_menu(&app).map_err(|e| e.to_string())?;

       Ok(())
   }
   ```

2. Register commands in `main.rs`:
   ```rust
   .invoke_handler(tauri::generate_handler![
       // ... existing commands
       ui::get_offline_mode,
       ui::set_offline_mode,
   ])
   ```

3. Add UI toggle in React Preferences tab:
   ```tsx
   const [offlineMode, setOfflineMode] = useState(false);

   useEffect(() => {
       invoke('get_offline_mode').then(setOfflineMode);

       const unlisten = listen('offline-mode-changed', (event) => {
           setOfflineMode(event.payload);
       });

       return () => { unlisten.then(f => f()); };
   }, []);

   const handleToggle = async () => {
       await invoke('set_offline_mode', { enabled: !offlineMode });
   };

   // UI Component - Use clear terminology (Online/Offline, not "disabled offline")
   <div className="setting-row">
       <label>
           <input
               type="checkbox"
               checked={offlineMode}
               onChange={handleToggle}
           />
           {offlineMode ? "Offline Mode" : "Online Mode"}
       </label>
       <p className="help-text">
           {offlineMode
               ? "Only local providers are available. External cloud APIs are blocked."
               : "All providers available. External cloud APIs can be used."}
       </p>
   </div>
   ```

### 5. Provider UI Updates

**File**: `src/components/providers/ProviderDetailPage.tsx` or provider forms

**Changes**:
- Add "Is Local Provider" checkbox for OpenAI-compatible providers
- Checkbox auto-updates based on URL while typing (before save)
- Once user manually touches checkbox, it "sticks" and no longer auto-updates
- After provider is created, checkbox doesn't auto-update from URL changes
- Show help text explaining usage in offline mode

**Implementation**:

```tsx
// State management
const [baseUrl, setBaseUrl] = useState('');
const [isLocal, setIsLocal] = useState(false);
const [isLocalUserSet, setIsLocalUserSet] = useState(false); // Track if user touched it
const [isEditing, setIsEditing] = useState(false); // true for new providers, false for editing existing

// Auto-detect local URL helper
const isLocalUrl = (url: string) => {
  return url.includes('localhost') ||
         url.includes('127.0.0.1') ||
         url.includes('::1');
};

// When URL changes (only during creation, not editing)
useEffect(() => {
  if (!isEditing && !isLocalUserSet && baseUrl) {
    setIsLocal(isLocalUrl(baseUrl));
  }
}, [baseUrl, isEditing, isLocalUserSet]);

// When user manually changes checkbox
const handleIsLocalChange = (e: React.ChangeEvent<HTMLInputElement>) => {
  setIsLocal(e.target.checked);
  setIsLocalUserSet(true); // Checkbox now "sticks"
};

// UI Component
{providerType === 'openai_compatible' && (
  <>
    <div className="form-field">
      <label>Base URL</label>
      <input
        type="text"
        value={baseUrl}
        onChange={(e) => setBaseUrl(e.target.value)}
        placeholder="http://localhost:8080/v1"
      />
    </div>

    <div className="form-field">
      <label>
        <input
          type="checkbox"
          checked={isLocal}
          onChange={handleIsLocalChange}
        />
        Local Provider (no internet required)
      </label>
      <p className="help-text">
        Used by <strong>Offline Mode</strong> to determine if this provider should be blocked.
        Auto-detected from URL but you can override.
        {!isEditing && !isLocalUserSet && " (Auto-updating as you type)"}
      </p>
    </div>
  </>
)}
```

**Key behaviors**:
1. **New provider creation**: Checkbox auto-updates as user types URL
2. **User touches checkbox**: Auto-update stops, value "sticks"
3. **Editing existing provider**: Checkbox never auto-updates, only manual changes
4. **Help text**: Explains connection to Offline Mode

### 6. Update Router State Management

**File**: `src-tauri/src/main.rs` (initialization) and Tauri commands

**Changes**:
- Router is initialized with offline mode from config
- When offline mode changes via Tauri command, Router is updated in-place
- No need to recreate Router or restart server

**In main.rs initialization**:
```rust
// Read initial offline mode from config
let offline_mode = config.server.offline_mode;

// Create router with offline mode
let router = Router::new(
    provider_registry.clone(),
    offline_mode,
);
let router = Arc::new(tokio::sync::Mutex::new(router));
```

**In set_offline_mode Tauri command** (already shown in section 4):
```rust
// Update Router in-place
let mut router_guard = router.lock().await;
router_guard.set_offline_mode(enabled);
drop(router_guard);
```

This approach:
- Updates immediately (no restart required)
- Doesn't disrupt in-flight requests
- Simple and clean

### 7. Error Response Updates

**File**: `src-tauri/src/server/middleware/error.rs`

**Changes**:
- Ensure offline mode errors return appropriate HTTP status
- Use 403 Forbidden or 503 Service Unavailable
- Add clear error message

The existing `AppError::Router` should already map to appropriate HTTP status. Verify the error conversion logic includes a helpful message.

## Critical Files to Modify

1. **src-tauri/src/config/mod.rs** - Add offline_mode to ServerConfig
2. **src-tauri/src/router/mod.rs** - Add offline mode enforcement logic
3. **src-tauri/src/ui/tray.rs** - Add system tray toggle
4. **src-tauri/src/ui/mod.rs** - Add Tauri commands
5. **src-tauri/src/main.rs** - Pass offline mode to Router constructor, register commands
6. **src-tauri/src/server/state.rs** - Update Router state on config changes
7. **src/components/** - Add UI toggle (Settings tab or Server tab)
8. **src/components/providers/** - Add is_local checkbox for OpenAI-compatible

## Provider Classification

**Always Local** (hard-coded):
- `ollama` - localhost:11434
- `lmstudio` - localhost:1234

**Configurable** (check is_local flag + base_url):
- `openai_compatible` - user configurable

**Always External** (blocked in offline mode):
- `openai`, `anthropic`, `gemini`, `groq`, `mistral`, `cohere`, `togetherai`, `perplexity`, `deepinfra`, `cerebras`, `xai`, `openrouter`

## Testing Plan

### Manual Testing

1. **Config Persistence**:
   - Enable offline mode via UI
   - Restart app
   - Verify offline mode is still enabled

2. **System Tray**:
   - Toggle offline mode from tray menu
   - Verify checkmark appears/disappears
   - Verify UI updates automatically

3. **Request Blocking**:
   - Enable offline mode
   - Try to use external provider (OpenAI, Anthropic)
   - Verify request is blocked with clear error message
   - Try local provider (Ollama)
   - Verify request succeeds

4. **Auto-routing**:
   - Enable offline mode
   - Use auto-routing with mix of local and external models
   - Verify only local providers are tried

5. **OpenAI-Compatible Providers**:
   - Add local OpenAI-compatible provider with `is_local: true`
   - Enable offline mode
   - Verify it still works
   - Add remote OpenAI-compatible provider without `is_local` flag
   - Enable offline mode
   - Verify it's blocked

### Unit Tests

Add tests in `src-tauri/src/router/mod.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_offline_mode_blocks_external_providers() {
        // Create router with offline mode enabled
        // Try to execute request with external provider
        // Verify AppError::Router is returned
    }

    #[test]
    fn test_offline_mode_allows_local_providers() {
        // Create router with offline mode enabled
        // Try to execute request with local provider
        // Verify request succeeds
    }

    #[test]
    fn test_is_provider_local_detection() {
        // Test provider type detection
        // Test OpenAI-compatible with is_local flag
        // Test OpenAI-compatible with localhost URL
    }
}
```

## Migration Considerations

- No config version bump needed (offline_mode has sensible default: false = Online Mode)
- Existing configs will deserialize with `offline_mode: false` (Online Mode)
- Existing OpenAI-compatible providers without `is_local` flag will fall back to URL detection
- No breaking changes to existing functionality
- Users can opt-in to offline mode when ready

**Terminology**:
- Use "Online Mode" and "Offline Mode" in UI
- Never use "disabled offline mode" (double negative)
- Config field: `offline_mode: bool` (false = Online, true = Offline)

## Documentation Updates

Add to documentation:
- How offline mode works
- Which providers are considered local
- How to mark OpenAI-compatible providers as local
- Error messages when requests are blocked
- Privacy benefits of offline mode

## Future Enhancements

1. **Per-client offline mode**: Allow different clients to have different offline settings
2. **Provider allowlist**: Instead of binary local/external, allow specific providers in offline mode
3. **Offline mode metrics**: Track how many requests were blocked
4. **Network detection**: Auto-enable offline mode when no internet connection detected
5. **Grace period**: Allow in-flight requests to complete when enabling offline mode

## Verification Checklist

After implementation:
- [ ] Config serialization/deserialization works
- [ ] Offline mode persists across restarts
- [ ] System tray toggle works and shows checkmark
- [ ] UI toggle works and syncs with tray
- [ ] External provider requests are blocked with clear error
- [ ] Local provider requests work normally
- [ ] Auto-routing skips external providers
- [ ] OpenAI-compatible providers respect is_local flag
- [ ] Error messages are clear and helpful
- [ ] No console errors or warnings
- [ ] Tests pass (cargo test)
- [ ] Clippy passes (cargo clippy)
- [ ] Code is formatted (cargo fmt)
