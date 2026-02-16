# Website Demo Implementation Plan

Create an interactive demo of the LocalRouter app embedded in the website by **reusing the actual Tauri frontend code** with mocked commands.

## Architecture Overview

```
Code Reuse Strategy:
├── Main App (/src)              # Actual Tauri frontend components
│   ├── App.tsx                  # Main app shell
│   ├── components/              # UI components (Sidebar, layout, etc.)
│   └── views/                   # Dashboard, Clients, etc.
│
└── Website (/website/src)
    ├── components/demo/
    │   ├── LocalRouterDemo.tsx  # Demo container with macOS chrome
    │   ├── MacOSMenuBar.tsx     # Menu bar with tray icon
    │   ├── MacOSTrayMenu.tsx    # System tray menu
    │   ├── MacOSWindow.tsx      # Window chrome
    │   ├── TauriMockSetup.ts    # Initialize @tauri-apps/api/mocks
    │   └── mockData.ts          # Mock command responses
    └── pages/Home.tsx           # Integrate demo
```

---

## Maintainability & Future-Proofing

### 1. Unimplemented Command Handling

The mock setup should **warn users** when a command hasn't been implemented:

```ts
// website/src/components/demo/TauriMockSetup.ts

import { toast } from 'sonner'

// Track which commands we've warned about (only warn once per command)
const warnedCommands = new Set<string>()

export function setupTauriMocks() {
  mockIPC((cmd: string, args?: Record<string, unknown>) => {
    // Check if this command has a mock implementation
    if (!(cmd in mockHandlers)) {
      // Only show warning once per command
      if (!warnedCommands.has(cmd)) {
        warnedCommands.add(cmd)
        toast.info(`Demo mode: "${cmd}" is not implemented`, {
          description: 'This is a demo with limited functionality',
          duration: 3000,
        })
        console.warn(`[Demo Mock] Unimplemented command: ${cmd}`, args)
      }
      return null
    }

    return mockHandlers[cmd](args)
  }, { shouldMockEvents: true })
}

// Explicit map of implemented handlers
const mockHandlers: Record<string, (args?: any) => any> = {
  'list_clients': () => mockData.clients,
  'list_provider_instances': () => mockData.providers,
  'list_mcp_servers': () => mockData.mcpServers,
  'list_strategies': () => mockData.strategies,
  'get_setup_wizard_shown': () => true,
  // ... other implemented commands
}
```

### 2. Rust Code Comments for Tray Menu Sync

Add a comment block at the top of `tray_menu.rs` to remind developers:

```rust
// src-tauri/src/ui/tray_menu.rs

//! Tray menu building and event handlers
//!
//! ⚠️ WEBSITE DEMO SYNC REQUIRED
//! ============================
//! The tray menu structure is replicated in the website demo at:
//!   website/src/components/demo/MacOSTrayMenu.tsx
//!
//! When modifying the menu structure (adding/removing items, changing
//! labels, or restructuring), please also update the website demo
//! to keep them in sync.
//!
//! Key sync points:
//! - Menu item labels and icons (TRAY_INDENT, ICON_PAD patterns)
//! - Menu structure (headers, separators, submenus)
//! - Client submenu structure (Copy ID, strategies, MCP, skills)
//! - Event IDs (for any interactive demo functionality)
```

### 3. Command Registry Documentation

Add a comment block in the commands file:

```rust
// src-tauri/src/ui/commands.rs (or wherever commands are defined)

//! ⚠️ WEBSITE DEMO SYNC REQUIRED
//! ============================
//! Commands used by the frontend are mocked in the website demo at:
//!   website/src/components/demo/TauriMockSetup.ts
//!   website/src/components/demo/mockData.ts
//!
//! When adding new commands or changing command signatures:
//! 1. Add a mock handler in TauriMockSetup.ts
//! 2. Add mock data in mockData.ts if needed
//! 3. If no mock is added, a toast will warn users in demo mode
//!
//! Currently mocked commands:
//! - list_clients, list_provider_instances, list_mcp_servers
//! - list_strategies, list_all_models, list_oauth_clients
//! - get_aggregate_stats, get_health_cache, get_server_config
//! - get_setup_wizard_shown, list_skills
```

### 4. Demo Mode Indicator

