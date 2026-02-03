import { useState, useEffect, useCallback } from "react"
import { invoke } from "@tauri-apps/api/core"
import { listen } from "@tauri-apps/api/event"
import { toast } from "sonner"
import { PermissionTreeSelector } from "./PermissionTreeSelector"
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

export function McpPermissionTree({ clientId, permissions, onUpdate }: McpPermissionTreeProps) {
  const [servers, setServers] = useState<McpServer[]>([])
  const [capabilities, setCapabilities] = useState<Record<string, McpServerCapabilities>>({})
  const [loading, setLoading] = useState(true)
  const [saving, setSaving] = useState(false)

  const loadServers = useCallback(async () => {
    try {
      const serverList = await invoke<McpServer[]>("list_mcp_servers")
      const enabledServers = serverList.filter((s) => s.enabled)
      setServers(enabledServers)

      // Eagerly load capabilities for all enabled servers
      for (const server of enabledServers) {
        try {
          const caps = await invoke<McpServerCapabilities>("get_mcp_server_capabilities", {
            serverId: server.id,
          })
          setCapabilities((prev) => ({ ...prev, [server.id]: caps }))
        } catch (error) {
          console.error(`Failed to load capabilities for ${server.id}:`, error)
        }
      }
    } catch (error) {
      console.error("Failed to load MCP servers:", error)
    } finally {
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

  const loadCapabilities = async (serverId: string) => {
    if (capabilities[serverId]) return // Already loaded

    try {
      const caps = await invoke<McpServerCapabilities>("get_mcp_server_capabilities", {
        serverId,
      })
      setCapabilities((prev) => ({ ...prev, [serverId]: caps }))
    } catch (error) {
      console.error(`Failed to load capabilities for ${serverId}:`, error)
    }
  }

  const handlePermissionChange = async (key: string, state: PermissionState) => {
    setSaving(true)
    try {
      // Parse the key to determine the level
      // Format: server_id or server_id__type__name
      const parts = key.split("__")

      if (parts.length === 1) {
        // Server level
        await invoke("set_client_mcp_permission", {
          clientId,
          level: "server",
          key,
          state,
        })
        // Load capabilities when server is enabled
        if (state !== "off") {
          loadCapabilities(key)
        }
      } else if (parts.length === 3) {
        // Tool/resource/prompt level
        const [serverId, type, name] = parts
        await invoke("set_client_mcp_permission", {
          clientId,
          level: type as "tool" | "resource" | "prompt",
          key: `${serverId}__${name}`,
          state,
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
    if (permissions.tools) {
      for (const [key, state] of Object.entries(permissions.tools)) {
        const [serverId, toolName] = key.split("__")
        map[`${serverId}__tool__${toolName}`] = state
      }
    }

    // Resource permissions
    if (permissions.resources) {
      for (const [key, state] of Object.entries(permissions.resources)) {
        const [serverId, uri] = key.split("__")
        map[`${serverId}__resource__${uri}`] = state
      }
    }

    // Prompt permissions
    if (permissions.prompts) {
      for (const [key, state] of Object.entries(permissions.prompts)) {
        const [serverId, promptName] = key.split("__")
        map[`${serverId}__prompt__${promptName}`] = state
      }
    }

    return map
  }

  return (
    <PermissionTreeSelector
      nodes={buildTree()}
      permissions={buildPermissionsMap()}
      globalPermission={permissions.global}
      onPermissionChange={handlePermissionChange}
      onGlobalChange={handleGlobalChange}
      disabled={saving}
      loading={loading}
      globalLabel="All MCP Servers"
      emptyMessage="No MCP servers configured. Add servers in Resources."
    />
  )
}
