import dagre from 'dagre'
import type { Node, Edge } from 'reactflow'
import type {
  Client,
  Provider,
  McpServer,
  Skill,
  CodingAgent,
  GraphStrategy,
  HealthCacheState,
  GraphNode,
  GraphEdge,
  AccessKeyNodeData,
  ProviderNodeData,
  McpServerNodeData,
  SkillNodeData,
  CodingAgentNodeData,
  MarketplaceNodeData,
  EndpointNodeData,
  RouterGroupNodeData,
  ItemHealthStatus,
} from '../types'
import type { ClientMode } from '@/types/tauri-commands'

// Node dimensions for layout calculation
const NODE_WIDTH = 140
const NODE_HEIGHT = 36

// Get health status for a provider
function getProviderHealth(
  providerId: string,
  healthState: HealthCacheState | null
): ItemHealthStatus {
  if (!healthState) return 'pending'
  const health = healthState.providers[providerId]
  return health?.status ?? 'pending'
}

// Get health status for an MCP server
function getMcpServerHealth(
  serverId: string,
  healthState: HealthCacheState | null
): ItemHealthStatus {
  if (!healthState) return 'pending'
  const health = healthState.mcp_servers[serverId]
  return health?.status ?? 'pending'
}

// Build nodes from data, only including providers/servers/skills/agents that are connected to a client
function buildNodes(
  clients: Client[],
  providers: Provider[],
  mcpServers: McpServer[],
  skills: Skill[],
  codingAgents: CodingAgent[],
  healthState: HealthCacheState | null,
  activeConnections: string[],
  connectedTargetIds: Set<string>
): GraphNode[] {
  const nodes: GraphNode[] = []

  // Filter to only enabled items
  const enabledClients = clients.filter(c => c.enabled)
  const enabledProviders = providers.filter(p => p.enabled)
  const enabledMcpServers = mcpServers.filter(s => s.enabled)

  // Add Access Key nodes (left column)
  enabledClients.forEach((client) => {
    // Derive allowed providers from model_permissions (both 'allow' and 'ask' are enabled)
    const allowedProviders = client.model_permissions.global !== 'off'
      ? [] // Empty means all providers
      : Object.entries(client.model_permissions.providers ?? {})
          .filter(([, state]) => state !== 'off')
          .map(([name]) => name)

    // Derive MCP servers from mcp_permissions (both 'allow' and 'ask' are enabled)
    const mcpServers = client.mcp_permissions.global !== 'off'
      ? [] // Empty means all servers
      : Object.entries(client.mcp_permissions.servers ?? {})
          .filter(([, state]) => state !== 'off')
          .map(([name]) => name)

    const nodeData: AccessKeyNodeData = {
      id: client.id,
      name: client.name,
      type: 'accessKey',
      isConnected: activeConnections.includes(client.id),
      enabled: client.enabled,
      allowedProviders,
      mcpServers,
    }

    nodes.push({
      id: `client-${client.id}`,
      type: 'accessKey',
      data: nodeData,
      position: { x: 0, y: 0 }, // Will be set by dagre
    })
  })

  // Add Provider nodes — only those connected to at least one client
  enabledProviders.forEach((provider) => {
    if (!connectedTargetIds.has(`provider-${provider.instance_name}`)) return

    const nodeData: ProviderNodeData = {
      id: provider.instance_name,
      name: provider.instance_name,
      type: 'provider',
      providerType: provider.provider_type,
      healthStatus: getProviderHealth(provider.instance_name, healthState),
      enabled: provider.enabled,
    }

    nodes.push({
      id: `provider-${provider.instance_name}`,
      type: 'provider',
      data: nodeData,
      position: { x: 0, y: 0 },
    })
  })

  // Add MCP Server nodes — only those connected to at least one client
  enabledMcpServers.forEach((server) => {
    if (!connectedTargetIds.has(`mcp-${server.id}`)) return

    const nodeData: McpServerNodeData = {
      id: server.id,
      name: server.name,
      type: 'mcpServer',
      healthStatus: getMcpServerHealth(server.id, healthState),
      enabled: server.enabled,
    }

    nodes.push({
      id: `mcp-${server.id}`,
      type: 'mcpServer',
      data: nodeData,
      position: { x: 0, y: 0 },
    })
  })

  // Add Skill nodes — only those connected to at least one client
  skills.forEach((skill) => {
    if (!connectedTargetIds.has(`skill-${skill.name}`)) return

    const nodeData: SkillNodeData = {
      id: skill.name,
      name: skill.name,
      type: 'skill',
    }

    nodes.push({
      id: `skill-${skill.name}`,
      type: 'skill',
      data: nodeData,
      position: { x: 0, y: 0 },
    })
  })

  // Add Coding Agent nodes — only installed agents connected to at least one client
  codingAgents.forEach((agent) => {
    if (!connectedTargetIds.has(`coding-agent-${agent.agentType}`)) return

    const nodeData: CodingAgentNodeData = {
      id: agent.agentType,
      name: agent.displayName,
      type: 'codingAgent',
    }

    nodes.push({
      id: `coding-agent-${agent.agentType}`,
      type: 'codingAgent',
      data: nodeData,
      position: { x: 0, y: 0 },
    })
  })

  // Add Marketplace node if any client has marketplace permission enabled (both 'allow' and 'ask')
  const hasMarketplaceClient = enabledClients.some(c => c.marketplace_permission !== 'off')
  if (hasMarketplaceClient && connectedTargetIds.has('marketplace')) {
    const nodeData: MarketplaceNodeData = {
      id: 'marketplace',
      name: 'Marketplace',
      type: 'marketplace',
    }

    nodes.push({
      id: 'marketplace',
      type: 'marketplace',
      data: nodeData,
      position: { x: 0, y: 0 },
    })
  }

  return nodes
}

