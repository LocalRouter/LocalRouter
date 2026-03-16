/**
 * Tauri Mock Setup for Website Demo
 *
 * This module sets up mock IPC handlers so the actual Tauri frontend
 * components can run in the website demo without a real backend.
 *
 * !! SYNC REQUIRED - UPDATE WHEN MODIFYING TAURI COMMANDS !!
 *
 * When you add or modify a Tauri command:
 * 1. Add/update the mock handler in mockHandlers below
 * 2. Import the return type from @app/types/tauri-commands
 * 3. Add explicit return type annotation: `'cmd': (args): MyType => ...`
 * 4. Update mockData.ts if the mock needs persistent state
 *
 * Return types must match: src/types/tauri-commands.ts
 * See CLAUDE.md "Adding/Modifying Tauri Commands" for full checklist.
 */

import { mockIPC, mockWindows, clearMocks } from '@tauri-apps/api/mocks'
import { emit } from '@tauri-apps/api/event'
import type { InvokeArgs } from '@tauri-apps/api/core'
import { toast } from 'sonner'
import { mockData } from './mockData'
// Types for mock return values - see src/types/tauri-commands.ts for full type definitions
import type { RouteLLMTestResult, GraphData, ProviderFeatureSupport, FeatureEndpointMatrix } from '@app/types/tauri-commands'

// Track warned commands to avoid spam (only warn once per command)
const warnedCommands = new Set<string>()

// Helper to generate a random ID
const generateId = () => Math.random().toString(36).substring(2, 15)

// Helper to emit mock provider health events
function emitProviderHealthEvents() {
  // Emit health events for each provider after a short delay
  mockData.providers.forEach((provider, index) => {
    setTimeout(() => {
      const healthCache = mockData.healthCache.providers as Record<string, { status: string; latency_ms: number | null }>
      const health = healthCache[provider.instance_name]
      emit('provider-health-check', {
        provider_name: provider.instance_name,
        status: provider.enabled ? (health?.status || 'healthy') : 'disabled',
        latency_ms: health?.latency_ms || Math.floor(Math.random() * 300) + 100,
        error_message: null,
      })
    }, 100 + index * 50) // Stagger the events
  })
}

// Helper to emit mock MCP server health events
function emitMcpHealthEvents() {
  // Emit health events for each MCP server after a short delay
  mockData.mcpServers.forEach((server, index) => {
    setTimeout(() => {
      const healthCache = mockData.healthCache.mcp_servers as Record<string, { status: string; latency_ms: number | null }>
      const health = healthCache[server.id]
      emit('mcp-health-check', {
        server_id: server.id,
        status: server.enabled ? (health?.status || 'healthy') : 'disabled',
        latency_ms: health?.latency_ms || Math.floor(Math.random() * 200) + 50,
        error: null,
      })
    }, 100 + index * 50) // Stagger the events
  })
}

// Helper to generate mock graph data for metrics
// Returns: GraphData (src/types/tauri-commands.ts)
function generateMockGraphData(datasetLabel = "Requests", baseValue = 200, variance = 150): GraphData {
  const now = new Date()
  const labels: string[] = []
  const data: number[] = []

  // Generate 24 hourly data points with realistic daily patterns
  for (let i = 23; i >= 0; i--) {
    const time = new Date(now.getTime() - i * 60 * 60 * 1000)
    const hour = time.getHours()
    labels.push(time.toISOString())

    // Simulate realistic usage pattern (lower at night, higher during day)
    const timeMultiplier = hour >= 9 && hour <= 18 ? 1.5 : (hour >= 6 && hour <= 21 ? 1.0 : 0.3)
    const noise = (Math.random() - 0.5) * variance
    const trendComponent = Math.sin(i / 4) * 30 // Add some wave pattern
    const value = Math.max(10, Math.floor(baseValue * timeMultiplier + noise + trendComponent))
    data.push(value)
  }

  return {
    labels,
    datasets: [{
      label: datasetLabel,
      data,
      border_color: "#3b82f6",
      background_color: "#3b82f6",
    }],
  }
}

/**
 * Mock handlers for Tauri commands.
 *
 * Return types should match the TypeScript types defined in:
 * src/types/tauri-commands.ts
 *
 * When adding/modifying handlers, ensure the return value structure
 * matches the corresponding type (e.g., RouteLLMTestResult, ClientInfo, etc.)
 */
