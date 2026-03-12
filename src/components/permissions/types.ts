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
  /** Disable permission toggling for this node (e.g. non-indexable tools) */
  disabled?: boolean
}

export interface PermissionTreeProps {
  clientId: string
  onUpdate: () => void
}
