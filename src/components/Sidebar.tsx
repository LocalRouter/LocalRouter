import { useState, useEffect } from 'react'
import { invoke } from '@tauri-apps/api/core'
import { listen } from '@tauri-apps/api/event'
import ProviderIcon from './ProviderIcon'

type MainTab = 'home' | 'server' | 'clients' | 'api-keys' | 'providers' | 'models' | 'oauth-clients' | 'mcp-servers' | 'logs' | 'documentation'

interface SidebarProps {
  activeTab: MainTab
  activeSubTab: string | null
  onTabChange: (tab: MainTab, subTab?: string) => void
}

interface ProviderInstance {
  instance_name: string
  provider_type: string
  enabled: boolean
}

interface Client {
  id: string
  name: string
  client_id: string
  enabled: boolean
  allowed_llm_providers: string[]
  allowed_mcp_servers: string[]
}

interface Model {
  id: string
  provider: string
}

interface OAuthClient {
  id: string
  name: string
  enabled: boolean
}

interface McpServer {
  id: string
  name: string
  enabled: boolean
}

export default function Sidebar({ activeTab, activeSubTab, onTabChange }: SidebarProps) {
  const [providers, setProviders] = useState<ProviderInstance[]>([])
  const [clients, setClients] = useState<Client[]>([])
  const [models, setModels] = useState<Model[]>([])
  const [oauthClients, setOauthClients] = useState<OAuthClient[]>([])
  const [mcpServers, setMcpServers] = useState<McpServer[]>([])
  const [expandedSections, setExpandedSections] = useState<Set<string>>(new Set())

  useEffect(() => {
    // Initial load
    loadProviders()
    loadClients()
    loadModels()
    loadOAuthClients()
    loadMcpServers()

    // Subscribe to data change events (no polling needed)
    const unsubscribeProviders = listen('providers-changed', () => {
      loadProviders()
    })

    const unsubscribeClients = listen('clients-changed', () => {
      loadClients()
    })

    const unsubscribeModels = listen('models-changed', () => {
      loadModels()
    })

    const unsubscribeOAuthClients = listen('oauth-clients-changed', () => {
      loadOAuthClients()
    })

    const unsubscribeMcpServers = listen('mcp-servers-changed', () => {
      loadMcpServers()
    })

    return () => {
      unsubscribeProviders.then((fn: any) => fn())
      unsubscribeClients.then((fn: any) => fn())
      unsubscribeModels.then((fn: any) => fn())
      unsubscribeOAuthClients.then((fn: any) => fn())
      unsubscribeMcpServers.then((fn: any) => fn())
    }
  }, [])

  const loadProviders = async () => {
    try {
      const instances = await invoke<ProviderInstance[]>('list_provider_instances')
      setProviders(instances)
    } catch (err) {
      console.error('Failed to load providers:', err)
    }
  }

  const loadClients = async () => {
    try {
      const clientList = await invoke<Client[]>('list_clients')
      setClients(clientList)
    } catch (err) {
      console.error('Failed to load clients:', err)
    }
  }


  const loadModels = async () => {
    try {
      const modelList = await invoke<Model[]>('list_all_models')
      setModels(modelList)
    } catch (err) {
      console.error('Failed to load models:', err)
    }
  }

  const loadOAuthClients = async () => {
    try {
      const clients = await invoke<OAuthClient[]>('list_oauth_clients')
      setOauthClients(clients)
    } catch (err) {
      console.error('Failed to load OAuth clients:', err)
    }
  }

  const loadMcpServers = async () => {
    try {
      const servers = await invoke<McpServer[]>('list_mcp_servers')
      setMcpServers(servers)
    } catch (err) {
      console.error('Failed to load MCP servers:', err)
    }
  }

  const toggleSection = (section: string) => {
    const newExpanded = new Set(expandedSections)
    if (newExpanded.has(section)) {
      newExpanded.delete(section)
    } else {
      newExpanded.add(section)
    }
    setExpandedSections(newExpanded)
  }

  const mainTabs = [
    { id: 'home' as MainTab, label: 'Home' },
    { id: 'server' as MainTab, label: 'Preferences' },
    { id: 'clients' as MainTab, label: 'Clients', hasSubTabs: true },
    { id: 'providers' as MainTab, label: 'Providers', hasSubTabs: true },
    { id: 'models' as MainTab, label: 'Models', hasSubTabs: true },
    { id: 'mcp-servers' as MainTab, label: 'MCP Servers', hasSubTabs: true },
    { id: 'logs' as MainTab, label: 'Logs' },
    { id: 'documentation' as MainTab, label: 'Documentation' },
  ]

  return (
    <nav className="w-[240px] bg-white dark:bg-gray-800 border-r border-gray-200 dark:border-gray-700 shadow-sm py-4 overflow-y-auto">
      {mainTabs.map((tab) => (
        <div key={tab.id}>
          {/* Main Tab */}
          <div
            onClick={() => {
              if (tab.hasSubTabs) {
                toggleSection(tab.id)
              }
              onTabChange(tab.id)
            }}
            className={`
              px-6 py-3 cursor-pointer transition-all font-medium border-l-4 flex items-center justify-between
              ${
                activeTab === tab.id && !activeSubTab
                  ? 'bg-blue-50 dark:bg-blue-900/30 text-blue-600 dark:text-blue-400 border-blue-600 dark:border-blue-400'
                  : 'text-gray-600 dark:text-gray-300 border-transparent hover:bg-gray-50 dark:hover:bg-gray-700 hover:text-gray-900 dark:hover:text-gray-100'
              }
            `}
          >
            <span>{tab.label}</span>
            {tab.hasSubTabs && (
              <span className="text-xs">
                {expandedSections.has(tab.id) ? '▼' : '▶'}
              </span>
            )}
          </div>

          {/* Sub Tabs for Providers */}
          {tab.id === 'providers' && expandedSections.has('providers') && (
            <div className="bg-gray-50 dark:bg-gray-900/50">
              {providers.map((provider) => (
                <div
                  key={provider.instance_name}
                  onClick={() => onTabChange('providers', provider.instance_name)}
                  className={`
                    px-4 py-2 cursor-pointer transition-all text-sm border-l-4 flex items-center gap-2
                    ${
                      activeTab === 'providers' && activeSubTab === provider.instance_name
                        ? 'bg-blue-50 dark:bg-blue-900/30 text-blue-600 dark:text-blue-400 border-blue-600 dark:border-blue-400'
                        : 'text-gray-600 dark:text-gray-300 border-transparent hover:bg-gray-100 dark:hover:bg-gray-800'
                    }
                  `}
                >
                  <ProviderIcon providerId={provider.provider_type} size={20} />
                  <span className="truncate flex-1">{provider.instance_name}</span>
                  {!provider.enabled && (
                    <span className="w-2 h-2 bg-red-500 rounded-full" title="Disabled" />
                  )}
                </div>
              ))}
            </div>
          )}

          {/* Sub Tabs for Clients */}
          {tab.id === 'clients' && expandedSections.has('clients') && (
            <div className="bg-gray-50 dark:bg-gray-900/50">
              {clients.map((client) => (
                <div
                  key={client.id}
                  onClick={() => onTabChange('clients', client.client_id)}
                  className={`
                    px-4 py-2 cursor-pointer transition-all text-sm border-l-4 flex items-center gap-2
                    ${
                      activeTab === 'clients' && activeSubTab === client.client_id
                        ? 'bg-blue-50 dark:bg-blue-900/30 text-blue-600 dark:text-blue-400 border-blue-600 dark:border-blue-400'
                        : 'text-gray-600 dark:text-gray-300 border-transparent hover:bg-gray-100 dark:hover:bg-gray-800'
                    }
                  `}
                >
                  <span className="truncate flex-1">{client.name}</span>
                  {!client.enabled && (
                    <span className="w-2 h-2 bg-red-500 rounded-full" title="Disabled" />
                  )}
                </div>
              ))}
            </div>
          )}

          {/* Sub Tabs for Models */}
          {tab.id === 'models' && expandedSections.has('models') && (
            <div className="bg-gray-50 dark:bg-gray-900/50">
              {models.map((model) => (
                <div
                  key={`${model.provider}-${model.id}`}
                  onClick={() => onTabChange('models', `${model.provider}/${model.id}`)}
                  className={`
                    px-4 py-2 cursor-pointer transition-all text-sm border-l-4 flex items-center gap-2
                    ${
                      activeTab === 'models' && activeSubTab === `${model.provider}/${model.id}`
                        ? 'bg-blue-50 dark:bg-blue-900/30 text-blue-600 dark:text-blue-400 border-blue-600 dark:border-blue-400'
                        : 'text-gray-600 dark:text-gray-300 border-transparent hover:bg-gray-100 dark:hover:bg-gray-800'
                    }
                  `}
                >
                  <span className="truncate flex-1 text-xs">{model.provider}/{model.id}</span>
                </div>
              ))}
            </div>
          )}

          {/* Sub Tabs for OAuth Clients */}
          {tab.id === 'oauth-clients' && expandedSections.has('oauth-clients') && (
            <div className="bg-gray-50 dark:bg-gray-900/50">
              {oauthClients.map((client) => (
                <div
                  key={client.id}
                  onClick={() => onTabChange('oauth-clients', client.id)}
                  className={`
                    px-4 py-2 cursor-pointer transition-all text-sm border-l-4 flex items-center gap-2
                    ${
                      activeTab === 'oauth-clients' && activeSubTab === client.id
                        ? 'bg-blue-50 dark:bg-blue-900/30 text-blue-600 dark:text-blue-400 border-blue-600 dark:border-blue-400'
                        : 'text-gray-600 dark:text-gray-300 border-transparent hover:bg-gray-100 dark:hover:bg-gray-800'
                    }
                  `}
                >
                  <span className="truncate flex-1">{client.name}</span>
                  {!client.enabled && (
                    <span className="w-2 h-2 bg-red-500 rounded-full" title="Disabled" />
                  )}
                </div>
              ))}
            </div>
          )}

          {/* Sub Tabs for MCP Servers */}
          {tab.id === 'mcp-servers' && expandedSections.has('mcp-servers') && (
            <div className="bg-gray-50 dark:bg-gray-900/50">
              {mcpServers.map((server) => (
                <div
                  key={server.id}
                  onClick={() => onTabChange('mcp-servers', server.id)}
                  className={`
                    px-4 py-2 cursor-pointer transition-all text-sm border-l-4 flex items-center gap-2
                    ${
                      activeTab === 'mcp-servers' && activeSubTab === server.id
                        ? 'bg-blue-50 dark:bg-blue-900/30 text-blue-600 dark:text-blue-400 border-blue-600 dark:border-blue-400'
                        : 'text-gray-600 dark:text-gray-300 border-transparent hover:bg-gray-100 dark:hover:bg-gray-800'
                    }
                  `}
                >
                  <span className="truncate flex-1">{server.name}</span>
                  {!server.enabled && (
                    <span className="w-2 h-2 bg-red-500 rounded-full" title="Disabled" />
                  )}
                </div>
              ))}
            </div>
          )}
        </div>
      ))}
    </nav>
  )
}
