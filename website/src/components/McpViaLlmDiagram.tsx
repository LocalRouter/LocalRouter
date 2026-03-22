/**
 * MCP via LLM diagram for the website.
 *
 * Reuses the actual ConnectionGraph node types and buildGraph layout engine
 * with static mock data to show the mcp_via_llm client mode.
 */

import { useMemo } from 'react'
import ReactFlow, {
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

export default function McpViaLlmDiagram() {
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
      'mcp_via_llm',
    )
    // Patch the client node to use the openclaw icon
    for (const node of result.nodes) {
      if (node.type === 'accessKey') {
        (node.data as any).iconUrl = '/icons/openclaw.png'
      }
    }
    return result
  }, [])

  const containerHeight = Math.max(150, bounds.height)

  return (
    <div style={{ height: containerHeight }} className="w-full">
      <ReactFlow
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
    </div>
  )
}
