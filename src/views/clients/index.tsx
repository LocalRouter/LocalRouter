import { useState, useEffect, useRef } from "react"
import { invoke } from "@tauri-apps/api/core"
import { listenSafe } from "@/hooks/useTauriListener"
import { toast } from "sonner"
import { Plus, Users, ArrowLeft, Copy, Trash2 } from "lucide-react"
import { Button } from "@/components/ui/Button"
import { Input } from "@/components/ui/Input"
import { Tabs, TabsContent, TabsList, TabsTrigger } from "@/components/ui/tabs"
import {
  AlertDialog,
  AlertDialogAction,
  AlertDialogCancel,
  AlertDialogContent,
  AlertDialogDescription,
  AlertDialogFooter,
  AlertDialogHeader,
  AlertDialogTitle,
} from "@/components/ui/alert-dialog"
import { TAB_ICONS, TAB_ICON_CLASS } from "@/constants/tab-icons"
import { ClientDetail } from "./client-detail"
import { ClientCreationWizard } from "@/components/wizard/ClientCreationWizard"
import { ServerTab } from "@/views/settings/server-tab"

import type { McpPermissions, SkillsPermissions, ModelPermissions, PermissionState } from "@/components/permissions"
import type { CodingAgentType, ClientInfo, CloneClientParams, DeleteClientParams } from "@/types/tauri-commands"

interface Client {
  id: string
  name: string
  client_id: string
  enabled: boolean
  strategy_id: string
  context_management_enabled: boolean | null
  catalog_compression_enabled: boolean | null
  mcp_permissions: McpPermissions
  skills_permissions: SkillsPermissions
  coding_agent_permission: PermissionState
  coding_agent_type: CodingAgentType | null
  model_permissions: ModelPermissions
  marketplace_permission: PermissionState
  sync_config: boolean
  guardrails_active: boolean
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
  const [wizardTemplateId, setWizardTemplateId] = useState<string | null>(null)
  const [search, setSearch] = useState("")
  const [clientToDelete, setClientToDelete] = useState<Client | null>(null)

  const handleCloneClient = async (e: React.MouseEvent, client: Client) => {
    e.stopPropagation()
    try {
      const [, cloned] = await invoke<[string, ClientInfo]>("clone_client", { clientId: client.client_id } satisfies CloneClientParams)
      toast.success(`Cloned as "${cloned.name}"`)
    } catch (error) {
      toast.error(`Failed to clone client: ${error}`)
    }
  }

  const handleDeleteClient = async () => {
    if (!clientToDelete) return
    try {
      await invoke("delete_client", { clientId: clientToDelete.client_id } satisfies DeleteClientParams)
      toast.success("Client deleted")
      loadClients()
    } catch (error) {
      toast.error(`Failed to delete client: ${error}`)
    } finally {
      setClientToDelete(null)
    }
  }