// Determine edge style based on permission state for a client's connection
function getPermissionEdgeStyle(
  client: Client,
  targetType: 'server' | 'skill',
  targetId: string,
  isConnected: boolean
): Record<string, string | number> {
  // Resolve permission state from the hierarchical permissions
  let state: string
  if (targetType === 'server') {
    state = client.mcp_permissions?.servers?.[targetId] ?? client.mcp_permissions?.global ?? 'off'
  } else {
    state = client.skills_permissions?.skills?.[targetId] ?? client.skills_permissions?.global ?? 'off'
  }

  if (state === 'off') {
    return {
      stroke: '#ef4444',
      strokeWidth: isConnected ? 2 : 1,
      strokeDasharray: '5,5',
    }
  }
  if (state === 'ask') {
    return {
      stroke: isConnected ? '#f59e0b' : '#64748b',
      strokeWidth: isConnected ? 2 : 1,
      strokeDasharray: '4,4',
    }
  }

  // Allow — standard style
  const defaultColor = targetType === 'server' ? '#10b981' : '#f59e0b'
  return {
    stroke: isConnected ? defaultColor : '#64748b',
    strokeWidth: isConnected ? 2 : 1,
  }
}

// Get the set of provider instance names a client's strategy actually routes to.
// Returns null if the strategy doesn't restrict providers (show all).
function getStrategyProviders(client: Client, strategies: GraphStrategy[]): Set<string> | null {
  const strategy = strategies.find(s => s.id === client.strategy_id)
  if (!strategy) return null

  const auto = strategy.auto_config
  if (!auto) return null

  // Collect all provider instance names from auto_config model tuples
  const providers = new Set<string>()
  for (const [providerId] of auto.prioritized_models ?? []) {
    providers.add(providerId)
  }
  for (const [providerId] of auto.available_models ?? []) {
    providers.add(providerId)
  }
  if (auto.routellm_config?.enabled) {
    for (const [providerId] of auto.routellm_config.weak_models ?? []) {
      providers.add(providerId)
    }
  }

  return providers.size > 0 ? providers : null
}

