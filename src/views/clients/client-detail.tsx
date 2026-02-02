
import { useState, useEffect } from "react"
import { invoke } from "@tauri-apps/api/core"
import { listen } from "@tauri-apps/api/event"
import { toast } from "sonner"
import {
  MoreHorizontal,
} from "lucide-react"
import { Button } from "@/components/ui/Button"
import { Badge } from "@/components/ui/Badge"
import { Tabs, TabsContent, TabsList, TabsTrigger } from "@/components/ui/tabs"
import { ScrollArea } from "@/components/ui/scroll-area"
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuTrigger,
} from "@/components/ui/dropdown-menu"
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
import { Switch } from "@/components/ui/Toggle"
import { ClientConfigTab } from "./tabs/config-tab"
import { ClientModelsTab } from "./tabs/models-tab"
import { ClientMcpTab } from "./tabs/mcp-tab"
import { ClientSkillsTab } from "./tabs/skills-tab"

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

interface ClientDetailProps {
  clientId: string
  client?: Client | null
  initialTab?: string | null
  initialMode?: "forced" | "multi" | "prioritized" | null
  onDeselect: () => void
  onViewChange?: (view: string, subTab?: string | null) => void
}

export function ClientDetail({
  clientId,
  client: initialClient,
  initialTab,
  initialMode,
  onDeselect,
  onViewChange,
}: ClientDetailProps) {
  const [client, setClient] = useState<Client | null>(initialClient || null)
  const [loading, setLoading] = useState(!initialClient)
  const [activeTab, setActiveTab] = useState(initialTab || "config")

  useEffect(() => {
    if (!initialClient) {
      loadClient()
    }
  }, [clientId])

  useEffect(() => {
    if (initialClient) {
      setClient(initialClient)
    }
  }, [initialClient])

  // Listen for clients-changed events to refresh data
  useEffect(() => {
    const unsubscribe = listen("clients-changed", () => {
      loadClient()
    })

    return () => {
      unsubscribe.then((fn) => fn())
    }
  }, [clientId])

  useEffect(() => {
    if (initialTab) {
      setActiveTab(initialTab)
    }
  }, [initialTab])

  const loadClient = async () => {
    try {
      // Only set loading when we don't have data - prevents scroll reset
      if (!client) {
        setLoading(true)
      }
      const clients = await invoke<Client[]>("list_clients")
      const found = clients.find((c) => c.client_id === clientId)
      setClient(found || null)
    } catch (error) {
      console.error("Failed to load client:", error)
    } finally {
      setLoading(false)
    }
  }

  const [showDeleteDialog, setShowDeleteDialog] = useState(false)

  const handleToggleEnabled = async () => {
    if (!client) return
    try {
      await invoke("toggle_client_enabled", {
        clientId: client.client_id,
        enabled: !client.enabled,
      })
      setClient({ ...client, enabled: !client.enabled })
      toast.success(`Client ${client.enabled ? "disabled" : "enabled"}`)
    } catch (error) {
      console.error("Failed to toggle client:", error)
      toast.error("Failed to update client")
    }
  }

  const handleDeleteConfirm = async () => {
    if (!client) return
    try {
      await invoke("delete_client", { clientId: client.client_id })
      toast.success("Client deleted")
      onDeselect()
    } catch (error) {
      console.error("Failed to delete client:", error)
      toast.error("Failed to delete client")
    } finally {
      setShowDeleteDialog(false)
    }
  }

  // Only show loading state when we don't have data yet
  if (loading && !client) {
    return (
      <div className="flex items-center justify-center h-full">
        <p className="text-muted-foreground">Loading client...</p>
      </div>
    )
  }

  if (!client) {
    return (
      <div className="flex items-center justify-center h-full">
        <p className="text-muted-foreground">Client not found</p>
      </div>
    )
  }

  return (
    <>
      <ScrollArea className="h-full">
        <div className="p-6 space-y-6">
          {/* Header */}
          <div className="flex items-start justify-between">
            <div>
              <div className="flex items-center gap-2">
                <h2 className="text-xl font-bold">{client.name}</h2>
                <Badge variant={client.enabled ? "success" : "secondary"}>
                  {client.enabled ? "Enabled" : "Disabled"}
                </Badge>
              </div>
            </div>

            <div className="flex items-center gap-2">
              <div className="flex items-center gap-2">
                <span className="text-sm text-muted-foreground">Enabled</span>
                <Switch
                  checked={client.enabled}
                  onCheckedChange={handleToggleEnabled}
                />
              </div>

              <DropdownMenu>
                <DropdownMenuTrigger asChild>
                  <Button variant="outline" size="icon">
                    <MoreHorizontal className="h-4 w-4" />
                  </Button>
                </DropdownMenuTrigger>
                <DropdownMenuContent align="end">
                  <DropdownMenuItem
                    onSelect={() => setShowDeleteDialog(true)}
                    className="text-red-600 dark:text-red-400 focus:text-red-600 dark:focus:text-red-400"
                  >
                    Delete Client
                  </DropdownMenuItem>
                </DropdownMenuContent>
              </DropdownMenu>
            </div>
          </div>

          {/* Tabs */}
          <Tabs value={activeTab} onValueChange={setActiveTab}>
            <TabsList>
              <TabsTrigger value="config">Config</TabsTrigger>
              <TabsTrigger value="models">Models</TabsTrigger>
              <TabsTrigger value="mcp">MCP</TabsTrigger>
              <TabsTrigger value="skills">Skills</TabsTrigger>
            </TabsList>

            <TabsContent value="config">
              <ClientConfigTab client={client} onUpdate={loadClient} />
            </TabsContent>

            <TabsContent value="models">
              <ClientModelsTab
                client={client}
                onUpdate={loadClient}
                initialMode={initialMode}
                onViewChange={onViewChange}
              />
            </TabsContent>

            <TabsContent value="mcp">
              <ClientMcpTab client={client} onUpdate={loadClient} />
            </TabsContent>

            <TabsContent value="skills">
              <ClientSkillsTab client={client} onUpdate={loadClient} />
            </TabsContent>
          </Tabs>
        </div>
      </ScrollArea>

      <AlertDialog open={showDeleteDialog} onOpenChange={setShowDeleteDialog}>
        <AlertDialogContent>
          <AlertDialogHeader>
            <AlertDialogTitle>Delete Client?</AlertDialogTitle>
            <AlertDialogDescription>
              This will permanently delete "{client.name}" and revoke its API key.
              This action cannot be undone.
            </AlertDialogDescription>
          </AlertDialogHeader>
          <AlertDialogFooter>
            <AlertDialogCancel>Cancel</AlertDialogCancel>
            <AlertDialogAction
              onClick={handleDeleteConfirm}
              className="bg-red-600 text-white hover:bg-red-700 dark:bg-red-600 dark:hover:bg-red-700"
            >
              Delete
            </AlertDialogAction>
          </AlertDialogFooter>
        </AlertDialogContent>
      </AlertDialog>
    </>
  )
}
