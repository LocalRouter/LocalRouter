# Configuration Subscription Guide

This guide explains how the frontend can subscribe to configuration changes and react to real-time updates.

## Overview

The configuration system now supports:
1. **Event emission** - Backend emits `config-changed` events when config changes
2. **File watching** - Automatically detects external changes to `settings.yaml`
3. **Manual reload** - Frontend can force a config reload via command

## Backend Events

### `config-changed` Event

Emitted whenever the configuration changes, either:
- **Programmatically** - Via `ConfigManager::update()` + `save()`
- **External file edit** - User manually edits `settings.yaml`
- **Manual reload** - Via `reload_config` command

**Event payload**: The complete `AppConfig` structure as JSON

```typescript
interface ConfigChangedEvent {
  version: number;
  server: {
    host: string;
    port: number;
    enable_cors: boolean;
  };
  providers: ProviderConfig[];
  routers: RouterConfig[];
  logging: LoggingConfig;
  api_keys: ApiKeyConfig[];
}
```

---

## Frontend Integration

### 1. Subscribe to Config Changes

Use Tauri's event system to listen for config changes:

```typescript
import { listen } from '@tauri-apps/api/event';

// Subscribe to config changes
const unlisten = await listen('config-changed', (event) => {
  const newConfig = event.payload as AppConfig;
  console.log('Configuration changed:', newConfig);

  // Update your UI state
  updateUIWithNewConfig(newConfig);
});

// Later, unsubscribe when component unmounts
unlisten();
```

### 2. In a React Component

```tsx
import { useEffect, useState } from 'react';
import { listen } from '@tauri-apps/api/event';
import { invoke } from '@tauri-apps/api/core';

function ConfigDisplay() {
  const [config, setConfig] = useState<AppConfig | null>(null);

  useEffect(() => {
    // Load initial config
    invoke<AppConfig>('get_config').then(setConfig);

    // Subscribe to changes
    const subscription = listen<AppConfig>('config-changed', (event) => {
      console.log('Config updated!', event.payload);
      setConfig(event.payload);
    });

    // Cleanup subscription on unmount
    return () => {
      subscription.then(unlisten => unlisten());
    };
  }, []);

  if (!config) return <div>Loading...</div>;

  return (
    <div>
      <h2>Server Configuration</h2>
      <p>Host: {config.server.host}</p>
      <p>Port: {config.server.port}</p>

      <h2>Providers</h2>
      <ul>
        {config.providers.map(provider => (
          <li key={provider.name}>
            {provider.name} ({provider.provider_type})
            {provider.enabled ? ' ✓' : ' ✗'}
          </li>
        ))}
      </ul>
    </div>
  );
}
```

### 3. Global State Management (with Context)

```tsx
import { createContext, useContext, useEffect, useState } from 'react';
import { listen } from '@tauri-apps/api/event';
import { invoke } from '@tauri-apps/api/core';

interface ConfigContextType {
  config: AppConfig | null;
  reloadConfig: () => Promise<void>;
}

const ConfigContext = createContext<ConfigContextType | null>(null);

export function ConfigProvider({ children }: { children: React.ReactNode }) {
  const [config, setConfig] = useState<AppConfig | null>(null);

  // Load initial config
  useEffect(() => {
    invoke<AppConfig>('get_config').then(setConfig);
  }, []);

  // Subscribe to config changes
  useEffect(() => {
    const subscription = listen<AppConfig>('config-changed', (event) => {
      console.log('Config changed:', event.payload);
      setConfig(event.payload);
    });

    return () => {
      subscription.then(unlisten => unlisten());
    };
  }, []);

  const reloadConfig = async () => {
    await invoke('reload_config');
    // Event will be emitted automatically, no need to call get_config
  };

  return (
    <ConfigContext.Provider value={{ config, reloadConfig }}>
      {children}
    </ConfigContext.Provider>
  );
}

export function useConfig() {
  const context = useContext(ConfigContext);
  if (!context) {
    throw new Error('useConfig must be used within ConfigProvider');
  }
  return context;
}
```

### 4. Using the Context in Components

```tsx
function ProvidersList() {
  const { config, reloadConfig } = useConfig();

  if (!config) return <div>Loading...</div>;

  return (
    <div>
      <button onClick={reloadConfig}>Reload Config</button>

      <ul>
        {config.providers.map(provider => (
          <li key={provider.name}>{provider.name}</li>
        ))}
      </ul>
    </div>
  );
}
```

---

## Tauri Commands

### `get_config`

Get the current configuration snapshot.

```typescript
import { invoke } from '@tauri-apps/api/core';

const config = await invoke<AppConfig>('get_config');
```

**Returns**: Full `AppConfig` object

**Use when**:
- Initial load
- Need immediate config value
- Not subscribing to changes

---

### `reload_config`

Manually reload configuration from disk.

```typescript
import { invoke } from '@tauri-apps/api/core';

await invoke('reload_config');
// Emits 'config-changed' event automatically
```

**Returns**: `void` (success) or throws error

