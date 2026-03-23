/**
 * MCP via LLM diagram for the website.
 *
 * Reuses the actual ConnectionGraph node types and buildGraph layout engine
 * with static mock data to show the mcp_via_llm client mode.
 */

import { useMemo, useState } from 'react'
import ReactFlow, {
  ReactFlowProvider,
  Background,
  type NodeTypes,
  type Node,
  type Edge,
} from 'reactflow'
import 'reactflow/dist/style.css'
import { buildGraph } from '@app/components/connection-graph/utils/buildGraph'
import { AccessKeyNode } from '@app/components/connection-graph/nodes/AccessKeyNode'
import { ProviderNode } from '@app/components/connection-graph/nodes/ProviderNode'
import { McpServerNode } from '@app/components/connection-graph/nodes/McpServerNode'
import { SkillNode } from '@app/components/connection-graph/nodes/SkillNode'
import { CodingAgentNode } from '@app/components/connection-graph/nodes/CodingAgentNode'
import { MarketplaceNode } from '@app/components/connection-graph/nodes/MarketplaceNode'
import { EndpointNode } from '@app/components/connection-graph/nodes/EndpointNode'
import { RouterGroupNode } from '@app/components/connection-graph/nodes/RouterGroupNode'
import type { GraphNodeData } from '@app/components/connection-graph/types'
import type { ClientMode } from '@app/types/tauri-commands'

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

// Static mock data for the demo graph
const mockClient = {
  id: 'demo-openclaw',
  name: 'OpenClaw',
  client_id: 'openclaw',
  enabled: true,
  strategy_id: 'strategy-default',
  created_at: '2025-01-20T08:00:00Z',
  last_used: '2025-02-03T15:45:00Z',
  mcp_permissions: { global: 'allow' as const, servers: {}, tools: {}, resources: {}, prompts: {} },
  skills_permissions: { global: 'off' as const, skills: {}, tools: {} },
  coding_agent_permission: 'off' as const,
  coding_agent_type: null,
  model_permissions: { global: 'allow' as const, providers: {}, models: {} },
  marketplace_permission: 'off' as const,
}

const mockProviders = [
  { instance_name: 'OpenRouter', provider_type: 'openrouter', enabled: true },
  { instance_name: 'Ollama', provider_type: 'ollama', enabled: true },
]

const mockMcpServers = [
  { id: 'github', name: 'GitHub', enabled: true },
  { id: 'jira', name: 'Jira', enabled: true },
]

const mockHealthState = {
  server_running: true,
  aggregate_status: 'green' as const,
  providers: {
    'OpenRouter': { name: 'OpenRouter', status: 'healthy' as const, latency_ms: 95, last_checked: new Date().toISOString() },
    'Ollama': { name: 'Ollama', status: 'healthy' as const, latency_ms: 15, last_checked: new Date().toISOString() },
  },
  mcp_servers: {
    'github': { name: 'GitHub', status: 'healthy' as const, latency_ms: 80, last_checked: new Date().toISOString() },
    'jira': { name: 'Jira', status: 'healthy' as const, latency_ms: 110, last_checked: new Date().toISOString() },
  },
}

// Pretend the client is connected for animated edges
const mockActiveConnections = ['demo-openclaw']

const mockStrategies = [{
  id: 'strategy-default',
  allowed_models: { mode: 'all' as const, models: [] },
  auto_config: {
    prioritized_models: [
      ['OpenRouter', 'openrouter/anthropic/claude-sonnet-4-20250514'],
      ['Ollama', 'ollama/llama3'],
    ] as [string, string][],
    available_models: [] as [string, string][],
  },
}]

const modes: { value: ClientMode; label: string }[] = [
  { value: 'mcp_via_llm', label: 'MCP via LLM' },
  { value: 'both', label: 'MCP & LLM' },
]

export default function McpViaLlmDiagram() {
  const [mode, setMode] = useState<ClientMode>('mcp_via_llm')

  const { nodes, edges, bounds } = useMemo(() => {
    const result = buildGraph(
      [mockClient],
      mockProviders,
      mockMcpServers,
      [],      // no skills
      [],      // no coding agents
      mockHealthState,
      mockActiveConnections,
      mockStrategies,
      mode,
    )
    // Patch the client node to use the openclaw icon
    for (const node of result.nodes) {
      if (node.type === 'accessKey') {
        (node.data as any).iconUrl = '/icons/openclaw.png'
      }
    }
    // In mcp_via_llm mode, add "Injected" label on the LLM→MCP edge
    if (mode === 'mcp_via_llm') {
      for (const edge of result.edges) {
        if (edge.id === 'edge-llm-to-mcp') {
          edge.label = 'Injected'
          edge.labelStyle = { fill: '#8b5cf6', fontSize: 11, fontWeight: 600 }
          edge.labelBgStyle = { fill: '#1e1b4b', fillOpacity: 0.9 }
          edge.labelBgPadding = [4, 6] as [number, number]
          edge.labelBgBorderRadius = 4
        }
      }
    }
    return result
  }, [mode])

  const containerHeight = Math.max(150, bounds.height)

  return (
    <div className="w-full">
      <div style={{ height: containerHeight }}>
        <ReactFlowProvider>
          <ReactFlow
            key={mode}
            nodes={nodes as Node<GraphNodeData>[]}
            edges={edges as Edge[]}
            nodeTypes={nodeTypes}
            defaultViewport={{ x: 0, y: 0, zoom: 1 }}
            fitView
            fitViewOptions={{ padding: 0.15 }}
            minZoom={0.5}
            maxZoom={1.5}
            nodesDraggable={false}
            nodesConnectable={false}
            elementsSelectable={false}
            panOnDrag={false}
            zoomOnScroll={false}
            zoomOnPinch={false}
            zoomOnDoubleClick={false}
            preventScrolling={true}
            proOptions={{ hideAttribution: true }}
          >
            <Background color="#94a3b8" gap={16} size={1} />
          </ReactFlow>
        </ReactFlowProvider>
      </div>
      <div className="flex justify-center py-3 border-t border-border/50">
        <div className="inline-flex rounded-lg bg-muted p-0.5 text-sm">
          {modes.map(m => (
            <button
              key={m.value}
              onClick={() => setMode(m.value)}
              className={`rounded-md px-3 py-1.5 font-medium transition-colors ${
                mode === m.value
                  ? 'bg-background text-foreground shadow-sm'
                  : 'text-muted-foreground hover:text-foreground'
              }`}
            >
              {m.label}
            </button>
          ))}
        </div>
      </div>
    </div>
  )
}
