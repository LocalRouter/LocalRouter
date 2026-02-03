import { Node, Edge } from 'reactflow'

// Health status from backend
export type ItemHealthStatus = 'healthy' | 'degraded' | 'unhealthy' | 'ready' | 'pending' | 'disabled'

// Graph node types
export type GraphNodeType = 'accessKey' | 'provider' | 'mcpServer' | 'skill'

// Base node data shared by all node types
export interface BaseNodeData {
  id: string
  name: string
  type: GraphNodeType
}

// Access Key (Client) node data
export interface AccessKeyNodeData extends BaseNodeData {
  type: 'accessKey'
  isConnected: boolean
  enabled: boolean
  allowedProviders: string[]
  mcpServers: string[]
}

// Provider node data
export interface ProviderNodeData extends BaseNodeData {
  type: 'provider'
  providerType: string
  healthStatus: ItemHealthStatus
  enabled: boolean
}

// MCP Server node data
export interface McpServerNodeData extends BaseNodeData {
  type: 'mcpServer'
  healthStatus: ItemHealthStatus
  enabled: boolean
}

// Skill node data
export interface SkillNodeData extends BaseNodeData {
  type: 'skill'
}

// Union type for all node data
export type GraphNodeData = AccessKeyNodeData | ProviderNodeData | McpServerNodeData | SkillNodeData

// Typed nodes
export type AccessKeyNode = Node<AccessKeyNodeData, 'accessKey'>
export type ProviderNode = Node<ProviderNodeData, 'provider'>
export type McpServerNode = Node<McpServerNodeData, 'mcpServer'>
export type SkillNode = Node<SkillNodeData, 'skill'>
export type GraphNode = Node<GraphNodeData>

// Edge type (use React Flow's Edge type directly)
export type GraphEdge = Edge<{ isActive?: boolean }>

// Client info from backend
export interface Client {
  id: string
  name: string
  client_id: string
  enabled: boolean
  strategy_id: string
  allowed_llm_providers: string[]
  mcp_access_mode: 'none' | 'all' | 'specific'
  mcp_servers: string[]
  skills_access_mode: 'none' | 'all' | 'specific'
  skills_names: string[]
  firewall?: {
    default_policy?: 'allow' | 'ask' | 'deny'
    server_rules?: Record<string, 'allow' | 'ask' | 'deny'>
    tool_rules?: Record<string, 'allow' | 'ask' | 'deny'>
    skill_rules?: Record<string, 'allow' | 'ask' | 'deny'>
    skill_tool_rules?: Record<string, 'allow' | 'ask' | 'deny'>
  }
}

// Provider info from backend
export interface Provider {
  instance_name: string
  provider_type: string
  enabled: boolean
}

// MCP Server info from backend
export interface McpServer {
  id: string
  name: string
  enabled: boolean
}

// Skill info from backend
export interface Skill {
  name: string
}

// Health cache state from backend
export interface ItemHealth {
  name: string
  status: ItemHealthStatus
  latency_ms?: number
  error?: string
  last_checked: string
}

export interface HealthCacheState {
  server_running: boolean
  server_host?: string
  server_port?: number
  providers: Record<string, ItemHealth>
  mcp_servers: Record<string, ItemHealth>
  last_refresh?: string
  aggregate_status: 'red' | 'yellow' | 'green'
}

// Hook return type
export interface UseGraphDataResult {
  clients: Client[]
  providers: Provider[]
  mcpServers: McpServer[]
  skills: Skill[]
  healthState: HealthCacheState | null
  activeConnections: string[]
  loading: boolean
  error: string | null
}
