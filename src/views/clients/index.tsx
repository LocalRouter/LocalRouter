import { useState, useEffect } from "react"
import { invoke } from "@tauri-apps/api/core"
import { listen } from "@tauri-apps/api/event"
import { Plus, Users } from "lucide-react"
import { Button } from "@/components/ui/Button"
import { Input } from "@/components/ui/Input"
import { ScrollArea } from "@/components/ui/scroll-area"
import {
  ResizablePanelGroup,
  ResizablePanel,
  ResizableHandle,
} from "@/components/ui/resizable"
import { ClientDetail } from "./client-detail"
import { ClientCreationWizard } from "@/components/wizard/ClientCreationWizard"
import { cn } from "@/lib/utils"

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
  skills_access_mode: "none" | "all" | "specific"
  skills_names: string[]
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
  const [wizardOpen, setWizardOpen] = useState(false)
  const [search, setSearch] = useState("")

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

  const handleDeselectClient = () => {
    onTabChange("clients", null)
  }

  const handleWizardComplete = (clientId: string) => {
    setWizardOpen(false)
    loadClients()
    onTabChange("clients", `${clientId}|config`)
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

  const { clientId: selectedClientId, innerTab, mode } = parseSubTab(activeSubTab)

  const selectedClient = clients.find((c) => c.client_id === selectedClientId)

  const filteredClients = clients.filter((c) =>
    c.name.toLowerCase().includes(search.toLowerCase()) ||
    c.client_id.toLowerCase().includes(search.toLowerCase())
  )

  return (
    <div className="flex flex-col h-full min-h-0">
      <div className="flex-shrink-0 pb-4">
        <h1 className="text-2xl font-bold tracking-tight flex items-center gap-2"><Users className="h-6 w-6" />Clients</h1>
        <p className="text-sm text-muted-foreground">
          Give access to your LLM-powered applications by creating a client
        </p>
      </div>

      <ResizablePanelGroup direction="horizontal" className="flex-1 min-h-0 rounded-lg border">
        {/* List Panel */}
        <ResizablePanel defaultSize={35} minSize={25}>
          <div className="flex flex-col h-full">
            <div className="p-4 border-b">
              <div className="flex items-center gap-2">
                <Input
                  placeholder="Search clients..."
                  value={search}
                  onChange={(e) => setSearch(e.target.value)}
                  className="flex-1"
                />
                <Button size="icon" onClick={() => setWizardOpen(true)}>
                  <Plus className="h-4 w-4" />
                </Button>
              </div>
            </div>
            <ScrollArea className="flex-1">
              <div className="p-2 space-y-1">
                {loading ? (
                  <p className="text-sm text-muted-foreground p-4">Loading...</p>
                ) : filteredClients.length === 0 ? (
                  <p className="text-sm text-muted-foreground p-4">No clients found</p>
                ) : (
                  filteredClients.map((client) => (
                    <div
                      key={client.client_id}
                      onClick={() => handleSelectClient(client.client_id)}
                      className={cn(
                        "flex items-center gap-3 p-3 rounded-md cursor-pointer",
                        selectedClientId === client.client_id
                          ? "bg-accent"
                          : "hover:bg-muted"
                      )}
                    >
                      <div className="flex-1 min-w-0">
                        <p className="font-medium truncate">{client.name}</p>
                        <p className="text-xs text-muted-foreground truncate">
                          {client.client_id.slice(0, 16)}...
                        </p>
                      </div>
                      {!client.enabled && (
                        <span className="text-xs text-muted-foreground shrink-0">Disabled</span>
                      )}
                    </div>
                  ))
                )}
              </div>
            </ScrollArea>
          </div>
        </ResizablePanel>

        <ResizableHandle withHandle />

        {/* Detail Panel */}
        <ResizablePanel defaultSize={65}>
          {selectedClient ? (
            <ClientDetail
              clientId={selectedClient.client_id}
              client={selectedClient}
              initialTab={innerTab}
              initialMode={mode as "forced" | "multi" | "prioritized" | null}
              onDeselect={handleDeselectClient}
              onViewChange={onTabChange}
            />
          ) : (
            <div className="flex flex-col items-center justify-center h-full text-muted-foreground gap-4">
              <Users className="h-12 w-12 opacity-30" />
              <div className="text-center">
                <p className="font-medium">Select a client to view details</p>
                <p className="text-sm">
                  or add a new one with the + button
                </p>
              </div>
            </div>
          )}
        </ResizablePanel>
      </ResizablePanelGroup>

      <ClientCreationWizard
        open={wizardOpen}
        onOpenChange={setWizardOpen}
        onComplete={handleWizardComplete}
      />
    </div>
  )
}