// Build edges from data
function buildEdges(
  clients: Client[],
  providers: Provider[],
  mcpServers: McpServer[],
  skills: Skill[],
  codingAgents: CodingAgent[],
  strategies: GraphStrategy[],
  activeConnections: string[]
): GraphEdge[] {
  const edges: GraphEdge[] = []

  // Filter to only enabled items
  const enabledClients = clients.filter(c => c.enabled)
  const enabledProviders = providers.filter(p => p.enabled)
  const enabledMcpServers = mcpServers.filter(s => s.enabled)

  const providerNames = new Set(enabledProviders.map(p => p.instance_name))
  const mcpServerIds = new Set(enabledMcpServers.map(s => s.id))
  const skillNames = new Set(skills.map(s => s.name))
  const codingAgentTypes = new Set(codingAgents.map(a => a.agentType))

  enabledClients.forEach((client) => {
    const isConnected = activeConnections.includes(client.id)

    // Determine which providers this client connects to:
    // 1. If model_permissions.global is 'off', use explicit provider permissions
    // 2. Otherwise, narrow by strategy's auto_config model list (if configured)
    // 3. Fall back to all enabled providers
    let clientProviders: string[]
    if (client.model_permissions.global === 'off') {
      clientProviders = Object.entries(client.model_permissions.providers ?? {})
        .filter(([name, state]) => state !== 'off' && providerNames.has(name))
        .map(([name]) => name)
    } else {
      const strategyProviders = getStrategyProviders(client, strategies)
      if (strategyProviders) {
        clientProviders = Array.from(strategyProviders).filter(p => providerNames.has(p))
      } else {
        clientProviders = Array.from(providerNames)
      }
    }

    clientProviders.forEach((providerId) => {
      edges.push({
        id: `edge-${client.id}-${providerId}`,
        source: `client-${client.id}`,
        target: `provider-${providerId}`,
        animated: isConnected,
        style: {
          stroke: isConnected ? '#3b82f6' : '#64748b',
          strokeWidth: isConnected ? 2 : 1,
        },
        data: { isActive: isConnected },
      })
    })

    // Create edges to MCP servers based on mcp_permissions
    // If global is not 'off', client has access to all servers
    // Otherwise, check specific server permissions (both 'allow' and 'ask' are enabled)
    const clientMcpServers = client.mcp_permissions.global !== 'off'
      ? Array.from(mcpServerIds)
      : Object.entries(client.mcp_permissions.servers ?? {})
          .filter(([id, state]) => state !== 'off' && mcpServerIds.has(id))
          .map(([id]) => id)

    clientMcpServers.forEach((serverId) => {
      const firewallStyle = getPermissionEdgeStyle(client, 'server', serverId, isConnected)
      edges.push({
        id: `edge-${client.id}-mcp-${serverId}`,
        source: `client-${client.id}`,
        target: `mcp-${serverId}`,
        animated: isConnected,
        style: firewallStyle,
        data: { isActive: isConnected },
      })
    })

    // Create edges to skills based on skills_permissions
    // If global is not 'off', client has access to all skills
    // Otherwise, check specific skill permissions (both 'allow' and 'ask' are enabled)
    const clientSkills = client.skills_permissions.global !== 'off'
      ? Array.from(skillNames)
      : Object.entries(client.skills_permissions.skills ?? {})
          .filter(([name, state]) => state !== 'off' && skillNames.has(name))
          .map(([name]) => name)

    clientSkills.forEach((skillName) => {
      const firewallStyle = getPermissionEdgeStyle(client, 'skill', skillName, isConnected)
      edges.push({
        id: `edge-${client.id}-skill-${skillName}`,
        source: `client-${client.id}`,
        target: `skill-${skillName}`,
        animated: isConnected,
        style: firewallStyle,
        data: { isActive: isConnected },
      })
    })

    // Create edge to coding agent if permission is not off and agent type is set
    if (client.coding_agent_permission !== 'off' && client.coding_agent_type && codingAgentTypes.has(client.coding_agent_type)) {
      const defaultColor = '#f97316' // orange for coding agents
      const style = client.coding_agent_permission === 'ask'
        ? { stroke: isConnected ? '#f59e0b' : '#64748b', strokeWidth: isConnected ? 2 : 1, strokeDasharray: '4,4' }
        : { stroke: isConnected ? defaultColor : '#64748b', strokeWidth: isConnected ? 2 : 1 }
      edges.push({
        id: `edge-${client.id}-coding-agent-${client.coding_agent_type}`,
        source: `client-${client.id}`,
        target: `coding-agent-${client.coding_agent_type}`,
        animated: isConnected,
        style,
        data: { isActive: isConnected },
      })
    }

    // Create edge to marketplace if client has marketplace permission enabled (both 'allow' and 'ask')
    if (client.marketplace_permission !== 'off') {
      edges.push({
        id: `edge-${client.id}-marketplace`,
        source: `client-${client.id}`,
        target: 'marketplace',
        animated: isConnected,
        style: {
          stroke: isConnected ? '#ec4899' : '#64748b', // Pink for marketplace
          strokeWidth: isConnected ? 2 : 1,
        },
        data: { isActive: isConnected },
      })
    }
  })

  return edges
}

