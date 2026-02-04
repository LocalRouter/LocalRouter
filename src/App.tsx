import { useState, useEffect, lazy, Suspense } from 'react'
import { listen } from '@tauri-apps/api/event'
import { invoke } from '@tauri-apps/api/core'
import { check } from '@tauri-apps/plugin-updater'
import { relaunch } from '@tauri-apps/plugin-process'
import { toast } from 'sonner'
import { AppShell, type View } from './components/layout'
import { DashboardView } from './views/dashboard'
import { ClientsView } from './views/clients'
import { ResourcesView } from './views/resources'
import { McpServersView } from './views/mcp-servers'
import { SkillsView } from './views/skills'
import { MarketplaceView } from './views/marketplace'
import { TryItOutView } from './views/try-it-out'
import { SettingsView } from './views/settings'
import { ClientCreationWizard } from './components/wizard/ClientCreationWizard'

const DebugView = import.meta.env.DEV
  ? lazy(() => import('./views/debug').then(m => ({ default: m.DebugView })))
  : () => null

import type { McpPermissions, SkillsPermissions, ModelPermissions, PermissionState } from '@/components/permissions'

interface Client {
  id: string
  name: string
  client_id: string
  enabled: boolean
  strategy_id: string
  mcp_deferred_loading: boolean
  created_at: string
  last_used: string | null
  mcp_permissions: McpPermissions
  skills_permissions: SkillsPermissions
  model_permissions: ModelPermissions
  marketplace_permission: PermissionState
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

    // Subscribe to open-resources-tab event from tray menu (for provider health issues)
    const unsubscribeResourcesTab = listen('open-resources-tab', () => {
      console.log('Opening Resources tab from tray menu')
      setActiveView('resources')
      setActiveSubTab('providers')
    })

    // Subscribe to open-mcp-server event from tray menu (for MCP health issues)
    const unsubscribeMcpServer = listen<string>('open-mcp-server', (event) => {
      const serverId = event.payload
      console.log('Opening MCP server from tray menu:', serverId)
      setActiveView('mcp-servers')
      setActiveSubTab(serverId)
    })

    // Subscribe to update-and-restart event from tray menu
    const unsubscribeUpdateAndRestart = listen('update-and-restart', async () => {
      console.log('Update and restart requested from tray menu')
      try {
        const update = await check()
        if (update?.available) {
          toast.info(`Installing update ${update.version}...`)
          await update.downloadAndInstall()
          await invoke('set_update_notification', { available: false })
          await relaunch()
        } else {
          toast.info('No update available')
        }
      } catch (err: any) {
        const errorMessage = err?.message || (typeof err === 'string' ? err : JSON.stringify(err)) || 'Unknown error'
        console.error('Update failed:', errorMessage)
        toast.error(`Update failed: ${errorMessage}`)
      }
    })

    return () => {
      unsubscribeConfig.then((fn: any) => fn())
      unsubscribeClients.then((fn: any) => fn())
      unsubscribePrioritized.then((fn: any) => fn())
      unsubscribeUpdatesTab.then((fn: any) => fn())
      unsubscribeResourcesTab.then((fn: any) => fn())
      unsubscribeMcpServer.then((fn: any) => fn())
      unsubscribeUpdateAndRestart.then((fn: any) => fn())
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
      case 'skills':
        return (
          <SkillsView
            activeSubTab={activeSubTab}
            onTabChange={handleChildViewChange}
          />
        )
      case 'marketplace':
        return (
          <MarketplaceView
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
      case 'debug':
        return import.meta.env.DEV ? (
          <Suspense fallback={null}>
            <DebugView
              activeSubTab={activeSubTab}
              onTabChange={handleChildViewChange}
            />
          </Suspense>
        ) : null
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
        showWelcome={true}
      />
    </>
  )
}

export default App
