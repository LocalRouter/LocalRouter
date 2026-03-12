import { useState, useEffect, useCallback } from "react"
import { invoke } from "@tauri-apps/api/core"
import { listen } from "@tauri-apps/api/event"
import { toast } from "sonner"
import { PermissionTreeSelector } from "./PermissionTreeSelector"
import { IndexingStateButton } from "./IndexingStateButton"
import type { TreeNode } from "./types"
import type { IndexingState, GatewayIndexingPermissions } from "@/types/tauri-commands"

interface McpServer {
  id: string
  name: string
  enabled: boolean
}

interface McpServerCapabilities {
  tools: Array<{ name: string; description: string | null }>
  resources: Array<{ uri: string; name: string; description: string | null }>
  prompts: Array<{ name: string; description: string | null }>
}

interface GatewayIndexingTreeProps {
  permissions: GatewayIndexingPermissions
  onUpdate: () => void
}

/** Replicate backend slugify: lowercase, non-alphanumeric → dash, trim trailing dash */
function slugify(name: string): string {
  let slug = ""
  let lastWasSep = true
  for (const ch of name) {
    if (/[a-zA-Z0-9]/.test(ch)) {
      slug += ch.toLowerCase()
      lastWasSep = false
    } else if (!lastWasSep) {
      slug += "-"
      lastWasSep = true
    }
  }
  if (slug.endsWith("-")) slug = slug.slice(0, -1)
  return slug
}

export function GatewayIndexingTree({ permissions, onUpdate }: GatewayIndexingTreeProps) {
  const [servers, setServers] = useState<McpServer[]>([])
  const [capabilities, setCapabilities] = useState<Record<string, McpServerCapabilities>>({})
  const [loading, setLoading] = useState(true)
  const [saving, setSaving] = useState(false)

  const loadServers = useCallback(async () => {
    try {
      const serverList = await invoke<McpServer[]>("list_mcp_servers")
      const enabledServers = serverList.filter((s) => s.enabled)
      setServers(enabledServers)
      setLoading(false)

      // Load capabilities in parallel
      await Promise.all(
        enabledServers.map(async (server) => {
          try {
            const caps = await invoke<McpServerCapabilities>("get_mcp_server_capabilities", {
              serverId: server.id,
            })
            setCapabilities((prev) => ({ ...prev, [server.id]: caps }))
          } catch (error) {
            console.error(`Failed to load capabilities for ${server.id}:`, error)
          }
        })
      )
    } catch (error) {
      console.error("Failed to load MCP servers:", error)
      setLoading(false)
    }
  }, [])

  useEffect(() => {
    loadServers()

    const unsubscribe = listen("mcp-servers-changed", () => {
      loadServers()
    })

    return () => {
      unsubscribe.then((fn) => fn())
    }
  }, [loadServers])

  // Build tree nodes: server → tools
  // Use slugified server names as IDs to match backend key format
  const nodes: TreeNode[] = servers.map((server) => {
    const slug = slugify(server.name)
    const caps = capabilities[server.id]
    const toolChildren: TreeNode[] = (caps?.tools || []).map((tool) => ({
      id: `${slug}__${tool.name}`,
      label: tool.name,
      description: tool.description || undefined,
    }))

    return {
      id: slug,
      label: server.name,
      children: toolChildren.length > 0 ? toolChildren : undefined,
    }
  })

  // Build flat permissions map for the tree selector
  // The tree uses server IDs at top level and "server_id__tool_name" at tool level
  const flatPermissions: Record<string, IndexingState> = {}
  for (const [key, value] of Object.entries(permissions.servers ?? {})) {
    flatPermissions[key] = value
  }
  for (const [key, value] of Object.entries(permissions.tools ?? {})) {
    flatPermissions[key] = value
  }

  const handlePermissionChange = async (key: string, state: IndexingState, parentState: IndexingState) => {
    setSaving(true)
    try {
      // If new state equals parent state, clear the override
      const shouldClear = state === parentState
      const isToolKey = key.includes("__")

      const params = shouldClear
        ? { level: isToolKey ? "tool_clear" : "server_clear", key, state: "enable" }
        : { level: isToolKey ? "tool" : "server", key, state }

      await invoke("set_gateway_indexing_permission", params)
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
      await invoke("set_gateway_indexing_permission", { level: "global", state })
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
      globalLabel="All Gateway Tools"
      emptyMessage="No MCP servers configured"
      loading={loading}
    />
  )
}