// Apply dagre layout to nodes
function applyDagreLayout(nodes: Node[], edges: Edge[]): Node[] {
  if (nodes.length === 0) return nodes

  const g = new dagre.graphlib.Graph()
  g.setGraph({
    rankdir: 'LR', // Left to right
    nodesep: 25,
    ranksep: 80,
    marginx: 15,
    marginy: 15,
  })
  g.setDefaultEdgeLabel(() => ({}))

  // Add nodes to graph
  nodes.forEach((node) => {
    g.setNode(node.id, { width: NODE_WIDTH, height: NODE_HEIGHT })
  })

  // Add edges to graph
  edges.forEach((edge) => {
    g.setEdge(edge.source, edge.target)
  })

  // Run layout
  dagre.layout(g)

  // Apply positions to nodes
  return nodes.map((node) => {
    const nodeWithPosition = g.node(node.id)
    if (!nodeWithPosition) return node

    return {
      ...node,
      position: {
        x: nodeWithPosition.x - NODE_WIDTH / 2,
        y: nodeWithPosition.y - NODE_HEIGHT / 2,
      },
    }
  })
}

// Calculate bounding box of laid out nodes
function calculateBounds(nodes: Node[]): { width: number; height: number } {
  if (nodes.length === 0) return { width: 0, height: 0 }

  let maxX = -Infinity
  let maxY = -Infinity

  nodes.forEach((node) => {
    // Skip child nodes — their parent accounts for their space
    if ((node as any).parentNode) return
    const w = (typeof node.style?.width === 'number' ? node.style.width : null) ?? NODE_WIDTH
    const h = (typeof node.style?.height === 'number' ? node.style.height : null) ?? NODE_HEIGHT
    maxX = Math.max(maxX, node.position.x + w)
    maxY = Math.max(maxY, node.position.y + h)
  })

  // Add padding for controls and breathing room
  const padding = 40
  return {
    width: maxX + padding,
    height: maxY + padding,
  }
}

// MCP-related node types: mcpServer, skill, codingAgent, marketplace
const MCP_RELATED_TYPES = new Set(['mcpServer', 'skill', 'codingAgent', 'marketplace'])

