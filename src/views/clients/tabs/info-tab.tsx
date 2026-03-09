import { useState, useEffect, useMemo } from "react"
import { invoke } from "@tauri-apps/api/core"
import ReactFlow, {
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
import { useGraphData } from "@/components/connection-graph/hooks/useGraphData"
import { buildGraph } from "@/components/connection-graph/utils/buildGraph"
import { AccessKeyNode } from "@/components/connection-graph/nodes/AccessKeyNode"
import { ProviderNode } from "@/components/connection-graph/nodes/ProviderNode"
import { McpServerNode } from "@/components/connection-graph/nodes/McpServerNode"
import { SkillNode } from "@/components/connection-graph/nodes/SkillNode"
import { CodingAgentNode } from "@/components/connection-graph/nodes/CodingAgentNode"
import { MarketplaceNode } from "@/components/connection-graph/nodes/MarketplaceNode"
import type { GraphNodeData } from "@/components/connection-graph/types"
import type {
  ClientMode, CodingAgentType, ClientEffectiveConfig,
  GetClientEffectiveConfigParams, Strategy,
} from "@/types/tauri-commands"
import type { McpPermissions, SkillsPermissions, ModelPermissions, PermissionState } from "@/components/permissions"

const nodeTypes: NodeTypes = {
  accessKey: AccessKeyNode,
  provider: ProviderNode,
  mcpServer: McpServerNode,
  skill: SkillNode,
  codingAgent: CodingAgentNode,
  marketplace: MarketplaceNode,
}

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
  sync_config: boolean
  guardrails_active: boolean
  created_at: string
  last_used: string | null
}

interface InfoTabProps {
  client: Client
}

function formatModelName(fullName: string): string {
  const parts = fullName.split("/")
  return parts.length > 1 ? parts.slice(1).join("/") : fullName
}

export function ClientInfoTab({ client }: InfoTabProps) {
  const [effectiveConfig, setEffectiveConfig] = useState<ClientEffectiveConfig | null>(null)
  const [strategy, setStrategy] = useState<Strategy | null>(null)

  const { clients: allClients, providers, mcpServers, skills, codingAgents, strategies, healthState, activeConnections, loading } = useGraphData()

  const clientMode = client.client_mode || "both"
  const showLlm = clientMode !== "mcp_only"
  const showMcp = clientMode !== "llm_only"

  useEffect(() => {
    invoke<ClientEffectiveConfig>("get_client_effective_config", {
      clientId: client.client_id,
    } satisfies GetClientEffectiveConfigParams).then(setEffectiveConfig).catch(console.error)

    invoke<Strategy>("get_strategy", {
      strategyId: client.strategy_id,
    }).then(setStrategy).catch(console.error)
  }, [client.client_id, client.strategy_id, client.context_management_enabled, client.indexing_tools_enabled])

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
    )
    return { graphNodes: nodes as Node<GraphNodeData>[], graphEdges: edges as Edge[], graphBounds: bounds }
  }, [allClients, client.id, providers, mcpServers, skills, codingAgents, healthState, activeConnections])

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

  // Feature rows
  const features: { label: string; value: string; source?: string }[] = []

  if (showLlm) {
    if (strongModels.length > 0) {
      features.push({
        label: "Strong models",
        value: strongModels.map(([, m]) => formatModelName(m)).join(", "),
      })
    }
    if (weakModels.length > 0) {
      features.push({
        label: "Weak models",
        value: weakModels.map(([, m]) => formatModelName(m)).join(", "),
      })
    }
    if (hasRouteLLM) {
      features.push({
        label: "RouteLLM",
        value: `threshold ${autoConfig?.routellm_config?.threshold}`,
      })
    }
    if (client.guardrails_active) {
      features.push({ label: "GuardRails", value: "Active" })
    }
  }

  if (showMcp && effectiveConfig) {
    if (effectiveConfig.context_management_effective) {
      features.push({
        label: "Catalog Compression",
        value: "On",
        source: effectiveConfig.context_management_source,
      })
    }
    if (effectiveConfig.indexing_tools_effective) {
      features.push({
        label: "Indexing Tools",
        value: "On",
        source: effectiveConfig.indexing_tools_source,
      })
    }
    if (effectiveConfig.catalog_compression_effective) {
      features.push({
        label: "Deferred Loading",
        value: "On",
        source: effectiveConfig.catalog_compression_source,
      })
    }
  }

  const containerHeight = Math.max(120, graphBounds.height)

  return (
    <div className="space-y-4">
      {/* Connection Graph */}
      {loading ? (
        <Skeleton className="w-full h-[150px] rounded-lg" />
      ) : graphNodes.length > 0 ? (
        <div style={{ height: containerHeight }} className="w-full rounded-lg border overflow-hidden">
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
        </div>
      ) : null}

      {/* Features table */}
      {features.length > 0 && (
        <table className="w-full text-sm">
          <tbody>
            {features.map((f) => (
              <tr key={f.label} className="border-b last:border-b-0">
                <td className="py-1.5 pr-4 text-muted-foreground whitespace-nowrap">{f.label}</td>
                <td className="py-1.5 font-medium">{f.value}</td>
                {f.source && (
                  <td className="py-1.5 pl-2 text-[10px] text-muted-foreground whitespace-nowrap">
                    {f.source === "global" ? "inherited" : "override"}
                  </td>
                )}
              </tr>
            ))}
          </tbody>
        </table>
      )}
    </div>
  )
}
