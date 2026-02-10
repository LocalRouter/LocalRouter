import * as React from "react"
import { useState, useEffect } from "react"
import { invoke } from "@tauri-apps/api/core"
import { listen } from "@tauri-apps/api/event"
import { Sidebar, type View } from "./sidebar"
import { Header } from "./header"
import { CommandPalette } from "./command-palette"
import { Toaster } from "@/components/ui/sonner"

interface Client {
  id: string
  name: string
  client_id: string
  enabled: boolean
}

interface ProviderInstance {
  instance_name: string
  provider_type: string
  enabled: boolean
}

interface ProviderType {
  provider_type: string
  display_name: string
  category: string
  description: string
}

interface McpServer {
  id: string
  name: string
  enabled: boolean
}

interface Model {
  id: string
  provider: string
}

// DEPRECATED: Strategy UI hidden - 1:1 client-to-strategy relationship
// interface Strategy {
//   id: string
//   name: string
//   parent: string | null
// }

interface AppShellProps {
  children: React.ReactNode
  activeView: View
  activeSubTab: string | null
  onViewChange: (view: View, subTab?: string | null) => void
}

export function AppShell({
  children,
  activeView,
  activeSubTab: _activeSubTab,
  onViewChange,
}: AppShellProps) {
  const [commandOpen, setCommandOpen] = useState(false)
  const [clients, setClients] = useState<Client[]>([])
  const [providers, setProviders] = useState<ProviderInstance[]>([])
  const [providerTypes, setProviderTypes] = useState<ProviderType[]>([])
  const [mcpServers, setMcpServers] = useState<McpServer[]>([])
  const [models, setModels] = useState<Model[]>([])
  // DEPRECATED: Strategy UI hidden - 1:1 client-to-strategy relationship
  // const [strategies, setStrategies] = useState<Strategy[]>([])

  // Load data for command palette search
  useEffect(() => {
    loadData()

    // Subscribe to data changes
    const unsubscribers = [
      listen('clients-changed', loadClients),
      listen('providers-changed', loadProviders),
      listen('mcp-servers-changed', loadMcpServers),
      listen('models-changed', loadModels),
      // DEPRECATED: Strategy UI hidden - 1:1 client-to-strategy relationship
      // listen('strategies-changed', loadStrategies),
    ]

    return () => {
      unsubscribers.forEach(async (unsub) => {
        const fn = await unsub
        fn()
      })
    }
  }, [])

  const loadData = async () => {
    await Promise.all([
      loadClients(),
      loadProviders(),
      loadProviderTypes(),
      loadMcpServers(),
      loadModels(),
      // DEPRECATED: loadStrategies(),
    ])
  }

  const loadClients = async () => {
    try {
      const clientList = await invoke<Client[]>('list_clients')
      setClients(clientList)
    } catch (err) {
      console.error('Failed to load clients:', err)
    }
  }

  const loadProviders = async () => {
    try {
      const providerList = await invoke<ProviderInstance[]>('list_provider_instances')
      setProviders(providerList)
    } catch (err) {
      console.error('Failed to load providers:', err)
    }
  }

  const loadProviderTypes = async () => {
    try {
      const typeList = await invoke<ProviderType[]>('list_provider_types')
      setProviderTypes(typeList)
    } catch (err) {
      console.error('Failed to load provider types:', err)
    }
  }

  const loadMcpServers = async () => {
    try {
      const serverList = await invoke<McpServer[]>('list_mcp_servers')
      setMcpServers(serverList)
    } catch (err) {
      console.error('Failed to load MCP servers:', err)
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

  // DEPRECATED: Strategy UI hidden - 1:1 client-to-strategy relationship
  // const loadStrategies = async () => {
  //   try {
  //     const strategyList = await invoke<Strategy[]>('list_strategies')
  //     setStrategies(strategyList)
  //   } catch (err) {
  //     console.error('Failed to load strategies:', err)
  //   }
  // }

  const handleViewChange = (view: View, subTab?: string | null) => {
    onViewChange(view, subTab)
  }

  const handleAddProvider = (providerType: string) => {
    // Navigate to providers panel with the add provider dialog open for this type
    onViewChange('resources', `providers/add/${providerType}`)
  }

  const handleAddMcpServer = (templateId: string) => {
    // Navigate to MCP servers panel with the add server dialog open for this template
    onViewChange('mcp-servers', `add/${templateId}`)
  }

  return (
    <div className="flex h-full w-full bg-background overflow-hidden">
      {/* Sidebar */}
      <Sidebar activeView={activeView} onViewChange={handleViewChange} />

      {/* Main content area */}
      <div className="flex flex-1 flex-col min-h-0 overflow-hidden">
        {/* Header */}
        <Header onOpenCommandPalette={() => setCommandOpen(true)} />

        {/* Content */}
        <main className="flex-1 min-h-0 overflow-auto p-4">
          {children}
        </main>
      </div>

      {/* Command Palette */}
      <CommandPalette
        open={commandOpen}
        onOpenChange={setCommandOpen}
        onViewChange={handleViewChange}
        onAddProvider={handleAddProvider}
        onAddMcpServer={handleAddMcpServer}
        clients={clients}
        providers={providers}
        providerTypes={providerTypes}
        models={models}
        mcpServers={mcpServers}
        strategies={[] /* DEPRECATED: Strategy UI hidden - 1:1 client-to-strategy relationship */}
      />

      {/* Toast notifications */}
      <Toaster position="bottom-right" />
    </div>
  )
}
