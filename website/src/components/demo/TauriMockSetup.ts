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
import type { RouteLLMTestResult, GraphData } from '@app/types/tauri-commands'

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
  'get_client': (args) => mockData.clients.find(c => c.id === args?.id || c.client_id === args?.clientId),
  'create_client': (args) => {
    const newClient = {
      id: `client-${generateId()}`,
      client_id: args?.name?.toLowerCase().replace(/\s+/g, '-') || generateId(),
      name: args?.name || 'New Client',
      enabled: true,
      strategy_id: 'strategy-default',
      mcp_deferred_loading: false,
      created_at: new Date().toISOString(),
      last_used: null,
      mcp_permissions: { global: 'ask' as const, servers: {}, tools: {}, resources: {}, prompts: {} },
      skills_permissions: { global: 'ask' as const, skills: {}, tools: {} },
      model_permissions: { global: 'allow' as const, providers: {}, models: {} },
      marketplace_permission: 'ask' as const,
      client_mode: 'both' as const,
      template_id: null,
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
  'toggle_client_deferred_loading': (args) => {
    const client = mockData.clients.find(c => c.client_id === args?.clientId || c.id === args?.id)
    if (client) {
      client.mcp_deferred_loading = args?.enabled !== undefined ? args.enabled : !client.mcp_deferred_loading
      setTimeout(() => emit('clients-changed', {}), 10)
    }
    return null
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
  'rotate_client_secret': () => {
    setTimeout(() => emit('clients-changed', {}), 10)
    return { secret: `demo-secret-${generateId()}` }
  },
  'assign_client_strategy': (args) => {
    const client = mockData.clients.find(c => c.client_id === args?.clientId || c.id === args?.clientId)
    if (client) {
      client.strategy_id = args?.strategyId
      setTimeout(() => emit('clients-changed', {}), 10)
    }
    return null
  },
  'get_client_value': () => null,

  // Client mode and template
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
      // Emit clients-changed event to trigger UI refresh
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
    const idx = mockData.mcpServers.findIndex(s => s.id === args?.id)
    if (idx !== -1) mockData.mcpServers.splice(idx, 1)
    toast.success('MCP Server deleted (demo)')
    return null
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
    const toolsByServer: Record<string, { name: string; description: string }[]> = {
      'mcp-github': [
        { name: 'create_issue', description: 'Create a new GitHub issue' },
        { name: 'create_pull_request', description: 'Create a new pull request' },
        { name: 'list_repos', description: 'List repositories for a user or organization' },
        { name: 'search_code', description: 'Search for code across repositories' },
        { name: 'get_file_contents', description: 'Get contents of a file from a repository' },
        { name: 'create_branch', description: 'Create a new branch' },
        { name: 'merge_pull_request', description: 'Merge a pull request' },
        { name: 'list_workflows', description: 'List GitHub Actions workflows' },
        { name: 'trigger_workflow', description: 'Trigger a GitHub Actions workflow' },
        { name: 'get_commit_history', description: 'Get commit history for a repository' },
        { name: 'create_release', description: 'Create a new release' },
        { name: 'add_comment', description: 'Add a comment to an issue or PR' },
      ],
      'mcp-filesystem': [
        { name: 'read_file', description: 'Read contents of a file' },
        { name: 'write_file', description: 'Write contents to a file' },
        { name: 'list_directory', description: 'List files in a directory' },
        { name: 'create_directory', description: 'Create a new directory' },
        { name: 'delete_file', description: 'Delete a file' },
        { name: 'move_file', description: 'Move or rename a file' },
        { name: 'copy_file', description: 'Copy a file to another location' },
        { name: 'search_files', description: 'Search for files by pattern' },
      ],
      'mcp-slack': [
        { name: 'send_message', description: 'Send a message to a channel' },
        { name: 'list_channels', description: 'List available channels' },
        { name: 'get_channel_history', description: 'Get message history from a channel' },
        { name: 'create_channel', description: 'Create a new channel' },
        { name: 'invite_user', description: 'Invite a user to a channel' },
        { name: 'upload_file', description: 'Upload a file to a channel' },
        { name: 'add_reaction', description: 'Add a reaction to a message' },
        { name: 'search_messages', description: 'Search for messages' },
        { name: 'get_user_info', description: 'Get information about a user' },
        { name: 'set_status', description: 'Set user status' },
        { name: 'schedule_message', description: 'Schedule a message for later' },
        { name: 'pin_message', description: 'Pin a message to a channel' },
        { name: 'create_reminder', description: 'Create a reminder' },
        { name: 'list_emojis', description: 'List available custom emojis' },
        { name: 'get_team_info', description: 'Get workspace information' },
      ],
      'mcp-postgres': [
        { name: 'execute_query', description: 'Execute a SQL query' },
        { name: 'list_tables', description: 'List all tables in the database' },
        { name: 'describe_table', description: 'Get table schema information' },
        { name: 'list_databases', description: 'List available databases' },
        { name: 'create_table', description: 'Create a new table' },
        { name: 'insert_row', description: 'Insert a row into a table' },
      ],
      'mcp-browser': [
        { name: 'navigate', description: 'Navigate to a URL' },
        { name: 'screenshot', description: 'Take a screenshot of the current page' },
        { name: 'click', description: 'Click an element on the page' },
        { name: 'type', description: 'Type text into an input field' },
        { name: 'scroll', description: 'Scroll the page' },
        { name: 'get_text', description: 'Get text content from an element' },
        { name: 'wait_for_element', description: 'Wait for an element to appear' },
        { name: 'evaluate', description: 'Execute JavaScript in the page context' },
        { name: 'get_html', description: 'Get HTML content from the page' },
        { name: 'fill_form', description: 'Fill out a form with provided data' },
      ],
    }
    const tools = server?.id ? (toolsByServer[server.id] || [
      { name: 'execute', description: 'Execute the primary action' },
      { name: 'query', description: 'Query for information' },
      { name: 'list', description: 'List available items' },
      { name: 'get', description: 'Get a specific item' },
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
  'list_all_models_detailed': () => mockData.models.map(m => ({
    ...m,
    capabilities: ['chat', 'completion'],
    pricing: { input: 0.001, output: 0.002 },
  })),
  'list_provider_models': (args) => {
    const provider = args?.instanceName || args?.provider
    return mockData.models.filter(m => m.provider === provider)
  },

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
  'add_skills_path': () => {
    toast.success('Skills path added (demo)')
    return null
  },
  'remove_skills_path': () => {
    toast.success('Skills path removed (demo)')
    return null
  },

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
    const clients = ['cursor-ide', 'claude-code', 'open-webui']
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
    mockData.marketplaceConfig.enabled = args?.enabled ?? true
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
    if (args?.settings) {
      Object.assign(mockData.trayGraphSettings, args.settings)
    }
    return null
  },

  // ============================================================================
  // Firewall
  // ============================================================================
  'get_firewall_approval_details': () => null,
  'get_firewall_full_arguments': () => JSON.stringify({ path: '/tmp/test.txt', content: 'hello world' }),
  'submit_firewall_approval': () => {
    toast.success('Approval submitted (demo)')
    return null
  },
  'debug_trigger_firewall_popup': () => {
    toast.info('Firewall popup triggered (demo)')
    return null
  },

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
