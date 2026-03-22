import { Node, Edge } from 'reactflow'
import type { McpPermissions, SkillsPermissions, ModelPermissions, PermissionState } from '@/components/permissions'
import type { CodingAgentType } from '@/types/tauri-commands'

// Health status from backend
export type ItemHealthStatus = 'healthy' | 'degraded' | 'unhealthy' | 'ready' | 'pending' | 'disabled'

// Graph node types
export type GraphNodeType = 'accessKey' | 'provider' | 'mcpServer' | 'skill' | 'marketplace' | 'codingAgent' | 'endpoint' | 'routerGroup'

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
  /** Optional icon URL to show instead of the default Key icon */
  iconUrl?: string
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

// Coding agent node data
export interface CodingAgentNodeData extends BaseNodeData {
  type: 'codingAgent'
}

// Marketplace node data
export interface MarketplaceNodeData extends BaseNodeData {
  type: 'marketplace'
}

// Endpoint node data (intermediary routing node)
export interface EndpointNodeData extends BaseNodeData {
  type: 'endpoint'
  variant: 'llm' | 'mcp'
}

// Router group node data (container for endpoint nodes)
export interface RouterGroupNodeData extends BaseNodeData {
  type: 'routerGroup'
}

// Union type for all node data
export type GraphNodeData = AccessKeyNodeData | ProviderNodeData | McpServerNodeData | SkillNodeData | CodingAgentNodeData | MarketplaceNodeData | EndpointNodeData | RouterGroupNodeData

// Typed nodes
export type AccessKeyNode = Node<AccessKeyNodeData, 'accessKey'>
export type ProviderNode = Node<ProviderNodeData, 'provider'>
export type McpServerNode = Node<McpServerNodeData, 'mcpServer'>
export type SkillNode = Node<SkillNodeData, 'skill'>
export type CodingAgentNode = Node<CodingAgentNodeData, 'codingAgent'>
export type MarketplaceNode = Node<MarketplaceNodeData, 'marketplace'>
export type EndpointNode = Node<EndpointNodeData, 'endpoint'>
export type GraphNode = Node<GraphNodeData>

// Edge type (use React Flow's Edge type directly)
export type GraphEdge = Edge<{ isActive?: boolean; phantom?: boolean; visualOnly?: boolean }>

// Client info from backend
export interface Client {
  id: string
  name: string
  client_id: string
  enabled: boolean
  strategy_id: string
  created_at: string
  last_used: string | null
  mcp_permissions: McpPermissions
  skills_permissions: SkillsPermissions
  coding_agent_permission: PermissionState
  coding_agent_type: CodingAgentType | null
  model_permissions: ModelPermissions
  marketplace_permission: PermissionState
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

// Coding agent info from backend
export interface CodingAgent {
  agentType: string
  displayName: string
  installed: boolean
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

// Strategy info for graph filtering
export interface GraphStrategy {
  id: string
  allowed_models: { mode: 'all' | 'selected'; models: string[] }
  auto_config?: {
    prioritized_models: [string, string][]
    available_models: [string, string][]
    routellm_config?: { enabled: boolean; weak_models: [string, string][] } | null
  } | null
}

// Hook return type
export interface UseGraphDataResult {
  clients: Client[]
  providers: Provider[]
  mcpServers: McpServer[]
  skills: Skill[]
  codingAgents: CodingAgent[]
  strategies: GraphStrategy[]
  healthState: HealthCacheState | null
  activeConnections: string[]
  loading: boolean
  error: string | null
}
