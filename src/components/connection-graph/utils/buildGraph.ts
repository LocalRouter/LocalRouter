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
  skills: Skill[],
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

  // Add Skill nodes
  skills.forEach((skill) => {
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

  return nodes
}

// Determine edge style based on firewall rules for a client's connection
function getFirewallEdgeStyle(
  client: Client,
  targetType: 'server' | 'skill',
  targetId: string,
  isConnected: boolean
): Record<string, string | number> {
  const fw = client.firewall
  if (!fw || !fw.default_policy) {
    // No firewall rules — use default colors
    const defaultColor = targetType === 'server' ? '#10b981' : '#f59e0b'
    return {
      stroke: isConnected ? defaultColor : '#64748b',
      strokeWidth: isConnected ? 2 : 1,
    }
  }

  // Check for server/skill-level rule
  const rules = targetType === 'server' ? fw.server_rules : fw.skill_rules
  const policy = rules[targetId] ?? fw.default_policy

  if (policy === 'deny') {
    return {
      stroke: '#ef4444',
      strokeWidth: isConnected ? 2 : 1,
      strokeDasharray: '5,5',
    }
  }
  if (policy === 'ask') {
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
      const firewallStyle = getFirewallEdgeStyle(client, 'server', serverId, isConnected)
      edges.push({
        id: `edge-${client.id}-mcp-${serverId}`,
        source: `client-${client.id}`,
        target: `mcp-${serverId}`,
        animated: isConnected,
        style: firewallStyle,
        data: { isActive: isConnected },
      })
    })

    // Create edges to skills
    const clientSkills = client.skills_access_mode === 'all'
      ? Array.from(skillNames)
      : client.skills_access_mode === 'specific'
        ? client.skills_names.filter(s => skillNames.has(s))
        : []

    clientSkills.forEach((skillName) => {
      const firewallStyle = getFirewallEdgeStyle(client, 'skill', skillName, isConnected)
      edges.push({
        id: `edge-${client.id}-skill-${skillName}`,
        source: `client-${client.id}`,
        target: `skill-${skillName}`,
        animated: isConnected,
        style: firewallStyle,
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
  skills: Skill[],
  healthState: HealthCacheState | null,
  activeConnections: string[]
): { nodes: GraphNode[]; edges: GraphEdge[]; bounds: { width: number; height: number } } {
  const nodes = buildNodes(clients, providers, mcpServers, skills, healthState, activeConnections)
  const edges = buildEdges(clients, providers, mcpServers, skills, activeConnections)
  const layoutedNodes = applyDagreLayout(nodes, edges)
  const bounds = calculateBounds(layoutedNodes)

  return {
    nodes: layoutedNodes as GraphNode[],
    edges,
    bounds,
  }
}