Show a persistent banner in the demo that this is not the real app:

```tsx
// website/src/components/demo/DemoBanner.tsx

export function DemoBanner() {
  return (
    <div className="absolute bottom-4 left-1/2 -translate-x-1/2 z-50">
      <div className="px-4 py-2 rounded-full bg-amber-100 border border-amber-300 text-amber-800 text-sm font-medium shadow-lg">
        🎭 Interactive Demo — Not connected to real backend
      </div>
    </div>
  )
}
```

### 5. Type Safety for Mock Data

Generate or validate mock data matches actual types:

```ts
// website/src/components/demo/mockData.ts

// Import types from the main app to ensure type safety
import type { Client, ProviderInstance, McpServer, Strategy } from '@app/types'

// This ensures mock data stays in sync with actual types
// TypeScript will error if the shape changes
export const mockData: {
  clients: Client[]
  providers: ProviderInstance[]
  mcpServers: McpServer[]
  strategies: Strategy[]
  // ...
} = {
  clients: [
    // TypeScript ensures these match the Client interface
  ],
  // ...
}
```

### 6. CI/CD Validation (Optional)

Add a build-time check that warns if new commands are detected:

```ts
// scripts/check-demo-mocks.ts (run during website build)

import { execSync } from 'child_process'

// Grep for invoke() calls in main app
const invokePattern = /invoke<[^>]*>\(['"]([^'"]+)['"]/g
const usedCommands = new Set<string>()

// ... scan /src for all invoke() calls ...

// Check against mock handlers
import { mockHandlers } from '../website/src/components/demo/TauriMockSetup'

const unmockedCommands = [...usedCommands].filter(cmd => !(cmd in mockHandlers))

if (unmockedCommands.length > 0) {
  console.warn('⚠️  Commands used in app but not mocked in demo:')
  unmockedCommands.forEach(cmd => console.warn(`   - ${cmd}`))
  // Could exit(1) to fail build, or just warn
}
```

---

## Step 1: Configure Vite for Code Sharing

Update `website/vite.config.ts` to resolve the main app's `/src`:

```ts
import { defineConfig } from 'vite'
import react from '@vitejs/plugin-react'
import path from 'path'

export default defineConfig({
  plugins: [react()],
  base: '/',
  resolve: {
    alias: {
      "@": path.resolve(__dirname, "./src"),           // Website src
      "@app": path.resolve(__dirname, "../src"),       // Main Tauri app src
      // Stub Tauri plugins
      "@tauri-apps/plugin-dialog": path.resolve(__dirname, "./src/stubs/tauri-plugin-dialog.ts"),
      "@tauri-apps/plugin-shell": path.resolve(__dirname, "./src/stubs/tauri-plugin-shell.ts"),
      "@tauri-apps/plugin-updater": path.resolve(__dirname, "./src/stubs/tauri-plugin-updater.ts"),
      "@tauri-apps/plugin-process": path.resolve(__dirname, "./src/stubs/tauri-plugin-process.ts"),
    },
  },
  optimizeDeps: {
    include: ['@tauri-apps/api'],
  },
})
```

## Step 2: Add Missing Dependencies to Website

Add to `website/package.json` dependencies:

```json
{
  "dependencies": {
    // Existing...

    // Add for Tauri mocking
    "@tauri-apps/api": "^2.9.1",

    // Add Radix components used by main app
    "@radix-ui/react-alert-dialog": "^1.1.15",
    "@radix-ui/react-checkbox": "^1.3.3",
    "@radix-ui/react-collapsible": "^1.1.12",
    "@radix-ui/react-dialog": "^1.1.15",
    "@radix-ui/react-dropdown-menu": "^2.1.16",
    "@radix-ui/react-hover-card": "^1.1.15",
    "@radix-ui/react-label": "^2.1.8",
    "@radix-ui/react-popover": "^1.1.15",
    "@radix-ui/react-progress": "^1.1.8",
    "@radix-ui/react-radio-group": "^1.3.8",
    "@radix-ui/react-scroll-area": "^1.2.10",
    "@radix-ui/react-select": "^2.2.6",
    "@radix-ui/react-separator": "^1.1.8",
    "@radix-ui/react-slider": "^1.3.6",
    "@radix-ui/react-switch": "^1.2.6",
    "@radix-ui/react-tabs": "^1.1.13",
    "@radix-ui/react-tooltip": "^1.2.8",

    // Other deps used by main app components
    "sonner": "^2.0.7",
    "recharts": "^3.6.0",
    "react-resizable-panels": "^4.4.1",
    "dagre": "^0.8.5",
    "@types/dagre": "^0.7.53",
    "react-markdown": "^10.1.0",
    "remark-gfm": "^4.0.1",
    "@tanstack/react-table": "^8.21.3",
    "cmdk": "^1.1.1",
    "@dnd-kit/core": "^6.3.1",
    "@dnd-kit/sortable": "^10.0.0",
    "@dnd-kit/utilities": "^3.2.2",
    "tailwindcss-animate": "^1.0.7"
  }
}
```