// Apply client mode filtering and endpoint node insertion.
// All modes route edges through endpoint nodes.
// Coding agents and marketplace are MCP-related.
function applyClientMode(
  nodes: GraphNode[],
  edges: GraphEdge[],
  clientMode: ClientMode,
  clientId: string,
  isConnected: boolean,
): { nodes: GraphNode[]; edges: GraphEdge[] } {
  const clientNodeId = `client-${clientId}`

  switch (clientMode) {
    case 'llm_only': {
      // LLM endpoint only, remove all MCP-related nodes
      const newEdges: GraphEdge[] = []
      let hasLlm = false

      for (const edge of edges) {
        if (edge.source !== clientNodeId) { newEdges.push(edge); continue }
        const targetNode = nodes.find(n => n.id === edge.target)
        if (targetNode && MCP_RELATED_TYPES.has(targetNode.type!)) continue
        if (targetNode?.type === 'provider') {
          if (!hasLlm) {
            newEdges.push({
              id: `edge-${clientId}-endpoint-llm`,
              source: clientNodeId,
              target: 'endpoint-llm',
              animated: isConnected,
              style: { stroke: isConnected ? '#3b82f6' : '#64748b', strokeWidth: isConnected ? 2 : 1 },
              data: { isActive: isConnected },
            })
            hasLlm = true
          }
          newEdges.push({ ...edge, id: `edge-ep-llm-${edge.target}`, source: 'endpoint-llm' })
        } else {
          newEdges.push(edge)
        }
      }

      const newNodes = nodes.filter(n => !MCP_RELATED_TYPES.has(n.type!))
      if (hasLlm) {
        newNodes.push({
          id: 'endpoint-llm',
          type: 'endpoint',
          data: { id: 'endpoint-llm', name: 'LLM', type: 'endpoint', variant: 'llm' } as EndpointNodeData,
          position: { x: 0, y: 0 },
        })
      }

      return { nodes: newNodes, edges: newEdges }
    }
    case 'mcp_only': {
      // MCP endpoint only, remove all providers
      const newEdges: GraphEdge[] = []
      let hasMcp = false

      for (const edge of edges) {
        if (edge.source !== clientNodeId) { newEdges.push(edge); continue }
        const targetNode = nodes.find(n => n.id === edge.target)
        if (targetNode?.type === 'provider') continue
        if (targetNode && MCP_RELATED_TYPES.has(targetNode.type!)) {
          if (!hasMcp) {
            newEdges.push({
              id: `edge-${clientId}-endpoint-mcp`,
              source: clientNodeId,
              target: 'endpoint-mcp',
              animated: isConnected,
              style: { stroke: isConnected ? '#10b981' : '#64748b', strokeWidth: isConnected ? 2 : 1 },
              data: { isActive: isConnected },
            })
            hasMcp = true
          }
          newEdges.push({ ...edge, id: `edge-ep-mcp-${edge.target}`, source: 'endpoint-mcp' })
        } else {
          newEdges.push(edge)
        }
      }

      const newNodes = nodes.filter(n => n.type !== 'provider')
      if (hasMcp) {
        newNodes.push({
          id: 'endpoint-mcp',
          type: 'endpoint',
          data: { id: 'endpoint-mcp', name: 'MCP', type: 'endpoint', variant: 'mcp' } as EndpointNodeData,
          position: { x: 0, y: 0 },
        })
      }

      return { nodes: newNodes, edges: newEdges }
    }
    case 'both': {
      // Two endpoints: LLM for providers, MCP for MCP-related
      const newEdges: GraphEdge[] = []
      let hasLlm = false
      let hasMcp = false

      for (const edge of edges) {
        if (edge.source !== clientNodeId) { newEdges.push(edge); continue }
        const targetNode = nodes.find(n => n.id === edge.target)
        if (targetNode?.type === 'provider') {
          if (!hasLlm) {
            newEdges.push({
              id: `edge-${clientId}-endpoint-llm`,
              source: clientNodeId,
              target: 'endpoint-llm',
              animated: isConnected,
              style: { stroke: isConnected ? '#3b82f6' : '#64748b', strokeWidth: isConnected ? 2 : 1 },
              data: { isActive: isConnected },
            })
            hasLlm = true
          }
          newEdges.push({ ...edge, id: `edge-ep-llm-${edge.target}`, source: 'endpoint-llm' })
        } else if (targetNode && MCP_RELATED_TYPES.has(targetNode.type!)) {
          if (!hasMcp) {
            newEdges.push({
              id: `edge-${clientId}-endpoint-mcp`,
              source: clientNodeId,
              target: 'endpoint-mcp',
              animated: isConnected,
              style: { stroke: isConnected ? '#10b981' : '#64748b', strokeWidth: isConnected ? 2 : 1 },
              data: { isActive: isConnected },
            })
            hasMcp = true
          }
          newEdges.push({ ...edge, id: `edge-ep-mcp-${edge.target}`, source: 'endpoint-mcp' })
        } else {
          newEdges.push(edge)
        }
      }

      const newNodes = [...nodes]
      if (hasLlm) {
        newNodes.push({
          id: 'endpoint-llm',
          type: 'endpoint',
          data: { id: 'endpoint-llm', name: 'LLM', type: 'endpoint', variant: 'llm' } as EndpointNodeData,
          position: { x: 0, y: 0 },
        })
      }
      if (hasMcp) {
        newNodes.push({
          id: 'endpoint-mcp',
          type: 'endpoint',
          data: { id: 'endpoint-mcp', name: 'MCP', type: 'endpoint', variant: 'mcp' } as EndpointNodeData,
          position: { x: 0, y: 0 },
        })
      }

      return { nodes: newNodes, edges: newEdges }
    }
    case 'mcp_via_llm': {
      // Two endpoints like 'both', but client→LLM only, LLM→MCP visual link.
      // A phantom client→MCP edge is used for dagre layout (same rank) but not rendered.
      // The visual LLM→MCP edge is excluded from dagre to avoid rank shift.
      const newEdges: GraphEdge[] = []
      let hasLlm = false
      let hasMcp = false

      for (const edge of edges) {
        if (edge.source !== clientNodeId) { newEdges.push(edge); continue }
        const targetNode = nodes.find(n => n.id === edge.target)
        if (targetNode?.type === 'provider') {
          if (!hasLlm) {
            newEdges.push({
              id: `edge-${clientId}-endpoint-llm`,
              source: clientNodeId,
              target: 'endpoint-llm',
              animated: isConnected,
              style: { stroke: isConnected ? '#3b82f6' : '#64748b', strokeWidth: isConnected ? 2 : 1 },
              data: { isActive: isConnected },
            })
            hasLlm = true
          }
          newEdges.push({ ...edge, id: `edge-ep-llm-${edge.target}`, source: 'endpoint-llm' })
        } else if (targetNode && MCP_RELATED_TYPES.has(targetNode.type!)) {
          if (!hasMcp) {
            // Phantom edge: keeps MCP at same dagre rank as LLM, filtered before render
            newEdges.push({
              id: `edge-${clientId}-endpoint-mcp-phantom`,
              source: clientNodeId,
              target: 'endpoint-mcp',
              data: { phantom: true },
            })
            hasMcp = true
          }
          newEdges.push({ ...edge, id: `edge-ep-mcp-${edge.target}`, source: 'endpoint-mcp' })
        } else {
          newEdges.push(edge)
        }
      }

      // Visual-only LLM→MCP edge (excluded from dagre, added after layout)
      if (hasLlm && hasMcp) {
        newEdges.push({
          id: 'edge-llm-to-mcp',
          source: 'endpoint-llm',
          target: 'endpoint-mcp',
          animated: isConnected,
          style: { stroke: isConnected ? '#8b5cf6' : '#94a3b8', strokeWidth: isConnected ? 2 : 1, strokeDasharray: '4,3' },
          data: { isActive: isConnected, visualOnly: true },
        })
      }

      const newNodes = [...nodes]
      if (hasLlm) {
        newNodes.push({
          id: 'endpoint-llm',
          type: 'endpoint',
          data: { id: 'endpoint-llm', name: 'LLM', type: 'endpoint', variant: 'llm' } as EndpointNodeData,
          position: { x: 0, y: 0 },
        })
      }
      if (hasMcp) {
        newNodes.push({
          id: 'endpoint-mcp',
          type: 'endpoint',
          data: { id: 'endpoint-mcp', name: 'MCP', type: 'endpoint', variant: 'mcp' } as EndpointNodeData,
          position: { x: 0, y: 0 },
        })
      }

      return { nodes: newNodes, edges: newEdges }
    }
    default:
      return { nodes, edges }
  }
}

