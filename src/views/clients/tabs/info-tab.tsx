import { useState, useEffect, useMemo } from "react"
import { invoke } from "@tauri-apps/api/core"
import { toast } from "sonner"
import { FEATURES } from "@/constants/features"
import ReactFlow, {
  ReactFlowProvider,
  Background,
  Controls,
  useNodesState,
  useEdgesState,
  type NodeTypes,
  type Node,
  type Edge,
} from "reactflow"
import "reactflow/dist/style.css"
import { Skeleton } from "@/components/ui/skeleton"
import { Card, CardContent, CardHeader, CardTitle, CardDescription } from "@/components/ui/Card"
import { ClientModeSelector } from "@/components/client/ClientModeSelector"
import { CLIENT_TEMPLATES } from "@/components/client/ClientTemplates"
import { useGraphData } from "@/components/connection-graph/hooks/useGraphData"
import { buildGraph } from "@/components/connection-graph/utils/buildGraph"
import { AccessKeyNode } from "@/components/connection-graph/nodes/AccessKeyNode"
import { ProviderNode } from "@/components/connection-graph/nodes/ProviderNode"
import { McpServerNode } from "@/components/connection-graph/nodes/McpServerNode"
import { SkillNode } from "@/components/connection-graph/nodes/SkillNode"
import { CodingAgentNode } from "@/components/connection-graph/nodes/CodingAgentNode"
import { MarketplaceNode } from "@/components/connection-graph/nodes/MarketplaceNode"
import { EndpointNode } from "@/components/connection-graph/nodes/EndpointNode"
import { RouterGroupNode } from "@/components/connection-graph/nodes/RouterGroupNode"
import type { GraphNodeData } from "@/components/connection-graph/types"
import type {
  ClientMode, CodingAgentType, ClientEffectiveConfig,
  GetClientEffectiveConfigParams, Strategy, SetClientModeParams,
} from "@/types/tauri-commands"
import type { McpPermissions, SkillsPermissions, ModelPermissions, PermissionState } from "@/components/permissions"

const nodeTypes: NodeTypes = {
  accessKey: AccessKeyNode,
  provider: ProviderNode,
  mcpServer: McpServerNode,
  skill: SkillNode,
  codingAgent: CodingAgentNode,
  marketplace: MarketplaceNode,
  endpoint: EndpointNode,
  routerGroup: RouterGroupNode,
}

interface Client {
  id: string
  name: string
  client_id: string
  enabled: boolean
  strategy_id: string
  context_management_enabled: boolean | null
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
  json_repair_active: boolean
  created_at: string
  last_used: string | null
}

interface InfoTabProps {
  client: Client
  onUpdate: () => void
}

function formatModelName(fullName: string): string {
  const parts = fullName.split("/")
  return parts.length > 1 ? parts.slice(1).join("/") : fullName
}