// eslint-disable-next-line @typescript-eslint/no-explicit-any
const mockHandlers: Record<string, (args?: any) => unknown> = {
  // ============================================================================
  // Setup & Configuration
  // ============================================================================
  'get_setup_wizard_shown': () => true, // Skip wizard in demo
  'set_setup_wizard_shown': () => null,
  'get_home_dir': () => mockData.homeDir,
  'get_config_dir': () => mockData.configDir,
  'get_config': () => ({
    providers: [
      { name: 'openai', api_key: 'sk-demo-fake-key-1234', models: ['gpt-4o', 'gpt-4o-mini'] },
      { name: 'anthropic', api_key: 'sk-ant-demo-fake-key-5678', models: ['claude-sonnet-4-20250514'] },
    ],
    server: { host: '127.0.0.1', port: 3625 },
  }),

  // ============================================================================
  // Server Configuration
  // ============================================================================
  'get_server_config': () => mockData.serverConfig,
  'update_server_config': () => mockData.serverConfig,
  'restart_server': () => {
    toast.success('Server restarted (demo)')
    return null
  },
  'get_network_interfaces': () => mockData.networkInterfaces,

  // ============================================================================
  // Clients
  // ============================================================================
  'list_clients': () => mockData.clients,
  'get_feature_clients_status': () => mockData.clients.map((c: { client_id: string; name: string }) => ({
    client_id: c.client_id,
    client_name: c.name,
    active: true,
    source: 'global' as const,
  })),
  'get_client': (args) => mockData.clients.find(c => c.id === args?.id || c.client_id === args?.clientId),
  'create_client': (args) => {
    const newClient = {
      id: `client-${generateId()}`,
      client_id: args?.name?.toLowerCase().replace(/\s+/g, '-') || generateId(),
      name: args?.name || 'New Client',
      enabled: true,
      strategy_id: 'strategy-default',
      context_management_enabled: null,
      created_at: new Date().toISOString(),
      last_used: null,
      mcp_permissions: { global: 'ask' as const, servers: {}, tools: {}, resources: {}, prompts: {} },
      skills_permissions: { global: 'ask' as const, skills: {}, tools: {} },
      coding_agent_permission: 'ask' as const,
      coding_agent_type: null,
      model_permissions: { global: 'allow' as const, providers: {}, models: {} },
      marketplace_permission: 'ask' as const,
      mcp_sampling_permission: 'ask' as const,
      mcp_elicitation_permission: 'ask' as const,
      client_mode: 'both' as const,
      template_id: null,
      sync_config: false,
      guardrails_active: false,
    }
    mockData.clients.push(newClient)
    toast.success(`Client "${args?.name}" created (demo)`)
    // Emit clients-changed event to trigger UI refresh
    setTimeout(() => emit('clients-changed', {}), 10)
    // Return tuple [secret, clientInfo] as expected by the wizard
    const secret = `lr_demo_${generateId()}_${generateId()}`
    return [secret, newClient]
  },
  'update_client_name': (args) => {
    const client = mockData.clients.find(c => c.client_id === args?.clientId || c.id === args?.id)
    if (client) {
      client.name = args?.name
      setTimeout(() => emit('clients-changed', {}), 10)
    }
    return null
  },
  'toggle_client_enabled': (args) => {
    const client = mockData.clients.find(c => c.client_id === args?.clientId || c.id === args?.id)
    if (client) {
      client.enabled = args?.enabled !== undefined ? args.enabled : !client.enabled
      setTimeout(() => emit('clients-changed', {}), 10)
    }
    return null
  },
  'toggle_client_context_management': (args) => {
    const client = mockData.clients.find(c => c.client_id === args?.clientId || c.id === args?.id)
    if (client) {
      client.context_management_enabled = args?.enabled ?? null
      setTimeout(() => emit('clients-changed', {}), 10)
    }
    return null
  },
  'toggle_client_catalog_compression': (args) => {
    const client = mockData.clients.find(c => c.client_id === args?.clientId || c.id === args?.id)
    if (client) {
      (client as any).catalog_compression_enabled = args?.enabled ?? null
      setTimeout(() => emit('clients-changed', {}), 10)
    }
    return null
  },
  'get_client_effective_config': (args) => {
    const client = mockData.clients.find(c => c.client_id === args?.clientId || c.id === args?.clientId)
    const strategy = client ? mockData.strategies.find(s => s.id === client.strategy_id) : null
    return {
      strategy_name: strategy?.name || 'Unknown',
      context_management_effective: client?.context_management_enabled ?? false,
      context_management_source: client?.context_management_enabled !== null && client?.context_management_enabled !== undefined ? 'client' : 'global',
      catalog_compression_effective: true,
      catalog_compression_source: 'global' as const,
    }
  },
  'delete_client': (args) => {
    const idx = mockData.clients.findIndex(c => c.client_id === args?.clientId || c.id === args?.id)
    if (idx !== -1) {
      mockData.clients.splice(idx, 1)
      toast.success('Client deleted (demo)')
      setTimeout(() => emit('clients-changed', {}), 10)
    }
    return null
  },
  'clone_client': (args) => {
    const source = mockData.clients.find(c => c.client_id === args?.clientId || c.id === args?.clientId)
    if (!source) return null
    const existingNames = mockData.clients.map(c => c.name)
    let cloneName = `Clone of ${source.name}`
    let n = 2
    while (existingNames.includes(cloneName)) { cloneName = `Clone of ${source.name} (${n++})` }
    const newId = generateId()
    const cloned = { ...source, id: newId, client_id: newId, name: cloneName, sync_config: false, created_at: new Date().toISOString(), last_used: null }
    mockData.clients.push(cloned)
    setTimeout(() => emit('clients-changed', {}), 10)
    return [`demo-secret-${newId}`, cloned]
  },
  'rotate_client_secret': () => {
    setTimeout(() => emit('clients-changed', {}), 10)
    return { secret: `demo-secret-${generateId()}` }
  },
  'get_client_value': () => null,

  // Client mode, template, and guardrails
  'get_client_guardrails_config': (args) => {
    const client = mockData.clients.find(c => c.id === args?.clientId || c.client_id === args?.clientId)
    const guardrails = client ? (client as Record<string, unknown>).guardrails : undefined
    return guardrails || { category_actions: null }
  },
  'update_client_guardrails_config': (args) => {
    const client = mockData.clients.find(c => c.id === args?.clientId || c.client_id === args?.clientId)
    if (client && args?.configJson) {
      try {
        (client as Record<string, unknown>).guardrails = JSON.parse(args.configJson)
      } catch { /* ignore */ }
      setTimeout(() => emit('clients-changed', {}), 10)
    }
    toast.success('Client guardrails config updated (demo)')
    return null
  },
  'set_client_mode': (args) => {
    const client = mockData.clients.find(c => c.client_id === args?.clientId || c.id === args?.clientId)
    if (client && args?.mode) {
      (client as Record<string, unknown>).client_mode = args.mode
      setTimeout(() => emit('clients-changed', {}), 10)
    }
    return null
  },
  'set_client_template': (args) => {
    const client = mockData.clients.find(c => c.client_id === args?.clientId || c.id === args?.clientId)
    if (client) {
      (client as Record<string, unknown>).template_id = args?.templateId || null
      setTimeout(() => emit('clients-changed', {}), 10)
    }
    return null
  },
  'get_app_capabilities': (args) => {
    const installed = ['claude-code', 'cursor'].includes(args?.templateId || '')
    const tryItOutApps = ['claude-code', 'codex', 'aider', 'goose']
    return {
      installed,
      binary_path: installed ? `/usr/local/bin/${args?.templateId}` : null,
      version: installed ? '1.0.0' : null,
      supports_try_it_out: tryItOutApps.includes(args?.templateId || ''),
      supports_permanent_config: true,
    }
  },
  'try_it_out_app': (args) => {
    toast.success(`Try it out for client ${args?.clientId} (demo)`)
    return {
      success: true,
      message: 'Run the command below in your terminal:',
      modified_files: [],
      backup_files: [],
      terminal_command: 'ANTHROPIC_BASE_URL=http://127.0.0.1:3625 ANTHROPIC_API_KEY=lr_demo_secret claude --mcp-config \'{"mcpServers":{"localrouter":{"type":"http","url":"http://127.0.0.1:3625","headers":{"Authorization":"Bearer lr_demo_secret"}}}}\'',
    }
  },
  'configure_app_permanent': (args) => {
    toast.success(`App configured permanently for client ${args?.clientId} (demo)`)
    return {
      success: true,
      message: 'MCP configured in ~/.claude.json. For LLM routing, use env vars at launch time.',
      modified_files: ['~/.claude.json'],
      backup_files: ['~/.claude.json.20260210_120000.bak'],
    }
  },
  'toggle_client_sync_config': (args) => {
    const client = mockData.clients.find(c => c.client_id === args?.clientId || c.id === args?.clientId)
    if (client) {
      (client as Record<string, unknown>).sync_config = args?.enabled ?? false
      setTimeout(() => emit('clients-changed', {}), 10)
    }
    if (args?.enabled) {
      toast.success(`Config sync enabled for client ${args?.clientId} (demo)`)
      return {
        success: true,
        message: 'Config synced successfully.',
        modified_files: ['~/.config/opencode/opencode.json'],
        backup_files: [],
      }
    }
    toast.success(`Config sync disabled for client ${args?.clientId} (demo)`)
    return null
  },
  'sync_client_config': (args) => {
    toast.success(`Config synced for client ${args?.clientId} (demo)`)
    return {
      success: true,
      message: 'Config synced successfully.',
      modified_files: ['~/.config/opencode/opencode.json'],
      backup_files: [],
    }
  },

  // Client permissions - find by either client_id (string identifier) or id (uuid)
  'set_client_mcp_permission': (args) => {
    const client = mockData.clients.find(c => c.client_id === args?.clientId || c.id === args?.clientId)
    if (client && args?.level && args?.state) {
      // Handle clear flag - if set, remove the permission instead of setting it
      if (args.clear && args.level !== 'global') {
        if (args.level === 'server' && args.key) {
          delete client.mcp_permissions.servers[args.key]
        } else if (args.level === 'tool' && args.key) {
          delete client.mcp_permissions.tools[args.key]
        } else if (args.level === 'resource' && args.key) {
          delete client.mcp_permissions.resources[args.key]
        } else if (args.level === 'prompt' && args.key) {
          delete client.mcp_permissions.prompts[args.key]
        }
      } else {
        if (args.level === 'global') {
          client.mcp_permissions.global = args.state
        } else if (args.level === 'server' && args.key) {
          client.mcp_permissions.servers[args.key] = args.state
        } else if (args.level === 'tool' && args.key) {
          client.mcp_permissions.tools[args.key] = args.state
        } else if (args.level === 'resource' && args.key) {
          client.mcp_permissions.resources[args.key] = args.state
        } else if (args.level === 'prompt' && args.key) {
          client.mcp_permissions.prompts[args.key] = args.state
        }
      }
      // Emit clients-changed event to trigger UI refresh
      setTimeout(() => emit('clients-changed', {}), 10)
    }
    return null
  },
  'set_client_skills_permission': (args) => {
    const client = mockData.clients.find(c => c.client_id === args?.clientId || c.id === args?.clientId)
    if (client && args?.level && args?.state) {
      // Handle clear flag
      if (args.clear && args.level !== 'global') {
        if (args.level === 'skill' && args.key) {
          delete client.skills_permissions.skills[args.key]
        } else if (args.level === 'tool' && args.key) {
          delete client.skills_permissions.tools[args.key]
        }
      } else {
        if (args.level === 'global') {
          client.skills_permissions.global = args.state
        } else if (args.level === 'skill' && args.key) {
          client.skills_permissions.skills[args.key] = args.state
        } else if (args.level === 'tool' && args.key) {
          client.skills_permissions.tools[args.key] = args.state
        }
      }
      // Emit clients-changed event to trigger UI refresh
      setTimeout(() => emit('clients-changed', {}), 10)
    }
    return null
  },
  'set_client_model_permission': (args) => {
    const client = mockData.clients.find(c => c.client_id === args?.clientId || c.id === args?.clientId)
    if (client && args?.level && args?.state) {
      // Handle clear flag
      if (args.clear && args.level !== 'global') {
        if (args.level === 'provider' && args.key) {
          delete client.model_permissions.providers[args.key]
        } else if (args.level === 'model' && args.key) {
          delete client.model_permissions.models[args.key]
        }
      } else {
        if (args.level === 'global') {
          client.model_permissions.global = args.state
        } else if (args.level === 'provider' && args.key) {
          client.model_permissions.providers[args.key] = args.state
        } else if (args.level === 'model' && args.key) {
          client.model_permissions.models[args.key] = args.state
        }
      }
      // Emit clients-changed event to trigger UI refresh
      setTimeout(() => emit('clients-changed', {}), 10)
    }
    return null
  },
  'set_client_marketplace_permission': (args) => {
    const client = mockData.clients.find(c => c.client_id === args?.clientId || c.id === args?.clientId)
    if (client && args?.state) {
      client.marketplace_permission = args.state
      setTimeout(() => emit('clients-changed', {}), 10)
    }
    return null
  },
  'set_client_sampling_permission': (args) => {
    const client = mockData.clients.find(c => c.client_id === args?.clientId || c.id === args?.clientId)
    if (client && args?.state) {
      client.mcp_sampling_permission = args.state
      setTimeout(() => emit('clients-changed', {}), 10)
    }
    return null
  },
  'set_client_elicitation_permission': (args) => {
    const client = mockData.clients.find(c => c.client_id === args?.clientId || c.id === args?.clientId)
    if (client && args?.state) {
      client.mcp_elicitation_permission = args.state
      setTimeout(() => emit('clients-changed', {}), 10)
    }
    return null
  },
  'clear_client_mcp_child_permissions': (args) => {
    const client = mockData.clients.find(c => c.client_id === args?.clientId || c.id === args?.clientId)
    if (client) {
      const serverId = args?.serverId
      if (serverId) {
        // Only clear children of the specific server
        const prefix = `${serverId}__`
        for (const key of Object.keys(client.mcp_permissions.tools)) {
          if (key.startsWith(prefix)) delete client.mcp_permissions.tools[key]
        }
        for (const key of Object.keys(client.mcp_permissions.resources)) {
          if (key.startsWith(prefix)) delete client.mcp_permissions.resources[key]
        }
        for (const key of Object.keys(client.mcp_permissions.prompts)) {
          if (key.startsWith(prefix)) delete client.mcp_permissions.prompts[key]
        }
      } else {
        // Clear all children
        client.mcp_permissions.servers = {}
        client.mcp_permissions.tools = {}
        client.mcp_permissions.resources = {}
        client.mcp_permissions.prompts = {}
      }
      // Emit clients-changed event to trigger UI refresh
      setTimeout(() => emit('clients-changed', {}), 10)
    }
    return null
  },
  'clear_client_skills_child_permissions': (args) => {
    const client = mockData.clients.find(c => c.client_id === args?.clientId || c.id === args?.clientId)
    if (client) {
      const skillName = args?.skillName
      if (skillName) {
        // Only clear children of the specific skill
        const prefix = `${skillName}__`
        for (const key of Object.keys(client.skills_permissions.tools)) {
          if (key.startsWith(prefix)) delete client.skills_permissions.tools[key]
        }
      } else {
        // Clear all children
        client.skills_permissions.skills = {}
        client.skills_permissions.tools = {}
      }
      // Emit clients-changed event to trigger UI refresh
      setTimeout(() => emit('clients-changed', {}), 10)
    }
    return null
  },
  'clear_client_model_child_permissions': (args) => {
    const client = mockData.clients.find(c => c.client_id === args?.clientId || c.id === args?.clientId)
    if (client) {
      const provider = args?.provider
      if (provider) {
        // Only clear children of the specific provider
        const prefix = `${provider}__`
        for (const key of Object.keys(client.model_permissions.models)) {
          if (key.startsWith(prefix)) delete client.model_permissions.models[key]
        }
      } else {
        // Clear all children
        client.model_permissions.providers = {}
        client.model_permissions.models = {}
      }
      // Emit clients-changed event to trigger UI refresh
      setTimeout(() => emit('clients-changed', {}), 10)
    }
    return null
  },

  // ============================================================================
  // Providers
  // ============================================================================
  'list_provider_instances': () => mockData.providers,
  'list_provider_types': () => mockData.providerTypes,
  'get_provider_instance': (args) => mockData.providers.find(p => p.instance_name === args?.instanceName),
  'get_provider_config': (args) => {
    const provider = mockData.providers.find(p => p.instance_name === args?.instanceName)
    if (!provider) return {}
    return { api_key: 'sk-demo-key-1234567890', base_url: 'https://api.openai.com/v1' }
  },
  'create_provider_instance': (args) => {
    const newProvider = {
      instance_name: args?.instanceName || `provider-${generateId()}`,
      provider_type: args?.providerType || 'openai',
      enabled: true,
      display_name: args?.displayName || args?.instanceName,
      api_key_set: !!args?.apiKey,
    }
    mockData.providers.push(newProvider)
    toast.success(`Provider "${args?.instanceName}" created (demo)`)
    return newProvider
  },
  'update_provider_instance': (args) => {
    const provider = mockData.providers.find(p => p.instance_name === args?.instanceName)
    if (provider && args?.config) {
      Object.assign(provider, { config: args.config })
    }
    return null
  },
  'rename_provider_instance': (args) => {
    const provider = mockData.providers.find(p => p.instance_name === args?.instanceName)
    if (provider && args?.newName) {
      provider.instance_name = args.newName
    }
    return null
  },
  'get_provider_api_key': (_args) => {
    return 'sk-demo-key-1234567890'
  },
  'remove_provider_instance': (args) => {
    const idx = mockData.providers.findIndex(p => p.instance_name === args?.instanceName)
    if (idx !== -1) mockData.providers.splice(idx, 1)
    toast.success('Provider removed (demo)')
    setTimeout(() => emit('providers-changed', {}), 10)
    return null
  },
  'clone_provider_instance': (args) => {
    const source = mockData.providers.find(p => p.instance_name === args?.instanceName)
    if (!source) return null
    const existingNames = mockData.providers.map(p => p.instance_name)
    let cloneName = `Clone of ${source.instance_name}`
    let n = 2
    while (existingNames.includes(cloneName)) { cloneName = `Clone of ${source.instance_name} (${n++})` }
    const cloned = { ...source, instance_name: cloneName }
    mockData.providers.push(cloned)
    setTimeout(() => emit('providers-changed', {}), 10)
    return null
  },
  'set_provider_enabled': (args) => {
    const provider = mockData.providers.find(p => p.instance_name === args?.instanceName)
    if (provider) provider.enabled = args?.enabled ?? true
    return null
  },

  // ============================================================================
  // MCP Servers
  // ============================================================================
  'list_mcp_servers': () => mockData.mcpServers,
  'get_mcp_server': (args) => mockData.mcpServers.find(s => s.id === args?.id),
  'create_mcp_server': (args) => {
    const newServer = {
      id: `mcp-${generateId()}`,
      name: args?.name || 'New MCP Server',
      enabled: true,
      transport_type: args?.transportType || 'stdio',
      description: args?.description || '',
      tools_count: 0,
      auth_type: 'none',
      ...args,
    }
    mockData.mcpServers.push(newServer)
    toast.success(`MCP Server "${args?.name}" created (demo)`)
    return newServer
  },
  'update_mcp_server': (args) => {
    const server = mockData.mcpServers.find(s => s.id === args?.id)
    if (server && args?.updates) {
      Object.assign(server, args.updates)
    }
    return null
  },
  'delete_mcp_server': (args) => {
    const idx = mockData.mcpServers.findIndex(s => s.id === (args?.serverId || args?.id))
    if (idx !== -1) mockData.mcpServers.splice(idx, 1)
    toast.success('MCP Server deleted (demo)')
    setTimeout(() => emit('mcp-servers-changed', {}), 10)
    return null
  },
  'clone_mcp_server': (args) => {
    const source = mockData.mcpServers.find(s => s.id === args?.serverId)
    if (!source) return null
    const existingNames = mockData.mcpServers.map(s => s.name)
    let cloneName = `Clone of ${source.name}`
    let n = 2
    while (existingNames.includes(cloneName)) { cloneName = `Clone of ${source.name} (${n++})` }
    const newId = generateId()
    const cloned = { ...source, id: newId, name: cloneName, created_at: new Date().toISOString() }
    mockData.mcpServers.push(cloned)
    setTimeout(() => emit('mcp-servers-changed', {}), 10)
    return cloned
  },
  'toggle_mcp_server_enabled': (args) => {
    const server = mockData.mcpServers.find(s => s.id === args?.id)
    if (server) server.enabled = !server.enabled
    return null
  },
  'start_mcp_health_checks': () => {
    emitMcpHealthEvents()
    return null
  },
  'check_single_mcp_health': (args) => {
    const serverId = args?.serverId || args?.id
    if (serverId) {
      const server = mockData.mcpServers.find(s => s.id === serverId)
      const healthCache = mockData.healthCache.mcp_servers as Record<string, { status: string; latency_ms: number | null }>
      const health = healthCache[serverId]
      setTimeout(() => {
        emit('mcp-health-check', {
          server_id: serverId,
          status: server?.enabled ? (health?.status || 'healthy') : 'disabled',
          latency_ms: health?.latency_ms || Math.floor(Math.random() * 200) + 50,
          error: null,
        })
      }, 100)
    }
    return { status: 'healthy', latency_ms: Math.floor(Math.random() * 200) + 50 }
  },

  // MCP capabilities - return server-specific tools
  'get_mcp_server_capabilities': (args) => {
    const server = mockData.mcpServers.find(s => s.id === args?.id || s.id === args?.serverId)
    // Return server-specific tools based on the server type
    const toolsByServer: Record<string, { name: string; description: string; input_schema: Record<string, unknown> }[]> = {
      'mcp-github': [
        {
          name: 'create_issue',
          description: 'Create a new issue in a GitHub repository. Supports setting the title, body, labels, assignees, and milestone. The repository must exist and the authenticated user must have write access to it.',
          input_schema: {
            type: 'object',
            properties: {
              owner: { type: 'string', description: 'The account owner of the repository. This is the GitHub username or organization name that owns the repo.' },
              repo: { type: 'string', description: 'The name of the repository where the issue will be created. Must not include the owner prefix.' },
              title: { type: 'string', description: 'A concise, descriptive title for the issue. Should clearly summarize the problem or feature request.' },
              body: { type: 'string', description: 'The full body text of the issue, formatted as GitHub-flavored Markdown. May include code blocks, task lists, and references to other issues.' },
              labels: { type: 'array', items: { type: 'string' }, description: 'An array of label names to apply to the issue. Labels must already exist in the repository.' },
              assignees: { type: 'array', items: { type: 'string' }, description: 'An array of GitHub usernames to assign to the issue. Each assignee must have access to the repository.' },
              milestone: { type: 'number', description: 'The numeric ID of a milestone to associate with this issue. The milestone must exist in the repository.' },
            },
            required: ['owner', 'repo', 'title'],
          },
        },
        {
          name: 'create_pull_request',
          description: 'Create a new pull request in a GitHub repository from a head branch into a base branch. Supports setting the title, body, draft status, and whether maintainer edits are allowed. The head branch must contain commits not present in the base branch.',
          input_schema: {
            type: 'object',
            properties: {
              owner: { type: 'string', description: 'The account owner of the repository where the pull request will be created.' },
              repo: { type: 'string', description: 'The name of the repository. Must not include the owner prefix.' },
              title: { type: 'string', description: 'A concise, descriptive title for the pull request summarizing the changes.' },
              body: { type: 'string', description: 'The full description of the pull request in GitHub-flavored Markdown. Should explain what changes were made and why.' },
              head: { type: 'string', description: 'The name of the branch where your changes are implemented. For cross-repository PRs, use the format owner:branch.' },
              base: { type: 'string', description: 'The name of the branch you want the changes pulled into. This is usually "main" or "master".' },
              draft: { type: 'boolean', description: 'Whether to create the pull request as a draft. Draft PRs cannot be merged until marked as ready for review.' },
              maintainer_can_modify: { type: 'boolean', description: 'Whether maintainers of the base repository can push to the head branch. Defaults to true for non-fork PRs.' },
            },
            required: ['owner', 'repo', 'title', 'head', 'base'],
          },
        },
        {
          name: 'list_repos',
          description: 'List repositories accessible to the authenticated user, optionally filtered by owner, type, or sort order. Returns paginated results including repository metadata such as name, description, visibility, and language statistics.',
          input_schema: {
            type: 'object',
            properties: {
              owner: { type: 'string', description: 'Filter repositories by this owner (username or organization). If omitted, returns repositories for the authenticated user.' },
              type: { type: 'string', description: 'The type of repositories to list.', enum: ['all', 'owner', 'public', 'private', 'member'] },
              sort: { type: 'string', description: 'The property to sort results by.', enum: ['created', 'updated', 'pushed', 'full_name'] },
              direction: { type: 'string', description: 'The sort direction for results.', enum: ['asc', 'desc'] },
              per_page: { type: 'number', description: 'Number of results per page. Maximum is 100. Defaults to 30.' },
              page: { type: 'number', description: 'The page number of results to fetch. Starts at 1.' },
            },
            required: [],
          },
        },
        {
          name: 'search_code',
          description: 'Search for code across all repositories accessible to the authenticated user using GitHub code search syntax. Returns matching file paths, repository information, and text matches with surrounding context. Supports qualifiers for language, filename, path, and repository filtering.',
          input_schema: {
            type: 'object',
            properties: {
              q: { type: 'string', description: 'The search query using GitHub code search syntax. Can include qualifiers like "language:python", "filename:config.yml", "repo:owner/name", or "path:src/". Example: "addClass in:file language:js repo:jquery/jquery".' },
              sort: { type: 'string', description: 'Sort field for results. Can only be "indexed" which sorts by last index time.', enum: ['indexed'] },
              order: { type: 'string', description: 'Sort order for results.', enum: ['asc', 'desc'] },
              per_page: { type: 'number', description: 'Number of results per page. Maximum is 100. Defaults to 30.' },
              page: { type: 'number', description: 'The page number of results to fetch, starting at 1.' },
            },
            required: ['q'],
          },
        },
        {
          name: 'get_file_contents',
          description: 'Retrieve the decoded contents of a file from a GitHub repository at a specific path and optional Git reference. Returns the file content, encoding, size, and SHA hash. For files larger than 1MB, the content may be returned as a download URL instead.',
          input_schema: {
            type: 'object',
            properties: {
              owner: { type: 'string', description: 'The account owner of the repository containing the file.' },
              repo: { type: 'string', description: 'The name of the repository.' },
              path: { type: 'string', description: 'The file path relative to the root of the repository. Must not start with a slash.' },
              ref: { type: 'string', description: 'The name of the commit, branch, or tag to retrieve the file from. Defaults to the default branch if omitted.' },
            },
            required: ['owner', 'repo', 'path'],
          },
        },
        {
          name: 'create_branch',
          description: 'Create a new Git branch reference in a GitHub repository. The branch is created by pointing to an existing commit SHA. You must first retrieve the SHA of the commit you want the branch to point to, typically the HEAD of another branch.',
          input_schema: {
            type: 'object',
            properties: {
              owner: { type: 'string', description: 'The account owner of the repository where the branch will be created.' },
              repo: { type: 'string', description: 'The name of the repository.' },
              branch: { type: 'string', description: 'The name for the new branch. Must not already exist in the repository. Do not include "refs/heads/" prefix.' },
              from_branch: { type: 'string', description: 'The name of the existing branch to create the new branch from. The new branch will start at the same commit as this branch.' },
            },
            required: ['owner', 'repo', 'branch'],
          },
        },
        {
          name: 'merge_pull_request',
          description: 'Merge an open pull request in a GitHub repository. Supports merge, squash, and rebase merge strategies. The pull request must be in a mergeable state with all required status checks passing and no merge conflicts.',
          input_schema: {
            type: 'object',
            properties: {
              owner: { type: 'string', description: 'The account owner of the repository containing the pull request.' },
              repo: { type: 'string', description: 'The name of the repository.' },
              pull_number: { type: 'number', description: 'The pull request number to merge.' },
              commit_title: { type: 'string', description: 'Title for the merge commit. If omitted, a default title is generated based on the merge method.' },
              commit_message: { type: 'string', description: 'Extra detail for the merge commit body. Only used for merge and squash methods.' },
              merge_method: { type: 'string', description: 'The merge strategy to use for combining the pull request commits.', enum: ['merge', 'squash', 'rebase'] },
              sha: { type: 'string', description: 'SHA that the pull request head must match to allow the merge. Prevents merging if new commits were pushed after review.' },
            },
            required: ['owner', 'repo', 'pull_number'],
          },
        },
        {
          name: 'list_workflows',
          description: 'List all GitHub Actions workflows defined in a repository. Returns workflow metadata including the workflow ID, name, file path, state, and creation timestamp. Only workflows stored in the .github/workflows directory are included.',
          input_schema: {
            type: 'object',
            properties: {
              owner: { type: 'string', description: 'The account owner of the repository.' },
              repo: { type: 'string', description: 'The name of the repository to list workflows for.' },
              per_page: { type: 'number', description: 'Number of results per page. Maximum is 100. Defaults to 30.' },
              page: { type: 'number', description: 'The page number of results to fetch.' },
            },
            required: ['owner', 'repo'],
          },
        },
        {
          name: 'trigger_workflow',
          description: 'Manually trigger a GitHub Actions workflow dispatch event for a specified workflow. The workflow must have a workflow_dispatch trigger configured in its YAML definition. Supports passing custom input parameters defined in the workflow file.',
          input_schema: {
            type: 'object',
            properties: {
              owner: { type: 'string', description: 'The account owner of the repository containing the workflow.' },
              repo: { type: 'string', description: 'The name of the repository.' },
              workflow_id: { type: 'string', description: 'The workflow ID (numeric) or the workflow file name (e.g., "ci.yml") to trigger.' },
              ref: { type: 'string', description: 'The Git reference (branch or tag) to run the workflow on. The workflow file must exist on this ref.' },
              inputs: { type: 'object', description: 'A key-value map of input parameters to pass to the workflow. Keys must match inputs defined in the workflow_dispatch trigger.' },
            },
            required: ['owner', 'repo', 'workflow_id', 'ref'],
          },
        },
        {
          name: 'get_commit_history',
          description: 'Retrieve the commit history for a repository, optionally filtered by branch, path, author, or date range. Returns a paginated list of commits with their SHA, message, author information, and timestamp. Commits are returned in reverse chronological order.',
          input_schema: {
            type: 'object',
            properties: {
              owner: { type: 'string', description: 'The account owner of the repository.' },
              repo: { type: 'string', description: 'The name of the repository to get commit history for.' },
              sha: { type: 'string', description: 'The branch name or commit SHA to start listing commits from. Defaults to the default branch.' },
              path: { type: 'string', description: 'Only include commits that modified this file path.' },
              author: { type: 'string', description: 'Filter commits by this GitHub username or email address.' },
              since: { type: 'string', description: 'Only show commits after this date. ISO 8601 format: YYYY-MM-DDTHH:MM:SSZ.' },
              until: { type: 'string', description: 'Only show commits before this date. ISO 8601 format: YYYY-MM-DDTHH:MM:SSZ.' },
              per_page: { type: 'number', description: 'Number of results per page. Maximum is 100.' },
            },
            required: ['owner', 'repo'],
          },
        },
        {
          name: 'create_release',
          description: 'Create a new release for a repository on GitHub. Releases are deployable software iterations based on Git tags. This endpoint creates both the Git tag (if it does not exist) and the associated release with release notes and optional binary assets.',
          input_schema: {
            type: 'object',
            properties: {
              owner: { type: 'string', description: 'The account owner of the repository.' },
              repo: { type: 'string', description: 'The name of the repository.' },
              tag_name: { type: 'string', description: 'The name of the tag for this release. If the tag does not exist, it will be created from the target_commitish.' },
              target_commitish: { type: 'string', description: 'The commitish value (branch or SHA) that determines where the Git tag is created from. Ignored if the tag already exists.' },
              name: { type: 'string', description: 'The human-readable name for the release. Often matches or describes the tag name (e.g., "v1.0.0 - Initial Release").' },
              body: { type: 'string', description: 'Markdown-formatted text describing the release contents, changes, and upgrade notes.' },
              draft: { type: 'boolean', description: 'Whether to create the release as an unpublished draft. Draft releases are not visible to the public.' },
              prerelease: { type: 'boolean', description: 'Whether to mark the release as a prerelease. Prerelease versions indicate the software is not production-ready.' },
              generate_release_notes: { type: 'boolean', description: 'Whether to automatically generate release notes based on merged pull requests since the last release.' },
            },
            required: ['owner', 'repo', 'tag_name'],
          },
        },
        {
          name: 'add_comment',
          description: 'Add a comment to an existing issue or pull request in a GitHub repository. The comment body supports full GitHub-flavored Markdown including code blocks, mentions, task lists, and embedded images. The authenticated user must have read access to the repository.',
          input_schema: {
            type: 'object',
            properties: {
              owner: { type: 'string', description: 'The account owner of the repository.' },
              repo: { type: 'string', description: 'The name of the repository.' },
              issue_number: { type: 'number', description: 'The number of the issue or pull request to comment on. Pull requests are treated as issues for the comment API.' },
              body: { type: 'string', description: 'The contents of the comment in GitHub-flavored Markdown format. Supports @mentions, #references, task lists, and code blocks.' },
            },
            required: ['owner', 'repo', 'issue_number', 'body'],
          },
        },
      ],
      'mcp-filesystem': [
        {
          name: 'read_file',
          description: 'Read the complete contents of a file from the filesystem. Returns file content as UTF-8 text, or base64-encoded data for binary files. The file must be within the configured allowed directory paths.',
          input_schema: {
            type: 'object',
            properties: {
              path: { type: 'string', description: 'Absolute or relative path to the file to read. Must be within the allowed directories configured for the server.' },
              encoding: { type: 'string', description: 'File encoding to use when reading. Defaults to utf-8 for text files.', enum: ['utf-8', 'utf-16', 'ascii', 'base64'] },
            },
            required: ['path'],
          },
        },
        {
          name: 'write_file',
          description: 'Write text content to a file on the filesystem, creating the file if it does not exist or overwriting it if it does. Parent directories must already exist. The file path must be within the configured allowed directory paths.',
          input_schema: {
            type: 'object',
            properties: {
              path: { type: 'string', description: 'Absolute or relative path to the file to write. Must be within the allowed directories.' },
              content: { type: 'string', description: 'The full text content to write to the file. Existing content will be completely replaced.' },
              encoding: { type: 'string', description: 'File encoding to use when writing. Defaults to utf-8.', enum: ['utf-8', 'utf-16', 'ascii'] },
              create_parents: { type: 'boolean', description: 'Whether to create parent directories if they do not exist. Defaults to false.' },
            },
            required: ['path', 'content'],
          },
        },
        {
          name: 'list_directory',
          description: 'List all files and subdirectories within a directory on the filesystem. Returns file names, sizes, types, and modification timestamps. Results can be filtered by file extension or name pattern. The directory must be within the allowed paths.',
          input_schema: {
            type: 'object',
            properties: {
              path: { type: 'string', description: 'Absolute or relative path to the directory to list. Must be within the allowed directories.' },
              recursive: { type: 'boolean', description: 'Whether to recursively list contents of subdirectories. Defaults to false for a single-level listing.' },
              include_hidden: { type: 'boolean', description: 'Whether to include hidden files and directories (those starting with a dot). Defaults to false.' },
              pattern: { type: 'string', description: 'Glob pattern to filter results (e.g., "*.ts", "**/*.json"). Only matching entries will be returned.' },
            },
            required: ['path'],
          },
        },
        {
          name: 'create_directory',
          description: 'Create a new directory on the filesystem at the specified path. Can optionally create parent directories recursively if they do not exist. The path must be within the configured allowed directory paths. Fails if the directory already exists unless ignore_existing is set.',
          input_schema: {
            type: 'object',
            properties: {
              path: { type: 'string', description: 'Absolute or relative path for the new directory. Must be within the allowed directories.' },
              recursive: { type: 'boolean', description: 'Whether to create parent directories if they do not exist, similar to "mkdir -p". Defaults to false.' },
              ignore_existing: { type: 'boolean', description: 'If true, do not return an error when the directory already exists. Defaults to false.' },
            },
            required: ['path'],
          },
        },
        {
          name: 'delete_file',
          description: 'Permanently delete a file from the filesystem. This action cannot be undone. The file must exist and be within the configured allowed directory paths. Does not support deleting directories; use delete_directory for that purpose.',
          input_schema: {
            type: 'object',
            properties: {
              path: { type: 'string', description: 'Absolute or relative path to the file to delete. Must be within the allowed directories.' },
              force: { type: 'boolean', description: 'If true, do not return an error if the file does not exist. Defaults to false.' },
            },
            required: ['path'],
          },
        },
        {
          name: 'move_file',
          description: 'Move or rename a file from one location to another on the filesystem. Both the source and destination paths must be within the configured allowed directory paths. If the destination already exists, the operation will fail unless overwrite is enabled.',
          input_schema: {
            type: 'object',
            properties: {
              source: { type: 'string', description: 'The current absolute or relative path of the file to move.' },
              destination: { type: 'string', description: 'The new absolute or relative path where the file should be moved to.' },
              overwrite: { type: 'boolean', description: 'Whether to overwrite the destination file if it already exists. Defaults to false.' },
            },
            required: ['source', 'destination'],
          },
        },
        {
          name: 'copy_file',
          description: 'Copy a file from a source path to a destination path on the filesystem. Both paths must be within the configured allowed directory paths. The original file remains unchanged. If the destination file already exists, the operation will fail unless overwrite is enabled.',
          input_schema: {
            type: 'object',
            properties: {
              source: { type: 'string', description: 'The absolute or relative path of the file to copy from.' },
              destination: { type: 'string', description: 'The absolute or relative path to copy the file to.' },
              overwrite: { type: 'boolean', description: 'Whether to overwrite the destination file if it already exists. Defaults to false.' },
            },
            required: ['source', 'destination'],
          },
        },
        {
          name: 'search_files',
          description: 'Search for files within a directory tree by matching file names against a glob pattern or regular expression. Returns all matching file paths with metadata including size and modification time. Supports recursive searching through subdirectories.',
          input_schema: {
            type: 'object',
            properties: {
              path: { type: 'string', description: 'The root directory to start searching from. Must be within the allowed directories.' },
              pattern: { type: 'string', description: 'Glob pattern (e.g., "*.ts", "**/*.json") or regular expression to match file names against.' },
              regex: { type: 'boolean', description: 'If true, treat the pattern as a regular expression instead of a glob pattern. Defaults to false.' },
              include_hidden: { type: 'boolean', description: 'Whether to include hidden files and directories in the search. Defaults to false.' },
              max_depth: { type: 'number', description: 'Maximum directory depth to search. A value of 1 only searches the immediate directory. No limit if omitted.' },
            },
            required: ['path', 'pattern'],
          },
        },
      ],
      'mcp-slack': [
        {
          name: 'send_message',
          description: 'Send a message to a Slack channel or direct message conversation. Supports rich text formatting using Slack mrkdwn syntax including bold, italic, code blocks, links, and emoji. Messages can optionally be sent as a reply to an existing thread.',
          input_schema: {
            type: 'object',
            properties: {
              channel: { type: 'string', description: 'The channel ID or name to send the message to. Channel names should be prefixed with #. For DMs, use the user\'s ID.' },
              text: { type: 'string', description: 'The message text to send, formatted using Slack mrkdwn syntax. Supports *bold*, _italic_, `code`, ```code blocks```, and <url|link text> formatting.' },
              thread_ts: { type: 'string', description: 'The timestamp of the parent message to reply to in a thread. If omitted, sends as a new top-level message.' },
              unfurl_links: { type: 'boolean', description: 'Whether to enable link previews for URLs in the message. Defaults to true.' },
              unfurl_media: { type: 'boolean', description: 'Whether to enable media previews for media URLs. Defaults to true.' },
            },
            required: ['channel', 'text'],
          },
        },
        {
          name: 'list_channels',
          description: 'List all public and private channels visible to the authenticated user in the Slack workspace. Returns channel metadata including name, topic, purpose, member count, and creation timestamp. Results are paginated and can be filtered by type.',
          input_schema: {
            type: 'object',
            properties: {
              types: { type: 'string', description: 'Comma-separated list of channel types to include. Valid values: public_channel, private_channel, mpim, im. Defaults to public_channel.' },
              exclude_archived: { type: 'boolean', description: 'Whether to exclude archived channels from the results. Defaults to false.' },
              limit: { type: 'number', description: 'Maximum number of channels to return. Defaults to 100, maximum is 1000.' },
              cursor: { type: 'string', description: 'Pagination cursor returned from a previous request. Use this to fetch the next page of results.' },
            },
            required: [],
          },
        },
        {
          name: 'get_channel_history',
          description: 'Retrieve the message history from a Slack channel or conversation. Returns messages in reverse chronological order with full metadata including author, timestamp, reactions, and thread reply counts. Supports filtering by time range and pagination.',
          input_schema: {
            type: 'object',
            properties: {
              channel: { type: 'string', description: 'The ID of the channel to fetch history from.' },
              limit: { type: 'number', description: 'Maximum number of messages to return. Defaults to 100, maximum is 1000.' },
              oldest: { type: 'string', description: 'Only return messages after this Unix timestamp (inclusive). Used for filtering by date range.' },
              latest: { type: 'string', description: 'Only return messages before this Unix timestamp (exclusive). Used for filtering by date range.' },
              inclusive: { type: 'boolean', description: 'Whether to include messages with the exact oldest or latest timestamps. Defaults to false.' },
              cursor: { type: 'string', description: 'Pagination cursor for fetching additional pages of results.' },
            },
            required: ['channel'],
          },
        },
        {
          name: 'create_channel',
          description: 'Create a new public or private channel in the Slack workspace. Channel names must be lowercase, without spaces, and no longer than 80 characters. The authenticated user will automatically be added as a member of the new channel.',
          input_schema: {
            type: 'object',
            properties: {
              name: { type: 'string', description: 'Name for the new channel. Must be lowercase, no spaces, max 80 characters. Hyphens and underscores are allowed.' },
              is_private: { type: 'boolean', description: 'Whether to create a private channel. Private channels are only visible to invited members. Defaults to false.' },
              description: { type: 'string', description: 'A short description of the channel purpose, displayed in the channel details panel.' },
              topic: { type: 'string', description: 'The topic text displayed at the top of the channel. Can be updated later by any channel member.' },
            },
            required: ['name'],
          },
        },
        {
          name: 'invite_user',
          description: 'Invite one or more users to join a Slack channel. The authenticated user must be a member of the channel and have permission to invite others. Users who are already members of the channel will be silently ignored.',
          input_schema: {
            type: 'object',
            properties: {
              channel: { type: 'string', description: 'The ID of the channel to invite users to.' },
              users: { type: 'string', description: 'A comma-separated list of user IDs to invite to the channel. Maximum of 1000 users per request.' },
            },
            required: ['channel', 'users'],
          },
        },
        {
          name: 'upload_file',
          description: 'Upload a file to one or more Slack channels. Supports text files, images, PDFs, and other common file types up to the workspace file size limit. The file can include an optional title and initial comment message.',
          input_schema: {
            type: 'object',
            properties: {
              channels: { type: 'string', description: 'Comma-separated list of channel IDs to share the file with.' },
              content: { type: 'string', description: 'The text content of the file. Use this for creating text-based files directly. Mutually exclusive with file_url.' },
              filename: { type: 'string', description: 'The name of the file including extension (e.g., "report.csv"). Determines the file type and icon shown in Slack.' },
              title: { type: 'string', description: 'A descriptive title for the file, displayed prominently in the Slack interface.' },
              initial_comment: { type: 'string', description: 'An optional message to post alongside the file upload. Supports Slack mrkdwn formatting.' },
              filetype: { type: 'string', description: 'The file type identifier (e.g., "python", "json", "markdown"). Used for syntax highlighting of text content.' },
            },
            required: ['channels', 'filename'],
          },
        },
        {
          name: 'add_reaction',
          description: 'Add an emoji reaction to a message in a Slack channel. The reaction is added on behalf of the authenticated user. If the user has already reacted with the same emoji, the operation will succeed silently without duplicating the reaction.',
          input_schema: {
            type: 'object',
            properties: {
              channel: { type: 'string', description: 'The ID of the channel containing the message to react to.' },
              timestamp: { type: 'string', description: 'The timestamp of the message to add the reaction to. This uniquely identifies the message within the channel.' },
              name: { type: 'string', description: 'The name of the emoji reaction to add, without surrounding colons (e.g., "thumbsup" not ":thumbsup:").' },
            },
            required: ['channel', 'timestamp', 'name'],
          },
        },
        {
          name: 'search_messages',
          description: 'Search for messages across all channels and conversations visible to the authenticated user. Supports advanced query syntax including filters for channel, sender, date range, and boolean operators. Returns matching messages with surrounding context.',
          input_schema: {
            type: 'object',
            properties: {
              query: { type: 'string', description: 'The search query string. Supports Slack search syntax: "in:#channel", "from:@user", "before:2024-01-01", "after:2024-01-01", and boolean operators AND, OR, NOT.' },
              sort: { type: 'string', description: 'How to sort the results.', enum: ['score', 'timestamp'] },
              sort_dir: { type: 'string', description: 'Sort direction.', enum: ['asc', 'desc'] },
              count: { type: 'number', description: 'Number of results to return per page. Defaults to 20, maximum is 100.' },
              page: { type: 'number', description: 'The page number of results to return, starting at 1.' },
            },
            required: ['query'],
          },
        },
        {
          name: 'get_user_info',
          description: 'Retrieve detailed profile information about a Slack workspace member. Returns the user display name, real name, email address, status, timezone, profile image URLs, and account status flags such as admin, owner, and bot indicators.',
          input_schema: {
            type: 'object',
            properties: {
              user: { type: 'string', description: 'The unique user ID (e.g., "U0123456789") of the member to look up.' },
              include_locale: { type: 'boolean', description: 'Whether to include the user locale information in the response. Defaults to false.' },
            },
            required: ['user'],
          },
        },
        {
          name: 'set_status',
          description: 'Set or update the status for the authenticated user in the Slack workspace. The status appears next to the username in the sidebar and in the user profile. Supports custom status text, an emoji icon, and an optional expiration timestamp.',
          input_schema: {
            type: 'object',
            properties: {
              status_text: { type: 'string', description: 'The text to display as the user status. Maximum 100 characters. Set to empty string to clear the status.' },
              status_emoji: { type: 'string', description: 'The emoji to display alongside the status text, using colon format (e.g., ":coffee:"). Set to empty string to remove.' },
              status_expiration: { type: 'number', description: 'Unix timestamp when the status should automatically expire and be cleared. Set to 0 for no expiration.' },
            },
            required: ['status_text'],
          },
        },
        {
          name: 'schedule_message',
          description: 'Schedule a message to be posted to a Slack channel at a specific time in the future. The message will appear as if sent at the scheduled time by the authenticated user. Scheduled messages can be listed and deleted before they are sent.',
          input_schema: {
            type: 'object',
            properties: {
              channel: { type: 'string', description: 'The ID of the channel to post the scheduled message to.' },
              text: { type: 'string', description: 'The message text to send at the scheduled time. Supports Slack mrkdwn formatting.' },
              post_at: { type: 'number', description: 'Unix timestamp for when the message should be posted. Must be in the future and within 120 days.' },
              thread_ts: { type: 'string', description: 'The timestamp of a parent message to post the scheduled message as a threaded reply.' },
            },
            required: ['channel', 'text', 'post_at'],
          },
        },
        {
          name: 'pin_message',
          description: 'Pin a message to a Slack channel so it appears in the channel pinned items list. Pinned messages remain easily accessible for all channel members. Each channel can have a maximum of 100 pinned messages. Only channel members can pin messages.',
          input_schema: {
            type: 'object',
            properties: {
              channel: { type: 'string', description: 'The ID of the channel containing the message to pin.' },
              timestamp: { type: 'string', description: 'The timestamp of the message to pin. This uniquely identifies the message within the channel.' },
            },
            required: ['channel', 'timestamp'],
          },
        },
        {
          name: 'create_reminder',
          description: 'Create a personal or channel reminder in Slack that fires at a specified time. Reminders can notify the authenticated user or an entire channel. The reminder text supports basic Slack formatting and will appear as a Slackbot notification.',
          input_schema: {
            type: 'object',
            properties: {
              text: { type: 'string', description: 'The reminder text that will be displayed when the reminder fires. Supports basic Slack formatting.' },
              time: { type: 'string', description: 'When to fire the reminder. Accepts Unix timestamp, ISO 8601 date string, or natural language (e.g., "in 15 minutes", "tomorrow at 9am").' },
              user: { type: 'string', description: 'The user ID to create the reminder for. Defaults to the authenticated user if omitted.' },
            },
            required: ['text', 'time'],
          },
        },
        {
          name: 'list_emojis',
          description: 'List all custom emoji available in the Slack workspace. Returns a mapping of emoji names to their image URLs or aliases. This includes only custom emoji uploaded by workspace members, not the standard Unicode emoji set built into Slack.',
          input_schema: {
            type: 'object',
            properties: {
              include_categories: { type: 'boolean', description: 'Whether to include emoji category groupings in the response. Defaults to false.' },
            },
            required: [],
          },
        },
        {
          name: 'get_team_info',
          description: 'Retrieve detailed information about the Slack workspace (team). Returns the workspace name, domain, email domain restrictions, icon URLs, and plan information. Useful for verifying workspace identity and available features based on the subscription plan.',
          input_schema: {
            type: 'object',
            properties: {
              team: { type: 'string', description: 'The team ID to fetch information for. If omitted, returns info for the workspace associated with the current authentication token.' },
            },
            required: [],
          },
        },
      ],
      'mcp-postgres': [
        {
          name: 'execute_query',
          description: 'Execute a SQL query against the connected PostgreSQL database and return the result set. Supports SELECT, INSERT, UPDATE, DELETE, and DDL statements. Parameterized queries are strongly recommended to prevent SQL injection vulnerabilities.',
          input_schema: {
            type: 'object',
            properties: {
              query: { type: 'string', description: 'The SQL query string to execute. Supports full PostgreSQL syntax including CTEs, window functions, and JSON operators. Use $1, $2, etc. for parameter placeholders.' },
              params: { type: 'array', items: { type: 'string' }, description: 'An ordered array of parameter values to bind to the $1, $2, etc. placeholders in the query. All values are passed as strings and cast by PostgreSQL.' },
              timeout_ms: { type: 'number', description: 'Maximum time in milliseconds to wait for the query to complete before cancelling it. Defaults to 30000 (30 seconds).' },
              read_only: { type: 'boolean', description: 'If true, execute the query in a read-only transaction. Prevents accidental data modification. Defaults to false.' },
            },
            required: ['query'],
          },
        },
        {
          name: 'list_tables',
          description: 'List all tables and views in the connected PostgreSQL database, optionally filtered by schema name. Returns table names, types (table or view), estimated row counts, and total disk size. System catalog tables are excluded by default.',
          input_schema: {
            type: 'object',
            properties: {
              schema: { type: 'string', description: 'The schema name to filter tables by. Defaults to "public". Use "*" to list tables across all schemas.' },
              include_views: { type: 'boolean', description: 'Whether to include views and materialized views in the results. Defaults to true.' },
              include_system: { type: 'boolean', description: 'Whether to include PostgreSQL system catalog tables (pg_catalog, information_schema). Defaults to false.' },
            },
            required: [],
          },
        },
        {
          name: 'describe_table',
          description: 'Get detailed schema information for a specific table or view in the PostgreSQL database. Returns column names, data types, nullability constraints, default values, primary key information, foreign key references, and index definitions.',
          input_schema: {
            type: 'object',
            properties: {
              table: { type: 'string', description: 'The name of the table or view to describe. Can be schema-qualified (e.g., "public.users") or just the table name for the default schema.' },
              include_indexes: { type: 'boolean', description: 'Whether to include index definitions and statistics in the output. Defaults to true.' },
              include_constraints: { type: 'boolean', description: 'Whether to include check constraints, unique constraints, and foreign key details. Defaults to true.' },
              include_statistics: { type: 'boolean', description: 'Whether to include column statistics such as null fraction, distinct values, and most common values. Defaults to false.' },
            },
            required: ['table'],
          },
        },
        {
          name: 'list_databases',
          description: 'List all databases available on the connected PostgreSQL server. Returns database names, owners, encoding settings, collation, and size on disk. Template databases and databases the user does not have access to may be excluded depending on permissions.',
          input_schema: {
            type: 'object',
            properties: {
              include_templates: { type: 'boolean', description: 'Whether to include template databases (template0, template1) in the results. Defaults to false.' },
              include_size: { type: 'boolean', description: 'Whether to calculate and include the disk size of each database. May be slow for large servers. Defaults to true.' },
            },
            required: [],
          },
        },
        {
          name: 'create_table',
          description: 'Create a new table in the PostgreSQL database with the specified columns, data types, and constraints. Supports primary keys, foreign keys, unique constraints, check constraints, and default values. The table name must not already exist in the target schema.',
          input_schema: {
            type: 'object',
            properties: {
              table_name: { type: 'string', description: 'The name for the new table. Can be schema-qualified (e.g., "public.users"). Must follow PostgreSQL naming rules.' },
              columns: { type: 'array', items: { type: 'object', properties: { name: { type: 'string' }, type: { type: 'string' }, nullable: { type: 'boolean' }, default_value: { type: 'string' } } }, description: 'Array of column definitions. Each column must have a name and PostgreSQL data type (e.g., "text", "integer", "timestamptz", "jsonb").' },
              primary_key: { type: 'array', items: { type: 'string' }, description: 'Array of column names that form the primary key. May be a single column or composite key.' },
              if_not_exists: { type: 'boolean', description: 'If true, do not raise an error if the table already exists. Defaults to false.' },
              schema: { type: 'string', description: 'The schema to create the table in. Defaults to "public".' },
            },
            required: ['table_name', 'columns'],
          },
        },
        {
          name: 'insert_row',
          description: 'Insert one or more rows into a PostgreSQL table. Supports inserting a single row or batch inserting multiple rows in a single statement. Returns the inserted rows if the table has a RETURNING clause. Column values are automatically type-cast by PostgreSQL.',
          input_schema: {
            type: 'object',
            properties: {
              table: { type: 'string', description: 'The name of the table to insert into. Can be schema-qualified (e.g., "public.users").' },
              rows: { type: 'array', items: { type: 'object' }, description: 'An array of row objects to insert. Each object maps column names to their values. All rows must have the same set of keys.' },
              on_conflict: { type: 'string', description: 'Conflict resolution strategy. Specify a column name or constraint for ON CONFLICT handling (e.g., "id" for upsert behavior).', enum: ['error', 'ignore', 'update'] },
              returning: { type: 'array', items: { type: 'string' }, description: 'Array of column names to return from the inserted rows. Use ["*"] to return all columns.' },
            },
            required: ['table', 'rows'],
          },
        },
      ],
      'mcp-browser': [
        {
          name: 'navigate',
          description: 'Navigate the browser to a specified URL and wait for the page to fully load. Supports HTTP and HTTPS protocols. The page load is considered complete when the document reaches the "load" event or the specified timeout is exceeded.',
          input_schema: {
            type: 'object',
            properties: {
              url: { type: 'string', description: 'The full URL to navigate to, including protocol (e.g., "https://example.com"). Relative URLs are resolved against the current page.' },
              wait_until: { type: 'string', description: 'The event to wait for before considering navigation complete.', enum: ['load', 'domcontentloaded', 'networkidle0', 'networkidle2'] },
              timeout_ms: { type: 'number', description: 'Maximum time in milliseconds to wait for the page to load. Defaults to 30000 (30 seconds).' },
              referer: { type: 'string', description: 'The referer URL to include in the navigation request headers.' },
            },
            required: ['url'],
          },
        },
        {
          name: 'screenshot',
          description: 'Capture a screenshot of the current browser page or a specific element on the page. Returns the image as base64-encoded PNG or JPEG data. Supports full-page screenshots that capture the entire scrollable area, or viewport-only captures.',
          input_schema: {
            type: 'object',
            properties: {
              selector: { type: 'string', description: 'CSS selector of a specific element to screenshot. If omitted, captures the entire viewport or full page.' },
              full_page: { type: 'boolean', description: 'Whether to capture the entire scrollable page instead of just the visible viewport. Defaults to false.' },
              format: { type: 'string', description: 'Image format for the screenshot output.', enum: ['png', 'jpeg', 'webp'] },
              quality: { type: 'number', description: 'Image quality for JPEG/WebP format, from 0 to 100. Ignored for PNG format. Defaults to 80.' },
              omit_background: { type: 'boolean', description: 'Whether to hide the default white background and capture with transparency (PNG only). Defaults to false.' },
            },
            required: [],
          },
        },
        {
          name: 'click',
          description: 'Click on an element in the current browser page identified by a CSS selector. Waits for the element to be visible and clickable before performing the click action. Supports left, right, and middle mouse button clicks as well as modifier keys.',
          input_schema: {
            type: 'object',
            properties: {
              selector: { type: 'string', description: 'CSS selector identifying the element to click on. Must match exactly one visible element on the page.' },
              button: { type: 'string', description: 'The mouse button to use for the click action.', enum: ['left', 'right', 'middle'] },
              click_count: { type: 'number', description: 'Number of times to click. Use 2 for double-click, 3 for triple-click. Defaults to 1.' },
              delay_ms: { type: 'number', description: 'Time in milliseconds to wait between mousedown and mouseup events. Defaults to 0.' },
              modifiers: { type: 'array', items: { type: 'string', enum: ['Alt', 'Control', 'Meta', 'Shift'] }, description: 'Keyboard modifier keys to hold during the click.' },
            },
            required: ['selector'],
          },
        },
        {
          name: 'type',
          description: 'Type text into an input field or editable element on the current browser page. The element is identified by a CSS selector and must be focusable. Each character is typed individually with configurable delay to simulate realistic human typing speed.',
          input_schema: {
            type: 'object',
            properties: {
              selector: { type: 'string', description: 'CSS selector identifying the input field or editable element to type into.' },
              text: { type: 'string', description: 'The text string to type into the element. Special characters are typed as-is.' },
              delay_ms: { type: 'number', description: 'Delay in milliseconds between each keystroke to simulate human typing speed. Defaults to 0 for instant typing.' },
              clear_first: { type: 'boolean', description: 'Whether to clear the existing content of the input field before typing new text. Defaults to false.' },
              press_enter: { type: 'boolean', description: 'Whether to press the Enter key after typing the text. Useful for form submission. Defaults to false.' },
            },
            required: ['selector', 'text'],
          },
        },
        {
          name: 'scroll',
          description: 'Scroll the browser page or a specific scrollable element by a given amount in pixels or to a specific element. Supports both vertical and horizontal scrolling. The scroll operation is performed smoothly to simulate natural user interaction.',
          input_schema: {
            type: 'object',
            properties: {
              direction: { type: 'string', description: 'The direction to scroll the page or element.', enum: ['up', 'down', 'left', 'right'] },
              amount: { type: 'number', description: 'The number of pixels to scroll in the specified direction. Defaults to one viewport height for vertical or viewport width for horizontal.' },
              selector: { type: 'string', description: 'CSS selector of a specific scrollable element. If omitted, scrolls the main page document.' },
              behavior: { type: 'string', description: 'The scrolling behavior to use.', enum: ['auto', 'smooth', 'instant'] },
            },
            required: ['direction'],
          },
        },
        {
          name: 'get_text',
          description: 'Extract the visible text content from one or more elements on the current browser page, identified by a CSS selector. Returns the inner text of all matching elements, stripped of HTML tags. Useful for reading content from specific sections of a page.',
          input_schema: {
            type: 'object',
            properties: {
              selector: { type: 'string', description: 'CSS selector identifying the element(s) to extract text from. If multiple elements match, text from all matches is concatenated.' },
              include_hidden: { type: 'boolean', description: 'Whether to include text from hidden elements (display:none, visibility:hidden). Defaults to false.' },
              trim: { type: 'boolean', description: 'Whether to trim leading and trailing whitespace from the extracted text. Defaults to true.' },
            },
            required: ['selector'],
          },
        },
        {
          name: 'wait_for_element',
          description: 'Wait for an element matching a CSS selector to appear in the DOM of the current browser page. Configurable to wait for the element to be visible, hidden, attached to DOM, or detached. Times out with an error if the condition is not met within the specified duration.',
          input_schema: {
            type: 'object',
            properties: {
              selector: { type: 'string', description: 'CSS selector for the element to wait for.' },
              state: { type: 'string', description: 'The state to wait for the element to reach.', enum: ['visible', 'hidden', 'attached', 'detached'] },
              timeout_ms: { type: 'number', description: 'Maximum time in milliseconds to wait for the element. Defaults to 30000 (30 seconds).' },
              poll_interval_ms: { type: 'number', description: 'How frequently in milliseconds to check for the element. Lower values are more responsive but use more CPU. Defaults to 100.' },
            },
            required: ['selector'],
          },
        },
        {
          name: 'evaluate',
          description: 'Execute arbitrary JavaScript code in the context of the current browser page and return the result. The code has access to the full DOM API, window object, and any JavaScript libraries loaded on the page. Return values are serialized as JSON.',
          input_schema: {
            type: 'object',
            properties: {
              expression: { type: 'string', description: 'The JavaScript code to execute in the page context. Can be a single expression or multiple statements. The last expression value is returned.' },
              await_promise: { type: 'boolean', description: 'Whether to await the result if the expression returns a Promise. Defaults to true.' },
              timeout_ms: { type: 'number', description: 'Maximum time in milliseconds to wait for the JavaScript execution to complete. Defaults to 30000.' },
              return_by_value: { type: 'boolean', description: 'Whether to return the result by value (serialized) or by reference (handle). Defaults to true (by value).' },
            },
            required: ['expression'],
          },
        },
        {
          name: 'get_html',
          description: 'Retrieve the HTML content of the current browser page or a specific element identified by a CSS selector. Returns either the outer HTML (including the element itself) or inner HTML (only the element children). Useful for inspecting page structure and content.',
          input_schema: {
            type: 'object',
            properties: {
              selector: { type: 'string', description: 'CSS selector identifying the element to get HTML from. If omitted, returns the full page document HTML.' },
              outer: { type: 'boolean', description: 'Whether to return outer HTML (includes the matched element tag) or inner HTML (only children). Defaults to false (inner HTML).' },
              pretty_print: { type: 'boolean', description: 'Whether to format the HTML output with indentation for readability. Defaults to false.' },
            },
            required: [],
          },
        },
        {
          name: 'fill_form',
          description: 'Fill out a web form on the current browser page by providing a mapping of field selectors to values. Supports text inputs, selects, checkboxes, radio buttons, and file inputs. Optionally submits the form after filling all fields.',
          input_schema: {
            type: 'object',
            properties: {
              fields: { type: 'object', description: 'A mapping of CSS selectors to values. Each key is a selector for a form field, and the value is what to enter or select. For checkboxes, use true/false. For selects, use the option value.' },
              submit: { type: 'boolean', description: 'Whether to submit the form after filling all fields. Clicks the submit button or triggers form submission. Defaults to false.' },
              submit_selector: { type: 'string', description: 'CSS selector for the submit button to click. Only used when submit is true. Defaults to the first submit button in the form.' },
              clear_first: { type: 'boolean', description: 'Whether to clear existing field values before filling in new values. Defaults to true.' },
            },
            required: ['fields'],
          },
        },
      ],
    }
    const tools = server?.id ? (toolsByServer[server.id] || [
      { name: 'execute', description: 'Execute the primary action for this server. Accepts arbitrary input parameters and returns the result of the operation.', input_schema: { type: 'object', properties: { action: { type: 'string', description: 'The action to execute.' }, params: { type: 'object', description: 'Additional parameters for the action.' } }, required: ['action'] } },
      { name: 'query', description: 'Query for information from this server. Returns structured data matching the specified criteria and filters.', input_schema: { type: 'object', properties: { query: { type: 'string', description: 'The query string to search for.' }, limit: { type: 'number', description: 'Maximum number of results to return.' } }, required: ['query'] } },
      { name: 'list', description: 'List all available items from this server. Returns a paginated collection of items with metadata.', input_schema: { type: 'object', properties: { page: { type: 'number', description: 'Page number to retrieve.' }, per_page: { type: 'number', description: 'Number of items per page.' } }, required: [] } },
      { name: 'get', description: 'Get a specific item by its unique identifier. Returns the full details and metadata of the requested item.', input_schema: { type: 'object', properties: { id: { type: 'string', description: 'The unique identifier of the item to retrieve.' } }, required: ['id'] } },
    ]) : []
    return {
      tools,
      resources: server?.id === 'mcp-filesystem' ? [
        { uri: 'file:///', name: 'Root filesystem', description: 'Access to the filesystem' },
      ] : [],
      prompts: server?.id === 'mcp-github' ? [
        { name: 'summarize_pr', description: 'Summarize a pull request' },
        { name: 'review_code', description: 'Review code changes' },
      ] : [],
      server_name: server?.name || 'Unknown',
    }
  },

  // MCP OAuth
  'start_mcp_oauth_browser_flow': () => ({ flow_id: generateId() }),
  'poll_mcp_oauth_browser_status': () => ({ status: 'pending' }),
  'cancel_mcp_oauth_browser_flow': () => null,
  'test_mcp_oauth_connection': () => ({ success: true, message: 'Connection successful (demo)' }),
  'revoke_mcp_oauth_tokens': () => {
    toast.success('OAuth tokens revoked (demo)')
    return null
  },

  // ============================================================================
  // Strategies
  // ============================================================================
  'list_strategies': () => mockData.strategies,
  'get_strategy': (args) => mockData.strategies.find(s => s.id === args?.strategyId || s.id === args?.id),
  'create_strategy': (args) => {
    const newStrategy = {
      id: `strategy-${generateId()}`,
      name: args?.name || 'New Strategy',
      parent: args?.parent || null,
      allowed_models: args?.allowedModels || { mode: 'all' as const, models: [] },
      auto_config: args?.autoConfig || null,
      rate_limits: args?.rateLimits || [],
    }
    mockData.strategies.push(newStrategy)
    toast.success(`Strategy "${args?.name}" created (demo)`)
    return newStrategy
  },
  'update_strategy': (args) => {
    const strategy = mockData.strategies.find(s => s.id === args?.strategyId || s.id === args?.id)
    if (strategy) {
      // Handle individual field updates (the API passes individual fields, not an updates object)
      if (args?.name !== undefined && args.name !== null) strategy.name = args.name
      if (args?.allowedModels !== undefined && args.allowedModels !== null) strategy.allowed_models = args.allowedModels
      if (args?.autoConfig !== undefined) strategy.auto_config = args.autoConfig
      if (args?.rateLimits !== undefined && args.rateLimits !== null) strategy.rate_limits = args.rateLimits
      if (args?.freeTierOnly !== undefined && args.freeTierOnly !== null) (strategy as Record<string, unknown>).free_tier_only = args.freeTierOnly
      if (args?.freeTierFallback !== undefined && args.freeTierFallback !== null) (strategy as Record<string, unknown>).free_tier_fallback = args.freeTierFallback
    }
    return null
  },
  'delete_strategy': (args) => {
    const idx = mockData.strategies.findIndex(s => s.id === args?.id)
    if (idx !== -1) mockData.strategies.splice(idx, 1)
    toast.success('Strategy deleted (demo)')
    return null
  },

  // ============================================================================
  // Models
  // ============================================================================
  'list_all_models': () => mockData.models,
  'get_cached_models': () => mockData.models,
  'refresh_models_incremental': (_args?: { force?: boolean }) => {},
  'list_all_models_detailed': () => {
    const pricingMap: Record<string, { input: number; output: number; source: string }> = {
      'gpt-4o': { input: 2.50, output: 10.00, source: 'catalog' },
      'gpt-4o-mini': { input: 0.15, output: 0.60, source: 'catalog' },
      'gpt-4-turbo': { input: 10.00, output: 30.00, source: 'catalog' },
      'o1-preview': { input: 15.00, output: 60.00, source: 'catalog' },
      'o1-mini': { input: 3.00, output: 12.00, source: 'catalog' },
      'claude-3-5-sonnet-20241022': { input: 3.00, output: 15.00, source: 'catalog' },
      'claude-3-5-haiku-20241022': { input: 0.80, output: 4.00, source: 'catalog' },
      'claude-3-opus-20240229': { input: 15.00, output: 75.00, source: 'catalog' },
      'gemini-1.5-pro': { input: 1.25, output: 5.00, source: 'catalog' },
      'gemini-1.5-flash': { input: 0.075, output: 0.30, source: 'catalog' },
    }
    const providerTypeMap: Record<string, string> = {
      'openai-primary': 'openai',
      'anthropic-main': 'anthropic',
      'ollama-local': 'ollama',
      'gemini-google': 'gemini',
      'groq-fast': 'groq',
      'openrouter-backup': 'openrouter',
    }
    return mockData.models.map(m => {
      const pricing = pricingMap[m.id]
      return {
        model_id: m.id,
        provider_instance: m.provider,
        provider_type: providerTypeMap[m.provider] || 'unknown',
        capabilities: ['chat', 'completion'],
        context_window: m.context_length,
        supports_streaming: true,
        input_price_per_million: pricing?.input ?? null,
        output_price_per_million: pricing?.output ?? null,
        pricing_source: pricing?.source ?? null,
        parameter_count: null,
      }
    })
  },
  'list_provider_models': (args) => {
    const provider = args?.instanceName || args?.provider
    return mockData.models.filter(m => m.provider === provider)
  },

  // ============================================================================
  // Feature Support Matrix
  // ============================================================================
  'get_provider_feature_support': (args): ProviderFeatureSupport => ({
    provider_type: 'openai',
    provider_instance: args?.instanceName || 'openai',
    endpoints: [
      { name: 'Chat Completions', endpoint: '/v1/chat/completions', support: 'supported', notes: 'Send messages and receive AI responses' },
      { name: 'Completions (legacy)', endpoint: '/v1/completions', support: 'supported', notes: 'Converted to chat completions internally by LocalRouter' },
      { name: 'Streaming', endpoint: '/v1/chat/completions', support: 'supported', notes: 'Server-sent events for real-time token streaming' },
      { name: 'Embeddings', endpoint: '/v1/embeddings', support: 'supported', notes: 'Generate vector embeddings for text' },
      { name: 'Image Generation', endpoint: '/v1/images/generations', support: 'supported', notes: 'DALL-E 3 and DALL-E 2 image generation' },
      { name: 'Audio Transcription', endpoint: '/v1/audio/transcriptions', support: 'supported', notes: 'Whisper for speech-to-text, TTS-1/TTS-1-HD for text-to-speech' },
      { name: 'Audio Speech (TTS)', endpoint: '/v1/audio/speech', support: 'supported', notes: 'Whisper for speech-to-text, TTS-1/TTS-1-HD for text-to-speech' },
      { name: 'Moderations', endpoint: '/v1/moderations', support: 'not_implemented', notes: 'OpenAI supports natively via text-moderation-latest; LocalRouter proxy not yet built' },
      { name: 'Responses API', endpoint: '/v1/responses', support: 'not_implemented', notes: 'OpenAI supports natively; LocalRouter proxy not yet built' },
      { name: 'Batch Processing', endpoint: '/v1/batches', support: 'not_implemented', notes: 'OpenAI supports native async batches; LocalRouter proxy not yet built' },
      { name: 'Realtime (WebSocket)', endpoint: '/v1/realtime', support: 'not_implemented', notes: 'WebSocket-based real-time audio/text streaming not yet available in LocalRouter' },
    ],
    model_features: [
      { name: 'Function Calling', support: 'supported', notes: 'GPT-4o, GPT-4 Turbo, and GPT-3.5 Turbo support tool calling' },
      { name: 'Vision', support: 'supported', notes: 'GPT-4o and GPT-4 Turbo can process images' },
      { name: 'Structured Outputs', support: 'supported', notes: 'GPT-4o supports strict JSON schema enforcement via response_format' },
      { name: 'JSON Mode', support: 'supported', notes: 'All GPT-4 and GPT-3.5 Turbo models support JSON output mode' },
      { name: 'Log Probabilities', support: 'supported', notes: 'Available on GPT-4o and GPT-3.5 Turbo via logprobs parameter' },
      { name: 'Reasoning Tokens', support: 'partial', notes: 'Only o1-preview and o1-mini models use reasoning tokens; other models do not' },
      { name: 'Extended Thinking', support: 'not_supported', notes: 'OpenAI does not support extended thinking; this is an Anthropic feature' },
      { name: 'Thinking Level', support: 'not_supported', notes: 'OpenAI does not support thinking level; this is a Gemini feature' },
      { name: 'Prompt Caching', support: 'not_supported', notes: 'OpenAI does not support server-side prompt caching' },
    ],
    optimization_features: [
      { name: 'Guardrails', support: 'supported', notes: 'Content safety scanning on chat/completion requests' },
      { name: 'Prompt Compression', support: 'supported', notes: 'LLMLingua-2 token-level compression for chat requests' },
      { name: 'JSON Repair', support: 'supported', notes: 'Automatic fix of malformed JSON responses' },
      { name: 'RouteLLM Routing', support: 'supported', notes: 'Strong/weak model routing based on request complexity' },
      { name: 'Secret Scanning', support: 'supported', notes: 'Detect potential secrets in outbound requests' },
      { name: 'Rate Limiting', support: 'supported', notes: 'Available for all endpoints' },
      { name: 'Model Firewall', support: 'supported', notes: 'Available for all LLM endpoints' },
      { name: 'Generation Tracking', support: 'supported', notes: 'Available for all endpoints' },
      { name: 'Cost Calculation', support: 'supported', notes: 'Based on catalog pricing data' },
    ],
  }),
  'get_all_provider_feature_support': (): ProviderFeatureSupport[] => {
    const mockHandlerFn = mockHandlers['get_provider_feature_support'] as (args?: InvokeArgs) => ProviderFeatureSupport
    const openai = mockHandlerFn({ instanceName: 'openai' })

    const anthropic: ProviderFeatureSupport = {
      ...openai,
      provider_type: 'anthropic',
      provider_instance: 'anthropic',
      endpoints: openai.endpoints.map(e => {
        if (e.name === 'Embeddings') return { ...e, support: 'not_supported' as const, notes: 'Anthropic does not offer an embeddings API' }
        if (e.name === 'Image Generation') return { ...e, support: 'not_supported' as const, notes: 'Anthropic does not offer image generation' }
        if (e.name === 'Audio Transcription' || e.name === 'Audio Speech (TTS)') return { ...e, support: 'not_implemented' as const, notes: 'Anthropic does not offer audio endpoints' }
        return e
      }),
      model_features: openai.model_features.map(f => {
        if (f.name === 'Extended Thinking') return { ...f, support: 'partial' as const, notes: 'Only Claude 4.5 Sonnet/Opus support extended thinking with configurable budget (1K\u201399K tokens); other Claude models do not' }
        if (f.name === 'Prompt Caching') return { ...f, support: 'supported' as const, notes: 'Anthropic cache_control blocks reduce cost for repeated prefixes' }
        if (f.name === 'Reasoning Tokens') return { ...f, support: 'not_supported' as const, notes: 'Anthropic uses extended thinking instead of reasoning tokens' }
        if (f.name === 'Log Probabilities') return { ...f, support: 'not_supported' as const, notes: 'Anthropic API does not expose token log probabilities' }
        if (f.name === 'Function Calling') return { ...f, notes: 'All Claude 4.x and 3.5 models support tool use' }
        if (f.name === 'Vision') return { ...f, notes: 'All Claude 4.x and 3.5 models can process images' }
        if (f.name === 'Structured Outputs') return { ...f, notes: 'Claude supports JSON schema enforcement via tool use' }
        return f
      }),
    }

    const gemini: ProviderFeatureSupport = {
      ...openai,
      provider_type: 'gemini',
      provider_instance: 'gemini',
      endpoints: openai.endpoints.map(e => {
        if (e.name === 'Embeddings') return { ...e, notes: 'Gemini text-embedding models; single-input only (no batch)' }
        return e
      }),
      model_features: openai.model_features.map(f => {
        if (f.name === 'Thinking Level') return { ...f, support: 'partial' as const, notes: 'Only Gemini 2.0 Flash Thinking and Gemini 3 models support thinking level (low/medium/high); other Gemini models do not' }
        if (f.name === 'Reasoning Tokens') return { ...f, support: 'not_supported' as const, notes: 'Gemini uses thinking level instead of reasoning tokens' }
        if (f.name === 'Extended Thinking') return { ...f, support: 'not_supported' as const, notes: 'Gemini does not support extended thinking; uses thinking level instead' }
        if (f.name === 'Prompt Caching') return { ...f, support: 'not_supported' as const, notes: 'Gemini does not support server-side prompt caching via the API' }
        if (f.name === 'Log Probabilities') return { ...f, support: 'not_supported' as const, notes: 'Gemini API does not expose token log probabilities' }
        if (f.name === 'Function Calling') return { ...f, notes: 'Gemini Pro and Flash models support function calling' }
        if (f.name === 'Vision') return { ...f, notes: 'Gemini Pro and Flash models can process images and video' }
        if (f.name === 'JSON Mode') return { ...f, notes: 'Gemini supports JSON output via response MIME type' }
        return f
      }),
    }

    const ollama: ProviderFeatureSupport = {
      ...openai,
      provider_type: 'ollama',
      provider_instance: 'ollama',
      endpoints: openai.endpoints.map(e => {
        if (e.name === 'Image Generation') return { ...e, support: 'not_supported' as const, notes: 'Ollama does not support image generation' }
        if (e.name === 'Audio Transcription' || e.name === 'Audio Speech (TTS)') return { ...e, support: 'not_supported' as const, notes: 'Ollama does not support audio endpoints' }
        if (e.name === 'Embeddings') return { ...e, notes: 'Ollama supports embeddings for models that have embedding capabilities' }
        return e
      }),
      model_features: openai.model_features.map(f => {
        if (f.name === 'Structured Outputs') return { ...f, support: 'not_supported' as const, notes: 'Ollama does not support strict JSON schema enforcement' }
        if (f.name === 'Log Probabilities') return { ...f, support: 'not_supported' as const, notes: 'Ollama API does not expose token log probabilities' }
        if (f.name === 'Reasoning Tokens') return { ...f, support: 'not_supported' as const, notes: 'Ollama does not support reasoning token models' }
        if (f.name === 'Extended Thinking') return { ...f, support: 'not_supported' as const, notes: 'Ollama does not support extended thinking' }
        if (f.name === 'Thinking Level') return { ...f, support: 'not_supported' as const, notes: 'Ollama does not support thinking level control' }
        if (f.name === 'Prompt Caching') return { ...f, support: 'not_supported' as const, notes: 'Ollama does not support server-side prompt caching' }
        if (f.name === 'Function Calling') return { ...f, notes: 'Depends on the model; some Ollama models support tool calling' }
        if (f.name === 'Vision') return { ...f, notes: 'Depends on the model; LLaVA and similar multimodal models support vision' }
        if (f.name === 'JSON Mode') return { ...f, notes: 'Ollama supports JSON output mode for compatible models' }
        return f
      }),
    }

    return [openai, anthropic, gemini, ollama]
  },
  'get_feature_endpoint_matrix': (): FeatureEndpointMatrix => ({
    endpoints: ['Chat', 'Completions', 'Embeddings', 'Images', 'Audio', 'Moderations', 'Responses', 'Batches', 'Realtime'],
    client_modes: ['LLM Only', 'MCP Only', 'MCP & LLM', 'MCP via LLM'],
    feature_rows: [
      { feature_name: 'Guardrails', cells: [
        { support: 'supported', notes: null }, { support: 'supported', notes: null }, { support: 'not_supported', notes: null }, { support: 'not_supported', notes: null }, { support: 'not_supported', notes: null }, { support: 'not_supported', notes: null }, { support: 'translated', notes: 'Via translation to chat completions' }, { support: 'translated', notes: 'Per-request in translated batch mode' }, { support: 'not_supported', notes: null },
      ]},
      { feature_name: 'Prompt Compression', cells: [
        { support: 'supported', notes: null }, { support: 'not_supported', notes: null }, { support: 'not_supported', notes: null }, { support: 'not_supported', notes: null }, { support: 'not_supported', notes: null }, { support: 'not_supported', notes: null }, { support: 'translated', notes: 'Via translation to chat completions' }, { support: 'translated', notes: 'Per-request in translated batch mode' }, { support: 'not_supported', notes: null },
      ]},
      { feature_name: 'JSON Repair', cells: [
        { support: 'supported', notes: null }, { support: 'supported', notes: null }, { support: 'not_supported', notes: null }, { support: 'not_supported', notes: null }, { support: 'not_supported', notes: null }, { support: 'not_supported', notes: null }, { support: 'translated', notes: 'Via translation to chat completions' }, { support: 'not_supported', notes: null }, { support: 'not_supported', notes: null },
      ]},
      { feature_name: 'RouteLLM Routing', cells: [
        { support: 'supported', notes: null }, { support: 'not_supported', notes: null }, { support: 'not_supported', notes: null }, { support: 'not_supported', notes: null }, { support: 'not_supported', notes: null }, { support: 'not_supported', notes: null }, { support: 'not_supported', notes: null }, { support: 'not_supported', notes: null }, { support: 'not_supported', notes: null },
      ]},
      { feature_name: 'Secret Scanning', cells: [
        { support: 'supported', notes: null }, { support: 'supported', notes: null }, { support: 'not_supported', notes: null }, { support: 'not_supported', notes: null }, { support: 'partial', notes: 'TTS input text only; audio binary not scannable' }, { support: 'not_supported', notes: null }, { support: 'translated', notes: 'Via translation to chat completions' }, { support: 'translated', notes: 'Per-request in translated batch mode' }, { support: 'not_supported', notes: null },
      ]},
      { feature_name: 'Rate Limiting', cells: [
        { support: 'supported', notes: null }, { support: 'supported', notes: null }, { support: 'supported', notes: null }, { support: 'supported', notes: null }, { support: 'supported', notes: null }, { support: 'supported', notes: null }, { support: 'supported', notes: null }, { support: 'supported', notes: null }, { support: 'partial', notes: 'Connection-time only, no per-message' },
      ]},
      { feature_name: 'Model Firewall', cells: [
        { support: 'supported', notes: null }, { support: 'supported', notes: null }, { support: 'not_supported', notes: null }, { support: 'not_supported', notes: null }, { support: 'supported', notes: null }, { support: 'not_supported', notes: null }, { support: 'supported', notes: null }, { support: 'partial', notes: 'Approve at batch creation time' }, { support: 'supported', notes: null },
      ]},
      { feature_name: 'Generation Tracking', cells: [
        { support: 'supported', notes: null }, { support: 'supported', notes: null }, { support: 'supported', notes: null }, { support: 'supported', notes: null }, { support: 'supported', notes: null }, { support: 'supported', notes: null }, { support: 'supported', notes: null }, { support: 'supported', notes: null }, { support: 'partial', notes: 'Per-session aggregation' },
      ]},
      { feature_name: 'Cost Calculation', cells: [
        { support: 'supported', notes: null }, { support: 'supported', notes: null }, { support: 'supported', notes: null }, { support: 'not_supported', notes: null }, { support: 'supported', notes: null }, { support: 'not_supported', notes: null }, { support: 'supported', notes: null }, { support: 'supported', notes: null }, { support: 'supported', notes: null },
      ]},
    ],
    mode_rows: [
      { name: 'Chat Completions', cells: [{ support: 'supported', notes: null }, { support: 'not_supported', notes: null }, { support: 'supported', notes: null }, { support: 'supported', notes: null }] },
      { name: 'Completions', cells: [{ support: 'supported', notes: null }, { support: 'not_supported', notes: null }, { support: 'supported', notes: null }, { support: 'not_supported', notes: null }] },
      { name: 'Embeddings', cells: [{ support: 'supported', notes: null }, { support: 'not_supported', notes: null }, { support: 'supported', notes: null }, { support: 'not_supported', notes: null }] },
      { name: 'Image Generation', cells: [{ support: 'supported', notes: null }, { support: 'not_supported', notes: null }, { support: 'supported', notes: null }, { support: 'not_supported', notes: null }] },
      { name: 'Audio (STT/TTS)', cells: [{ support: 'supported', notes: null }, { support: 'not_supported', notes: null }, { support: 'supported', notes: null }, { support: 'not_supported', notes: null }] },
      { name: 'MCP Gateway', cells: [{ support: 'not_supported', notes: null }, { support: 'supported', notes: null }, { support: 'supported', notes: null }, { support: 'not_supported', notes: null }] },
      { name: 'MCP \u2192 LLM Tools', cells: [{ support: 'not_supported', notes: null }, { support: 'not_supported', notes: null }, { support: 'not_supported', notes: null }, { support: 'supported', notes: null }] },
      { name: 'Guardrails', cells: [{ support: 'supported', notes: null }, { support: 'not_supported', notes: null }, { support: 'supported', notes: null }, { support: 'supported', notes: null }] },
      { name: 'RouteLLM', cells: [{ support: 'supported', notes: null }, { support: 'not_supported', notes: null }, { support: 'supported', notes: null }, { support: 'supported', notes: null }] },
      { name: 'Secret Scanning', cells: [{ support: 'supported', notes: null }, { support: 'not_supported', notes: null }, { support: 'supported', notes: null }, { support: 'supported', notes: null }] },
      { name: 'Context Management', cells: [{ support: 'not_supported', notes: null }, { support: 'supported', notes: null }, { support: 'supported', notes: null }, { support: 'supported', notes: null }] },
    ],
  }),

  // ============================================================================
  // Stats & Health
  // ============================================================================
  'get_aggregate_stats': () => {
    // Return stats with slight variations to make it feel alive
    const base = mockData.stats
    const variation = Math.floor(Math.random() * 20) - 10
    return {
      ...base,
      total_requests: base.total_requests + Math.floor(Math.random() * 50),
      total_tokens: base.total_tokens + Math.floor(Math.random() * 10000),
      requests_today: base.requests_today + variation,
      tokens_today: base.tokens_today + Math.floor(Math.random() * 5000),
      cost_today: Number((base.cost_today + Math.random() * 2).toFixed(2)),
    }
  },
  'get_feature_stats': () => ({
    routellm_strong: 847,
    routellm_weak: 1253,
    json_repairs: 34,
    compression_tokens_saved: 128400,
    compression_cost_saved_micros: 385200,
    context_mgmt_tokens_saved: 256800,
  }),
  'get_global_metrics': () => generateMockGraphData("Total Requests", 300, 200),
  'get_api_key_metrics': () => generateMockGraphData("API Requests", 150, 100),
  'get_provider_metrics': () => generateMockGraphData("Provider Requests", 200, 150),
  'get_model_metrics': () => generateMockGraphData("Model Requests", 180, 120),
  'get_strategy_metrics': () => generateMockGraphData("Strategy Requests", 100, 80),
  'get_global_mcp_metrics': () => generateMockGraphData("MCP Requests", 80, 60),
  'get_client_mcp_metrics': () => generateMockGraphData("Client MCP", 50, 40),
  'get_mcp_server_metrics': () => generateMockGraphData("Server Requests", 60, 50),
  'get_mcp_method_breakdown': () => generateMockGraphData("Method Calls", 40, 30),
  'get_health_cache': () => mockData.healthCache,
  'get_periodic_health_enabled': () => true,
  'set_periodic_health_enabled': (_args?: { enabled?: boolean }) => {},
  'refresh_all_health': () => {
    toast.info('Health check initiated (demo)')
    // Emit health events for both providers and MCP servers
    emitProviderHealthEvents()
    emitMcpHealthEvents()
    return null
  },
  'start_provider_health_checks': () => {
    emitProviderHealthEvents()
    return null
  },
  'check_single_provider_health': (args) => {
    const instanceName = args?.instanceName || args?.instance_name
    if (instanceName) {
      const provider = mockData.providers.find(p => p.instance_name === instanceName)
      const healthCache = mockData.healthCache.providers as Record<string, { status: string; latency_ms: number | null }>
      const health = healthCache[instanceName]
      setTimeout(() => {
        emit('provider-health-check', {
          provider_name: instanceName,
          status: provider?.enabled ? (health?.status || 'healthy') : 'disabled',
          latency_ms: health?.latency_ms || Math.floor(Math.random() * 300) + 100,
          error_message: null,
        })
      }, 100)
    }
    return { status: 'healthy', latency_ms: Math.floor(Math.random() * 300) + 100 }
  },
  'get_active_connections': () => {
    // Return active connections with fresh timestamps
    const now = new Date()
    return mockData.activeConnections.map((conn, i) => ({
      ...conn,
      connected_at: new Date(now.getTime() - (3600000 * (i + 1))).toISOString(),
      last_activity: new Date(now.getTime() - (30000 * (i + 1))).toISOString(),
      requests_count: conn.requests_count + Math.floor(Math.random() * 10),
    }))
  },

  // ============================================================================
  // OAuth
  // ============================================================================
  'list_oauth_clients': () => mockData.oauthClients,
  'list_oauth_credentials': () => mockData.oauthCredentials,
  'start_oauth_flow': () => ({ flow_id: generateId() }),
  'poll_oauth_status': () => ({ status: 'pending' }),
  'cancel_oauth_flow': () => null,

  // ============================================================================
  // Skills
  // ============================================================================
  'list_skills': () => mockData.skills,
  'get_skill': (args) => mockData.skills.find(s => s.name === args?.name),
  'get_skill_files': (args) => {
    const skill = mockData.skills.find(s => s.name === args?.name || s.name === args?.skillName)
    // Return skill-specific files
    const filesBySkill: Record<string, { name: string; category: string; content_preview: string }[]> = {
      'web-search': [
        { name: "search.js", category: "script", content_preview: "export async function search(query) {\n  const results = await fetch(`https://api.search.com?q=${query}`);\n  return results.json();\n}" },
        { name: "summarize.js", category: "script", content_preview: "export function summarize(results) {\n  return results.map(r => `${r.title}: ${r.snippet}`).join('\\n');\n}" },
        { name: "config.json", category: "reference", content_preview: '{\n  "api_endpoint": "https://api.search.com",\n  "max_results": 10\n}' },
        { name: "icon.svg", category: "asset", content_preview: '<svg>...</svg>' },
      ],
      'code-review': [
        { name: "analyze.js", category: "script", content_preview: "export function analyzeCode(code) {\n  const issues = [];\n  // Security checks\n  if (code.includes('eval(')) issues.push('Avoid eval()');\n  return issues;\n}" },
        { name: "security.js", category: "script", content_preview: "export function checkSecurity(code) {\n  const vulnerabilities = [];\n  // Check for SQL injection\n  // Check for XSS\n  return vulnerabilities;\n}" },
        { name: "suggestions.js", category: "script", content_preview: "export function getSuggestions(code, analysis) {\n  return analysis.issues.map(i => ({ issue: i, fix: suggestFix(i) }));\n}" },
        { name: "patterns.js", category: "script", content_preview: "export const securityPatterns = [\n  /eval\\(/g,\n  /innerHTML\\s*=/g,\n];" },
        { name: "eslint-config.json", category: "reference", content_preview: '{\n  "rules": {\n    "no-eval": "error"\n  }\n}' },
        { name: "types.d.ts", category: "reference", content_preview: "interface AnalysisResult {\n  issues: Issue[];\n  suggestions: Suggestion[];\n}" },
      ],
      'doc-writer': [
        { name: "generate.js", category: "script", content_preview: "export function generateDocs(code) {\n  const functions = parseFunctions(code);\n  return functions.map(f => formatDoc(f));\n}" },
        { name: "markdown.js", category: "script", content_preview: "export function toMarkdown(docs) {\n  return docs.map(d => `## ${d.name}\\n${d.description}`);\n}" },
        { name: "template.md", category: "reference", content_preview: "# {{name}}\n\n{{description}}\n\n## Usage\n{{usage}}" },
      ],
      'test-generator': [
        { name: "generate-unit.js", category: "script", content_preview: "export function generateUnitTests(fn) {\n  const testCases = inferTestCases(fn);\n  return testCases.map(tc => generateTest(tc));\n}" },
        { name: "generate-integration.js", category: "script", content_preview: "export function generateIntegrationTests(module) {\n  const flows = analyzeFlows(module);\n  return flows.map(f => generateFlowTest(f));\n}" },
        { name: "mocks.js", category: "script", content_preview: "export function generateMocks(dependencies) {\n  return dependencies.map(d => createMock(d));\n}" },
        { name: "jest-template.js", category: "script", content_preview: "export const jestTemplate = `\ndescribe('{{name}}', () => {\n  {{tests}}\n});\n`;" },
      ],
    }
    return filesBySkill[skill?.name || ''] || [
      { name: "main.js", category: "script", content_preview: "// Main script" },
      { name: "config.json", category: "reference", content_preview: "{}" },
    ]
  },
  'get_skill_tools': (args) => {
    const skill = mockData.skills.find(s => s.name === args?.name || s.name === args?.skillName)
    // Return skill-specific tools
    const toolsBySkill: Record<string, { name: string; description: string }[]> = {
      'web-search': [
        { name: 'web_search', description: 'Search the web for information' },
        { name: 'summarize_results', description: 'Summarize search results into a concise response' },
        { name: 'fetch_page', description: 'Fetch and extract content from a specific URL' },
      ],
      'code-review': [
        { name: 'analyze_code', description: 'Analyze code for potential issues and improvements' },
        { name: 'check_security', description: 'Check code for security vulnerabilities' },
        { name: 'suggest_improvements', description: 'Suggest code improvements and best practices' },
        { name: 'lint_code', description: 'Run linting rules on the code' },
        { name: 'check_types', description: 'Check for type-related issues' },
      ],
      'doc-writer': [
        { name: 'generate_docs', description: 'Generate documentation from code and comments' },
        { name: 'generate_readme', description: 'Generate a README file for a project' },
        { name: 'generate_api_docs', description: 'Generate API documentation' },
      ],
      'test-generator': [
        { name: 'generate_unit_tests', description: 'Generate unit tests for functions' },
        { name: 'generate_integration_tests', description: 'Generate integration tests' },
        { name: 'generate_mocks', description: 'Generate mock objects for testing' },
        { name: 'suggest_test_cases', description: 'Suggest test cases based on code analysis' },
      ],
    }
    return toolsBySkill[skill?.name || ''] || [
      { name: `${skill?.name || 'skill'}_execute`, description: 'Execute the skill' },
    ]
  },
  'get_context_management_config': () => ({
    catalog_compression: true,
    catalog_threshold_bytes: 50000,
    response_threshold_bytes: 10000,
    gateway_indexing: {
      global: 'enable',
      servers: {},
      tools: {},
    },
    virtual_indexing: {
      global: 'enable',
      servers: {},
      tools: {},
    },
    client_tools_indexing_default: 'enable',
    search_tool_name: 'IndexSearch',
    read_tool_name: 'IndexRead',
    vector_search_enabled: true,
  }),
  'list_virtual_mcp_indexing_info': () => ([
    {
      id: '_context_mode',
      display_name: 'Context Management',
      tools: [
        { name: 'IndexSearch', indexable: false },
        { name: 'IndexRead', indexable: false },
      ],
    },
    {
      id: '_skills',
      display_name: 'Skills',
      tools: [
        { name: 'skill_read', indexable: true },
      ],
    },
    {
      id: '_marketplace',
      display_name: 'Marketplace',
      tools: [
        { name: 'marketplace__search', indexable: true },
        { name: 'marketplace__install', indexable: false },
      ],
    },
    {
      id: '_coding_agents',
      display_name: 'Coding Agents',
      tools: [
        { name: 'AgentStart', indexable: false },
        { name: 'AgentSay', indexable: false },
        { name: 'AgentStatus', indexable: true },
        { name: 'AgentList', indexable: true },
      ],
    },
  ]),
  'set_virtual_indexing_permission': () => null,
  'list_active_sessions': () => ([
    {
      session_id: 'a1b2c3d4-e5f6-7890-abcd-ef1234567890',
      client_id: 'cursor-ide',
      client_name: 'Cursor',
      duration_secs: 3420,
      initialized_servers: 3,
      failed_servers: 0,
      total_tools: 24,
      context_management_enabled: false,
      cm_indexed_sources: 0,
      cm_activated_tools: 0,
      cm_total_tools: 0,
      cm_catalog_threshold_bytes: 50000,
    },
    {
      session_id: 'b2c3d4e5-f6a7-8901-bcde-f12345678901',
      client_id: 'claude-code',
      client_name: 'Claude Code',
      duration_secs: 890,
      initialized_servers: 3,
      failed_servers: 0,
      total_tools: 12,
      context_management_enabled: true,
      cm_indexed_sources: 18,
      cm_activated_tools: 8,
      cm_total_tools: 24,
      cm_catalog_threshold_bytes: 50000,
    },
  ]),
  'update_context_management_config': (args) => {
    toast.success('Context management config updated (demo)')
    return null
  },
  'set_gateway_indexing_permission': (args) => {
    toast.success('Gateway indexing permission updated (demo)')
    return null
  },
  'get_known_client_tools': (args) => {
    const templateId = args?.templateId || ''
    if (templateId === 'claude-code') {
      return [
        { name: 'Read', default_state: 'enable', indexable: true },
        { name: 'Glob', default_state: 'enable', indexable: true },
        { name: 'Grep', default_state: 'enable', indexable: true },
        { name: 'WebFetch', default_state: 'enable', indexable: true },
        { name: 'WebSearch', default_state: 'enable', indexable: true },
        { name: 'LSP', default_state: 'enable', indexable: true },
        { name: 'Write', default_state: 'disable', indexable: false },
        { name: 'Edit', default_state: 'disable', indexable: false },
        { name: 'Bash', default_state: 'disable', indexable: false },
        { name: 'Agent', default_state: 'disable', indexable: false },
      ]
    }
    return []
  },
  'get_seen_client_tools': () => [],
  'get_client_tools_indexing': () => null,
  'set_client_tools_indexing': (args) => {
    toast.success('Client tools indexing updated (demo)')
    return null
  },
  'preview_catalog_compression': (args) => {
    const threshold = args?.catalogThresholdBytes ?? 5000
    // Simulate compression: lower threshold = more compression
    const isLowThreshold = threshold < 2000
    const isMidThreshold = threshold < 4000
    const uncompressedSize = 8420
    const compressedSize = threshold >= 10240 ? uncompressedSize : Math.max(threshold, Math.floor(uncompressedSize * 0.35))
    return {
      welcome_message: `Unified MCP Gateway.\n\n<context-management>\n- \`IndexSearch\` (tool)\n\nUse IndexSearch to discover MCP capabilities and retrieve compressed content.\n</context-management>\n\n<github>\n${isMidThreshold ? 'Indexed "mcp/github" — 45 lines, 2.1KB, 8 chunks\n\n## Contents\n- [L1] Server Description\n  - [L5] Issues\n  - [L12] Pull Requests\n  - [L25] Repository & Code\n  - [L40] Actions\n\nUse search(queries: [...]) to find specific content.\n' : "GitHub's official MCP server for repository management, issues, pull requests, code search, actions workflows, and code security scanning.\n\n## Issues\n- Use github__issue_read with method='get' to get issue details.\n- Use github__issue_write to create or update issues.\n\n## Pull Requests\n- Use github__pull_request_read to get PR data.\n- github__create_pull_request creates a new PR.\n\n## Repository & Code\n- github__get_file_contents retrieves file/directory contents.\n- github__search_code searches code across repositories.\n\n## Actions\n- github__list_workflow_runs lists workflow runs.\n- github__get_workflow_run_logs retrieves logs for a specific run.\n"}</github>\n\n<filesystem>\n${isLowThreshold ? 'Indexed "mcp/filesystem" — 12 lines, 0.8KB, 4 chunks\n' : "Secure filesystem operations with configurable access controls.\n\n- filesystem__read_file reads the complete contents of a file.\n- filesystem__write_file creates or overwrites a file.\n- filesystem__edit_file applies targeted edits using a diff-like format.\n- filesystem__search_files searches for files matching a glob pattern.\n"}</filesystem>\n\n<postgresql>\nPostgreSQL database integration for executing queries, managing schemas, and analyzing performance.\n\n- postgres__query executes a read-only SQL query. Use LIMIT to avoid excessive data.\n- postgres__execute runs write SQL statements. Use parameterized queries.\n- postgres__describe_table returns column definitions, indexes, and constraints.\n</postgresql>\n\n<slack>\n${isMidThreshold ? 'Indexed "mcp/slack" — 18 lines, 1.2KB, 5 chunks\n' : "Slack workspace integration for messaging, channel management, and conversation search.\n\n- slack__send_message posts a message to a channel or DM.\n- slack__search_messages supports modifiers: in:#channel, from:@user.\n- slack__get_thread retrieves replies given a channel ID and thread timestamp.\n"}</slack>\n`,
      welcome_message_uncompressed: `Unified MCP Gateway.\n\n<context-management>\n- \`IndexSearch\` (tool)\n\nUse IndexSearch to discover MCP capabilities and retrieve compressed content.\n</context-management>\n\n<github>\nGitHub's official MCP server for repository management, issues, pull requests, code search, actions workflows, and code security scanning.\n\n## Issues\n- Use github__issue_read with method='get' to get issue details, method='get_comments' for comments, method='get_sub_issues' for sub-issues, or method='get_labels' for labels.\n- Use github__issue_write to create or update issues. Always set the method parameter.\n- Use github__add_issue_comment to add a comment.\n\n## Pull Requests\n- Use github__pull_request_read to get PR data. The method parameter controls what data.\n- github__create_pull_request creates a new PR. Requires owner, repo, title, head, base.\n- github__update_pull_request modifies title, body, state, base, or maintainer_can_modify.\n- github__merge_pull_request merges via 'merge', 'squash', or 'rebase' method.\n\n## Repository & Code\n- github__get_file_contents retrieves file/directory contents.\n- github__create_or_update_file creates or updates a single file.\n- github__push_files commits and pushes multiple files in a single commit.\n- github__search_code searches code across repositories using GitHub code search syntax.\n\n## Actions\n- github__list_workflow_runs lists workflow runs with optional filtering.\n- github__get_workflow_run_logs retrieves logs for a specific run.\n- github__rerun_workflow re-runs a failed or completed workflow run.\n</github>\n\n<filesystem>\nSecure filesystem operations with configurable access controls.\n\n- filesystem__read_file reads the complete contents of a file. Returns text content with UTF-8 encoding.\n- filesystem__read_multiple_files reads several files at once. More efficient than multiple individual calls.\n- filesystem__write_file creates or overwrites a file. Creates parent directories if they don't exist.\n- filesystem__edit_file applies targeted edits using a diff-like format with oldText/newText.\n- filesystem__create_directory creates a directory (and parents). No error if it exists.\n- filesystem__list_directory lists entries in a directory with [FILE] or [DIR] prefixes.\n- filesystem__directory_tree returns a recursive tree structure up to configurable depth.\n- filesystem__move_file moves or renames a file or directory.\n- filesystem__search_files searches for files matching a glob pattern recursively.\n- filesystem__get_file_info returns metadata: size, timestamps, permissions.\n</filesystem>\n\n<postgresql>\nPostgreSQL database integration for executing queries, managing schemas, analyzing query performance, and browsing database structure.\n\n- postgres__query executes a read-only SQL query (SELECT, EXPLAIN, SHOW). Use LIMIT to avoid excessive data.\n- postgres__execute runs a write SQL statement. Returns the number of affected rows.\n- postgres__list_schemas returns all schemas in the database with their descriptions.\n- postgres__list_tables lists tables in a schema with row counts and descriptions.\n- postgres__describe_table returns column definitions, indexes, foreign keys, and constraints.\n- postgres__explain_query runs EXPLAIN ANALYZE within a rolled-back transaction.\n</postgresql>\n\n<slack>\nSlack workspace integration for messaging, channel management, user lookups, file sharing, and conversation search.\n\n- slack__send_message posts a message to a channel or DM. Use the channel ID (not name).\n- slack__list_channels returns workspace channels with IDs, names, topics, and member counts.\n- slack__search_messages performs a full-text search with modifiers: in:#channel, from:@user.\n- slack__get_thread retrieves all replies in a thread given a channel ID and thread timestamp.\n- slack__get_channel_history fetches recent messages from a channel.\n- slack__get_users lists workspace members with display names, real names, email, and status.\n- slack__add_reaction adds an emoji reaction to a message.\n- slack__upload_file uploads a file to a channel with optional initial comment.\n</slack>\n`,
      uncompressed_size: uncompressedSize,
      compressed_size: compressedSize,
      welcome_size: 3200,
      tool_definitions_size: 5220,
      compressed_tool_definitions_size: isLowThreshold ? 2100 : isMidThreshold ? 3800 : 5220,
      indexed_welcomes_count: isMidThreshold ? 3 : isLowThreshold ? 4 : 0,
      deferred_servers_count: isLowThreshold ? 2 : isMidThreshold ? 1 : 0,
      welcome_toc_dropped_count: 0,
      batch_toc_dropped_count: 0,
      servers: [
        {
          name: 'Context Management', is_virtual: true,
          tool_names: ['IndexSearch'], resource_names: [], prompt_names: [],
          description: 'Use IndexSearch to discover MCP capabilities and retrieve compressed content.',
          instructions: null, compression_state: 'visible',
          tools: [{
            name: 'IndexSearch', description: 'Search the indexed MCP catalog and tool execution results. Returns matching content from compressed descriptions, deferred tools, and previously executed tool outputs.',
            input_schema: { type: 'object', properties: { queries: { type: 'array', items: { type: 'string' }, description: 'One or more search queries to run against the FTS5 index' }, source: { type: 'string', description: 'Optional source filter (e.g., "catalog:github", "execute:filesystem__read_file")' }, limit: { type: 'number', description: 'Maximum results per query (default: 5)' } }, required: ['queries'] },
          }],
          resources: [], prompts: [],
        },
        {
          name: 'GitHub', is_virtual: false,
          tool_names: ['github__issue_read', 'github__issue_write', 'github__search_issues', 'github__list_issues', 'github__add_issue_comment', 'github__sub_issue_write', 'github__pull_request_read', 'github__create_pull_request', 'github__update_pull_request', 'github__merge_pull_request', 'github__add_pull_request_review_comment', 'github__get_file_contents', 'github__create_or_update_file', 'github__push_files', 'github__search_code', 'github__search_repositories', 'github__list_commits', 'github__list_branches', 'github__create_branch', 'github__list_workflow_runs', 'github__get_workflow_run_logs', 'github__rerun_workflow', 'github__get_code_scanning_alerts'],
          resource_names: [], prompt_names: [],
          description: "GitHub's official MCP server for repository management, issues, pull requests, code search, actions workflows, and code security scanning. Provides full read/write access to the authenticated user's repositories and organizations.",
          instructions: "## Issues\n- Use `github__issue_read` with method='get' to get issue details.\n- Use `github__issue_write` to create or update issues.\n\n## Pull Requests\n- Use `github__pull_request_read` to get PR data.\n- `github__create_pull_request` creates a new PR.\n\n## Repository & Code\n- `github__get_file_contents` retrieves file/directory contents.\n- `github__search_code` searches code across repositories.",
          compression_state: isMidThreshold ? 'deferred' : 'visible',
          tools: [
            { name: 'github__issue_read', description: 'Read issue details, comments, sub-issues, or labels from a GitHub repository. Supports multiple retrieval methods: "get" returns the full issue object including title, body, state, assignees, labels, milestone, and timeline; "get_comments" returns a paginated list of all comments on the issue in chronological order; "get_sub_issues" returns the sub-issue tree for tracking work breakdown on parent issues; "get_labels" returns all labels applied to the issue with their colors and descriptions.', input_schema: { type: 'object', properties: { owner: { type: 'string', description: 'The GitHub username or organization that owns the repository (e.g., "facebook", "microsoft"). Case-insensitive.' }, repo: { type: 'string', description: 'The name of the repository within the owner\'s account (e.g., "react", "vscode"). Do not include the owner prefix.' }, issue_number: { type: 'number', description: 'The unique numeric identifier for the issue within this repository. Found in the issue URL or via search/list endpoints.' }, method: { type: 'string', enum: ['get', 'get_comments', 'get_sub_issues', 'get_labels'], description: 'The retrieval operation to perform. "get" returns the issue details, "get_comments" returns comments, "get_sub_issues" returns child issues, "get_labels" returns applied labels.' } }, required: ['owner', 'repo', 'issue_number'] } },
            { name: 'github__issue_write', description: 'Create, update, close, or reopen issues in a GitHub repository. When creating a new issue, the title field is required and will be displayed as the issue heading. When updating, closing, or reopening an existing issue, the issue_number field is required to identify which issue to modify. Supports setting labels, assignees, milestones, and rich Markdown body content including task lists, code blocks, and image references.', input_schema: { type: 'object', properties: { owner: { type: 'string', description: 'The GitHub username or organization that owns the repository where the issue will be created or modified' }, repo: { type: 'string', description: 'The name of the repository. Must be an existing repository that the authenticated user has write access to.' }, method: { type: 'string', enum: ['create', 'update', 'close', 'reopen'], description: 'The write operation to perform. "create" opens a new issue, "update" modifies an existing issue\'s fields, "close" closes an open issue, "reopen" reopens a closed issue.' }, title: { type: 'string', description: 'The issue title displayed as the heading. Required when method is "create". For updates, only provided if changing the title.' }, body: { type: 'string', description: 'The issue body content in GitHub-flavored Markdown. Supports headings, lists, task lists, code blocks, tables, and image references.' }, issue_number: { type: 'number', description: 'The numeric issue identifier. Required for "update", "close", and "reopen" operations. Not needed for "create".' }, labels: { type: 'array', items: { type: 'string' }, description: 'Array of label names to apply to the issue. Labels must already exist in the repository. Use the labels API to create new labels first.' }, assignees: { type: 'array', items: { type: 'string' }, description: 'Array of GitHub usernames to assign to the issue. Each user must have access to the repository. Maximum 10 assignees.' } }, required: ['owner', 'repo', 'method'] } },
            { name: 'github__search_issues', description: 'Search issues and pull requests across all GitHub repositories using the powerful GitHub search syntax. Supports advanced qualifiers like repo:owner/name, is:open, is:pr, label:bug, author:username, assignee:username, milestone:name, state:open/closed, language:, created:>2024-01-01, updated:<2024-06-01, comments:>10, and boolean operators AND/OR/NOT for complex queries.', input_schema: { type: 'object', properties: { query: { type: 'string', description: 'Search query using GitHub search syntax. Supports qualifiers: repo:owner/name, is:issue or is:pr, label:name, author:user, assignee:user, state:open/closed, milestone:name, language:name, created/updated date ranges. Example: "repo:facebook/react is:open label:bug sort:updated-desc"' }, per_page: { type: 'number', description: 'Number of results to return per page. Minimum 1, maximum 100, default 30. Use in combination with page for pagination through large result sets.' }, page: { type: 'number', description: 'Page number for paginating through results. Starts at 1. Each page returns up to per_page results. Check total_count in response to determine total pages available.' } }, required: ['query'] } },
            { name: 'github__list_issues', description: 'List and filter issues in a specific repository with comprehensive filtering options. Returns issues sorted by the specified field and direction. Supports filtering by open/closed state, label names, assigned user, milestone, and creation/update dates. Results are paginated and include full issue details.', input_schema: { type: 'object', properties: { owner: { type: 'string', description: 'The GitHub username or organization that owns the repository to list issues from' }, repo: { type: 'string', description: 'The repository name to list issues from. Must be a repository the authenticated user can access.' }, state: { type: 'string', enum: ['open', 'closed', 'all'], description: 'Filter issues by their current state. "open" shows only open issues (default), "closed" shows resolved issues, "all" shows both.' }, labels: { type: 'string', description: 'Comma-separated list of label names to filter by. Only issues with ALL specified labels will be returned. Example: "bug,priority:high"' }, assignee: { type: 'string', description: 'Filter by assigned user. Pass a GitHub username to see their issues, or "none" for unassigned issues, or "*" for any assigned issue.' }, sort: { type: 'string', enum: ['created', 'updated', 'comments'], description: 'Field to sort results by. "created" sorts by creation date, "updated" by last modification, "comments" by comment count.' }, direction: { type: 'string', enum: ['asc', 'desc'], description: 'Sort direction. "desc" shows newest/most first (default), "asc" shows oldest/least first.' }, per_page: { type: 'number', description: 'Number of results per page, maximum 100. Use with page parameter for pagination.' } }, required: ['owner', 'repo'] } },
            { name: 'github__add_issue_comment', description: 'Add a comment to an issue or pull request. Pass the PR number as issue_number to comment on pull requests.', input_schema: { type: 'object', properties: { owner: { type: 'string', description: 'Repository owner' }, repo: { type: 'string', description: 'Repository name' }, issue_number: { type: 'number', description: 'Issue or PR number' }, body: { type: 'string', description: 'Comment body in Markdown' } }, required: ['owner', 'repo', 'issue_number', 'body'] } },
            { name: 'github__sub_issue_write', description: 'Create or manage sub-issues for tracking work breakdown on a parent issue.', input_schema: { type: 'object', properties: { owner: { type: 'string', description: 'Repository owner' }, repo: { type: 'string', description: 'Repository name' }, issue_number: { type: 'number', description: 'Parent issue number' }, sub_issue_id: { type: 'number', description: 'Sub-issue ID to link' }, method: { type: 'string', enum: ['add', 'remove'], description: 'Operation to perform' } }, required: ['owner', 'repo', 'issue_number', 'method'] } },
            { name: 'github__pull_request_read', description: 'Get pull request data including details, diff, status, changed files, reviews, and review comments. Use the method parameter to select the type of data.', input_schema: { type: 'object', properties: { owner: { type: 'string', description: 'Repository owner' }, repo: { type: 'string', description: 'Repository name' }, pull_number: { type: 'number', description: 'Pull request number' }, method: { type: 'string', enum: ['get', 'get_diff', 'get_status', 'get_files', 'get_reviews', 'get_review_comments'], description: 'The type of PR data to retrieve' } }, required: ['owner', 'repo', 'pull_number'] } },
            { name: 'github__create_pull_request', description: 'Create a new pull request. Requires owner, repo, title, head (source branch), and base (target branch).', input_schema: { type: 'object', properties: { owner: { type: 'string', description: 'Repository owner' }, repo: { type: 'string', description: 'Repository name' }, title: { type: 'string', description: 'PR title' }, body: { type: 'string', description: 'PR description in Markdown' }, head: { type: 'string', description: 'Source branch name' }, base: { type: 'string', description: 'Target branch name (e.g., "main")' }, draft: { type: 'boolean', description: 'Create as draft PR' } }, required: ['owner', 'repo', 'title', 'head', 'base'] } },
            { name: 'github__update_pull_request', description: 'Update an existing pull request\'s title, body, state, base branch, or maintainer access settings.', input_schema: { type: 'object', properties: { owner: { type: 'string', description: 'Repository owner' }, repo: { type: 'string', description: 'Repository name' }, pull_number: { type: 'number', description: 'Pull request number' }, title: { type: 'string', description: 'New title' }, body: { type: 'string', description: 'New description' }, state: { type: 'string', enum: ['open', 'closed'], description: 'New state' }, base: { type: 'string', description: 'New base branch' } }, required: ['owner', 'repo', 'pull_number'] } },
            { name: 'github__merge_pull_request', description: 'Merge a pull request using merge commit, squash, or rebase strategy.', input_schema: { type: 'object', properties: { owner: { type: 'string', description: 'Repository owner' }, repo: { type: 'string', description: 'Repository name' }, pull_number: { type: 'number', description: 'Pull request number' }, merge_method: { type: 'string', enum: ['merge', 'squash', 'rebase'], description: 'Merge strategy (default: merge)' }, commit_title: { type: 'string', description: 'Custom commit title for merge/squash' }, commit_message: { type: 'string', description: 'Custom commit message' } }, required: ['owner', 'repo', 'pull_number'] } },
            { name: 'github__add_pull_request_review_comment', description: 'Add an inline review comment on a specific line in a pull request diff.', input_schema: { type: 'object', properties: { owner: { type: 'string', description: 'Repository owner' }, repo: { type: 'string', description: 'Repository name' }, pull_number: { type: 'number', description: 'Pull request number' }, body: { type: 'string', description: 'Comment body' }, path: { type: 'string', description: 'File path relative to repo root' }, line: { type: 'number', description: 'Line number in the diff' }, side: { type: 'string', enum: ['LEFT', 'RIGHT'], description: 'Side of the diff (LEFT=before, RIGHT=after)' } }, required: ['owner', 'repo', 'pull_number', 'body', 'path', 'line'] } },
            { name: 'github__get_file_contents', description: 'Retrieve file or directory contents from a repository at a specific ref (branch/tag/commit).', input_schema: { type: 'object', properties: { owner: { type: 'string', description: 'Repository owner' }, repo: { type: 'string', description: 'Repository name' }, path: { type: 'string', description: 'File or directory path' }, ref: { type: 'string', description: 'Git ref (branch, tag, or commit SHA)' } }, required: ['owner', 'repo', 'path'] } },
            { name: 'github__create_or_update_file', description: 'Create or update a single file in a repository. If updating, provide the SHA of the existing file to avoid conflicts.', input_schema: { type: 'object', properties: { owner: { type: 'string', description: 'Repository owner' }, repo: { type: 'string', description: 'Repository name' }, path: { type: 'string', description: 'File path' }, content: { type: 'string', description: 'New file content' }, message: { type: 'string', description: 'Commit message' }, sha: { type: 'string', description: 'SHA of existing file (required for updates)' }, branch: { type: 'string', description: 'Branch name' } }, required: ['owner', 'repo', 'path', 'content', 'message'] } },
            { name: 'github__push_files', description: 'Commit and push multiple files to a repository in a single commit.', input_schema: { type: 'object', properties: { owner: { type: 'string', description: 'Repository owner' }, repo: { type: 'string', description: 'Repository name' }, branch: { type: 'string', description: 'Target branch' }, message: { type: 'string', description: 'Commit message' }, files: { type: 'array', items: { type: 'object', properties: { path: { type: 'string' }, content: { type: 'string' } } }, description: 'Array of {path, content} objects' } }, required: ['owner', 'repo', 'branch', 'message', 'files'] } },
            { name: 'github__search_code', description: 'Search code across GitHub repositories using the code search syntax. Supports qualifiers like repo:, language:, path:, extension:.', input_schema: { type: 'object', properties: { query: { type: 'string', description: 'Code search query (e.g., "addClass repo:jquery/jquery language:js")' }, per_page: { type: 'number', description: 'Results per page (max 100)' }, page: { type: 'number', description: 'Page number' } }, required: ['query'] } },
            { name: 'github__search_repositories', description: 'Search GitHub repositories by name, description, language, topic, or other criteria.', input_schema: { type: 'object', properties: { query: { type: 'string', description: 'Repository search query' }, sort: { type: 'string', enum: ['stars', 'forks', 'updated', 'help-wanted-issues'], description: 'Sort field' }, order: { type: 'string', enum: ['asc', 'desc'], description: 'Sort order' }, per_page: { type: 'number', description: 'Results per page' } }, required: ['query'] } },
            { name: 'github__list_commits', description: 'List commits on a branch with optional filtering by path, author, and date range.', input_schema: { type: 'object', properties: { owner: { type: 'string', description: 'Repository owner' }, repo: { type: 'string', description: 'Repository name' }, sha: { type: 'string', description: 'Branch name or commit SHA' }, path: { type: 'string', description: 'Only commits touching this path' }, author: { type: 'string', description: 'Filter by author email or username' }, since: { type: 'string', description: 'ISO 8601 date — only commits after this date' }, until: { type: 'string', description: 'ISO 8601 date — only commits before this date' }, per_page: { type: 'number', description: 'Results per page' } }, required: ['owner', 'repo'] } },
            { name: 'github__list_branches', description: 'List branches in a repository with optional name pattern filtering.', input_schema: { type: 'object', properties: { owner: { type: 'string', description: 'Repository owner' }, repo: { type: 'string', description: 'Repository name' }, protected_only: { type: 'boolean', description: 'Only list protected branches' }, per_page: { type: 'number', description: 'Results per page' } }, required: ['owner', 'repo'] } },
            { name: 'github__create_branch', description: 'Create a new branch from an existing ref (branch, tag, or commit SHA).', input_schema: { type: 'object', properties: { owner: { type: 'string', description: 'Repository owner' }, repo: { type: 'string', description: 'Repository name' }, branch: { type: 'string', description: 'New branch name' }, from_ref: { type: 'string', description: 'Source ref to branch from (default: default branch HEAD)' } }, required: ['owner', 'repo', 'branch'] } },
            { name: 'github__list_workflow_runs', description: 'List GitHub Actions workflow runs with optional filtering by workflow name, status, branch, and event type.', input_schema: { type: 'object', properties: { owner: { type: 'string', description: 'Repository owner' }, repo: { type: 'string', description: 'Repository name' }, workflow_id: { type: 'string', description: 'Workflow filename or ID' }, status: { type: 'string', enum: ['completed', 'action_required', 'cancelled', 'failure', 'neutral', 'skipped', 'stale', 'success', 'timed_out', 'in_progress', 'queued', 'requested', 'waiting', 'pending'], description: 'Filter by status' }, branch: { type: 'string', description: 'Filter by branch' }, per_page: { type: 'number', description: 'Results per page' } }, required: ['owner', 'repo'] } },
            { name: 'github__get_workflow_run_logs', description: 'Retrieve logs for a specific GitHub Actions workflow run. May return large output for complex workflows.', input_schema: { type: 'object', properties: { owner: { type: 'string', description: 'Repository owner' }, repo: { type: 'string', description: 'Repository name' }, run_id: { type: 'number', description: 'Workflow run ID' } }, required: ['owner', 'repo', 'run_id'] } },
            { name: 'github__rerun_workflow', description: 'Re-run a failed or completed GitHub Actions workflow run.', input_schema: { type: 'object', properties: { owner: { type: 'string', description: 'Repository owner' }, repo: { type: 'string', description: 'Repository name' }, run_id: { type: 'number', description: 'Workflow run ID' } }, required: ['owner', 'repo', 'run_id'] } },
            { name: 'github__get_code_scanning_alerts', description: 'Get code scanning (CodeQL) alerts for a repository, including severity, state, and affected file locations.', input_schema: { type: 'object', properties: { owner: { type: 'string', description: 'Repository owner' }, repo: { type: 'string', description: 'Repository name' }, state: { type: 'string', enum: ['open', 'closed', 'dismissed', 'fixed'], description: 'Filter by alert state' }, severity: { type: 'string', enum: ['critical', 'high', 'medium', 'low', 'warning', 'note', 'error'], description: 'Filter by severity' }, per_page: { type: 'number', description: 'Results per page' } }, required: ['owner', 'repo'] } },
          ],
          resources: [], prompts: [],
        },
        {
          name: 'Filesystem', is_virtual: false,
          tool_names: ['filesystem__read_file', 'filesystem__write_file', 'filesystem__edit_file', 'filesystem__create_directory', 'filesystem__list_directory', 'filesystem__directory_tree', 'filesystem__move_file', 'filesystem__search_files', 'filesystem__get_file_info', 'filesystem__read_multiple_files'],
          resource_names: [], prompt_names: [],
          description: 'Secure filesystem operations with configurable access controls. Provides tools for reading, writing, creating, moving, and searching files and directories within allowed paths. All operations are sandboxed to the configured root directories.',
          instructions: "- `filesystem__read_file` reads the complete contents of a file. Returns text content with UTF-8 encoding.\n- `filesystem__write_file` creates or overwrites a file. Creates parent directories if they don't exist.\n- `filesystem__edit_file` applies targeted edits using a diff-like format with oldText/newText.\n- `filesystem__search_files` searches for files matching a glob pattern recursively.",
          compression_state: isMidThreshold ? 'compressed' : 'visible',
          tools: [
            { name: 'filesystem__read_file', description: 'Read the complete contents of a file from the filesystem. Returns text content decoded as UTF-8. For binary files (images, PDFs, archives), returns base64-encoded content with a content type indicator. Handles files up to 10MB; for larger files, consider reading specific byte ranges.', input_schema: { type: 'object', properties: { path: { type: 'string', description: 'Absolute path to the file to read. Must be within one of the configured allowed directories. Symlinks are resolved and validated against the sandbox. Example: "/home/user/project/src/main.ts"' } }, required: ['path'] } },
            { name: 'filesystem__write_file', description: 'Create a new file or overwrite an existing file with the provided content. Automatically creates all intermediate parent directories if they do not exist. Content must be provided as a UTF-8 string; for binary data, encode as base64 first. Preserves file permissions on overwrite. Returns the number of bytes written.', input_schema: { type: 'object', properties: { path: { type: 'string', description: 'Absolute path where the file will be created or overwritten. Parent directories are created automatically if they do not exist. Must be within the configured sandbox directories.' }, content: { type: 'string', description: 'The full content to write to the file as a UTF-8 encoded string. For binary files, provide base64-encoded content. The entire file is replaced; use edit_file for partial modifications.' } }, required: ['path', 'content'] } },
            { name: 'filesystem__edit_file', description: 'Apply one or more targeted edits to an existing file using a diff-like oldText/newText format. Each edit finds an exact match of oldText in the file and replaces it with newText. Multiple edits are applied sequentially. This is more reliable than write_file for partial modifications because it preserves the rest of the file content and avoids race conditions with concurrent editors.', input_schema: { type: 'object', properties: { path: { type: 'string', description: 'Absolute path to the file to edit. The file must already exist; use write_file to create new files. Must be within the configured sandbox directories.' }, edits: { type: 'array', items: { type: 'object', properties: { oldText: { type: 'string', description: 'The exact text to find in the file. Must match character-for-character including whitespace, indentation, and line endings. If not found, the edit fails with an error showing the closest match.' }, newText: { type: 'string', description: 'The replacement text that will replace the matched oldText. Can be empty string to delete the matched text. Preserves surrounding content.' } }, required: ['oldText', 'newText'] }, description: 'Array of edit operations to apply in order. Each operation specifies the exact text to find and its replacement. Earlier edits may affect the text available for later edits.' }, dryRun: { type: 'boolean', description: 'When true, validates that all edits would succeed and returns a preview diff without actually modifying the file. Useful for verifying edits before applying them.' } }, required: ['path', 'edits'] } },
            { name: 'filesystem__create_directory', description: 'Create a new directory along with all necessary parent directories recursively, similar to "mkdir -p". No error is raised if the directory already exists. Returns the absolute path of the created directory.', input_schema: { type: 'object', properties: { path: { type: 'string', description: 'Absolute path of the directory to create. All intermediate parent directories will be created if they do not already exist. Must be within the configured sandbox directories.' } }, required: ['path'] } },
            { name: 'filesystem__list_directory', description: 'List all entries in a directory with type indicators. Each entry is prefixed with [FILE] for regular files or [DIR] for directories. Does not recurse into subdirectories; use directory_tree for recursive listing. Entries are sorted alphabetically with directories listed before files.', input_schema: { type: 'object', properties: { path: { type: 'string', description: 'Absolute path of the directory to list. Must be an existing directory within the configured sandbox directories. Symlinks to directories are followed.' } }, required: ['path'] } },
            { name: 'filesystem__directory_tree', description: 'Generate a recursive tree structure of a directory hierarchy up to a configurable maximum depth. Returns an indented text representation showing the full directory tree with file and directory names, sizes, and entry counts. Useful for quickly understanding project layout and structure.', input_schema: { type: 'object', properties: { path: { type: 'string', description: 'Root directory path for the tree. The tree will be generated starting from this directory and recursing into subdirectories up to the specified depth.' }, depth: { type: 'number', description: 'Maximum depth to recurse into subdirectories. Default is 3. Set to 1 for just the immediate contents, or higher values for deeper exploration. Very large values may produce extensive output.' } }, required: ['path'] } },
            { name: 'filesystem__move_file', description: 'Move or rename a file or directory to a new location. The operation is atomic on most filesystems. Fails with an error if the destination path already exists to prevent accidental overwrites. Works across directories on the same filesystem.', input_schema: { type: 'object', properties: { source: { type: 'string', description: 'Current absolute path of the file or directory to move. Must exist and be within the configured sandbox directories.' }, destination: { type: 'string', description: 'New absolute path for the file or directory. Must not already exist. Parent directories are not automatically created; use create_directory first if needed.' } }, required: ['source', 'destination'] } },
            { name: 'filesystem__search_files', description: 'Search for files and directories matching a glob pattern, starting from a base directory and recursing through all subdirectories. Returns an array of matching absolute file paths sorted alphabetically. Supports standard glob syntax including wildcards (*), recursive wildcards (**), character classes ([abc]), and alternatives ({a,b}).', input_schema: { type: 'object', properties: { path: { type: 'string', description: 'Base directory to start the recursive search from. All subdirectories within this path will be searched. Must be within the configured sandbox directories.' }, pattern: { type: 'string', description: 'Glob pattern to match against file and directory names. Examples: "**/*.ts" matches all TypeScript files, "src/**/*.{js,jsx}" matches JS/JSX files in src, "**/test*" matches files starting with "test".' }, excludePatterns: { type: 'array', items: { type: 'string' }, description: 'Array of glob patterns to exclude from results. Commonly used to skip node_modules, .git, dist, and build directories. Example: ["**/node_modules/**", "**/.git/**"]' } }, required: ['path', 'pattern'] } },
            { name: 'filesystem__get_file_info', description: 'Get file metadata including size, creation time, modification time, permissions, and whether the path is a file or directory.', input_schema: { type: 'object', properties: { path: { type: 'string', description: 'Absolute path to get info for' } }, required: ['path'] } },
            { name: 'filesystem__read_multiple_files', description: 'Read several files at once. More efficient than multiple individual read_file calls. Returns results in the same order as requested paths.', input_schema: { type: 'object', properties: { paths: { type: 'array', items: { type: 'string' }, description: 'Array of absolute file paths to read' } }, required: ['paths'] } },
          ],
          resources: [], prompts: [],
        },
        {
          name: 'PostgreSQL', is_virtual: false,
          tool_names: ['postgres__query', 'postgres__execute', 'postgres__list_schemas', 'postgres__list_tables', 'postgres__describe_table', 'postgres__explain_query'],
          resource_names: ['postgres__schema://public'], prompt_names: [],
          description: 'PostgreSQL database integration for executing queries, managing schemas, analyzing query performance, and browsing database structure. Connected to the project\'s development database with read-write access.',
          instructions: "- `postgres__query` executes a read-only SQL query. Use LIMIT to avoid excessive data.\n- `postgres__execute` runs write SQL statements. Use parameterized queries for user-provided values.\n- `postgres__describe_table` returns column definitions, indexes, and constraints.",
          compression_state: 'visible',
          tools: [
            { name: 'postgres__query', description: 'Execute a read-only SQL query against the connected PostgreSQL database. Only SELECT, EXPLAIN, and SHOW statements are allowed; write operations will be rejected. Returns results as an array of JSON row objects with column names as keys. Supports parameterized queries using $1, $2, etc. placeholders for safe value interpolation. Always use LIMIT to avoid returning excessive data that could overwhelm the response.', input_schema: { type: 'object', properties: { sql: { type: 'string', description: 'The SQL query to execute. Must be a read-only statement (SELECT, EXPLAIN, or SHOW). Use $1, $2 etc. as parameter placeholders for dynamic values. Always include a LIMIT clause for potentially large result sets to avoid excessive output.' }, params: { type: 'array', items: { type: 'string' }, description: 'Array of parameter values that correspond to $1, $2, etc. placeholders in the SQL query. Values are automatically escaped and type-cast by the database driver. Pass all user-provided values through parameters to prevent SQL injection.' } }, required: ['sql'] } },
            { name: 'postgres__execute', description: 'Execute a write SQL statement against the connected PostgreSQL database. Supports INSERT, UPDATE, DELETE, CREATE, ALTER, DROP, and other DDL/DML statements. Returns the number of rows affected by the operation. For INSERT statements with RETURNING clause, also returns the inserted rows. Always use parameterized queries ($1, $2) for any user-provided values to prevent SQL injection attacks.', input_schema: { type: 'object', properties: { sql: { type: 'string', description: 'The SQL statement to execute. Can be any write operation: INSERT, UPDATE, DELETE, CREATE TABLE, ALTER TABLE, DROP TABLE, CREATE INDEX, etc. Use $1, $2 etc. as parameter placeholders. Supports RETURNING clause for INSERT/UPDATE/DELETE.' }, params: { type: 'array', items: { type: 'string' }, description: 'Array of parameter values for $1, $2, etc. placeholders. All user-provided or dynamic values should be passed as parameters rather than interpolated into the SQL string to prevent injection vulnerabilities.' } }, required: ['sql'] } },
            { name: 'postgres__list_schemas', description: 'List all schemas in the connected PostgreSQL database including their descriptions, table counts, and total sizes. Returns both system schemas (pg_catalog, information_schema) and user-created schemas. Useful for discovering the database structure before querying specific tables.', input_schema: { type: 'object', properties: {} } },
            { name: 'postgres__list_tables', description: 'List all tables within a specific database schema along with their approximate row counts, disk size estimates, and optional descriptions set via COMMENT ON TABLE. Returns table metadata useful for understanding the data model before writing queries. Defaults to the "public" schema if no schema is specified.', input_schema: { type: 'object', properties: { schema: { type: 'string', description: 'The database schema to list tables from. Common schemas include "public" (default), "auth", "storage", etc. Use list_schemas to discover available schemas first.' } } } },
            { name: 'postgres__describe_table', description: 'Get comprehensive column definitions for a database table including column name, data type, nullability, default value, character maximum length, and numeric precision. Also returns primary key constraints, foreign key relationships (referenced table and column), unique constraints, check constraints, and indexes defined on the table.', input_schema: { type: 'object', properties: { schema: { type: 'string', description: 'The database schema containing the table. Defaults to "public" if not specified. Use list_schemas to discover available schemas.' }, table: { type: 'string', description: 'The name of the table to describe. Returns detailed column definitions, constraints, indexes, and foreign key relationships for this table.' } }, required: ['table'] } },
            { name: 'postgres__explain_query', description: 'Run EXPLAIN ANALYZE on a SQL query to obtain the detailed execution plan with actual timing measurements, row count estimates vs actuals, buffer usage statistics, and I/O timing. The query is executed inside a transaction that is immediately rolled back, so no data modifications persist. Essential for diagnosing slow queries and understanding how PostgreSQL plans query execution.', input_schema: { type: 'object', properties: { sql: { type: 'string', description: 'The SQL query to analyze with EXPLAIN ANALYZE. Can be any valid SQL statement. The query will be executed (to get actual timing) but within a rolled-back transaction so no data is modified.' }, params: { type: 'array', items: { type: 'string' }, description: 'Parameter values for $1, $2, etc. placeholders in the query being analyzed. Providing actual representative values helps PostgreSQL generate a more accurate execution plan.' }, format: { type: 'string', enum: ['text', 'json', 'yaml'], description: 'Output format for the execution plan. "text" provides human-readable indented output (default), "json" provides machine-parseable structured output, "yaml" provides YAML-formatted output.' } }, required: ['sql'] } },
          ],
          resources: [
            { name: 'postgres__schema://public', uri: 'schema://public', description: 'Browse the public schema structure including all tables, views, and their relationships.' },
          ],
          prompts: [],
        },
        {
          name: 'Slack', is_virtual: false,
          tool_names: ['slack__send_message', 'slack__list_channels', 'slack__search_messages', 'slack__get_thread', 'slack__get_channel_history', 'slack__get_users', 'slack__add_reaction', 'slack__upload_file'],
          resource_names: [], prompt_names: [],
          description: 'Slack workspace integration for messaging, channel management, user lookups, file sharing, and conversation search. Operates in the authenticated user\'s workspace with permissions scoped to their access level.',
          instructions: "- `slack__send_message` posts a message to a channel or DM using the channel ID.\n- `slack__search_messages` supports modifiers: in:#channel, from:@user, before:2024-01-01.\n- `slack__get_thread` retrieves replies given a channel ID and thread timestamp.",
          compression_state: isMidThreshold ? 'compressed' : 'visible',
          tools: [
            { name: 'slack__send_message', description: 'Post a message to a Slack channel or direct message conversation. Messages must be sent using the channel ID (not the channel name); use list_channels to look up IDs. Supports full Slack mrkdwn formatting including *bold*, _italic_, ~strikethrough~, `code`, ```code blocks```, ordered and bullet lists, <URL|link text> hyperlinks, and @user mentions. For replying within an existing thread, include the thread_ts parameter with the parent message timestamp.', input_schema: { type: 'object', properties: { channel: { type: 'string', description: 'The Slack channel ID to post to (e.g., "C024BE91L" for public channels, "D012AB3CD" for DMs). Always use the ID, not the channel name. Use list_channels or search to find the correct channel ID.' }, text: { type: 'string', description: 'The message content to post. Supports Slack mrkdwn formatting: *bold*, _italic_, ~strikethrough~, `inline code`, ```code blocks```, bullet lists, numbered lists, blockquotes (>), and hyperlinks (<url|display text>). Mention users with <@U012ABCDE>.' }, thread_ts: { type: 'string', description: 'The timestamp of the parent message to reply to in a thread. When provided, the message appears as a threaded reply rather than a new message in the channel. Get this value from message objects returned by other Slack endpoints.' }, unfurl_links: { type: 'boolean', description: 'Controls whether URLs in the message text will show rich link previews (title, description, thumbnail). Set to false to suppress link previews for cleaner output. Default is true.' } }, required: ['channel', 'text'] } },
            { name: 'slack__list_channels', description: 'List all channels in the workspace that the authenticated user has access to, including public channels, private channels, direct messages, and multi-party DMs. Returns channel metadata including ID, name, topic, purpose, member count, creation date, and archive status. Results are paginated using cursor-based navigation for workspaces with many channels.', input_schema: { type: 'object', properties: { types: { type: 'string', description: 'Comma-separated list of channel types to include in results. Options: "public_channel" (default), "private_channel", "im" (direct messages), "mpim" (group DMs). Example: "public_channel,private_channel" to list all non-DM channels.' }, limit: { type: 'number', description: 'Maximum number of channels to return per page. Default is 100, maximum is 1000. Use in combination with cursor for paginating through all channels.' }, cursor: { type: 'string', description: 'Pagination cursor returned in the response_metadata.next_cursor field of a previous list_channels call. Pass this to retrieve the next page of results.' }, exclude_archived: { type: 'boolean', description: 'When true, archived channels are excluded from the results. Default is false, which includes both active and archived channels.' } } } },
            { name: 'slack__search_messages', description: 'Perform a full-text search across all messages in channels and conversations that the authenticated user has access to. Supports Slack\'s powerful search modifier syntax: "in:#channel-name" to search within a specific channel, "from:@username" to find messages from a specific user, "before:YYYY-MM-DD" and "after:YYYY-MM-DD" for date ranges, "has:link" for messages containing URLs, "has:reaction" for reacted messages, and "is:thread" for threaded messages.', input_schema: { type: 'object', properties: { query: { type: 'string', description: 'The search query string. Supports plain text search and Slack search modifiers: in:#channel, from:@user, before:YYYY-MM-DD, after:YYYY-MM-DD, has:link, has:reaction, has:pin, is:thread, is:starred. Modifiers can be combined. Example: "deployment error in:#ops-alerts after:2024-01-01"' }, count: { type: 'number', description: 'Number of matching messages to return per page. Default is 20, maximum is 100. Use with page parameter for pagination through large result sets.' }, sort: { type: 'string', enum: ['score', 'timestamp'], description: 'Sort order for results. "score" ranks by relevance using Slack\'s search algorithm (default). "timestamp" sorts by message date for chronological viewing.' }, sort_dir: { type: 'string', enum: ['asc', 'desc'], description: 'Sort direction. "desc" shows highest relevance or newest first (default). "asc" shows lowest relevance or oldest first.' } }, required: ['query'] } },
            { name: 'slack__get_thread', description: 'Retrieve all replies in a conversation thread. Returns messages in chronological order with sender information and timestamps.', input_schema: { type: 'object', properties: { channel: { type: 'string', description: 'Channel ID containing the thread' }, ts: { type: 'string', description: 'Thread parent message timestamp' }, limit: { type: 'number', description: 'Max replies to return (default: 100)' } }, required: ['channel', 'ts'] } },
            { name: 'slack__get_channel_history', description: 'Fetch recent messages from a channel with optional time range filtering. Returns messages in reverse chronological order.', input_schema: { type: 'object', properties: { channel: { type: 'string', description: 'Channel ID' }, oldest: { type: 'string', description: 'Unix timestamp — only messages after this time' }, latest: { type: 'string', description: 'Unix timestamp — only messages before this time' }, limit: { type: 'number', description: 'Max messages to return (default: 100)' } }, required: ['channel'] } },
            { name: 'slack__get_users', description: 'List workspace members with display names, real names, email addresses, status text, and online presence. Useful for resolving user IDs for @mentions.', input_schema: { type: 'object', properties: { limit: { type: 'number', description: 'Max users to return (default: 200)' }, cursor: { type: 'string', description: 'Pagination cursor' } } } },
            { name: 'slack__add_reaction', description: 'Add an emoji reaction to a specific message in a channel. The reaction name should not include colons (e.g., "thumbsup" not ":thumbsup:").', input_schema: { type: 'object', properties: { channel: { type: 'string', description: 'Channel ID containing the message' }, timestamp: { type: 'string', description: 'Message timestamp to react to' }, name: { type: 'string', description: 'Emoji name without colons (e.g., "thumbsup", "eyes", "rocket")' } }, required: ['channel', 'timestamp', 'name'] } },
            { name: 'slack__upload_file', description: 'Upload a file to a Slack channel with an optional initial comment. Supports text files, images, and other file types.', input_schema: { type: 'object', properties: { channels: { type: 'string', description: 'Comma-separated channel IDs to share the file with' }, content: { type: 'string', description: 'File content as a string' }, filename: { type: 'string', description: 'Filename with extension' }, title: { type: 'string', description: 'File title displayed in Slack' }, initial_comment: { type: 'string', description: 'Message to include with the file' }, filetype: { type: 'string', description: 'File type identifier (e.g., "python", "javascript", "csv")' } }, required: ['channels', 'content', 'filename'] } },
          ],
          resources: [], prompts: [],
        },
      ],
    }
  },
  'terminate_session': (args) => {
    toast.success(`Session ${args?.sessionId || 'unknown'} terminated (demo)`)
    return null
  },
  'get_session_context_sources': (args) => ([
    { source_label: 'catalog:filesystem', item_type: 'ServerWelcome', activated: true },
    { source_label: 'catalog:filesystem__read_file', item_type: 'Tool', activated: true },
    { source_label: 'catalog:filesystem__write_file', item_type: 'Tool', activated: true },
    { source_label: 'catalog:filesystem__list_directory', item_type: 'Tool', activated: false },
    { source_label: 'catalog:filesystem__search_files', item_type: 'Tool', activated: false },
    { source_label: 'catalog:github__create_issue', item_type: 'Tool', activated: true },
    { source_label: 'catalog:github__list_repos', item_type: 'Tool', activated: false },
    { source_label: 'catalog:github__search_code', item_type: 'Tool', activated: true },
    { source_label: 'catalog:db__users', item_type: 'Resource', activated: false },
    { source_label: 'catalog:db__query', item_type: 'Prompt', activated: true },
  ]),
  'get_session_context_stats': () => ({
    content: [{
      type: 'text',
      text: '📊 Context-Mode Stats\n━━━━━━━━━━━━━━━━━━━━━\n\nSources indexed: 18\nTotal entries: 142\nFTS5 database size: 48.2 KB\n\nBreakdown:\n  catalog:    10 sources, 86 entries\n  execute:     5 sources, 38 entries\n  batch:       3 sources, 18 entries\n\nSearch queries: 24\nAvg query time: 1.2ms',
    }],
  }),
  'query_session_context_index': (args) => ({
    content: [{
      type: 'text',
      text: `🔍 Search results for "${args?.query || 'query'}"\n━━━━━━━━━━━━━━━━━━━━━\n\n--- [catalog:filesystem__read_file] ---\nRead file content from the filesystem. Supports text and binary files.\nParams: path (string, required) - The file path to read\n\n--- [catalog:filesystem__write_file] ---\nWrite content to a file. Creates parent directories if needed.\nParams: path (string), content (string)\n\n--- [catalog:github__search_code] ---\nSearch for code across GitHub repositories.\nParams: query (string), repo (string, optional)\n\nFound 3 results (1.4ms)`,
    }],
  }),
  'preview_rag_index': (args) => {
    const content = args?.content ?? ''
    const label = args?.label ?? 'tool-response:1'
    const threshold = args?.responseThresholdBytes ?? 200
    const previewBytes = Math.max(200, Math.min(500, Math.floor(threshold / 8)))
    const preview = content.substring(0, previewBytes)
    return {
      compressed_preview: `[Response compressed — ${content.length} bytes indexed as ${label}]\n\n${preview}\n\nFull output indexed. Use IndexSearch(queries=["your search terms"], source="${label}") to retrieve specific sections.`,
      index_result: {
        source_id: 1,
        label,
        total_chunks: 8,
        code_chunks: 3,
        total_lines: content.split('\n').length,
        content_bytes: content.length,
        chunk_titles: [
          { title: 'API Reference - Authentication Service', line_ref: '1', depth: 0 },
          { title: 'API Reference > Overview', line_ref: '3', depth: 1 },
          { title: 'API Reference > Endpoints', line_ref: '8', depth: 1 },
          { title: 'API Reference > Endpoints > POST /auth/login', line_ref: '10', depth: 2 },
          { title: 'API Reference > Endpoints > POST /auth/refresh', line_ref: '32', depth: 2 },
          { title: 'API Reference > Configuration', line_ref: '40', depth: 1 },
          { title: 'API Reference > Error Codes', line_ref: '55', depth: 1 },
          { title: 'API Reference > SDK Usage', line_ref: '68', depth: 1 },
        ],
      },
      sources: [{ label, total_lines: content.split('\n').length, chunk_count: 8, code_chunk_count: 3 }],
    }
  },
  'preview_rag_search': (args) => {
    const query = args?.query ?? args?.queries?.[0] ?? 'login'
    return [{
      query,
      hits: [{
        title: 'API Reference > Endpoints > POST /auth/login',
        content: '  10\t### POST /auth/login\n  11\t\n  12\tAuthenticates a user and returns an access token.\n  13\t\n  14\t**Request body:**',
        source: args?.source ?? 'tool-response:1',
        rank: -1.5,
        content_type: 'prose',
        match_layer: 'porter',
        line_start: 10,
        line_end: 30,
      }],
      corrected_query: null,
    }]
  },
  'preview_rag_read': (args) => ({
    label: args?.label ?? 'tool-response:1',
    content: '   1\t# API Reference - Authentication Service\n   2\t\n   3\t## Overview\n   4\t\n   5\tThe Authentication Service provides OAuth 2.0 and API key based\n   6\tauthentication for all microservices.\n   7\t\n   8\t## Endpoints\n   9\t\n  10\t### POST /auth/login\n  11\t\n  12\tAuthenticates a user and returns an access token.',
    total_lines: 85,
    showing_start: args?.offset ?? '1',
    showing_end: '50',
  }),
  'get_skills_config': () => ({
    paths: ["~/.localrouter/skills"],
    disabled_skills: ["test-generator"],
    async_enabled: true,
  }),
  'set_skill_enabled': (args) => {
    const skill = mockData.skills.find(s => s.name === args?.name || s.name === args?.skillName)
    if (skill) skill.enabled = args?.enabled ?? true
    return null
  },
  'rescan_skills': () => {
    toast.success('Skills rescanned (demo)')
    return mockData.skills
  },
  'add_skill_source': () => {
    toast.success('Skill source added (demo)')
    return null
  },
  'remove_skill_source': () => {
    toast.success('Skill source removed (demo)')
    return null
  },
  'create_skill': (args) => {
    toast.success(`Skill "${args?.name}" created (demo)`)
    return null
  },
  'is_user_created_skill': () => false,
  'delete_user_skill': (args) => {
    toast.success(`Skill "${args?.skillName}" deleted (demo)`)
    return null
  },
  'add_skills_path': () => {
    toast.success('Skills path added (demo)')
    return null
  },
  'remove_skills_path': () => {
    toast.success('Skills path removed (demo)')
    return null
  },

  // ============================================================================
  // Coding Agents
  // ============================================================================
  'list_coding_agents': () => [
    { agentType: 'claude_code', displayName: 'Claude Code', binaryName: 'claude', installed: true, binaryPath: '/usr/local/bin/claude', description: "Anthropic's agentic coding tool.", supportsModelSelection: true, supportedPermissionModes: ['auto', 'supervised', 'plan'], mcpToolPrefix: 'claude_code' },
    { agentType: 'gemini_cli', displayName: 'Gemini CLI', binaryName: 'gemini', installed: true, binaryPath: '/usr/local/bin/gemini', description: "Google's AI coding assistant.", supportsModelSelection: true, supportedPermissionModes: ['supervised'], mcpToolPrefix: 'gemini_cli' },
    { agentType: 'codex', displayName: 'Codex', binaryName: 'codex', installed: false, binaryPath: null, description: "OpenAI's autonomous coding agent.", supportsModelSelection: true, supportedPermissionModes: ['auto', 'supervised'], mcpToolPrefix: 'codex' },
    { agentType: 'amp', displayName: 'Amp', binaryName: 'amp', installed: false, binaryPath: null, description: "Sourcegraph's AI coding agent.", supportsModelSelection: false, supportedPermissionModes: ['supervised'], mcpToolPrefix: 'amp' },
    { agentType: 'aider', displayName: 'Aider', binaryName: 'aider', installed: true, binaryPath: '/usr/local/bin/aider', description: 'AI pair programming in your terminal.', supportsModelSelection: true, supportedPermissionModes: ['supervised'], mcpToolPrefix: 'aider' },
    { agentType: 'opencode', displayName: 'Opencode', binaryName: 'opencode', installed: false, binaryPath: null, description: 'Open-source terminal AI coding assistant.', supportsModelSelection: true, supportedPermissionModes: ['supervised'], mcpToolPrefix: 'opencode' },
    { agentType: 'cursor', displayName: 'Cursor', binaryName: 'cursor', installed: false, binaryPath: null, description: "Cursor's CLI agent.", supportsModelSelection: false, supportedPermissionModes: ['supervised'], mcpToolPrefix: 'cursor' },
    { agentType: 'qwen_code', displayName: 'Qwen Code', binaryName: 'qwen', installed: false, binaryPath: null, description: "Alibaba's coding agent.", supportsModelSelection: false, supportedPermissionModes: ['supervised'], mcpToolPrefix: 'qwen_code' },
    { agentType: 'copilot', displayName: 'Copilot', binaryName: 'copilot', installed: false, binaryPath: null, description: "GitHub Copilot's CLI extension.", supportsModelSelection: false, supportedPermissionModes: ['supervised'], mcpToolPrefix: 'copilot' },
    { agentType: 'droid', displayName: 'Droid', binaryName: 'droid', installed: false, binaryPath: null, description: 'Autonomous coding agent.', supportsModelSelection: false, supportedPermissionModes: ['supervised'], mcpToolPrefix: 'droid' },
  ],
  'list_coding_sessions': () => [],
  'get_coding_session_detail': () => ({
    sessionId: 'demo-session',
    agentType: 'claude_code',
    clientId: 'demo',
    workingDirectory: '/home/user/project',
    displayText: 'Demo session',
    status: 'done',
    createdAt: new Date().toISOString(),
    recentOutput: ['> Task completed successfully'],
    costUsd: null,
    turnCount: null,
    result: null,
    error: null,
    exitCode: null,
  }),
  'get_coding_agent_version': () => 'v1.0.23',
  'end_coding_session': () => null,
  'get_max_coding_sessions': () => 10,
  'set_max_coding_sessions': () => null,
  'set_client_coding_agent_permission': (args) => {
    const client = mockData.clients.find(c => c.client_id === args?.clientId)
    if (client) {
      (client as Record<string, unknown>).coding_agent_permission = args?.permission ?? 'off'
    }
    return null
  },
  'set_client_coding_agent_type': (args) => {
    const client = mockData.clients.find(c => c.client_id === args?.clientId)
    if (client) {
      (client as Record<string, unknown>).coding_agent_type = args?.agentType ?? null
    }
    return null
  },
  'get_coding_agent_tool_definitions': () => [
    { name: 'AgentStart', description: 'Start a new Claude Code coding session with an initial prompt', input_schema: { type: 'object', properties: { prompt: { type: 'string', description: 'The initial task/prompt' }, workingDirectory: { type: 'string', description: 'Working directory for the session' }, model: { type: 'string', description: 'Model override' }, permissionMode: { type: 'string', enum: ['auto', 'supervised', 'plan'], description: 'Permission mode' } }, required: ['prompt'] } },
    { name: 'AgentSay', description: 'Send a message to a Claude Code session. Can interrupt current work and/or resume completed sessions with context preserved.', input_schema: { type: 'object', properties: { sessionId: { type: 'string', description: 'The session ID' }, message: { type: 'string', description: 'Message to send. If session is done/error, resumes with context.' }, interrupt: { type: 'boolean', description: 'If true, interrupts current work before sending message.' }, permissionMode: { type: 'string', enum: ['auto', 'supervised', 'plan'], description: 'Switch permission mode' } }, required: ['sessionId'] } },
    { name: 'AgentStatus', description: 'Get current status and recent output of a Claude Code session. Use wait=true to block until the session needs attention.', input_schema: { type: 'object', properties: { sessionId: { type: 'string', description: 'The session ID' }, outputLines: { type: 'number', description: 'Recent output lines to return (default: 50)' }, wait: { type: 'boolean', description: 'Block until session needs attention' }, timeoutSeconds: { type: 'number', description: 'Max seconds to wait (default: 300)' } }, required: ['sessionId'] } },
    { name: 'AgentList', description: 'List all Claude Code sessions for this client', input_schema: { type: 'object', properties: { limit: { type: 'number', description: 'Max sessions to return (default: 50)' } } } },
  ],
  'get_context_mode_tool_definitions': () => [
    { name: 'IndexSearch', description: 'Search indexed content. Pass ALL search questions as queries array in ONE call.\n\nTIPS: 2-4 specific terms per query. Use \'source\' to scope results.', input_schema: { type: 'object', properties: { query: { type: 'string', description: 'A single search query string.' }, queries: { type: 'array', items: { type: 'string' }, description: 'Array of search queries. Batch ALL questions in one call.' }, source: { type: 'string', description: 'Filter to a specific indexed source (partial match).' }, limit: { type: 'number', description: 'Results per query (default: 3)' } } } },
    { name: 'IndexRead', description: 'Read the full content of an indexed source. Use after IndexSearch to get complete context around a search hit.', input_schema: { type: 'object', properties: { label: { type: 'string', description: 'Source label to read (from search results)' }, offset: { type: 'string', description: 'Line offset to start from (e.g. "5" or "5-2" for sub-line). Default: start of content.' }, limit: { type: 'number', description: 'Number of lines to return (default: 15)' } }, required: ['label'] } },
  ],
  'get_marketplace_tool_definitions': () => [
    { name: 'marketplace__search', description: 'Search the marketplace for MCP servers and skills', input_schema: { type: 'object', properties: { query: { type: 'string', description: 'Search query' }, type: { type: 'string', enum: ['mcp', 'skill', 'all'], description: 'Item type' } }, required: ['query'] } },
    { name: 'marketplace__install', description: 'Install an MCP server or skill from the marketplace', input_schema: { type: 'object', properties: { name: { type: 'string', description: 'Item name' }, source: { type: 'string', description: 'Source ID' }, type: { type: 'string', enum: ['mcp', 'skill'], description: 'Item type' } }, required: ['name', 'source', 'type'] } },
  ],

  // ============================================================================
  // Logging
  // ============================================================================
  'get_logging_config': () => mockData.loggingConfig,
  'update_logging_config': (args) => {
    if (args?.updates) {
      Object.assign(mockData.loggingConfig, args.updates)
    }
    return null
  },
  'get_llm_logs': () => {
    // Generate fresh logs with current timestamps
    const models = ['gpt-4o', 'gpt-4o-mini', 'claude-3-5-sonnet-20241022', 'claude-3-5-haiku-20241022', 'gemini-1.5-pro']
    const clients = ['cursor-ide', 'claude-code']
    const providers = ['openai-primary', 'anthropic-main', 'gemini-google', 'groq-fast']
    const now = Date.now()
    return Array.from({ length: 20 }, (_, i) => {
      const model = models[Math.floor(Math.random() * models.length)]
      const provider = model.startsWith('gpt') ? 'openai-primary' :
                       model.startsWith('claude') ? 'anthropic-main' :
                       model.startsWith('gemini') ? 'gemini-google' : providers[Math.floor(Math.random() * providers.length)]
      return {
        id: `log-${i + 1}`,
        timestamp: new Date(now - (i * 30000 + Math.random() * 30000)).toISOString(),
        client_id: clients[Math.floor(Math.random() * clients.length)],
        model,
        provider,
        request_tokens: Math.floor(Math.random() * 3000) + 500,
        response_tokens: Math.floor(Math.random() * 2000) + 100,
        latency_ms: Math.floor(Math.random() * 3000) + 500,
        status: Math.random() > 0.05 ? 'success' : 'error',
        cost: Math.random() * 0.1,
      }
    })
  },
  'get_mcp_logs': () => {
    // Generate fresh MCP logs with current timestamps
    const servers = ['mcp-github', 'mcp-filesystem', 'mcp-slack', 'mcp-browser']
    const toolsByServer: Record<string, string[]> = {
      'mcp-github': ['create_issue', 'list_repos', 'search_code', 'get_file_contents'],
      'mcp-filesystem': ['read_file', 'write_file', 'list_directory', 'search_files'],
      'mcp-slack': ['send_message', 'list_channels', 'get_channel_history', 'search_messages'],
      'mcp-browser': ['navigate', 'screenshot', 'click', 'type'],
    }
    const clients = ['cursor-ide', 'claude-code']
    const now = Date.now()
    return Array.from({ length: 15 }, (_, i) => {
      const serverId = servers[Math.floor(Math.random() * servers.length)]
      const tools = toolsByServer[serverId] || ['execute']
      return {
        id: `mcp-log-${i + 1}`,
        timestamp: new Date(now - (i * 20000 + Math.random() * 20000)).toISOString(),
        client_id: clients[Math.floor(Math.random() * clients.length)],
        server_id: serverId,
        tool: tools[Math.floor(Math.random() * tools.length)],
        latency_ms: Math.floor(Math.random() * 500) + 10,
        status: Math.random() > 0.03 ? 'success' : 'error',
      }
    })
  },
  'open_logs_folder': () => {
    toast.info('Opening logs folder (demo)')
    return null
  },

  // ============================================================================
  // RouteLLM
  // ============================================================================
  'routellm_get_status': () => mockData.routellmStatus,
  'get_routellm_status': () => mockData.routellmStatus,
  'routellm_update_settings': (args) => {
    if (args?.settings) {
      Object.assign(mockData.routellmStatus, args.settings)
    }
    return null
  },
  'routellm_download_models': () => {
    toast.info('Downloading RouteLLM models (demo - not actually downloading)')
    return null
  },
  'routellm_unload': () => {
    mockData.routellmStatus.model_loaded = false
    toast.success('RouteLLM model unloaded (demo)')
    return null
  },
  // Returns: RouteLLMTestResult (src/types/tauri-commands.ts)
  'routellm_test_prediction': (args): RouteLLMTestResult => {
    // Simulate a realistic prediction based on prompt complexity
    const prompt = args?.prompt || ''
    const threshold = args?.threshold ?? 0.3
    // Generate a score that varies based on prompt characteristics
    const baseScore = prompt.length > 200 || prompt.includes('code') || prompt.includes('analyze') ? 0.7 : 0.2
    const winRate = Math.min(1, Math.max(0, baseScore + (Math.random() * 0.3 - 0.15)))
    return {
      win_rate: winRate,
      is_strong: winRate >= threshold,
      latency_ms: Math.floor(Math.random() * 50) + 20,
    }
  },
  'routellm_delete_model': () => {
    toast.success('Strong/Weak model deleted (demo)')
    return null
  },
  'open_routellm_folder': () => {
    toast.info('Opening RouteLLM folder (demo)')
    return null
  },

  // ============================================================================
  // Updates
  // ============================================================================
  'get_update_config': () => mockData.updateConfig,
  'update_update_config': (args) => {
    if (args?.updates) {
      Object.assign(mockData.updateConfig, args.updates)
    }
    return null
  },
  'set_update_notification': () => null,
  'mark_update_check_performed': () => {
    mockData.updateConfig.last_check = new Date().toISOString()
    return null
  },
  'skip_update_version': (args) => {
    if (args?.version) {
      mockData.updateConfig.skipped_version = args.version
    }
    return null
  },

  // ============================================================================
  // Marketplace
  // ============================================================================
  'get_marketplace_config': () => mockData.marketplaceConfig,
  'marketplace_set_enabled': (args) => {
    mockData.marketplaceConfig.mcp_enabled = args?.enabled ?? true
    mockData.marketplaceConfig.skills_enabled = args?.enabled ?? true
    return null
  },
  'marketplace_set_mcp_enabled': (args) => {
    mockData.marketplaceConfig.mcp_enabled = args?.enabled ?? true
    return null
  },
  'marketplace_set_skills_enabled': (args) => {
    mockData.marketplaceConfig.skills_enabled = args?.enabled ?? true
    return null
  },
  'marketplace_set_registry_url': () => null,
  'marketplace_refresh_cache': () => {
    toast.info('Refreshing marketplace cache (demo)')
    return null
  },
  'marketplace_clear_mcp_cache': () => {
    toast.success('MCP cache cleared (demo)')
    return null
  },
  'marketplace_clear_skills_cache': () => {
    toast.success('Skills cache cleared (demo)')
    return null
  },
  'marketplace_install_mcp_server_direct': () => {
    toast.success('MCP server installed (demo)')
    return null
  },
  'marketplace_install_skill_direct': () => {
    toast.success('Skill installed (demo)')
    return null
  },
  'marketplace_delete_skill': () => {
    toast.success('Skill deleted (demo)')
    return null
  },
  'marketplace_add_skill_source': () => {
    toast.success('Skill source added (demo)')
    return null
  },
  'marketplace_remove_skill_source': () => {
    toast.success('Skill source removed (demo)')
    return null
  },
  'marketplace_add_default_skill_sources': () => {
    toast.success('Default skill sources added (demo)')
    return null
  },
  'marketplace_reset_registry_url': () => {
    toast.success('Registry URL reset (demo)')
    return null
  },
  'marketplace_get_config': () => mockData.marketplaceConfig,
  'marketplace_get_cache_status': () => ({
    mcp_servers_cached: 25,
    skills_cached: 15,
    last_refresh: new Date().toISOString(),
  }),
  'marketplace_is_skill_from_marketplace': () => false,
  'marketplace_search_mcp_servers': (args) => {
    // McpServerListing format matches the actual interface
    const allServers = [
      {
        name: 'GitHub',
        description: 'Official GitHub MCP server for repository management, issues, PRs, and Actions',
        source_id: 'mcp-registry',
        homepage: 'https://github.com/modelcontextprotocol/servers/tree/main/src/github',
        vendor: 'GitHub',
        packages: [
          { registry: 'npm', name: '@modelcontextprotocol/server-github', version: '0.6.2', runtime: 'node', license: 'MIT' },
        ],
        remotes: [
          { transport_type: 'sse', url: 'https://mcp.github.com/sse' },
        ],
        available_transports: ['stdio', 'sse'],
        install_hint: 'npx -y @modelcontextprotocol/server-github',
      },
      {
        name: 'Slack',
        description: 'Connect to Slack workspaces for messaging and channel management',
        source_id: 'mcp-registry',
        homepage: 'https://github.com/modelcontextprotocol/servers/tree/main/src/slack',
        vendor: 'Slack',
        packages: [
          { registry: 'npm', name: '@modelcontextprotocol/server-slack', version: '0.6.2', runtime: 'node', license: 'MIT' },
        ],
        remotes: [
          { transport_type: 'sse', url: 'https://mcp.slack.com/sse' },
        ],
        available_transports: ['stdio', 'sse'],
        install_hint: 'npx -y @modelcontextprotocol/server-slack',
      },
      {
        name: 'Filesystem',
        description: 'Read, write, and manage local files with secure sandboxing',
        source_id: 'mcp-registry',
        homepage: 'https://github.com/modelcontextprotocol/servers/tree/main/src/filesystem',
        vendor: 'Anthropic',
        packages: [
          { registry: 'npm', name: '@modelcontextprotocol/server-filesystem', version: '0.6.2', runtime: 'node', license: 'MIT' },
        ],
        remotes: [],
        available_transports: ['stdio'],
        install_hint: 'npx -y @modelcontextprotocol/server-filesystem /path/to/allowed/directory',
      },
      {
        name: 'PostgreSQL',
        description: 'Query and manage PostgreSQL databases with natural language',
        source_id: 'mcp-registry',
        homepage: 'https://github.com/modelcontextprotocol/servers/tree/main/src/postgres',
        vendor: 'Community',
        packages: [
          { registry: 'npm', name: '@modelcontextprotocol/server-postgres', version: '0.6.2', runtime: 'node', license: 'MIT' },
        ],
        remotes: [],
        available_transports: ['stdio'],
        install_hint: 'npx -y @modelcontextprotocol/server-postgres postgresql://localhost/mydb',
      },
      {
        name: 'Puppeteer',
        description: 'Browser automation and web scraping with Puppeteer',
        source_id: 'mcp-registry',
        homepage: 'https://github.com/modelcontextprotocol/servers/tree/main/src/puppeteer',
        vendor: 'Community',
        packages: [
          { registry: 'npm', name: '@modelcontextprotocol/server-puppeteer', version: '0.6.2', runtime: 'node', license: 'MIT' },
        ],
        remotes: [],
        available_transports: ['stdio'],
        install_hint: 'npx -y @modelcontextprotocol/server-puppeteer',
      },
      {
        name: 'Brave Search',
        description: 'Search the web using Brave Search API',
        source_id: 'mcp-registry',
        homepage: 'https://github.com/modelcontextprotocol/servers/tree/main/src/brave-search',
        vendor: 'Brave',
        packages: [
          { registry: 'npm', name: '@modelcontextprotocol/server-brave-search', version: '0.6.2', runtime: 'node', license: 'MIT' },
        ],
        remotes: [],
        available_transports: ['stdio'],
        install_hint: 'BRAVE_API_KEY=your_key npx -y @modelcontextprotocol/server-brave-search',
      },
    ]
    const query = (args?.query || '').toLowerCase()
    if (!query) return allServers
    return allServers.filter(s =>
      s.name.toLowerCase().includes(query) ||
      s.description.toLowerCase().includes(query) ||
      (s.vendor || '').toLowerCase().includes(query)
    )
  },
  'marketplace_search_skills': (args) => {
    // SkillListing format matches the actual interface
    const baseUrl = 'https://github.com/localrouter/skills/blob/main'
    const allSkills = [
      {
        name: 'web-search-pro',
        description: 'Advanced web search with multiple engines (Google, Bing, DuckDuckGo)',
        source_id: 'official-skills',
        version: '2.0.0',
        author: 'LocalRouter',
        tags: ['search', 'web', 'research'],
        source_label: 'Official Skills',
        source_repo: 'https://github.com/localrouter/skills',
        source_path: 'skills/web-search-pro',
        source_branch: 'main',
        skill_md_url: `${baseUrl}/skills/web-search-pro/SKILL.md`,
        is_multi_file: true,
        files: [
          { path: 'search.js', url: `${baseUrl}/skills/web-search-pro/search.js` },
          { path: 'engines.js', url: `${baseUrl}/skills/web-search-pro/engines.js` },
          { path: 'config.json', url: `${baseUrl}/skills/web-search-pro/config.json` },
        ],
      },
      {
        name: 'code-analysis',
        description: 'Static code analysis, security scanning, and code quality metrics',
        source_id: 'community-skills',
        version: '1.5.0',
        author: 'Community',
        tags: ['code', 'security', 'analysis'],
        source_label: 'Community Skills',
        source_repo: 'https://github.com/community/localrouter-skills',
        source_path: 'code-analysis',
        source_branch: 'main',
        skill_md_url: 'https://github.com/community/localrouter-skills/blob/main/code-analysis/SKILL.md',
        is_multi_file: true,
        files: [
          { path: 'analyze.js', url: 'https://github.com/community/localrouter-skills/blob/main/code-analysis/analyze.js' },
          { path: 'security.js', url: 'https://github.com/community/localrouter-skills/blob/main/code-analysis/security.js' },
          { path: 'patterns.json', url: 'https://github.com/community/localrouter-skills/blob/main/code-analysis/patterns.json' },
        ],
      },
      {
        name: 'image-generator',
        description: 'Generate images using DALL-E, Midjourney, and Stable Diffusion APIs',
        source_id: 'official-skills',
        version: '1.2.0',
        author: 'LocalRouter',
        tags: ['image', 'ai', 'creative'],
        source_label: 'Official Skills',
        source_repo: 'https://github.com/localrouter/skills',
        source_path: 'skills/image-generator',
        source_branch: 'main',
        skill_md_url: `${baseUrl}/skills/image-generator/SKILL.md`,
        is_multi_file: true,
        files: [
          { path: 'generate.js', url: `${baseUrl}/skills/image-generator/generate.js` },
          { path: 'providers.js', url: `${baseUrl}/skills/image-generator/providers.js` },
        ],
      },
      {
        name: 'data-visualizer',
        description: 'Create charts, graphs, and data visualizations from datasets',
        source_id: 'community-skills',
        version: '1.0.1',
        author: 'Community',
        tags: ['data', 'charts', 'visualization'],
        source_label: 'Community Skills',
        source_repo: 'https://github.com/community/localrouter-skills',
        source_path: 'data-visualizer',
        source_branch: 'main',
        skill_md_url: 'https://github.com/community/localrouter-skills/blob/main/data-visualizer/SKILL.md',
        is_multi_file: true,
        files: [
          { path: 'chart.js', url: 'https://github.com/community/localrouter-skills/blob/main/data-visualizer/chart.js' },
          { path: 'templates.json', url: 'https://github.com/community/localrouter-skills/blob/main/data-visualizer/templates.json' },
        ],
      },
      {
        name: 'pdf-tools',
        description: 'Extract, summarize, and manipulate PDF documents',
        source_id: 'community-skills',
        version: '1.1.0',
        author: 'Community',
        tags: ['pdf', 'documents', 'extraction'],
        source_label: 'Community Skills',
        source_repo: 'https://github.com/community/localrouter-skills',
        source_path: 'pdf-tools',
        source_branch: 'main',
        skill_md_url: 'https://github.com/community/localrouter-skills/blob/main/pdf-tools/SKILL.md',
        is_multi_file: false,
        files: [
          { path: 'pdf.js', url: 'https://github.com/community/localrouter-skills/blob/main/pdf-tools/pdf.js' },
        ],
      },
      {
        name: 'universal-translator',
        description: 'Translate text between 100+ languages with context awareness',
        source_id: 'official-skills',
        version: '2.1.0',
        author: 'LocalRouter',
        tags: ['translation', 'language', 'i18n'],
        source_label: 'Official Skills',
        source_repo: 'https://github.com/localrouter/skills',
        source_path: 'skills/universal-translator',
        source_branch: 'main',
        skill_md_url: `${baseUrl}/skills/universal-translator/SKILL.md`,
        is_multi_file: true,
        files: [
          { path: 'translate.js', url: `${baseUrl}/skills/universal-translator/translate.js` },
          { path: 'languages.json', url: `${baseUrl}/skills/universal-translator/languages.json` },
        ],
      },
    ]
    const query = (args?.query || '').toLowerCase()
    if (!query) return allSkills
    return allSkills.filter(s =>
      s.name.toLowerCase().includes(query) ||
      (s.description || '').toLowerCase().includes(query) ||
      s.tags?.some(t => t.includes(query))
    )
  },
  'search_marketplace': (_args) => {
    // Combined search across both MCP servers and skills
    // In real implementation this would search both MCP servers and skills
    return [] as unknown[]
  },

  // ============================================================================
  // Tray & UI
  // ============================================================================
  'get_tray_graph_settings': () => mockData.trayGraphSettings,
  'update_tray_graph_settings': (args) => {
    if (args) {
      if (typeof args.enabled === 'boolean') {
        mockData.trayGraphSettings.enabled = args.enabled
      }
      if (typeof args.refreshRateSecs === 'number') {
        mockData.trayGraphSettings.refresh_rate_secs = args.refreshRateSecs
      }
    }
    return null
  },
  'get_sidebar_expanded': (): boolean => mockData.sidebarExpanded,
  'set_sidebar_expanded': (args): null => {
    if (args && typeof args.expanded === 'boolean') {
      mockData.sidebarExpanded = args.expanded
    }
    return null
  },

  // ============================================================================
  // Memory - Persistent Conversation Memory
  // ============================================================================
  'get_memory_config': () => ({
    compaction_model: null,
    search_top_k: 5,
    session_inactivity_minutes: 180,
    max_session_minutes: 480,
    recall_tool_name: 'MemorySearch',
  }),
  'update_memory_config': () => {
    toast.success('Memory configuration saved (demo)')
    return null
  },
  'open_memory_folder': () => {
    toast.info('Would open memory folder (demo)')
    return null
  },
  'memory_test_index': () => {
    toast.success('Content indexed (demo)')
    return null
  },
  'memory_test_reset': () => null,
  'memory_test_search': () => {
    return 'Found 1 result:\n\n1. We decided to use PostgreSQL for the auth service because MySQL had connection pooling issues. [score: 0.87]\n   Source: sessions/test-memory.md\n'
  },
  'get_client_memory_config': () => ({
    memory_enabled: null,
  }),
  'update_client_memory_config': () => {
    toast.success('Client memory config updated (demo)')
    return null
  },

  // ============================================================================
  // GuardRails - LLM-based Safety Models
  // ============================================================================
  'get_guardrails_config': () => ({
    scan_requests: true,
    safety_models: [
      { id: 'llama_guard', label: 'Llama Guard 3 1B via Ollama', model_type: 'llama_guard', provider_id: 'ollama', model_name: 'llama-guard3:1b', confidence_threshold: null, enabled_categories: null, prompt_template: null, safe_indicator: null, output_regex: null, category_mapping: null },
      { id: 'granite_guardian', label: 'Granite Guardian 3.0 2B via Ollama', model_type: 'granite_guardian', provider_id: 'ollama', model_name: 'granite3-guardian:2b', confidence_threshold: null, enabled_categories: null, prompt_template: null, safe_indicator: null, output_regex: null, category_mapping: null },
    ],
    category_actions: [
      { category: '__global', action: 'ask' },
    ],
    default_confidence_threshold: 0.5,
    parallel_guardrails: true,
    moderation_api_enabled: true,
  }),
  'update_guardrails_config': () => {
    toast.success('GuardRails configuration saved (demo)')
    return null
  },
  'rebuild_safety_engine': () => {
    return null
  },
  'test_safety_check': (args) => {
    const text = args?.text || ''
    const hasInjection = /ignore.*previous|ignore.*instructions|DAN\s+mode/i.test(text)
    return {
      verdicts: [
        {
          model_id: 'llama_guard',
          is_safe: !hasInjection,
          flagged_categories: hasInjection ? [{ category: 'ViolentCrimes', confidence: null, native_label: 'S1' }] : [],
          confidence: null,
          raw_output: hasInjection ? 'unsafe\nS1' : 'safe',
          check_duration_ms: 142,
        },
        {
          model_id: 'granite_guardian',
          is_safe: !hasInjection,
          flagged_categories: hasInjection ? [{ category: 'Jailbreak', confidence: 0.92, native_label: 'jailbreak' }] : [],
          confidence: hasInjection ? 0.92 : 0.03,
          raw_output: hasInjection ? 'Yes' : 'No',
          check_duration_ms: 245,
        },
      ],
      actions_required: hasInjection ? [
        { category: 'ViolentCrimes', action: 'ask' as const, model_id: 'llama_guard', confidence: null },
        { category: 'Jailbreak', action: 'ask' as const, model_id: 'granite_guardian', confidence: 0.92 },
      ] : [],
      total_duration_ms: hasInjection ? 387 : 340,
      scan_direction: 'request' as const,
    }
  },
  'test_safety_check_single_model': (args) => {
    const text = args?.text || ''
    const hasInjection = /ignore.*previous|ignore.*instructions|DAN\s+mode/i.test(text)
    return [{
      model_id: args?.modelId || 'granite_guardian',
      is_safe: !hasInjection,
      flagged_categories: hasInjection ? [{ category: 'Jailbreak', confidence: 0.88, native_label: 'jailbreak' }] : [],
      confidence: hasInjection ? 0.88 : null,
      raw_output: hasInjection ? 'Yes' : 'No',
      check_duration_ms: 312,
    }]
  },
  'get_safety_model_status': (args) => ({
    id: args?.modelId || 'granite_guardian',
    label: 'Granite Guardian',
    model_type: 'granite_guardian',
    provider_configured: true,
    model_available: true,
  }),
  'test_safety_model': (args) => {
    const text = args?.text || ''
    const hasInjection = /ignore.*previous|ignore.*instructions|DAN\s+mode/i.test(text)
    return {
      verdicts: [{
        model_id: args?.modelId || 'granite_guardian',
        is_safe: !hasInjection,
        flagged_categories: hasInjection ? [{ category: 'Jailbreak', confidence: 0.88, native_label: 'jailbreak' }] : [],
        confidence: hasInjection ? 0.88 : null,
        raw_output: hasInjection ? 'Yes' : 'No',
        check_duration_ms: 312,
      }],
      actions_required: hasInjection ? [{ category: 'Jailbreak', action: 'ask' as const, model_id: args?.modelId || 'granite_guardian', confidence: 0.88 }] : [],
      total_duration_ms: 312,
      scan_direction: 'request' as const,
    }
  },
  'get_all_safety_categories': () => ([
    { category: 'ViolentCrimes', display_name: 'Violent Crimes', description: 'Content promoting violent criminal activities', supported_by: ['llama_guard', 'nemotron'] },
    { category: 'ChildExploitation', display_name: 'Child Exploitation', description: 'Content involving child sexual abuse material', supported_by: ['llama_guard', 'nemotron'] },
    { category: 'Hate', display_name: 'Hate Speech', description: 'Content promoting hatred against protected groups', supported_by: ['llama_guard', 'nemotron', 'shield_gemma'] },
    { category: 'SelfHarm', display_name: 'Self-Harm', description: 'Content promoting self-harm or suicide', supported_by: ['llama_guard', 'nemotron'] },
    { category: 'SexualContent', display_name: 'Sexual Content', description: 'Explicit sexual content', supported_by: ['llama_guard', 'nemotron', 'shield_gemma'] },
    { category: 'DangerousContent', display_name: 'Dangerous Content', description: 'Content about creating weapons or dangerous materials', supported_by: ['shield_gemma', 'nemotron'] },
    { category: 'Harassment', display_name: 'Harassment', description: 'Content meant to harass or bully', supported_by: ['shield_gemma'] },
    { category: 'Jailbreak', display_name: 'Jailbreak', description: 'Attempts to bypass AI safety restrictions', supported_by: ['granite_guardian'] },
    { category: 'SocialBias', display_name: 'Social Bias', description: 'Content exhibiting social bias or stereotypes', supported_by: ['granite_guardian'] },
    { category: 'Groundedness', display_name: 'Groundedness', description: 'Responses not grounded in provided context (RAG)', supported_by: ['granite_guardian'] },
  ]),
  'add_safety_model': () => {
    toast.success('Safety model added (demo)')
    return generateId()
  },
  'remove_safety_model': () => {
    toast.success('Safety model removed (demo)')
    return null
  },
  'pull_provider_model': (args) => {
    toast.info(`Pulling model "${args?.modelName}" from ${args?.providerId} (demo)`)
    return null
  },

  // ============================================================================
  // Prompt Compression
  // ============================================================================
  'get_compression_config': () => ({
    enabled: false,
    model_size: 'bert',
    default_rate: 0.8,
    compress_system_prompt: false,
    min_messages: 6,
    preserve_recent: 4,
    min_message_words: 5,
    preserve_quoted_text: true,
    compression_notice: true,
  }),
  'update_compression_config': () => {
    toast.success('Compression configuration saved (demo)')
    return null
  },
  'get_compression_status': () => ({
    model_downloaded: false,
    model_loaded: false,
    model_size_bytes: null,
    model_repo: 'microsoft/llmlingua-2-bert-base-multilingual-cased-meetingbank',
  }),
  'install_compression': () => {
    toast.success('Compression model downloaded (demo)')
    return null
  },
  'rebuild_compression_engine': () => null,
  'get_embedding_status': () => ({
    downloaded: false,
    loaded: false,
    model_name: 'all-MiniLM-L6-v2',
    model_size_mb: null,
  }),
  'install_embedding_model': () => {
    toast.success('Embedding model downloaded (demo)')
    return 'Embedding model downloaded and loaded successfully'
  },
  'test_compression': (args) => {
    const text = args?.text || ''
    const rate = args?.rate || 0.5
    const preserveQuoted = args?.preserveQuoted ?? true
    const compressionNotice = args?.compressionNotice ?? false
    // Parse words with byte positions for whitespace-preserving reconstruction
    const wordTokens: { word: string; start: number; end: number }[] = []
    const wordRegex = /\S+/g
    let m
    while ((m = wordRegex.exec(text)) !== null) {
      wordTokens.push({ word: m[0], start: m.index, end: m.index + m[0].length })
    }
    const words = wordTokens.map(t => t.word)
    const keepCount = Math.max(1, Math.round(words.length * rate))

    // Basic protection detection for demo: words touching quotes or backticks
    const protectedIndices: number[] = []
    if (preserveQuoted) {
      let inFenced = false
      let inBacktick = false
      let inQuote = false
      for (let i = 0; i < words.length; i++) {
        const w = words[i]
        if (w.includes('```')) { inFenced = !inFenced; protectedIndices.push(i); continue }
        if (inFenced) { protectedIndices.push(i); continue }
        if (w.startsWith('`') && w.endsWith('`') && w.length > 1) { protectedIndices.push(i); continue }
        if (w.startsWith('`') && !inBacktick) { inBacktick = true; protectedIndices.push(i); continue }
        if (w.endsWith('`') && inBacktick) { inBacktick = false; protectedIndices.push(i); continue }
        if (inBacktick) { protectedIndices.push(i); continue }
        if (w.startsWith('"') && !inQuote) { inQuote = true; protectedIndices.push(i) }
        if (inQuote) { protectedIndices.push(i) }
        if (inQuote && (w.endsWith('"') || w.endsWith('",') || w.endsWith('".') || w.endsWith('":'))) { inQuote = false }
      }
    }

    const protectedSet = new Set(protectedIndices)
    const keptIndices = Array.from({ length: keepCount }, (_, i) => i)
    // Union with protected
    for (const pi of protectedIndices) {
      if (!keptIndices.includes(pi)) keptIndices.push(pi)
    }
    keptIndices.sort((a, b) => a - b)

    // Reconstruct preserving original whitespace
    let compressed = ''
    for (let k = 0; k < keptIndices.length; k++) {
      const idx = keptIndices[k]
      const { word: w, start: wStart, end: wEnd } = wordTokens[idx]
      if (k === 0) {
        compressed += idx === 0 ? text.slice(0, wEnd) : w
      } else {
        const prevKeptIdx = keptIndices[k - 1]
        if (prevKeptIdx + 1 === idx) {
          // Consecutive in original: preserve exact whitespace
          compressed += text.slice(wordTokens[idx - 1].end, wEnd)
        } else {
          const gap = text.slice(wordTokens[prevKeptIdx].end, wStart)
          if (gap.includes('\n')) {
            compressed += text.slice(wordTokens[idx - 1].end, wEnd)
          } else {
            compressed += ' ' + w
          }
        }
      }
    }
    if (compressionNotice) compressed = '[abridged] ' + compressed

    return {
      compressed_text: compressed,
      original_tokens: words.length,
      compressed_tokens: keptIndices.length,
      ratio: words.length / Math.max(1, keptIndices.length),
      kept_indices: keptIndices,
      protected_indices: protectedIndices,
    }
  },
  'get_client_compression_config': () => ({
    enabled: null,
  }),
  'update_client_compression_config': () => {
    toast.success('Client compression configuration saved (demo)')
    return null
  },

  // ============================================================================
  // JSON Repair
  // ============================================================================
  'get_json_repair_config': () => ({
    enabled: true,
    syntax_repair: true,
    schema_coercion: false,
    strip_extra_fields: false,
    add_defaults: false,
    normalize_enums: true,
  }),
  'update_json_repair_config': () => {
    toast.success('JSON repair configuration saved (demo)')
    return null
  },
  'test_json_repair': (args) => {
    const content = args?.content || ''
    // Simple demo: just try to fix trailing commas
    const repaired = content.replace(/,\s*([}\]])/g, '$1')
    const wasModified = repaired !== content
    return {
      original: content,
      repaired: repaired,
      was_modified: wasModified,
      repairs: wasModified ? ['syntax_repaired'] : [],
    }
  },

  // ============================================================================
  // Secret Scanning
  // ============================================================================
  'get_secret_scanning_config': () => ({
    action: 'off',
    entropy_threshold: 3.5,
    scan_system_messages: false,
    allowlist: [],
  }),
  'update_secret_scanning_config': () => {
    toast.success('Secret scanning configuration saved (demo)')
    return null
  },
  'rebuild_secret_scanner': () => null,
  'get_client_secret_scanning_config': () => ({
    action: null,
  }),
  'update_client_secret_scanning_config': () => {
    toast.success('Client secret scanning configuration saved (demo)')
    return null
  },
  'test_secret_scan': () => ({
    findings: [
      {
        rule_id: 'aws-access-key-id',
        rule_description: 'AWS Access Key ID',
        category: 'Cloud Provider',
        regex_pattern: '(?:^|[^A-Za-z0-9/+=])(AKIA[0-9A-Z]{16})(?:[^A-Za-z0-9/+=]|$)',
        keywords: ['AKIA'],
        rule_entropy_threshold: 3.0,
        message_index: 0,
        matched_text: 'AKIA**************MPLE',
        entropy: 3.42,
      },
    ],
    scan_duration_ms: 1,
    rules_evaluated: 30,
  }),
  'get_secret_scanning_patterns': () => [
    { id: 'aws-access-key-id', description: 'AWS Access Key ID', regex: '(?:^|[^A-Za-z0-9/+=])(AKIA[0-9A-Z]{16})(?:[^A-Za-z0-9/+=]|$)', category: 'Cloud Provider', entropy_threshold: 3.0, keywords: ['AKIA'] },
    { id: 'github-pat', description: 'GitHub Personal Access Token', regex: 'ghp_[A-Za-z0-9]{36,}', category: 'Version Control', entropy_threshold: 3.5, keywords: ['ghp_'] },
  ],

  // ============================================================================
  // Free Tier
  // ============================================================================
  'get_free_tier_status': () => {
    return [
      {
        provider_instance: 'ollama',
        provider_type: 'ollama',
        display_name: 'ollama',
        free_tier: { kind: 'always_free_local' },
        is_user_override: false,
        supports_credit_check: false,
        rate_rpm_used: null, rate_rpm_limit: null,
        rate_rpd_used: null, rate_rpd_limit: null,
        rate_tpm_used: null, rate_tpm_limit: null,
        rate_tpd_used: null, rate_tpd_limit: null,
        rate_monthly_calls_used: null, rate_monthly_calls_limit: null,
        rate_monthly_tokens_used: null, rate_monthly_tokens_limit: null,
        credit_used_usd: null, credit_budget_usd: null, credit_remaining_usd: null,
        is_backed_off: false, backoff_retry_after_secs: null, backoff_reason: null,
        has_capacity: true, status_message: 'Always free',
      },
      {
        provider_instance: 'groq-fast',
        provider_type: 'groq',
        display_name: 'groq-fast',
        free_tier: { kind: 'rate_limited_free', max_rpm: 30, max_rpd: 14400, max_tpm: 6000, max_tpd: 500000, max_monthly_calls: 0, max_monthly_tokens: 0 },
        is_user_override: false,
        supports_credit_check: false,
        rate_rpm_used: 12, rate_rpm_limit: 30,
        rate_rpd_used: 245, rate_rpd_limit: 14400,
        rate_tpm_used: 2100, rate_tpm_limit: 6000,
        rate_tpd_used: null, rate_tpd_limit: null,
        rate_monthly_calls_used: null, rate_monthly_calls_limit: null,
        rate_monthly_tokens_used: null, rate_monthly_tokens_limit: null,
        credit_used_usd: null, credit_budget_usd: null, credit_remaining_usd: null,
        is_backed_off: false, backoff_retry_after_secs: null, backoff_reason: null,
        has_capacity: true, status_message: 'Available',
      },
      {
        provider_instance: 'openai-primary',
        provider_type: 'openai',
        display_name: 'openai-primary',
        free_tier: { kind: 'none' },
        is_user_override: false,
        supports_credit_check: false,
        rate_rpm_used: null, rate_rpm_limit: null,
        rate_rpd_used: null, rate_rpd_limit: null,
        rate_tpm_used: null, rate_tpm_limit: null,
        rate_tpd_used: null, rate_tpd_limit: null,
        rate_monthly_calls_used: null, rate_monthly_calls_limit: null,
        rate_monthly_tokens_used: null, rate_monthly_tokens_limit: null,
        credit_used_usd: null, credit_budget_usd: null, credit_remaining_usd: null,
        is_backed_off: false, backoff_retry_after_secs: null, backoff_reason: null,
        has_capacity: false, status_message: 'No free tier',
      },
    ]
  },
  'set_provider_free_tier': () => {
    toast.success('Free tier config updated (demo)')
    return null
  },
  'reset_provider_free_tier_usage': () => {
    toast.success('Free tier usage reset (demo)')
    return null
  },
  'set_provider_free_tier_usage': () => {
    toast.success('Free tier usage updated (demo)')
    return null
  },
  'get_default_free_tier': (args) => {
    const defaults: Record<string, unknown> = {
      ollama: { kind: 'always_free_local' },
      lmstudio: { kind: 'always_free_local' },
      groq: { kind: 'rate_limited_free', max_rpm: 30, max_rpd: 14400, max_tpm: 6000, max_tpd: 500000, max_monthly_calls: 0, max_monthly_tokens: 0 },
      gemini: { kind: 'rate_limited_free', max_rpm: 10, max_rpd: 250, max_tpm: 250000, max_tpd: 0, max_monthly_calls: 0, max_monthly_tokens: 0 },
      openrouter: { kind: 'credit_based', budget_usd: 0.0, reset_period: 'never', detection: { type: 'provider_api' } },
      openai: { kind: 'none' },
      anthropic: { kind: 'none' },
    }
    return defaults[args?.providerType || ''] || { kind: 'none' }
  },

  // ============================================================================
  // Firewall
  // ============================================================================
  'get_firewall_approval_details': () => null,
  'get_firewall_full_arguments': () => JSON.stringify({
    model: 'gpt-4',
    messages: [
      { role: 'system', content: 'You are a helpful assistant.' },
      { role: 'user', content: 'Hello, how are you?' },
    ],
    temperature: 0.7,
    max_tokens: 1000,
    top_p: null,
    frequency_penalty: null,
    presence_penalty: null,
    seed: null,
  }),
  'submit_firewall_approval': () => {
    toast.success('Approval submitted (demo)')
    return null
  },
  'debug_trigger_firewall_popup': (args: Record<string, unknown>) => {
    const count = args?.sendMultiple ? 3 : 1
    toast.info(`Firewall popup triggered (demo, ${count} popup${count > 1 ? 's' : ''})`)
    return null
  },
  'debug_set_tray_overlay': (args: Record<string, unknown>) => {
    toast.info(`Tray overlay set to: ${args?.overlay ?? 'auto'}`)
    return null
  },
  'debug_discover_providers': () => ({
    discovered: [{ provider_type: 'ollama', instance_name: 'Ollama', base_url: 'http://localhost:11434' }],
    added: ['Ollama'],
    skipped: [],
  }),

  // ============================================================================
  // Window & System
  // ============================================================================
  'show_main_window': () => null,
  'hide_main_window': () => null,
  'open_path': (args) => {
    toast.info(`Opening: ${args?.path} (demo)`)
    return null
  },
  'copy_to_clipboard': (args) => {
    if (typeof args?.text === 'string') {
      navigator.clipboard.writeText(args.text).catch(() => {})
      toast.success('Copied to clipboard')
    }
    return null
  },
  'copy_image_to_clipboard': () => {
    toast.success('Screenshot copied to clipboard (demo)')
    return null
  },
  'copy_text_to_clipboard': (args) => {
    if (typeof args?.text === 'string') {
      navigator.clipboard.writeText(args.text).catch(() => {})
    }
    toast.success('Copied to clipboard (demo)')
    return null
  },

  // ============================================================================
  // App Info & System
  // ============================================================================
  'get_app_version': () => '0.1.0-demo',
  'get_executable_path': () => '/Applications/LocalRouter.app/Contents/MacOS/LocalRouter',
  'get_openapi_spec': () => ({
    openapi: '3.1.0',
    info: {
      title: 'LocalRouter API',
      version: '0.1.0',
      description: 'OpenAI-compatible API gateway with intelligent routing and MCP integration',
    },
    servers: [
      { url: 'http://127.0.0.1:3625', description: 'Local server' },
    ],
    paths: {
      '/v1/chat/completions': {
        post: {
          summary: 'Create chat completion',
          operationId: 'createChatCompletion',
          tags: ['Chat'],
          requestBody: {
            required: true,
            content: { 'application/json': { schema: { '$ref': '#/components/schemas/ChatCompletionRequest' } } },
          },
          responses: { '200': { description: 'Successful response' } },
        },
      },
      '/v1/models': {
        get: {
          summary: 'List models',
          operationId: 'listModels',
          tags: ['Models'],
          responses: { '200': { description: 'List of available models' } },
        },
      },
      '/v1/completions': {
        post: {
          summary: 'Create completion',
          operationId: 'createCompletion',
          tags: ['Completions'],
          responses: { '200': { description: 'Successful response' } },
        },
      },
      '/v1/embeddings': {
        post: {
          summary: 'Create embeddings',
          operationId: 'createEmbedding',
          tags: ['Embeddings'],
          responses: { '200': { description: 'Successful response' } },
        },
      },
      '/health': {
        get: {
          summary: 'Health check',
          operationId: 'healthCheck',
          tags: ['System'],
          responses: { '200': { description: 'Server is healthy' } },
        },
      },
    },
    components: {
      schemas: {
        ChatCompletionRequest: { type: 'object', properties: { model: { type: 'string' }, messages: { type: 'array' } } },
      },
      securitySchemes: {
        BearerAuth: { type: 'http', scheme: 'bearer' },
      },
    },
  }),

  // ============================================================================
  // Internal/Testing
  // ============================================================================
  'get_internal_test_token': () => 'demo-test-token',
  'create_test_client_for_strategy': (args) => ({
    id: `test-client-${generateId()}`,
    client_id: `test-${args?.strategyId || 'default'}`,
    name: `Test Client for ${args?.strategyId || 'default'}`,
    enabled: true,
    strategy_id: args?.strategyId || 'strategy-default',
  }),

  // ============================================================================
  // Tauri Plugin Commands (internal)
  // ============================================================================
  'plugin:event|unlisten': () => null,
  'plugin:event|listen': () => 0, // Return a listener ID
  'plugin:event|emit': () => null,
  'plugin:window|create': () => null,
  'plugin:window|close': () => null,
  'plugin:window|show': () => null,
  'plugin:window|hide': () => null,
  'plugin:webview|create': () => null,
  'plugin:webview|close': () => null,
}