**Use when**:
- User clicks a "Refresh" button
- After external file changes (though file watcher handles this)
- Need to force a reload

---

## File Watching Behavior

The backend automatically watches `settings.yaml` using native OS file system events (not polling).

### When File Changes

1. **File is saved** (external editor, script, etc.)
2. **Watcher detects change** (instant, no polling)
3. **Config reloads** from disk
4. **Validation runs** (ensures config is valid)
5. **Event emits** to all frontend listeners
6. **UI updates** automatically (if subscribed)

### Supported Platforms

- ✅ **macOS** - FSEvents
- ✅ **Linux** - inotify
- ✅ **Windows** - ReadDirectoryChangesW

All use native OS APIs for efficient, real-time monitoring.

### What Triggers Events

| Action | Event Emitted? | Notes |
|--------|---------------|-------|
| User edits `settings.yaml` externally | ✅ Yes | Via file watcher |
| Call `reload_config` command | ✅ Yes | Manual reload |
| Call `ConfigManager::update()` in backend | ✅ Yes | Programmatic update |
| Call `ConfigManager::save()` in backend | ⚠️ Via watcher | Save writes to disk, watcher picks it up |

---

## Example: Complete Implementation

```tsx
// App.tsx
import { ConfigProvider } from './contexts/ConfigContext';
import { ProvidersPage } from './pages/ProvidersPage';

function App() {
  return (
    <ConfigProvider>
      <ProvidersPage />
    </ConfigProvider>
  );
}

// pages/ProvidersPage.tsx
import { useConfig } from '../contexts/ConfigContext';

function ProvidersPage() {
  const { config, reloadConfig } = useConfig();

  if (!config) {
    return <div>Loading configuration...</div>;
  }

  return (
    <div>
      <h1>Providers</h1>

      <button onClick={reloadConfig}>
        Refresh Configuration
      </button>

      <div className="providers-list">
        {config.providers.map(provider => (
          <div key={provider.name} className="provider-card">
            <h3>{provider.name}</h3>
            <p>Type: {provider.provider_type}</p>
            <p>Status: {provider.enabled ? 'Enabled' : 'Disabled'}</p>

            {provider.provider_config && (
              <pre>
                {JSON.stringify(provider.provider_config, null, 2)}
              </pre>
            )}
          </div>
        ))}
      </div>
    </div>
  );
}
```

---

## Testing

### Test File Watching

1. **Start the app**
2. **Open** `~/Library/Application Support/LocalRouter/settings.yaml` (macOS)
3. **Edit** a provider's `enabled` field
4. **Save** the file
5. **UI should update instantly** (within ~100ms)

### Test Manual Reload

```typescript
// In browser console
await window.__TAURI__.invoke('reload_config');
// Should see config-changed event fire
```

### Test Programmatic Updates

```typescript
// When you update config via backend commands
await window.__TAURI__.invoke('set_provider_api_key', {
  provider: 'openai',
  api_key: 'sk-...'
});
// config-changed event should fire if ConfigManager.update() + save() is called
```

---

## Performance Considerations

1. **No polling** - Uses OS-native file watching (zero CPU overhead when idle)
2. **Debouncing** - File watcher naturally debounces rapid changes
3. **Efficient** - Only sends full config on actual changes
4. **Selective updates** - Frontend can choose which parts of config to watch

---

## Error Handling

```typescript
import { listen } from '@tauri-apps/api/event';
import { invoke } from '@tauri-apps/api/core';

// Handle config load errors
try {
  const config = await invoke<AppConfig>('get_config');
  setConfig(config);
} catch (error) {
  console.error('Failed to load config:', error);
  showErrorNotification('Could not load configuration');
}

// Handle reload errors
try {
  await invoke('reload_config');
} catch (error) {
  console.error('Failed to reload config:', error);
  showErrorNotification('Invalid configuration file');
}

// Listen for config changes (events don't throw errors)
await listen<AppConfig>('config-changed', (event) => {
  try {
    const newConfig = event.payload;
    validateConfig(newConfig); // Your validation
    setConfig(newConfig);
  } catch (error) {
    console.error('Invalid config received:', error);
  }
});
```

---

## Debugging

Enable debug logging to see config events:

```bash
# Set environment variable
RUST_LOG=localrouter_ai=debug cargo tauri dev
```

You'll see logs like:
```
[INFO] Started watching configuration file: ~/Library/Application Support/LocalRouter/settings.yaml
[INFO] Configuration file changed, reloading...
[INFO] Configuration reloaded successfully
[DEBUG] Emitted config-changed event to frontend
```

---

## Summary

**For the UI developer:**

1. ✅ **Use `listen('config-changed', ...)`** to subscribe to config updates
2. ✅ **Use `invoke('get_config')`** for initial load
3. ✅ **Use `invoke('reload_config')`** to force reload
4. ✅ **File watching is automatic** - no polling needed
5. ✅ **Events fire on all changes** - programmatic, file edits, manual reload

The system is fully reactive - just subscribe once and your UI will stay in sync!