// Wrap endpoint nodes in a LocalRouter group node after dagre layout
function wrapEndpointsInGroup(nodes: GraphNode[]): GraphNode[] {
  const endpointNodes = nodes.filter(n => n.type === 'endpoint')
  if (endpointNodes.length === 0) return nodes

  const paddingX = 24
  const paddingY = 14
  const labelHeight = 22
  // Endpoint nodes are visually narrow; use half of dagre's NODE_WIDTH for tighter group sizing
  const endpointVisualWidth = NODE_WIDTH / 2

  let minX = Infinity, minY = Infinity, maxX = -Infinity, maxY = -Infinity
  for (const node of endpointNodes) {
    minX = Math.min(minX, node.position.x)
    minY = Math.min(minY, node.position.y)
    maxX = Math.max(maxX, node.position.x + endpointVisualWidth)
    maxY = Math.max(maxY, node.position.y + NODE_HEIGHT)
  }

  const groupX = minX - paddingX
  const groupY = minY - paddingY - labelHeight
  const groupWidth = maxX - minX + 2 * paddingX
  const groupHeight = maxY - minY + 2 * paddingY + labelHeight

  const groupNode: GraphNode = {
    id: 'localrouter-group',
    type: 'routerGroup',
    data: { id: 'localrouter-group', name: 'LocalRouter', type: 'routerGroup' } as RouterGroupNodeData,
    position: { x: groupX, y: groupY },
    style: { width: groupWidth, height: groupHeight },
  }

  // Group node must come before children in the array
  const result: GraphNode[] = [groupNode]
  for (const node of nodes) {
    if (node.type === 'endpoint') {
      result.push({
        ...node,
        position: {
          x: node.position.x - groupX,
          y: node.position.y - groupY,
        },
        parentNode: 'localrouter-group',
        extent: 'parent' as const,
      } as GraphNode)
    } else {
      result.push(node)
    }
  }

  return result
}

