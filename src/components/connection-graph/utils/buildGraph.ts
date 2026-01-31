import dagre from 'dagre'
import type { Node, Edge } from 'reactflow'
import type {
  Client,
  Provider,
  McpServer,
  HealthCacheState,
  GraphNode,
  GraphEdge,
  AccessKeyNodeData,
  ProviderNodeData,
  McpServerNodeData,
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

// Build nodes from data
function buildNodes(
  clients: Client[],
  providers: Provider[],
  mcpServers: McpServer[],
  healthState: HealthCacheState | null,
  activeConnections: string[]
): GraphNode[] {
  const nodes: GraphNode[] = []

  // Filter to only enabled items
  const enabledClients = clients.filter(c => c.enabled)
  const enabledProviders = providers.filter(p => p.enabled)
  const enabledMcpServers = mcpServers.filter(s => s.enabled)

  // Add Access Key nodes (left column)
  enabledClients.forEach((client) => {
    const nodeData: AccessKeyNodeData = {
      id: client.id,
      name: client.name,
      type: 'accessKey',
      isConnected: activeConnections.includes(client.id),
      enabled: client.enabled,
      allowedProviders: client.allowed_llm_providers,
      mcpServers: client.mcp_servers,
    }

    nodes.push({
      id: `client-${client.id}`,
      type: 'accessKey',
      data: nodeData,
      position: { x: 0, y: 0 }, // Will be set by dagre
    })
  })

  // Add Provider nodes (right-top)
  enabledProviders.forEach((provider) => {
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

  // Add MCP Server nodes (right-bottom)
  enabledMcpServers.forEach((server) => {
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

  return nodes
}

// Build edges from data
function buildEdges(
  clients: Client[],
  providers: Provider[],
  mcpServers: McpServer[],
  activeConnections: string[]
): GraphEdge[] {
  const edges: GraphEdge[] = []

  // Filter to only enabled items
  const enabledClients = clients.filter(c => c.enabled)
  const enabledProviders = providers.filter(p => p.enabled)
  const enabledMcpServers = mcpServers.filter(s => s.enabled)

  const providerNames = new Set(enabledProviders.map(p => p.instance_name))
  const mcpServerIds = new Set(enabledMcpServers.map(s => s.id))

  enabledClients.forEach((client) => {
    const isConnected = activeConnections.includes(client.id)

    // Create edges to providers
    // If allowedProviders is empty, client has access to all providers
    const clientProviders = client.allowed_llm_providers.length > 0
      ? client.allowed_llm_providers.filter(p => providerNames.has(p))
      : Array.from(providerNames)

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

    // Create edges to MCP servers
    const clientMcpServers = client.mcp_access_mode === 'all'
      ? Array.from(mcpServerIds)
      : client.mcp_access_mode === 'specific'
        ? client.mcp_servers.filter(s => mcpServerIds.has(s))
        : []

    clientMcpServers.forEach((serverId) => {
      edges.push({
        id: `edge-${client.id}-mcp-${serverId}`,
        source: `client-${client.id}`,
        target: `mcp-${serverId}`,
        animated: isConnected,
        style: {
          stroke: isConnected ? '#10b981' : '#64748b',
          strokeWidth: isConnected ? 2 : 1,
        },
        data: { isActive: isConnected },
      })
    })
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
  healthState: HealthCacheState | null,
  activeConnections: string[]
): { nodes: GraphNode[]; edges: GraphEdge[]; bounds: { width: number; height: number } } {
  const nodes = buildNodes(clients, providers, mcpServers, healthState, activeConnections)
  const edges = buildEdges(clients, providers, mcpServers, activeConnections)
  const layoutedNodes = applyDagreLayout(nodes, edges)
  const bounds = calculateBounds(layoutedNodes)

  return {
    nodes: layoutedNodes as GraphNode[],
    edges,
    bounds,
  }
}
