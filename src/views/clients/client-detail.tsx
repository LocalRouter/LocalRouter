
import { useState, useEffect } from "react"
import { invoke } from "@tauri-apps/api/core"
import { listen } from "@tauri-apps/api/event"
import { Tabs, TabsContent, TabsList, TabsTrigger } from "@/components/ui/tabs"
import { ScrollArea } from "@/components/ui/scroll-area"
import { ClientConfigTab } from "./tabs/config-tab"
import { ClientModelsTab } from "./tabs/models-tab"
import { ClientMcpTab } from "./tabs/mcp-tab"
import { ClientSkillsTab } from "./tabs/skills-tab"
import { ClientSettingsTab } from "./tabs/settings-tab"

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
  const [activeTab, setActiveTab] = useState(initialTab || "connect")

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
    <ScrollArea className="h-full">
      <div className="p-6 space-y-6">
        {/* Header */}
        <div>
          <h2 className="text-xl font-bold">{client.name}</h2>
        </div>

        {/* Tabs */}
        <Tabs value={activeTab} onValueChange={setActiveTab}>
          <TabsList>
            <TabsTrigger value="connect">Connect</TabsTrigger>
            <TabsTrigger value="models">Models</TabsTrigger>
            <TabsTrigger value="mcp">MCP</TabsTrigger>
            <TabsTrigger value="skills">Skills</TabsTrigger>
            <TabsTrigger value="settings">Settings</TabsTrigger>
          </TabsList>

          <TabsContent value="connect">
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

          <TabsContent value="settings">
            <ClientSettingsTab
              client={client}
              onUpdate={loadClient}
              onDelete={onDeselect}
            />
          </TabsContent>
        </Tabs>
      </div>
    </ScrollArea>
  )
}
