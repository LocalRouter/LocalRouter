import { useState, useEffect, lazy, Suspense } from 'react'
import { listenSafe } from '@/hooks/useTauriListener'
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
import { SettingsView } from './views/settings'
import { GuardrailsView } from './views/guardrails'
import { StrongWeakView } from './views/strong-weak'
import { CodingAgentsView } from './views/coding-agents'
import { CatalogCompressionView } from './views/catalog-compression'
import { ResponseRagView } from './views/response-rag'
import { MemoryView } from './views/memory'
import { CompressionView } from './views/compression'
import { JsonRepairView } from './views/json-repair'
import { SecretScanningView } from './views/secret-scanning'
import { MarketplaceView } from './views/marketplace'
import { OptimizeOverviewView } from './views/optimize-overview'
import { MonitorView } from './views/monitor'
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
    handleViewChange('clients', `${clientId}|info`)
  }

  useEffect(() => {
    const listeners = [
      // Subscribe to configuration changes
      listenSafe('config-changed', (event: any) => {
        console.log('Configuration changed:', event.payload)
      }),

      // Subscribe to clients-changed events (for debugging)
      listenSafe('clients-changed', () => {
        console.log('Clients changed event received')
      }),

      // Subscribe to open-prioritized-list event from tray
      listenSafe<string>('open-prioritized-list', async (event) => {
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
      }),

      // Subscribe to open-updates-tab event from tray menu
      listenSafe('open-updates-tab', () => {
        console.log('Opening Updates tab from tray menu')
        setActiveView('settings')
        setActiveSubTab('updates')
      }),

      // Subscribe to open-resources-tab event from tray menu (for provider health issues)
      listenSafe('open-resources-tab', () => {
        console.log('Opening Resources tab from tray menu')
        setActiveView('resources')
        setActiveSubTab('providers')
      }),

      // Subscribe to open-mcp-server event from tray menu (for MCP health issues)
      listenSafe<string>('open-mcp-server', (event) => {
        const serverId = event.payload
        console.log('Opening MCP server from tray menu:', serverId)
        setActiveView('mcp-servers')
        setActiveSubTab(serverId)
      }),

      // Subscribe to open-client-tab event from tray menu (for "More…" overflow)
      listenSafe<string>('open-client-tab', (event) => {
        const payload = event.payload
        console.log('Opening client tab from tray menu:', payload)
        setActiveView('clients')
        setActiveSubTab(payload)
      }),

      // Subscribe to open-mcp-servers-page event from tray menu (for "More…" overflow)
      listenSafe('open-mcp-servers-page', () => {
        console.log('Opening MCP servers page from tray menu')
        setActiveView('mcp-servers')
        setActiveSubTab(null)
      }),

      // Subscribe to open-skills-page event from tray menu (for "More…" overflow)
      listenSafe('open-skills-page', () => {
        console.log('Opening Skills page from tray menu')
        setActiveView('skills')
        setActiveSubTab(null)
      }),

      // Subscribe to guardrail streaming response notification
      listenSafe<string>('guardrail-response-flagged', (event) => {
        try {
          const data = typeof event.payload === 'string' ? JSON.parse(event.payload) : event.payload
          const matchCount = data?.matches?.length || 0
          toast.error(`GuardRail: ${matchCount} rule${matchCount !== 1 ? 's' : ''} triggered — stream aborted`, {
            duration: 8000,
          })
        } catch {
          toast.error('GuardRail: Response flagged — stream aborted', { duration: 8000 })
        }
      }),

      // Subscribe to check-for-updates event from background timer
      // This must be in App.tsx (not UpdatesTab) so periodic checks work
      // even when the Updates tab isn't open
      listenSafe('check-for-updates', async () => {
        console.log('Background update check triggered')
        try {
          const update = await check()
          await invoke('mark_update_check_performed')
          if (update?.available) {
            // Check if this version was skipped
            const config = await invoke<{ skipped_version?: string }>('get_update_config')
            if (config.skipped_version === update.version) {
              await invoke('set_update_notification', { available: false })
            } else {
              await invoke('set_update_notification', { available: true })
              toast.info(`New version ${update.version} available!`, {
                action: {
                  label: 'View',
                  onClick: () => {
                    setActiveView('settings')
                    setActiveSubTab('updates')
                  },
                },
              })
            }
          } else {
            await invoke('set_update_notification', { available: false })
          }
        } catch (err: any) {
          const errorMessage = err?.message || (typeof err === 'string' ? err : JSON.stringify(err)) || 'Unknown error'
          console.error('Background update check failed:', errorMessage)
        }
      }),

      // Subscribe to update-and-restart event from tray menu
      listenSafe('update-and-restart', async () => {
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
      }),
    ]

    return () => {
      listeners.forEach(l => l.cleanup())
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
      case 'monitor':
        return <MonitorView />
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
      case 'catalog-compression':
        return (
          <CatalogCompressionView
            activeSubTab={activeSubTab}
            onTabChange={handleChildViewChange}
          />
        )
      case 'response-rag':
        return (
          <ResponseRagView
            activeSubTab={activeSubTab}
            onTabChange={handleChildViewChange}
          />
        )
      case 'memory':
        return (
          <MemoryView
            activeSubTab={activeSubTab}
            onTabChange={handleChildViewChange}
          />
        )
      case 'coding-agents':
        return (
          <CodingAgentsView
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
      case 'guardrails':
        return (
          <GuardrailsView
            activeSubTab={activeSubTab}
            onTabChange={handleChildViewChange}
          />
        )
      case 'strong-weak':
        return (
          <StrongWeakView
            activeSubTab={activeSubTab}
            onTabChange={handleChildViewChange}
          />
        )
      case 'compression':
        return (
          <CompressionView
            activeSubTab={activeSubTab}
            onTabChange={handleChildViewChange}
          />
        )
      case 'secret-scanning':
        return (
          <SecretScanningView
            activeSubTab={activeSubTab}
            onTabChange={handleChildViewChange}
          />
        )
      case 'json-repair':
        return (
          <JsonRepairView
            activeSubTab={activeSubTab}
            onTabChange={handleChildViewChange}
          />
        )
      case 'optimize-overview':
        return (
          <OptimizeOverviewView
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
