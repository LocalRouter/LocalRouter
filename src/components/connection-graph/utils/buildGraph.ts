import dagre from 'dagre'
import type { Node, Edge } from 'reactflow'
import type {
  Client,
  Provider,
  McpServer,
  Skill,
  HealthCacheState,
  GraphNode,
  GraphEdge,
  AccessKeyNodeData,
  ProviderNodeData,
  McpServerNodeData,
  SkillNodeData,
  MarketplaceNodeData,
  ItemHealthStatus,
} from '../types'

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

// Build nodes from data, only including providers/servers/skills that are connected to a client
function buildNodes(
  clients: Client[],
  providers: Provider[],
  mcpServers: McpServer[],
  skills: Skill[],
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

// Build edges from data
function buildEdges(
  clients: Client[],
  providers: Provider[],
  mcpServers: McpServer[],
  skills: Skill[],
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

  enabledClients.forEach((client) => {
    const isConnected = activeConnections.includes(client.id)

    // Create edges to providers based on model_permissions
    // If global is not 'off', client has access to all providers
    // Otherwise, check specific provider permissions (both 'allow' and 'ask' are enabled)
    const clientProviders = client.model_permissions.global !== 'off'
      ? Array.from(providerNames)
      : Object.entries(client.model_permissions.providers ?? {})
          .filter(([name, state]) => state !== 'off' && providerNames.has(name))
          .map(([name]) => name)

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
    maxX = Math.max(maxX, node.position.x + NODE_WIDTH)
    maxY = Math.max(maxY, node.position.y + NODE_HEIGHT)
  })

  // Add padding for controls and breathing room
  const padding = 40
  return {
    width: maxX + padding,
    height: maxY + padding,
  }
}

// Main function to build the complete graph
export function buildGraph(
  clients: Client[],
  providers: Provider[],
  mcpServers: McpServer[],
  skills: Skill[],
  healthState: HealthCacheState | null,
  activeConnections: string[]
): { nodes: GraphNode[]; edges: GraphEdge[]; bounds: { width: number; height: number } } {
  // Build edges first to determine which targets are connected to clients
  const edges = buildEdges(clients, providers, mcpServers, skills, activeConnections)
  const connectedTargetIds = new Set(edges.map(e => e.target))
  const nodes = buildNodes(clients, providers, mcpServers, skills, healthState, activeConnections, connectedTargetIds)
  const layoutedNodes = applyDagreLayout(nodes, edges)
  const bounds = calculateBounds(layoutedNodes)

  return {
    nodes: layoutedNodes as GraphNode[],
    edges,
    bounds,
  }
}