## Step 3: Tauri Mock Setup with Warning Toast

Create `website/src/components/demo/TauriMockSetup.ts`:

```ts
import { mockIPC, mockWindows, clearMocks } from '@tauri-apps/api/mocks'
import { toast } from 'sonner'
import { mockData } from './mockData'

// Track warned commands to avoid spam
const warnedCommands = new Set<string>()

// Explicit map of implemented mock handlers
const mockHandlers: Record<string, (args?: any) => any> = {
  // Setup wizard
  'get_setup_wizard_shown': () => true,
  'set_setup_wizard_shown': () => null,

  // Clients
  'list_clients': () => mockData.clients,
  'create_client': () => mockData.clients[0],
  'update_client_name': () => null,
  'toggle_client_enabled': () => null,
  'delete_client': () => null,

  // Providers
  'list_provider_instances': () => mockData.providers,

  // MCP Servers
  'list_mcp_servers': () => mockData.mcpServers,
  'start_mcp_health_checks': () => null,
  'check_single_mcp_health': () => ({ status: 'healthy' }),

  // Strategies
  'list_strategies': () => mockData.strategies,

  // Models
  'list_all_models': () => mockData.models,

  // Stats
  'get_aggregate_stats': () => mockData.stats,

  // Health
  'get_health_cache': () => mockData.healthCache,

  // Server config
  'get_server_config': () => mockData.serverConfig,

  // OAuth clients
  'list_oauth_clients': () => [],

  // Skills
  'list_skills': () => mockData.skills,

  // Firewall
  'get_firewall_approval_details': () => null,
  'submit_firewall_approval': () => null,
}

export function setupTauriMocks() {
  clearMocks()
  mockWindows('main')

  mockIPC((cmd: string, args?: Record<string, unknown>) => {
    console.log('[Demo Mock]', cmd, args)

    // Check if this command has a mock implementation
    if (!(cmd in mockHandlers)) {
      if (!warnedCommands.has(cmd)) {
        warnedCommands.add(cmd)
        toast.info(`Demo: "${cmd}" not implemented`, {
          description: 'This feature is not available in demo mode',
          duration: 4000,
        })
        console.warn(`[Demo Mock] ⚠️ Unimplemented command: ${cmd}`, args)
      }
      return null
    }

    return mockHandlers[cmd](args)
  }, { shouldMockEvents: true })
}

export function stubTauriPlugins() {
  if (typeof window !== 'undefined') {
    ;(window as any).__TAURI_INTERNALS__ = {
      metadata: {
        currentWebview: { label: 'demo-main' }
      }
    }
  }
}

// Export for validation/testing
export { mockHandlers }
```

## Step 4: Mock Data with Type Imports

Create `website/src/components/demo/mockData.ts`:

```ts
// Import types from main app for type safety
// If the types change, TypeScript will catch it here

export const mockData = {
  clients: [
    {
      id: "1",
      client_id: "cursor-client",
      name: "Cursor",
      enabled: true,
      strategy_id: "default",
      mcp_deferred_loading: false,
      created_at: "2025-01-15T10:00:00Z",
      last_used: "2025-02-03T14:30:00Z",
      mcp_permissions: { default: "allow" as const, servers: {} },
      skills_permissions: { default: "allow" as const, skills: {} },
      model_permissions: { default: "allow" as const, models: {} },
      marketplace_permission: "allow" as const,
    },
    {
      id: "2",
      client_id: "claude-code",
      name: "Claude Code",
      enabled: true,
      strategy_id: "default",
      mcp_deferred_loading: true,
      created_at: "2025-01-20T08:00:00Z",
      last_used: "2025-02-03T15:45:00Z",
      mcp_permissions: { default: "ask" as const, servers: { "github-mcp": "allow" } },
      skills_permissions: { default: "allow" as const, skills: {} },
      model_permissions: { default: "allow" as const, models: {} },
      marketplace_permission: "allow" as const,
    },
    {
      id: "3",
      client_id: "open-webui",
      name: "Open WebUI",
      enabled: false,
      strategy_id: "fast",
      mcp_deferred_loading: false,
      created_at: "2025-01-25T12:00:00Z",
      last_used: null,
      mcp_permissions: { default: "deny" as const, servers: {} },
      skills_permissions: { default: "deny" as const, skills: {} },
      model_permissions: { default: "allow" as const, models: {} },
      marketplace_permission: "deny" as const,
    },
  ],

  providers: [
    { instance_name: "openai-main", provider_type: "openai", enabled: true },
    { instance_name: "anthropic", provider_type: "anthropic", enabled: true },
    { instance_name: "ollama-local", provider_type: "ollama", enabled: true },
    { instance_name: "gemini", provider_type: "gemini", enabled: false },
  ],

  mcpServers: [
    { id: "github-mcp", name: "GitHub", enabled: true },
    { id: "filesystem", name: "Filesystem", enabled: true },
    { id: "slack", name: "Slack", enabled: false },
  ],

  strategies: [
    { id: "default", name: "Default Strategy", parent: null },
    { id: "fast", name: "Fast & Cheap", parent: null },
    { id: "quality", name: "High Quality", parent: null },
  ],

  models: [
    { id: "gpt-4o", provider: "openai" },
    { id: "gpt-4o-mini", provider: "openai" },
    { id: "claude-3-5-sonnet", provider: "anthropic" },
    { id: "llama3.2:latest", provider: "ollama" },
  ],

  stats: {
    total_requests: 15847,
    total_tokens: 2458923,
    total_cost: 127.45,
    successful_requests: 15782,
    failed_requests: 65,
  },

  healthCache: {
    aggregate_status: "green",
    providers: {
      "openai-main": { status: "healthy", name: "openai-main" },
      "anthropic": { status: "healthy", name: "anthropic" },
      "ollama-local": { status: "healthy", name: "ollama-local" },
    },
    mcp_servers: {
      "github-mcp": { status: "healthy", name: "GitHub" },
      "filesystem": { status: "healthy", name: "Filesystem" },
    },
  },

  serverConfig: {
    host: "127.0.0.1",
    port: 3625,
  },

  skills: [
    { id: "web-search", name: "Web Search", enabled: true },
    { id: "code-interpreter", name: "Code Interpreter", enabled: true },
  ],
}
```

## Step 5: macOS Window Components

### MacOSWindow.tsx

```tsx
interface MacOSWindowProps {
  title: string
  children: React.ReactNode
  width?: number
  height?: number
}

export function MacOSWindow({ title, children, width = 1000, height = 600 }: MacOSWindowProps) {
  return (
    <div
      className="rounded-lg overflow-hidden shadow-2xl border border-gray-300/50"
      style={{ width, maxWidth: '100%' }}
    >
      {/* Title bar */}
      <div className="h-7 bg-gradient-to-b from-[#e8e8e8] to-[#d3d3d3] flex items-center px-3 border-b border-gray-400/30">
        <div className="flex gap-2">
          <span className="w-3 h-3 rounded-full bg-[#ff5f57] border border-[#e14640]" />
          <span className="w-3 h-3 rounded-full bg-[#febc2e] border border-[#d4a029]" />
          <span className="w-3 h-3 rounded-full bg-[#28c840] border border-[#24a732]" />
        </div>
        <span className="flex-1 text-center text-[13px] font-medium text-gray-600">{title}</span>
        <div className="w-14" />
      </div>

      {/* Content */}
      <div className="bg-white overflow-hidden" style={{ height }}>
        {children}
      </div>
    </div>
  )
}
```

### MacOSMenuBar.tsx