/**
 * Initialize Tauri mocks for the demo.
 * Must be called before rendering any components that use Tauri APIs.
 */
export function setupTauriMocks() {
  clearMocks()
  mockWindows('main')

  mockIPC((cmd: string, args?: InvokeArgs) => {
    // Suppress verbose logging for frequent commands
    if (!cmd.startsWith('plugin:')) {
      console.log('[Demo Mock]', cmd, args)
    }

    // Check if this command has a mock implementation
    if (!(cmd in mockHandlers)) {
      // Silently handle all plugin:* commands - these are internal Tauri APIs
      if (cmd.startsWith('plugin:')) {
        // Return appropriate defaults for plugin commands
        if (cmd.includes('|listen')) return 0 // Listener ID
        if (cmd.includes('|unlisten')) return null
        if (cmd.includes('|emit')) return null
        return null
      }

      if (!warnedCommands.has(cmd)) {
        warnedCommands.add(cmd)
        toast.info(`Demo: "${cmd}" not implemented`, {
          description: 'This feature is not available in demo mode',
          duration: 4000,
        })
        console.warn(`[Demo Mock] Unimplemented command: ${cmd}`, args)
      }
      // Return empty array for commands that typically return arrays
      if (cmd.startsWith('list_') || cmd.startsWith('search_') || cmd.endsWith('_logs')) {
        return []
      }
      return null
    }

    return mockHandlers[cmd](args as Record<string, unknown>)
  }, { shouldMockEvents: true })
}

/**
 * Set up minimal window/Tauri internals needed for components to function.
 */
export function stubTauriInternals() {
  if (typeof window !== 'undefined') {
    // Stub __TAURI_INTERNALS__ if not present
    if (!(window as unknown as Record<string, unknown>).__TAURI_INTERNALS__) {
      (window as unknown as Record<string, unknown>).__TAURI_INTERNALS__ = {
        metadata: {
          currentWebview: { label: 'demo-main' },
          currentWindow: { label: 'demo-main' },
        },
        invoke: () => Promise.resolve(null),
        convertFileSrc: (path: string) => path,
      }
    }
  }
}

// Export for validation/testing
export { mockHandlers }
