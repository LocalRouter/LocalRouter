import { useState, useEffect } from 'react'
import { listen } from '@tauri-apps/api/event'
import Sidebar from './components/Sidebar'
import HomeTab from './components/tabs/HomeTab'
import ClientsTab from './components/tabs/ClientsTab'
import ApiKeysTab from './components/tabs/ApiKeysTab'
import ProvidersTab from './components/tabs/ProvidersTab'
import ModelsTab from './components/tabs/ModelsTab'
import OAuthClientsTab from './components/tabs/OAuthClientsTab'
import McpServersTab from './components/tabs/McpServersTab'
import RoutingTab from './components/tabs/RoutingTab'
import DocumentationTab from './components/tabs/DocumentationTab'
import LogsTab from './components/tabs/LogsTab'
import SettingsPage from './components/SettingsPage'
import { invoke } from '@tauri-apps/api/core'

type Tab = 'home' | 'clients' | 'api-keys' | 'providers' | 'models' | 'oauth-clients' | 'mcp-servers' | 'routing' | 'logs' | 'documentation' | 'settings'

interface ApiKeyInfo {
  id: string
  name: string
  enabled: boolean
  created_at: string
}

interface Client {
  id: string
  name: string
  client_id: string
  enabled: boolean
  allowed_llm_providers: string[]
  allowed_mcp_servers: string[]
  created_at: string
  last_used: string | null
}

function App() {
  const [activeTab, setActiveTab] = useState<Tab>('home')
  const [activeSubTab, setActiveSubTab] = useState<string | null>(null)

  const handleTabChange = (tab: string, subTab: string | null = null) => {
    setActiveTab(tab as Tab)
    setActiveSubTab(subTab)
  }

  useEffect(() => {
    // Subscribe to configuration changes
    const unsubscribeConfig = listen('config-changed', (event: any) => {
      console.log('Configuration changed:', event.payload)
    })

    // Subscribe to open-prioritized-list event from tray
    const unsubscribePrioritized = listen<string>('open-prioritized-list', async (event) => {
      const apiKeyId = event.payload
      console.log('Opening prioritized list for API key:', apiKeyId)

      try {
        // Try to find the client by ID (new unified client system)
        const clients = await invoke<Client[]>('list_clients')
        const client = clients.find((c) => c.id === apiKeyId || c.client_id === apiKeyId)

        if (client) {
          // Navigate to clients tab with this client ID, and append tab info
          setActiveTab('clients')
          setActiveSubTab(`${client.client_id}|models|prioritized`)
          return
        }

        // Fallback: try API keys (legacy system)
        const keys = await invoke<ApiKeyInfo[]>('list_api_keys')
        const key = keys.find((k) => k.id === apiKeyId)

        if (key) {
          // Navigate to API keys tab with this key ID
          setActiveTab('api-keys')
          setActiveSubTab(key.id)
        }
      } catch (err) {
        console.error('Failed to load client/API key:', err)
      }
    })

    // Subscribe to open-updates-tab event from tray menu
    const unsubscribeUpdatesTab = listen('open-updates-tab', () => {
      console.log('Opening Updates tab from tray menu')
      setActiveTab('settings')
      setActiveSubTab('updates')
    })

    return () => {
      unsubscribeConfig.then((fn: any) => fn())
      unsubscribePrioritized.then((fn: any) => fn())
      unsubscribeUpdatesTab.then((fn: any) => fn())
    }
  }, [])

  return (
    <div className="flex h-screen bg-gray-50 dark:bg-gray-900">
      <Sidebar
        activeTab={activeTab}
        activeSubTab={activeSubTab}
        onTabChange={handleTabChange}
      />

      <main className="flex-1 p-8 overflow-y-auto">
          {activeTab === 'home' && <HomeTab />}
          {activeTab === 'clients' && (
            <ClientsTab activeSubTab={activeSubTab} onTabChange={handleTabChange} />
          )}
          {activeTab === 'api-keys' && (
            <ApiKeysTab activeSubTab={activeSubTab} onTabChange={handleTabChange} />
          )}
          {activeTab === 'providers' && (
            <ProvidersTab activeSubTab={activeSubTab} onTabChange={handleTabChange} />
          )}
          {activeTab === 'models' && (
            <ModelsTab activeSubTab={activeSubTab} onTabChange={handleTabChange} />
          )}
          {activeTab === 'oauth-clients' && (
            <OAuthClientsTab activeSubTab={activeSubTab} onTabChange={handleTabChange} />
          )}
          {activeTab === 'mcp-servers' && (
            <McpServersTab activeSubTab={activeSubTab} onTabChange={handleTabChange} />
          )}
          {activeTab === 'routing' && (
            <RoutingTab activeSubTab={activeSubTab} onTabChange={handleTabChange} />
          )}
          {activeTab === 'logs' && <LogsTab />}
          {activeTab === 'documentation' && <DocumentationTab />}
          {activeTab === 'settings' && (
            <SettingsPage initialSubtab={(activeSubTab || undefined) as any} />
          )}
        </main>
    </div>
  )
}

export default App
