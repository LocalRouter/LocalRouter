import { useState, useEffect } from 'react'
import { listen } from '@tauri-apps/api/event'
import { invoke } from '@tauri-apps/api/core'
import { AppShell, type View } from './components/layout'
import { DashboardView } from './views/dashboard'
import { ClientsView } from './views/clients'
import { ResourcesView } from './views/resources'
import { McpServersView } from './views/mcp-servers'
import { TryItOutView } from './views/try-it-out'
import { SettingsView } from './views/settings'
import { ClientCreationWizard } from './components/wizard/ClientCreationWizard'

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
  const [showSetupWizard, setShowSetupWizard] = useState(false)
  const [appReady, setAppReady] = useState(false)

  const handleViewChange = (view: View, subTab: string | null = null) => {
    setActiveView(view)
    setActiveSubTab(subTab)
  }

  // Check if setup wizard should be shown (first-run detection)
  useEffect(() => {
    const checkSetupWizard = async () => {
      try {
        const shown = await invoke<boolean>('get_setup_wizard_shown')
        if (!shown) {
          setShowSetupWizard(true)
        }
      } catch (error) {
        console.error('Failed to check setup wizard status:', error)
      } finally {
        setAppReady(true)
      }
    }
    checkSetupWizard()
  }, [])

  const handleWizardComplete = async (clientId: string) => {
    try {
      await invoke('set_setup_wizard_shown')
    } catch (error) {
      console.error('Failed to mark setup wizard as shown:', error)
    }
    setShowSetupWizard(false)
    // Navigate to the newly created client
    handleViewChange('clients', `${clientId}/config`)
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
        return <DashboardView onViewChange={handleChildViewChange} />
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
      case 'mcp-servers':
        return (
          <McpServersView
            activeSubTab={activeSubTab}
            onTabChange={handleChildViewChange}
          />
        )
      case 'try-it-out':
        return (
          <TryItOutView
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
        return <DashboardView onViewChange={handleChildViewChange} />
    }
  }

  return (
    <>
      <AppShell
        activeView={activeView}
        activeSubTab={activeSubTab}
        onViewChange={handleViewChange}
      >
        {appReady && renderView()}
      </AppShell>

      <ClientCreationWizard
        open={showSetupWizard}
        onOpenChange={(open) => {
          if (!open) {
            // User dismissed wizard without completing - mark as shown anyway
            invoke('set_setup_wizard_shown').catch(console.error)
          }
          setShowSetupWizard(open)
        }}
        onComplete={handleWizardComplete}
      />
    </>
  )
}

export default App
