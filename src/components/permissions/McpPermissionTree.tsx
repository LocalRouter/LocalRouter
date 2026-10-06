import { useState, useEffect, useCallback } from "react"
import { invoke } from "@tauri-apps/api/core"
import { listenSafe } from "@/hooks/useTauriListener"
import { toast } from "sonner"
import { PermissionTreeSelector } from "./PermissionTreeSelector"
import { PermissionStateButton } from "./PermissionStateButton"
import type { PermissionState, TreeNode, McpPermissions, PermissionTreeProps } from "./types"

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

interface McpPermissionTreeProps extends PermissionTreeProps {
  permissions: McpPermissions
}

/** Split `prefix__rest` at the first separator; server IDs never contain `__`,
 *  but tool names and resource URIs may. */
function splitOnce(key: string): [string, string] | null {
  const idx = key.indexOf("__")
  return idx === -1 ? null : [key.slice(0, idx), key.slice(idx + 2)]
}

/** Parse a tree node key: `serverId` or `serverId__{tool|resource|prompt}__name`. */
function parseMcpNodeKey(
  key: string,
): { serverId: string } | { serverId: string; type: "tool" | "resource" | "prompt"; name: string } | null {
  const first = splitOnce(key)
  if (!first) return { serverId: key }
  const [serverId, rest] = first
  const second = splitOnce(rest)
  if (!second) return null
  const [type, name] = second
  if (type !== "tool" && type !== "resource" && type !== "prompt") return null
  return { serverId, type, name }
}