  useEffect(() => {
    loadClients()

    const l = listenSafe("clients-changed", () => {
      loadClients()
    })

    return () => {
      l.cleanup()
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

  // Handle add/<templateId> navigation from command palette
  const addTemplatePrefix = "add/"
  const isAddNavigation = activeSubTab?.startsWith(addTemplatePrefix)
  const addTemplateIdFromNav = isAddNavigation ? activeSubTab!.slice(addTemplatePrefix.length) : null
  const handledAddRef = useRef<string | null>(null)

  useEffect(() => {
    if (addTemplateIdFromNav && handledAddRef.current !== addTemplateIdFromNav) {
      handledAddRef.current = addTemplateIdFromNav
      setWizardTemplateId(addTemplateIdFromNav)
      setWizardOpen(true)
      // Clear the add/ subTab so it doesn't re-trigger
      onTabChange("clients", null)
    }
  }, [addTemplateIdFromNav])

  // Determine top-level tab: "client" or "settings"
  const topTab = activeSubTab === "settings" ? "settings" : "client"

  // Parse subTab to get client ID and optional inner tab (for client tab)
  // Format: "clientId" or "clientId|tab" or "clientId|tab|mode"
  const parseSubTab = (subTab: string | null) => {
    if (!subTab || subTab.startsWith(addTemplatePrefix) || subTab === "settings") {
      return { clientId: null, innerTab: null, mode: null }
    }
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

  const handleTopTabChange = (tab: string) => {
    if (tab === "settings") {
      onTabChange("clients", "settings")
    } else {
      // Switch back to client tab, preserve selected client if any
      onTabChange("clients", selectedClientId || null)
    }
  }

  return (
    <div className="flex flex-col h-full min-h-0 max-w-5xl">
      {selectedClient ? (
        // SPOKE: Full-screen client detail
        <>
          <div className="flex-shrink-0 pb-2">
            <Button variant="ghost" size="sm" className="gap-1 -ml-2" onClick={handleDeselectClient}>
              <ArrowLeft className="h-3 w-3" />
              Back to Clients
            </Button>
          </div>
          <div className="flex-1 min-h-0">
            <ClientDetail
              clientId={selectedClient.client_id}
              client={selectedClient}
              initialTab={innerTab}
              initialMode={mode as "forced" | "multi" | "prioritized" | null}
              onDeselect={handleDeselectClient}
              onViewChange={onTabChange}
            />
          </div>
        </>
      ) : (
        // HUB: Overview with card list
        <>
          <div className="flex-shrink-0 pb-4">
            <h1 className="text-2xl font-bold tracking-tight flex items-center gap-2"><Users className="h-6 w-6" />Clients</h1>
            <p className="text-sm text-muted-foreground">
              Give access to your LLM-powered applications by creating a client
            </p>
          </div>

          <Tabs
            value={topTab}
            onValueChange={handleTopTabChange}
            className="flex flex-col flex-1 min-h-0"
          >
            <TabsList className="flex-shrink-0 w-fit">
              <TabsTrigger value="client"><TAB_ICONS.client className={TAB_ICON_CLASS} />Client</TabsTrigger>
              <TabsTrigger value="settings"><TAB_ICONS.settings className={TAB_ICON_CLASS} />Settings</TabsTrigger>
            </TabsList>

            <TabsContent value="client" className="flex-1 min-h-0 mt-4">
              <div className="flex flex-col h-full rounded-lg border">
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
                <div className="flex-1 overflow-y-auto p-2 space-y-1">
                  {loading ? (
                    <p className="text-sm text-muted-foreground p-4">Loading...</p>
                  ) : filteredClients.length === 0 ? (
                    <p className="text-sm text-muted-foreground p-4">No clients found</p>
                  ) : (
                    filteredClients.map((client) => (
                      <div
                        key={client.client_id}
                        onClick={() => handleSelectClient(client.client_id)}
                        className="group flex items-center gap-3 p-3 rounded-md cursor-pointer hover:bg-muted"
                      >
                        <div className="flex-1 min-w-0">
                          <p className="font-medium truncate">{client.name}</p>
                          <p className="text-xs text-muted-foreground truncate">
                            {client.client_id.slice(0, 16)}...
                          </p>
                        </div>
                        <div className="flex items-center gap-1 opacity-0 group-hover:opacity-100 transition-opacity shrink-0">
                          <Button
                            variant="ghost"
                            size="icon"
                            className="h-7 w-7"
                            title="Clone client"
                            onClick={(e) => handleCloneClient(e, client)}
                          >
                            <Copy className="h-3.5 w-3.5" />
                          </Button>
                          <Button
                            variant="ghost"
                            size="icon"
                            className="h-7 w-7 text-destructive hover:text-destructive"
                            title="Delete client"
                            onClick={(e) => {
                              e.stopPropagation()
                              setClientToDelete(client)
                            }}
                          >
                            <Trash2 className="h-3.5 w-3.5" />
                          </Button>
                        </div>
                        {!client.enabled && (
                          <span className="text-xs text-muted-foreground shrink-0">Disabled</span>
                        )}
                      </div>
                    ))
                  )}
                </div>
              </div>
            </TabsContent>

            <TabsContent value="settings" className="flex-1 min-h-0 mt-4 overflow-y-auto">
              <ServerTab hideResourceLimits />
            </TabsContent>
          </Tabs>
        </>
      )}

      <ClientCreationWizard
        open={wizardOpen}
        onOpenChange={(open) => {
          setWizardOpen(open)
          if (!open) {
            setWizardTemplateId(null)
            handledAddRef.current = null
          }
        }}
        onComplete={handleWizardComplete}
        initialTemplateId={wizardTemplateId}
      />

      {/* Delete Confirmation Dialog */}
      <AlertDialog open={!!clientToDelete} onOpenChange={(open) => !open && setClientToDelete(null)}>
        <AlertDialogContent>
          <AlertDialogHeader>
            <AlertDialogTitle>Delete Client</AlertDialogTitle>
            <AlertDialogDescription>
              Are you sure you want to delete "{clientToDelete?.name}"? This will also delete its routing strategy. This action cannot be undone.
            </AlertDialogDescription>
          </AlertDialogHeader>
          <AlertDialogFooter>
            <AlertDialogCancel>Cancel</AlertDialogCancel>
            <AlertDialogAction onClick={handleDeleteClient} className="bg-destructive text-destructive-foreground hover:bg-destructive/90">
              Delete
            </AlertDialogAction>
          </AlertDialogFooter>
        </AlertDialogContent>
      </AlertDialog>
    </div>
  )
}