```tsx
interface MacOSMenuBarProps {
  onTrayClick: () => void
  trayOpen: boolean
}

export function MacOSMenuBar({ onTrayClick, trayOpen }: MacOSMenuBarProps) {
  return (
    <div className="h-6 bg-gradient-to-b from-[#f6f6f6] to-[#e8e8e8] border-b border-gray-300 flex items-center justify-between px-4 text-[13px] font-medium text-gray-800">
      <div className="flex items-center gap-5">
        <span className="font-bold"></span>
        <span>LocalRouter</span>
        <span className="text-gray-500">File</span>
        <span className="text-gray-500">Edit</span>
        <span className="text-gray-500">View</span>
        <span className="text-gray-500">Window</span>
        <span className="text-gray-500">Help</span>
      </div>
      <div className="flex items-center gap-3">
        <span className="text-gray-500 text-sm">🔋</span>
        <span className="text-gray-500 text-sm">📶</span>
        <button
          onClick={onTrayClick}
          className={`p-1 rounded ${trayOpen ? 'bg-blue-500 text-white' : 'hover:bg-gray-300/50'}`}
        >
          <TrayIcon className="w-4 h-4" />
        </button>
        <span className="text-gray-500 text-sm">10:30 AM</span>
      </div>
    </div>
  )
}
```

### MacOSTrayMenu.tsx

```tsx
// ⚠️ SYNC WITH: src-tauri/src/ui/tray_menu.rs
// When tray_menu.rs changes, update this component to match

// Unicode spacing from tray_menu.rs (keep in sync!)
const TRAY_INDENT = '\u{2003}\u{2009}\u{2009}'
const ICON_PAD = '\u{2009}\u{2009}'

export function MacOSTrayMenu({ onClose }: { onClose: () => void }) {
  const [openSubmenu, setOpenSubmenu] = useState<string | null>(null)

  return (
    <div className="absolute right-4 top-7 z-50 w-64 rounded-md bg-gray-100/95 backdrop-blur-xl shadow-xl border border-gray-300/50 py-1 text-[13px]">
      {/* Header - sync with tray_menu.rs line 52-57 */}
      <div className="px-3 py-1 text-gray-400 cursor-default">
        LocalRouter on 127.0.0.1:3625
      </div>

      <Separator />

      {/* Settings - sync with tray_menu.rs line 62-65 */}
      <MenuItem icon="⌘" label="Settings..." />
      {/* Copy URL - sync with tray_menu.rs line 68 */}
      <MenuItem icon="⧉" label="Copy URL" />

      <Separator />

      {/* Clients header - sync with tray_menu.rs line 189 */}
      <div className="px-3 py-1 text-gray-400 cursor-default">Clients</div>

      {/* Client submenus - sync with tray_menu.rs line 193-199 */}
      {mockData.clients.map(client => (
        <SubmenuItem
          key={client.id}
          label={client.name}
          isOpen={openSubmenu === client.id}
          onOpen={() => setOpenSubmenu(client.id)}
        >
          <ClientSubmenu client={client} />
        </SubmenuItem>
      ))}

      <Separator />

      {/* Add client - keep in sync with tray_menu.rs */}
      <MenuItem icon="＋" label="Add && Copy Key" />

      <Separator />

      <MenuItem icon="⏻" label="Quit" />
    </div>
  )
}
```

## Step 6: Demo Container with Banner

Create `website/src/components/demo/LocalRouterDemo.tsx`:

```tsx
import { useEffect, useState } from 'react'
import { Toaster } from 'sonner'
import { setupTauriMocks, stubTauriPlugins } from './TauriMockSetup'
import { MacOSMenuBar } from './MacOSMenuBar'
import { MacOSTrayMenu } from './MacOSTrayMenu'
import { MacOSWindow } from './MacOSWindow'
import { DemoBanner } from './DemoBanner'

// Import the actual App component from main Tauri app
import App from '@app/App'

export function LocalRouterDemo() {
  const [ready, setReady] = useState(false)
  const [trayOpen, setTrayOpen] = useState(false)

  useEffect(() => {
    // Initialize mocks before rendering the app
    stubTauriPlugins()
    setupTauriMocks()
    setReady(true)
  }, [])

  if (!ready) {
    return <div className="h-[700px] flex items-center justify-center">Loading demo...</div>
  }

  return (
    <div className="relative mx-auto max-w-5xl">
      {/* Toast container for mock warnings */}
      <Toaster position="bottom-right" />

      {/* macOS Menu Bar */}
      <div className="rounded-t-lg overflow-hidden border border-b-0 border-gray-300/50">
        <MacOSMenuBar
          onTrayClick={() => setTrayOpen(!trayOpen)}
          trayOpen={trayOpen}
        />
      </div>

      {/* Tray Menu Dropdown */}
      {trayOpen && (
        <MacOSTrayMenu onClose={() => setTrayOpen(false)} />
      )}

      {/* App Window */}
      <MacOSWindow title="LocalRouter" height={600}>
        <App />
      </MacOSWindow>

      {/* Demo mode indicator */}
      <DemoBanner />
    </div>
  )
}
```

## Step 7: Plugin Stubs

Create stub files in `website/src/stubs/`:

**tauri-plugin-dialog.ts:**
```ts
export const open = async () => null
export const save = async () => null
```

**tauri-plugin-shell.ts:**
```ts
export const open = async (url: string) => {
  window.open(url, '_blank')
}
```

**tauri-plugin-updater.ts:**
```ts
export const check = async () => null
export class Update {}
```

**tauri-plugin-process.ts:**
```ts
export const relaunch = async () => {}
```

## Step 8: Rust Code Comments

Add sync reminder comments to these files:

**src-tauri/src/ui/tray_menu.rs** (top of file):
```rust
//! Tray menu building and event handlers
//!
//! ⚠️ WEBSITE DEMO SYNC REQUIRED
//! =============================
//! The tray menu structure is replicated in the website demo:
//!   website/src/components/demo/MacOSTrayMenu.tsx
//!
//! When modifying menu structure, labels, or icons, please update
//! the website demo component to match.
//!
//! Key sync points:
//! - TRAY_INDENT and ICON_PAD constants
//! - Menu item order and labels
//! - Submenu structure for clients
//! - Header text format ("LocalRouter on {host}:{port}")
```

**src-tauri/src/ui/commands.rs** (or main commands file):
```rust
//! ⚠️ WEBSITE DEMO SYNC REQUIRED
//! =============================
//! Frontend commands are mocked in the website demo:
//!   website/src/components/demo/TauriMockSetup.ts
//!   website/src/components/demo/mockData.ts
//!
//! When adding new commands:
//! 1. Add mock handler in TauriMockSetup.ts
//! 2. Add mock data in mockData.ts if needed
//! 3. Unmocked commands will show a toast warning in demo
```

## Files Summary

### Create

| File | Purpose |
|------|---------|
| `website/src/components/demo/LocalRouterDemo.tsx` | Demo container |
| `website/src/components/demo/MacOSMenuBar.tsx` | Menu bar |
| `website/src/components/demo/MacOSTrayMenu.tsx` | Tray dropdown |
| `website/src/components/demo/MacOSWindow.tsx` | Window chrome |
| `website/src/components/demo/DemoBanner.tsx` | Demo mode indicator |
| `website/src/components/demo/TauriMockSetup.ts` | Mock init + warning toasts |
| `website/src/components/demo/mockData.ts` | Mock responses |
| `website/src/components/demo/index.ts` | Exports |
| `website/src/stubs/tauri-plugin-*.ts` | Plugin stubs (4 files) |

### Modify

| File | Change |
|------|--------|
| `website/vite.config.ts` | Add `@app` alias and plugin stubs |
| `website/package.json` | Add missing dependencies |
| `website/src/pages/Home.tsx` | Import and render demo |
| `website/tailwind.config.js` | Extend content to include `../src/**` |
| `src-tauri/src/ui/tray_menu.rs` | Add sync reminder comment |
| `src-tauri/src/ui/commands.rs` | Add sync reminder comment |

## Verification

1. `cd website && npm install && npm run dev`
2. Navigate to homepage
3. Verify demo renders below hero
4. Test that actual App component renders with mock data
5. Try clicking a feature that's not mocked - verify toast appears
6. Test tray menu structure matches Rust code
7. Verify no console errors for missing Tauri internals
