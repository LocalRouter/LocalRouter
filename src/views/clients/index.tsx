import { useState, useEffect } from "react"
import { invoke } from "@tauri-apps/api/core"
import { listen } from "@tauri-apps/api/event"
import { Plus } from "lucide-react"
import { Button } from "@/components/ui/Button"
import { ClientList } from "./client-list"
import { ClientDetail } from "./client-detail"
import { ClientCreateDialog } from "./client-create-dialog"

interface Client {
  id: string
  name: string
  client_id: string
  enabled: boolean
  strategy_id: string
  allowed_llm_providers: string[]
  mcp_access_mode: "none" | "all" | "specific"
  mcp_servers: string[]
  mcp_deferred_loading: boolean
  created_at: string
  last_used: string | null
}

interface ClientsViewProps {
  activeSubTab: string | null
  onTabChange: (view: string, subTab?: string | null) => void
}

export function ClientsView({ activeSubTab, onTabChange }: ClientsViewProps) {
  const [clients, setClients] = useState<Client[]>([])
  const [loading, setLoading] = useState(true)
  const [createDialogOpen, setCreateDialogOpen] = useState(false)

  useEffect(() => {
    loadClients()

    const unsubscribe = listen("clients-changed", () => {
      loadClients()
    })

    return () => {
      unsubscribe.then((fn) => fn())
    }
  }, [])

  const loadClients = async () => {
    try {
      setLoading(true)
      const clientList = await invoke<Client[]>("list_clients")
      setClients(clientList)
    } catch (error) {
      console.error("Failed to load clients:", error)
    } finally {
      setLoading(false)
    }
  }

  const handleSelectClient = (clientId: string) => {
    onTabChange("clients", clientId)
  }

  const handleBack = () => {
    onTabChange("clients", null)
  }

  const handleClientCreated = () => {
    setCreateDialogOpen(false)
    loadClients()
  }

  // Parse subTab to get client ID and optional inner tab
  // Format: "clientId" or "clientId|tab" or "clientId|tab|mode"
  const parseSubTab = (subTab: string | null) => {
    if (!subTab) return { clientId: null, innerTab: null, mode: null }
    const parts = subTab.split("|")
    return {
      clientId: parts[0] || null,
      innerTab: parts[1] || null,
      mode: parts[2] || null,
    }
  }

  const { clientId, innerTab, mode } = parseSubTab(activeSubTab)

  // If a client is selected, show detail view
  if (clientId) {
    const client = clients.find((c) => c.client_id === clientId)
    if (client || loading) {
      return (
        <ClientDetail
          clientId={clientId}
          client={client}
          initialTab={innerTab}
          initialMode={mode as "forced" | "multi" | "prioritized" | null}
          onBack={handleBack}
        />
      )
    }
  }

  // Show client list
  return (
    <div className="space-y-4">
      <div className="flex items-center justify-between">
        <div>
          <h1 className="text-2xl font-bold tracking-tight">Clients</h1>
          <p className="text-sm text-muted-foreground">
            Manage API clients and their permissions
          </p>
        </div>
        <Button onClick={() => setCreateDialogOpen(true)}>
          <Plus className="mr-2 h-4 w-4" />
          Create Client
        </Button>
      </div>

      <ClientList
        clients={clients}
        loading={loading}
        onSelect={handleSelectClient}
        onRefresh={loadClients}
      />

      <ClientCreateDialog
        open={createDialogOpen}
        onOpenChange={setCreateDialogOpen}
        onCreated={handleClientCreated}
      />
    </div>
  )
}