export function ClientInfoTab({ client, onUpdate }: InfoTabProps) {
  const [effectiveConfig, setEffectiveConfig] = useState<ClientEffectiveConfig | null>(null)
  const [strategy, setStrategy] = useState<Strategy | null>(null)

  const { clients: allClients, providers, mcpServers, skills, codingAgents, strategies, healthState, activeConnections, loading } = useGraphData()

  const clientMode = client.client_mode || "both"
  const showLlm = clientMode !== "mcp_only"
  const showMcp = clientMode !== "llm_only"

  const template = client.template_id
    ? CLIENT_TEMPLATES.find(t => t.id === client.template_id) || null
    : null

  const handleModeChange = async (mode: ClientMode) => {
    try {
      await invoke("set_client_mode", {
        clientId: client.client_id,
        mode,
      } satisfies SetClientModeParams)
      toast.success("Client mode updated")
      onUpdate()
    } catch (error) {
      console.error("Failed to update client mode:", error)
      toast.error("Failed to update client mode")
    }
  }

  useEffect(() => {
    // Reset before fetching so stale values from the previous client don't
    // briefly render, and so the second load can't be overwritten by the
    // first one's late-arriving response.
    setEffectiveConfig(null)
    setStrategy(null)

    let cancelled = false

    invoke<ClientEffectiveConfig>("get_client_effective_config", {
      clientId: client.client_id,
    } satisfies GetClientEffectiveConfigParams)
      .then((data) => {
        if (!cancelled) setEffectiveConfig(data)
      })
      .catch((err) => {
        if (!cancelled) console.error(err)
      })

    invoke<Strategy>("get_strategy", {
      strategyId: client.strategy_id,
    })
      .then((data) => {
        if (!cancelled) setStrategy(data)
      })
      .catch((err) => {
        if (!cancelled) console.error(err)
      })

    return () => {
      cancelled = true
    }
  }, [client.client_id, client.strategy_id, client.context_management_enabled])

  // Build graph filtered to just this client
  const { graphNodes, graphEdges, graphBounds } = useMemo(() => {
    const thisClient = allClients.find(c => c.id === client.id)
    if (!thisClient) return { graphNodes: [], graphEdges: [], graphBounds: { width: 0, height: 0 } }

    const { nodes, edges, bounds } = buildGraph(
      [thisClient],
      providers,
      mcpServers,
      skills,
      codingAgents,
      healthState,
      activeConnections,
      strategies,
      clientMode,
    )
    return { graphNodes: nodes as Node<GraphNodeData>[], graphEdges: edges as Edge[], graphBounds: bounds }
  }, [allClients, client.id, clientMode, providers, mcpServers, skills, codingAgents, healthState, activeConnections])

  const [nodes, setNodes, onNodesChange] = useNodesState<GraphNodeData>([])
  const [edges, setEdges, onEdgesChange] = useEdgesState([])

  useEffect(() => {
    setNodes(graphNodes)
    setEdges(graphEdges)
  }, [graphNodes, graphEdges, setNodes, setEdges])

  // Strategy model info
  const autoConfig = strategy?.auto_config
  const strongModels = autoConfig?.prioritized_models || []
  const weakModels = autoConfig?.routellm_config?.weak_models || []
  const hasRouteLLM = autoConfig?.routellm_config?.enabled === true

  // Feature pills
  const pills: { label: string; detail?: string; source?: string }[] = []

  if (showLlm) {
    if (strongModels.length > 0) {
      pills.push({
        label: "Strong Models",
        detail: strongModels.map(([, m]) => formatModelName(m)).join(", "),
      })
    }
    if (weakModels.length > 0) {
      pills.push({
        label: "Weak Models",
        detail: weakModels.map(([, m]) => formatModelName(m)).join(", "),
      })
    }
    if (hasRouteLLM) {
      pills.push({
        label: "RouteLLM",
        detail: `threshold ${autoConfig?.routellm_config?.threshold}`,
      })
    }
    if (client.guardrails_active) {
      pills.push({ label: "GuardRails" })
    }
    if (client.json_repair_active) {
      pills.push({ label: "JSON Repair" })
    }
  }

  if (showMcp && effectiveConfig) {
    if (effectiveConfig.context_management_effective) {
      pills.push({
        label: FEATURES.responseRag.name,
        source: effectiveConfig.context_management_source,
      })
    }
    if (effectiveConfig.catalog_compression_effective) {
      pills.push({
        label: FEATURES.catalogCompression.name,
        source: effectiveConfig.catalog_compression_source,
      })
    }
  }

  const containerHeight = Math.max(120, graphBounds.height)

  return (
    <div className="space-y-4">
      {/* Client Mode */}
      <Card>
        <CardHeader>
          <CardTitle>Client Mode</CardTitle>
          <CardDescription>
            Controls which features are available to this client
          </CardDescription>
        </CardHeader>
        <CardContent>
          <ClientModeSelector mode={clientMode} onModeChange={handleModeChange} template={template} />
        </CardContent>
      </Card>

      {/* Connection Graph */}
      {loading ? (
        <Skeleton className="w-full h-[150px] rounded-lg" />
      ) : graphNodes.length > 0 ? (
        <div style={{ height: containerHeight }} className="w-full rounded-lg border overflow-hidden">
          <ReactFlowProvider>
            <ReactFlow
              nodes={nodes}
              edges={edges}
              onNodesChange={onNodesChange}
              onEdgesChange={onEdgesChange}
              nodeTypes={nodeTypes}
              defaultViewport={{ x: 0, y: 0, zoom: 1 }}
              minZoom={0.5}
              maxZoom={1.5}
              nodesDraggable={false}
              nodesConnectable={false}
              elementsSelectable={false}
              panOnDrag={true}
              zoomOnScroll={true}
              preventScrolling={false}
              proOptions={{ hideAttribution: true }}
            >
              <Background color="#94a3b8" gap={16} size={1} />
              <Controls
                showZoom={true}
                showFitView={true}
                showInteractive={false}
                position="bottom-right"
              />
            </ReactFlow>
          </ReactFlowProvider>
        </div>
      ) : null}

      {/* Feature pills */}
      {pills.length > 0 && (
        <div className="flex flex-wrap gap-2">
          {pills.map((p) => (
            <span
              key={p.label}
              className="inline-flex items-center gap-1.5 rounded-full border px-3 py-1 text-xs font-medium bg-muted/50"
              title={p.detail || undefined}
            >
              <span className="h-1.5 w-1.5 rounded-full bg-green-500 shrink-0" />
              {p.label}
              {p.detail && (
                <span className="text-muted-foreground font-normal truncate max-w-[200px]">{p.detail}</span>
              )}
              {p.source && (
                <span className="text-[10px] text-muted-foreground/70 font-normal">
                  ({p.source === "global" ? "inherited" : "override"})
                </span>
              )}
            </span>
          ))}
        </div>
      )}
    </div>
  )
}
