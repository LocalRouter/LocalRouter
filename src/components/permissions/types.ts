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
}

export interface PermissionTreeProps {
  clientId: string
  onUpdate: () => void
}
