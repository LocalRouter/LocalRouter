import { useState, useEffect, useCallback } from "react"
import { invoke } from "@tauri-apps/api/core"
import { toast } from "sonner"
import { PermissionTreeSelector } from "./PermissionTreeSelector"
import { IndexingStateButton } from "./IndexingStateButton"
import type { TreeNode } from "./types"
import type {
  IndexingState,
  GatewayIndexingPermissions,
  VirtualMcpIndexingInfo,
} from "@/types/tauri-commands"

interface VirtualMcpIndexingTreeProps {
  permissions: GatewayIndexingPermissions
  onUpdate: () => void
}

export function VirtualMcpIndexingTree({ permissions, onUpdate }: VirtualMcpIndexingTreeProps) {
  const [servers, setServers] = useState<VirtualMcpIndexingInfo[]>([])
  const [loading, setLoading] = useState(true)
  const [saving, setSaving] = useState(false)

  const loadServers = useCallback(async () => {
    try {
      const infos = await invoke<VirtualMcpIndexingInfo[]>("list_virtual_mcp_indexing_info")
      setServers(infos)
    } catch (error) {
      console.error("Failed to load virtual MCP indexing info:", error)
    } finally {
      setLoading(false)
    }
  }, [])

  useEffect(() => {
    loadServers()
  }, [loadServers])

  // Build tree nodes: server → tools
  // Use virtual server IDs as keys (e.g., _context_mode, _skills)
  const nodes: TreeNode[] = servers.map((server) => {
    const allNonIndexable = server.tools.every((t) => !t.indexable)

    const toolChildren: TreeNode[] = server.tools.map((tool) => ({
      id: `${server.id}__${tool.name}`,
      label: tool.name,
      disabled: !tool.indexable,
    }))

    return {
      id: server.id,
      label: server.display_name,
      children: toolChildren.length > 0 ? toolChildren : undefined,
      disabled: allNonIndexable,
    }
  })

  // Build flat permissions map
  // Force non-indexable tools and fully-non-indexable servers to "disable"
  const flatPermissions: Record<string, IndexingState> = {}
  for (const [key, value] of Object.entries(permissions.servers ?? {})) {
    flatPermissions[key] = value
  }
  for (const [key, value] of Object.entries(permissions.tools ?? {})) {
    flatPermissions[key] = value
  }
  // Force non-indexable tools to disable
  for (const server of servers) {
    const allNonIndexable = server.tools.every((t) => !t.indexable)
    if (allNonIndexable) {
      flatPermissions[server.id] = "disable"
    }
    for (const tool of server.tools) {
      if (!tool.indexable) {
        flatPermissions[`${server.id}__${tool.name}`] = "disable"
      }
    }
  }

  const handlePermissionChange = async (key: string, state: IndexingState, parentState: IndexingState) => {
    setSaving(true)
    try {
      const shouldClear = state === parentState
      const isToolKey = key.includes("__")

      const params = shouldClear
        ? { level: isToolKey ? "tool_clear" : "server_clear", key, state: "enable" }
        : { level: isToolKey ? "tool" : "server", key, state }

      await invoke("set_virtual_indexing_permission", params)
      onUpdate()
    } catch (error) {
      toast.error(`Failed to update: ${error}`)
    } finally {
      setSaving(false)
    }
  }

  const handleGlobalChange = async (state: IndexingState) => {
    setSaving(true)
    try {
      await invoke("set_virtual_indexing_permission", { level: "global", state })
      onUpdate()
    } catch (error) {
      toast.error(`Failed to update: ${error}`)
    } finally {
      setSaving(false)
    }
  }

  return (
    <PermissionTreeSelector<IndexingState>
      nodes={nodes}
      permissions={flatPermissions}
      globalPermission={permissions.global}
      onPermissionChange={handlePermissionChange}
      onGlobalChange={handleGlobalChange}
      renderButton={(props) => <IndexingStateButton {...props} />}
      disabled={saving}
      globalLabel="Built-in MCPs"
      emptyMessage="No built-in MCP servers available"
      loading={loading}
      defaultExpanded={false}
    />
  )
}
