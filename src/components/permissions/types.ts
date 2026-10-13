export type PermissionState = "allow" | "ask" | "off"

export interface McpPermissions {
  global: PermissionState
  servers: Record<string, PermissionState>
  tools: Record<string, PermissionState>
  resources: Record<string, PermissionState>
  prompts: Record<string, PermissionState>
}

export interface SkillsPermissions {
  global: PermissionState
  /** Per-source overrides keyed by the configured source path (omitted when empty) */
  sources?: Record<string, PermissionState>
  skills: Record<string, PermissionState>
  tools: Record<string, PermissionState>
}

export interface CodingAgentsPermissions {
  global: PermissionState
  agents: Record<string, PermissionState>
}

export interface ModelPermissions {
  global: PermissionState
  providers: Record<string, PermissionState>
  models: Record<string, PermissionState>
}

export interface TreeNode {
  id: string
  label: string
  description?: string
  children?: TreeNode[]
  isGroup?: boolean
  depth?: number
  /** Show a loading indicator for this node (e.g. capabilities still loading) */
  loading?: boolean
  /** Error shown under the node (e.g. capabilities failed to load) */
  error?: string
  /** Disable permission toggling for this node (e.g. non-indexable tools) */
  disabled?: boolean
  /** Extra text matched by tree search (e.g. a path or tags), not displayed */
  searchText?: string
}

export interface PermissionTreeProps {
  clientId: string
  onUpdate: () => void
}
