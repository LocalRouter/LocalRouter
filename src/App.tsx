import { useState, useEffect } from 'react'
import { listen } from '@tauri-apps/api/event'
import { invoke } from '@tauri-apps/api/core'
import { AppShell, type View } from './components/layout'
import { DashboardView } from './views/dashboard'
import { ClientsView } from './views/clients'
import { ResourcesView } from './views/resources'
import { LogsView } from './views/logs'
import { SettingsView } from './views/settings'

type McpAccessMode = 'none' | 'all' | 'specific'

interface Client {
  id: string
  name: string
  client_id: string
  enabled: boolean
  allowed_llm_providers: string[]
  mcp_access_mode: McpAccessMode
  mcp_servers: string[]
  created_at: string
  last_used: string | null
}

function App() {
  const [activeView, setActiveView] = useState<View>('dashboard')
  const [activeSubTab, setActiveSubTab] = useState<string | null>(null)

  const handleViewChange = (view: View, subTab: string | null = null) => {
    setActiveView(view)
    setActiveSubTab(subTab)
  }

  useEffect(() => {
    // Subscribe to configuration changes
    const unsubscribeConfig = listen('config-changed', (event: any) => {
      console.log('Configuration changed:', event.payload)
    })

    // Subscribe to clients-changed events (for debugging)
    const unsubscribeClients = listen('clients-changed', () => {
      console.log('Clients changed event received')
    })

    // Subscribe to open-prioritized-list event from tray
    const unsubscribePrioritized = listen<string>('open-prioritized-list', async (event) => {
      const clientId = event.payload
      console.log('Opening prioritized list for client:', clientId)

      try {
        // Find the client by ID
        const clients = await invoke<Client[]>('list_clients')
        const client = clients.find((c) => c.id === clientId || c.client_id === clientId)

        if (client) {
          // Navigate to clients view with this client
          setActiveView('clients')
          setActiveSubTab(`${client.id}/models`)
        } else {
          console.warn('Client not found:', clientId)
        }
      } catch (err) {
        console.error('Failed to load client:', err)
      }
    })

    // Subscribe to open-updates-tab event from tray menu
    const unsubscribeUpdatesTab = listen('open-updates-tab', () => {
      console.log('Opening Updates tab from tray menu')
      setActiveView('settings')
      setActiveSubTab('updates')
    })

    return () => {
      unsubscribeConfig.then((fn: any) => fn())
      unsubscribeClients.then((fn: any) => fn())
      unsubscribePrioritized.then((fn: any) => fn())
      unsubscribeUpdatesTab.then((fn: any) => fn())
    }
  }, [])

  // Wrapper for child views that expect string view type
  const handleChildViewChange = (view: string, subTab?: string | null) => {
    handleViewChange(view as View, subTab)
  }

  const renderView = () => {
    switch (activeView) {
      case 'dashboard':
        return <DashboardView />
      case 'clients':
        return (
          <ClientsView
            activeSubTab={activeSubTab}
            onTabChange={handleChildViewChange}
          />
        )
      case 'resources':
        return (
          <ResourcesView
            activeSubTab={activeSubTab}
            onTabChange={handleChildViewChange}
          />
        )
      case 'logs':
        return (
          <LogsView
            activeSubTab={activeSubTab}
            onTabChange={handleChildViewChange}
          />
        )
      case 'settings':
        return (
          <SettingsView
            activeSubTab={activeSubTab}
            onTabChange={handleChildViewChange}
          />
        )
      default:
        return <DashboardView />
    }
  }

  return (
    <AppShell
      activeView={activeView}
      activeSubTab={activeSubTab}
      onViewChange={handleViewChange}
    >
      {renderView()}
    </AppShell>
  )
}

export default App
