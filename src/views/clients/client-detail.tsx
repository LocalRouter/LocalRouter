
import { useState, useEffect } from "react"
import { invoke } from "@tauri-apps/api/core"
import { listen } from "@tauri-apps/api/event"
import { Tabs, TabsContent, TabsList, TabsTrigger } from "@/components/ui/tabs"
import { ScrollArea } from "@/components/ui/scroll-area"
import { Button } from "@/components/ui/Button"
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuTrigger,
} from "@/components/ui/dropdown-menu"
import { FlaskConical, MessageSquare, Puzzle, Shield, ChevronDown } from "lucide-react"
import { ClientConfigTab } from "./tabs/config-tab"
import { ClientModelsTab } from "./tabs/models-tab"
import { ClientMcpTab } from "./tabs/mcp-tab"
import { ClientSkillsTab } from "./tabs/skills-tab"
import { ClientGuardrailsTab } from "./tabs/guardrails-tab"
import { ClientSettingsTab } from "./tabs/settings-tab"
import type { McpPermissions, SkillsPermissions, ModelPermissions, PermissionState } from "@/components/permissions"
import type { ClientMode } from "@/types/tauri-commands"

interface Client {
  id: string
  name: string
  client_id: string
  enabled: boolean
  strategy_id: string
  mcp_deferred_loading: boolean
  mcp_permissions: McpPermissions
  skills_permissions: SkillsPermissions
  model_permissions: ModelPermissions
  marketplace_permission: PermissionState
  client_mode?: ClientMode
  template_id?: string | null
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

  const clientMode = client?.client_mode || "both"
  const showModelsTab = clientMode !== "mcp_only"
  const showMcpTab = clientMode !== "llm_only"
  const showSkillsTab = clientMode !== "llm_only"

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

  // If active tab becomes hidden due to mode change, fall back to "connect"
  useEffect(() => {
    if (activeTab === "models" && !showModelsTab) setActiveTab("connect")
    if (activeTab === "mcp" && !showMcpTab) setActiveTab("connect")
    if (activeTab === "skills" && !showSkillsTab) setActiveTab("connect")
  }, [clientMode, activeTab, showModelsTab, showMcpTab, showSkillsTab])

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
        <div className="flex items-center justify-between">
          <h2 className="text-xl font-bold">{client.name}</h2>
          {onViewChange && client.enabled && (
            <DropdownMenu>
              <DropdownMenuTrigger asChild>
                <Button variant="outline" size="sm">
                  <FlaskConical className="h-4 w-4 mr-1" />
                  Try It Out
                  <ChevronDown className="h-3 w-3 ml-1" />
                </Button>
              </DropdownMenuTrigger>
              <DropdownMenuContent align="end">
                {showModelsTab && (
                  <DropdownMenuItem
                    onClick={() => onViewChange("try-it-out", `llm/init/client/${client.client_id}`)}
                  >
                    <MessageSquare className="h-4 w-4 mr-2" />
                    LLM
                  </DropdownMenuItem>
                )}
                {showMcpTab && (
                  <DropdownMenuItem
                    onClick={() => onViewChange("try-it-out", `mcp/init/client/${client.client_id}`)}
                  >
                    <Puzzle className="h-4 w-4 mr-2" />
                    MCP & Skills
                  </DropdownMenuItem>
                )}
                <DropdownMenuItem
                  onClick={() => onViewChange("try-it-out", `guardrails/init/client/${client.client_id}`)}
                >
                  <Shield className="h-4 w-4 mr-2" />
                  GuardRails
                </DropdownMenuItem>
              </DropdownMenuContent>
            </DropdownMenu>
          )}
        </div>

        {/* Tabs */}
        <Tabs value={activeTab} onValueChange={setActiveTab}>
          <TabsList>
            <TabsTrigger value="connect">Connect</TabsTrigger>
            {showModelsTab && <TabsTrigger value="models">Models</TabsTrigger>}
            {showMcpTab && <TabsTrigger value="mcp">MCP</TabsTrigger>}
            {showSkillsTab && <TabsTrigger value="skills">Skills</TabsTrigger>}
            <TabsTrigger value="guardrails">GuardRails</TabsTrigger>
            <TabsTrigger value="settings">Settings</TabsTrigger>
          </TabsList>

          <TabsContent value="connect">
            <ClientConfigTab client={client} onUpdate={loadClient} />
          </TabsContent>

          {showModelsTab && (
            <TabsContent value="models">
              <ClientModelsTab
                client={client}
                onUpdate={loadClient}
                initialMode={initialMode}
                onViewChange={onViewChange}
              />
            </TabsContent>
          )}

          {showMcpTab && (
            <TabsContent value="mcp">
              <ClientMcpTab client={client} onUpdate={loadClient} />
            </TabsContent>
          )}

          {showSkillsTab && (
            <TabsContent value="skills">
              <ClientSkillsTab client={client} onUpdate={loadClient} />
            </TabsContent>
          )}

          <TabsContent value="guardrails">
            <ClientGuardrailsTab
              client={client}
              onUpdate={loadClient}
              onViewChange={onViewChange}
            />
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