export function McpPermissionTree({ clientId, permissions, onUpdate }: McpPermissionTreeProps) {
  const [servers, setServers] = useState<McpServer[]>([])
  const [capabilities, setCapabilities] = useState<Record<string, McpServerCapabilities>>({})
  const [capabilityErrors, setCapabilityErrors] = useState<Record<string, string>>({})
  const [loadingServers, setLoadingServers] = useState<Set<string>>(new Set())
  const [loading, setLoading] = useState(true)
  const [saving, setSaving] = useState(false)

  const fetchCapabilities = useCallback(async (serverId: string) => {
    try {
      const caps = await invoke<McpServerCapabilities>("get_mcp_server_capabilities", { serverId })
      setCapabilities((prev) => ({ ...prev, [serverId]: caps }))
      setCapabilityErrors((prev) => {
        const { [serverId]: _, ...rest } = prev
        return rest
      })
    } catch (error) {
      console.error(`Failed to load capabilities for ${serverId}:`, error)
      setCapabilityErrors((prev) => ({ ...prev, [serverId]: `Failed to load tools: ${error}` }))
    }
  }, [])

  const loadServers = useCallback(async () => {
    try {
      const serverList = await invoke<McpServer[]>("list_mcp_servers")
      const enabledServers = serverList.filter((s) => s.enabled)
      setServers(enabledServers)
      setLoading(false)

      // Track which servers are loading capabilities
      const serverIds = new Set(enabledServers.map((s) => s.id))
      setLoadingServers(serverIds)

      // Load capabilities in parallel, updating state as each resolves
      await Promise.all(
        enabledServers.map(async (server) => {
          try {
            await fetchCapabilities(server.id)
          } finally {
            setLoadingServers((prev) => {
              const next = new Set(prev)
              next.delete(server.id)
              return next
            })
          }
        })
      )
    } catch (error) {
      console.error("Failed to load MCP servers:", error)
      setLoading(false)
    }
  }, [fetchCapabilities])

  useEffect(() => {
    loadServers()

    const l = listenSafe("mcp-servers-changed", () => {
      loadServers()
    })

    return () => {
      l.cleanup()
    }
  }, [loadServers])

  const loadCapabilities = async (serverId: string) => {
    if (capabilities[serverId]) return // Already loaded
    await fetchCapabilities(serverId)
  }

  const handlePermissionChange = async (key: string, state: PermissionState, parentState: PermissionState) => {
    setSaving(true)
    try {
      // If the new state matches the parent, clear the override (inherit from parent)
      // If the new state differs, set an explicit override
      const shouldClear = state === parentState

      // Parse the key to determine the level
      // Format: server_id or server_id__type__name
      const parsed = parseMcpNodeKey(key)
      if (!parsed) return

      if (!("type" in parsed)) {
        // Server level - also clear all child permissions (tools/resources/prompts)
        await invoke("clear_client_mcp_child_permissions", {
          clientId,
          serverId: key,
        })
        await invoke("set_client_mcp_permission", {
          clientId,
          level: "server",
          key,
          state,
          clear: shouldClear,
        })
        // Load capabilities when server is enabled
        if (state !== "off") {
          loadCapabilities(key)
        }
      } else {
        // Tool/resource/prompt level
        const { serverId, type, name } = parsed
        await invoke("set_client_mcp_permission", {
          clientId,
          level: type,
          key: `${serverId}__${name}`,
          state,
          clear: shouldClear,
        })
      }
      onUpdate()
    } catch (error) {
      console.error("Failed to set permission:", error)
      toast.error("Failed to update permission")
    } finally {
      setSaving(false)
    }
  }

  const handleGlobalChange = async (state: PermissionState) => {
    setSaving(true)
    try {
      // First clear all child customizations so they inherit the new global value
      await invoke("clear_client_mcp_child_permissions", { clientId })
      // Then set the global permission
      await invoke("set_client_mcp_permission", {
        clientId,
        level: "global",
        key: null,
        state,
      })
      onUpdate()
    } catch (error) {
      console.error("Failed to set global permission:", error)
      toast.error("Failed to update permission")
    } finally {
      setSaving(false)
    }
  }

  // Build tree nodes from servers
  const buildTree = (): TreeNode[] => {
    return servers.map((server) => {
      const caps = capabilities[server.id]
      const children: TreeNode[] = []

      if (caps) {
        // Tools group
        if (caps.tools.length > 0) {
          children.push({
            id: `${server.id}__tools`,
            label: "Tools",
            isGroup: true,
            children: caps.tools.map((tool) => ({
              id: `${server.id}__tool__${tool.name}`,
              label: tool.name,
              description: tool.description || undefined,
            })),
          })
        }

        // Resources group
        if (caps.resources.length > 0) {
          children.push({
            id: `${server.id}__resources`,
            label: "Resources",
            isGroup: true,
            children: caps.resources.map((res) => ({
              id: `${server.id}__resource__${res.uri}`,
              label: res.name,
              description: res.description || undefined,
            })),
          })
        }

        // Prompts group
        if (caps.prompts.length > 0) {
          children.push({
            id: `${server.id}__prompts`,
            label: "Prompts",
            isGroup: true,
            children: caps.prompts.map((prompt) => ({
              id: `${server.id}__prompt__${prompt.name}`,
              label: prompt.name,
              description: prompt.description || undefined,
            })),
          })
        }
      }

      return {
        id: server.id,
        label: server.name,
        children: children.length > 0 ? children : undefined,
        loading: loadingServers.has(server.id),
        error: capabilityErrors[server.id],
      }
    })
  }

  // Build flat permissions map for the tree
  const buildPermissionsMap = (): Record<string, PermissionState> => {
    const map: Record<string, PermissionState> = {}

    // Server permissions
    if (permissions.servers) {
      for (const [serverId, state] of Object.entries(permissions.servers)) {
        map[serverId] = state
      }
    }

    // Tool permissions
    const addChildren = (entries: Record<string, PermissionState> | undefined, type: string) => {
      for (const [key, state] of Object.entries(entries ?? {})) {
        const parts = splitOnce(key)
        if (parts) map[`${parts[0]}__${type}__${parts[1]}`] = state
      }
    }
    addChildren(permissions.tools, "tool")
    addChildren(permissions.resources, "resource")
    addChildren(permissions.prompts, "prompt")

    return map
  }

  return (
    <PermissionTreeSelector
      nodes={buildTree()}
      permissions={buildPermissionsMap()}
      globalPermission={permissions.global}
      onPermissionChange={handlePermissionChange}
      onGlobalChange={handleGlobalChange}
      renderButton={(props) => <PermissionStateButton {...props} />}
      disabled={saving}
      loading={loading}
      globalLabel="All MCP Servers"
      emptyMessage="No MCP servers configured. Add servers in Resources."
    />
  )
}
