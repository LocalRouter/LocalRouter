
import { useState, useEffect } from "react"
import { invoke } from "@tauri-apps/api/core"
import { listen } from "@tauri-apps/api/event"
import { Tabs, TabsContent, TabsList, TabsTrigger } from "@/components/ui/tabs"
import { ScrollArea } from "@/components/ui/scroll-area"
import { ClientConfigTab } from "./tabs/config-tab"
import { ClientModelsTab } from "./tabs/models-tab"
import { ClientMcpTab } from "./tabs/mcp-tab"
import { ClientSkillsTab } from "./tabs/skills-tab"
import { ClientCodingAgentsTab } from "./tabs/coding-agents-tab"
import { ClientContextTab } from "./tabs/context-tab"
import { ClientGuardrailsTab } from "./tabs/guardrails-tab"
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
  indexing_tools_enabled: boolean | null
  mcp_permissions: McpPermissions
  skills_permissions: SkillsPermissions
  coding_agent_permission: PermissionState
  coding_agent_type: CodingAgentType | null
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

  const [tryItOutSubTab, setTryItOutSubTab] = useState(() => {
    const mode = initialClient?.client_mode || "both"
    return mode === "mcp_only" ? "mcp" : "llm"
  })
  const [mcpInnerPath, setMcpInnerPath] = useState<string | null>(null)

  const clientMode = client?.client_mode || "both"
  const showModelsTab = clientMode !== "mcp_only"
  const showMcpTab = clientMode !== "llm_only"
  const showSkillsTab = clientMode !== "llm_only"
  const showCodingAgentsTab = clientMode !== "llm_only"
  const showGuardrailsTab = clientMode !== "mcp_only"
  const showTryItOutLlm = clientMode !== "mcp_only"
  const showTryItOutMcp = clientMode !== "llm_only"

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
    if (activeTab === "coding-agents" && !showCodingAgentsTab) setActiveTab("connect")
    if (activeTab === "context" && !showMcpTab) setActiveTab("connect")
    if (activeTab === "guardrails" && !showGuardrailsTab) setActiveTab("connect")
  }, [clientMode, activeTab, showModelsTab, showMcpTab, showSkillsTab, showCodingAgentsTab, showGuardrailsTab])

  // If try-it-out sub-tab becomes hidden due to mode change, switch to the available one
  useEffect(() => {
    if (tryItOutSubTab === "llm" && !showTryItOutLlm) setTryItOutSubTab("mcp")
    if (tryItOutSubTab === "mcp" && !showTryItOutMcp) setTryItOutSubTab("llm")
  }, [clientMode, tryItOutSubTab, showTryItOutLlm, showTryItOutMcp])

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
          <TabsList className="bg-transparent h-auto gap-2 p-0 items-end">
            <div className="inline-flex h-9 items-center rounded-lg bg-muted p-1">
              <TabsTrigger value="connect">Connect</TabsTrigger>
              <TabsTrigger value="try-it-out">Try It Out</TabsTrigger>
            </div>

            {showModelsTab && (
              <div>
                <div className="text-[10px] font-medium uppercase tracking-wider text-muted-foreground/50 pl-2 mb-0.5">LLM</div>
                <div className="inline-flex h-9 items-center rounded-lg bg-muted p-1">
                  <TabsTrigger value="models">Providers</TabsTrigger>
                  <TabsTrigger value="guardrails">GuardRails</TabsTrigger>
                </div>
              </div>
            )}

            {showMcpTab && (
              <div>
                <div className="text-[10px] font-medium uppercase tracking-wider text-muted-foreground/50 pl-2 mb-0.5">MCP</div>
                <div className="inline-flex h-9 items-center rounded-lg bg-muted p-1">
                  <TabsTrigger value="mcp">Servers</TabsTrigger>
                  <TabsTrigger value="skills">Skills</TabsTrigger>
                  <TabsTrigger value="coding-agents">Coding Agents</TabsTrigger>
                  <TabsTrigger value="context">Context</TabsTrigger>
                </div>
              </div>
            )}

            <div className="inline-flex h-9 items-center rounded-lg bg-muted p-1">
              <TabsTrigger value="settings">Settings</TabsTrigger>
            </div>
          </TabsList>

          <TabsContent value="connect">
            <ClientConfigTab client={client} onUpdate={loadClient} />
          </TabsContent>

          <TabsContent value="try-it-out">
            <Tabs value={tryItOutSubTab} onValueChange={setTryItOutSubTab} className="space-y-4">
              {showTryItOutLlm && showTryItOutMcp && (
                <TabsList className="w-fit">
                  <TabsTrigger value="llm">LLM Provider</TabsTrigger>
                  <TabsTrigger value="mcp">MCP</TabsTrigger>
                </TabsList>
              )}

              {showTryItOutLlm && (
                <TabsContent value="llm">
                  <LlmTab
                    initialMode="client"
                    initialClientId={client.client_id}
                    hideModeSwitcher
                  />
                </TabsContent>
              )}

              {showTryItOutMcp && (
                <TabsContent value="mcp">
                  <McpTab
                    initialMode="client"
                    initialClientId={client.client_id}
                    hideModeSwitcher
                    innerPath={mcpInnerPath}
                    onPathChange={setMcpInnerPath}
                  />
                </TabsContent>
              )}
            </Tabs>
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

          {showCodingAgentsTab && (
            <TabsContent value="coding-agents">
              <ClientCodingAgentsTab client={client} onUpdate={loadClient} />
            </TabsContent>
          )}

          {showMcpTab && (
            <TabsContent value="context">
              <ClientContextTab client={client} onUpdate={loadClient} onViewChange={onViewChange} />
            </TabsContent>
          )}

          {showGuardrailsTab && (
            <TabsContent value="guardrails">
              <ClientGuardrailsTab
                client={client}
                onUpdate={loadClient}
                onViewChange={onViewChange}
              />
            </TabsContent>
          )}

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
