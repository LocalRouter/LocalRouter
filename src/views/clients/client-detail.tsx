
import { useState, useEffect } from "react"
import { invoke } from "@tauri-apps/api/core"
import { listen } from "@tauri-apps/api/event"
import { Tabs, TabsContent, TabsList, TabsTrigger } from "@/components/ui/tabs"
import { TAB_ICONS, TAB_ICON_CLASS } from "@/constants/tab-icons"
import { ScrollArea } from "@/components/ui/scroll-area"
import { ClientInfoTab } from "./tabs/info-tab"
import { ClientConfigTab } from "./tabs/config-tab"
import { ClientModelsTab } from "./tabs/models-tab"
import { ClientMcpTab } from "./tabs/mcp-tab"
import { ClientContextTab } from "./tabs/context-tab"
import { ClientLlmOptimizeTab } from "./tabs/llm-optimize-tab"
import { ClientSettingsTab } from "./tabs/settings-tab"
import { LlmTab } from "@/views/try-it-out/llm-tab"
import { McpTab } from "@/views/try-it-out/mcp-tab"
import type { McpPermissions, SkillsPermissions, ModelPermissions, PermissionState } from "@/components/permissions"
import type { CodingAgentType } from "@/types/tauri-commands"
import type { ClientMode } from "@/types/tauri-commands"

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
  client_mode?: ClientMode
  template_id?: string | null
  sync_config: boolean
  guardrails_active: boolean
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
  const [activeTab, setActiveTab] = useState(initialTab || "info")

  const [mcpInnerPath, setMcpInnerPath] = useState<string | null>(null)

  const clientMode = client?.client_mode || "both"
  const showModelsTab = clientMode !== "mcp_only"
  const showMcpTab = clientMode !== "llm_only"
  const showTryItOutLlm = clientMode !== "mcp_only"
  // MCP via LLM clients speak only OpenAI protocol — no direct MCP try-it-out
  const showTryItOutMcp = clientMode !== "llm_only" && clientMode !== "mcp_via_llm"

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

  // If active tab becomes hidden due to mode change, fall back to "info"
  useEffect(() => {
    if (activeTab === "models" && !showModelsTab) setActiveTab("info")
    if (activeTab === "mcp" && !showMcpTab) setActiveTab("info")
    if (activeTab === "try-llm" && !showTryItOutLlm) setActiveTab("info")
    if (activeTab === "try-mcp" && !showTryItOutMcp) setActiveTab("info")
  }, [clientMode, activeTab, showModelsTab, showMcpTab, showTryItOutLlm, showTryItOutMcp])

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
        </div>

        {/* Tabs */}
        <Tabs value={activeTab} onValueChange={setActiveTab}>
          <TabsList className="bg-transparent h-auto gap-2 p-0 items-end flex-wrap">
            <div className="inline-flex h-9 items-center rounded-lg bg-muted p-1">
              <TabsTrigger value="info"><TAB_ICONS.overview className={TAB_ICON_CLASS} />Overview</TabsTrigger>
              <TabsTrigger value="connect"><TAB_ICONS.connect className={TAB_ICON_CLASS} />Connect</TabsTrigger>
            </div>

            {(showTryItOutLlm || showTryItOutMcp) && (
              <div>
                <div className="text-[10px] font-medium uppercase tracking-wider text-muted-foreground/50 pl-2 mb-0.5">Try It Out</div>
                <div className="inline-flex h-9 items-center rounded-lg bg-muted p-1">
                  {showTryItOutLlm && <TabsTrigger value="try-llm"><TAB_ICONS.llm className={TAB_ICON_CLASS} />LLM</TabsTrigger>}
                  {showTryItOutMcp && <TabsTrigger value="try-mcp"><TAB_ICONS.mcp className={TAB_ICON_CLASS} />MCP</TabsTrigger>}
                </div>
              </div>
            )}

            {(showModelsTab || showMcpTab) && (
              <div>
                <div className="text-[10px] font-medium uppercase tracking-wider text-muted-foreground/50 pl-2 mb-0.5">Configure</div>
                <div className="inline-flex h-9 items-center rounded-lg bg-muted p-1">
                  {showModelsTab && <TabsTrigger value="models"><TAB_ICONS.llm className={TAB_ICON_CLASS} />LLM</TabsTrigger>}
                  {showMcpTab && <TabsTrigger value="mcp"><TAB_ICONS.mcp className={TAB_ICON_CLASS} />MCP</TabsTrigger>}
                </div>
              </div>
            )}

            <div className="inline-flex h-9 items-center rounded-lg bg-muted p-1">
              <TabsTrigger value="optimize"><TAB_ICONS.optimize className={TAB_ICON_CLASS} />Optimize</TabsTrigger>
              <TabsTrigger value="settings"><TAB_ICONS.settings className={TAB_ICON_CLASS} />Settings</TabsTrigger>
            </div>
          </TabsList>

          <TabsContent value="info">
            <ClientInfoTab client={client} onUpdate={loadClient} />
          </TabsContent>

          <TabsContent value="connect">
            <ClientConfigTab client={client} onUpdate={loadClient} />
          </TabsContent>

          {showTryItOutLlm && (
            <TabsContent value="try-llm">
              <LlmTab
                initialMode="client"
                initialClientId={client.client_id}
                hideModeSwitcher
              />
            </TabsContent>
          )}

          {showTryItOutMcp && (
            <TabsContent value="try-mcp">
              <McpTab
                initialMode="client"
                initialClientId={client.client_id}
                hideModeSwitcher
                innerPath={mcpInnerPath}
                onPathChange={setMcpInnerPath}
              />
            </TabsContent>
          )}

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

          <TabsContent value="optimize">
            <div className="space-y-4">
              {showModelsTab && (
                <ClientLlmOptimizeTab
                  client={client}
                  onUpdate={loadClient}
                  onViewChange={onViewChange}
                />
              )}
              {showMcpTab && (
                <ClientContextTab client={client} onUpdate={loadClient} onViewChange={onViewChange} />
              )}
            </div>
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