// Ensure no root node has a negative position (group nodes can extend left/above dagre margins)
function normalizePositions(nodes: GraphNode[]): GraphNode[] {
  const margin = 10
  let minX = Infinity, minY = Infinity
  for (const node of nodes) {
    if ((node as any).parentNode) continue
    minX = Math.min(minX, node.position.x)
    minY = Math.min(minY, node.position.y)
  }

  const offsetX = minX < margin ? margin - minX : 0
  const offsetY = minY < margin ? margin - minY : 0
  if (offsetX === 0 && offsetY === 0) return nodes

  return nodes.map(node => {
    if ((node as any).parentNode) return node
    return { ...node, position: { x: node.position.x + offsetX, y: node.position.y + offsetY } }
  })
}

// Main function to build the complete graph
export function buildGraph(
  clients: Client[],
  providers: Provider[],
  mcpServers: McpServer[],
  skills: Skill[],
  codingAgents: CodingAgent[],
  healthState: HealthCacheState | null,
  activeConnections: string[],
  strategies: GraphStrategy[] = [],
  clientMode?: ClientMode,
): { nodes: GraphNode[]; edges: GraphEdge[]; bounds: { width: number; height: number } } {
  // Build edges first to determine which targets are connected to clients
  const allEdges = buildEdges(clients, providers, mcpServers, skills, codingAgents, strategies, activeConnections)
  const connectedTargetIds = new Set(allEdges.map(e => e.target))
  const allNodes = buildNodes(clients, providers, mcpServers, skills, codingAgents, healthState, activeConnections, connectedTargetIds)

  // Apply client mode transformation (filtering + endpoint nodes) for single-client views
  let finalNodes = allNodes
  let finalEdges = allEdges
  if (clientMode && clients.length === 1) {
    const isConnected = activeConnections.includes(clients[0].id)
    const result = applyClientMode(allNodes, allEdges, clientMode, clients[0].id, isConnected)
    finalNodes = result.nodes
    finalEdges = result.edges
  }

  // Separate visual-only edges (excluded from dagre) and phantom edges (dagre-only, not rendered)
  const visualOnlyEdges = finalEdges.filter(e => (e.data as any)?.visualOnly)
  const layoutEdges = finalEdges.filter(e => !(e.data as any)?.visualOnly)
  const renderedEdges = layoutEdges
    .filter(e => !(e.data as any)?.phantom)
    .concat(visualOnlyEdges)

  const layoutedNodes = applyDagreLayout(finalNodes, layoutEdges)
  const wrappedNodes = normalizePositions(wrapEndpointsInGroup(layoutedNodes))
  const bounds = calculateBounds(wrappedNodes)

  return {
    nodes: wrappedNodes as GraphNode[],
    edges: renderedEdges,
    bounds,
  }
}
