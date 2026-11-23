import { useState, useEffect, useCallback } from "react"
import { invoke } from "@tauri-apps/api/core"
import { listenSafe } from "@/hooks/useTauriListener"
import { toast } from "sonner"
import { PermissionTreeSelector } from "./PermissionTreeSelector"
import { PermissionStateButton } from "./PermissionStateButton"
import type { PermissionState, TreeNode, McpPermissions, PermissionTreeProps } from "./types"

import { LOCALROUTER_MCP_ID, LOCALROUTER_MCP_NAME } from "@/constants/localrouter-mcp"
import type { AdminMcpAccess, GetClientAdminMcpAccessParams, SetClientAdminMcpAccessParams } from "@/types/tauri-commands"

const adminScopes = [
  { key: "configuration", label: "Read configuration" },
  { key: "manage", label: "Change configuration" },
  { key: "providers", label: "Providers and models" },
  { key: "monitor", label: "Monitor" },
  { key: "intercept", label: "Active intercept" },
] as const

const adminOff: AdminMcpAccess = {
  ask: false, configuration: false, manage: false, providers: false, monitor: false, intercept: false,
}

function hasAdminAccess(access: AdminMcpAccess) {
  return adminScopes.some(scope => access[scope.key])
}

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
  const [admin, setAdmin] = useState<AdminMcpAccess | null>(null)
  const [adminError, setAdminError] = useState(false)

  useEffect(() => {
    let cancelled = false
    setAdmin(null)
    setAdminError(false)
    invoke<AdminMcpAccess>("get_client_admin_mcp_access", { clientId } satisfies GetClientAdminMcpAccessParams)
      .then(value => { if (!cancelled) setAdmin(value) })
      .catch(() => { if (!cancelled) setAdminError(true) })
    return () => { cancelled = true }
  }, [clientId])

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
      if (key === LOCALROUTER_MCP_ID || key.startsWith(`${LOCALROUTER_MCP_ID}:`)) {
        if (!admin) return
        const scope = adminScopes.find(item => key === `${LOCALROUTER_MCP_ID}:${item.key}`)
        const access = scope
          ? { ...admin, [scope.key]: state === "allow" }
          : state === "off"
            ? adminOff
            : { ...(hasAdminAccess(admin) ? admin : { ...adminOff, configuration: true }), ask: state === "ask" }
        setAdmin(await invoke<AdminMcpAccess>("set_client_admin_mcp_access", {
          clientId, access,
        } satisfies SetClientAdminMcpAccessParams))
        onUpdate()
        return
      }
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
    const adminNode: TreeNode = {
      id: LOCALROUTER_MCP_ID,
      label: LOCALROUTER_MCP_NAME,
      description: "BUILT-IN",
      independent: true,
      disabled: !admin,
      error: adminError ? "Could not load permissions." : undefined,
      children: adminScopes.map(scope => ({
        id: `${LOCALROUTER_MCP_ID}:${scope.key}`,
        label: scope.label,
        disabled: !admin || (scope.key === "configuration" && admin.manage),
      })),
    }
    return [adminNode, ...servers.map((server) => {
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
    })]
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
      renderButton={({ nodeId, ...props }) => (
        <PermissionStateButton
          {...props}
          allowedStates={nodeId?.startsWith(`${LOCALROUTER_MCP_ID}:`) ? ["allow", "off"] : undefined}
        />
      )}
      disabled={saving}
      loading={loading}
      globalLabel="All MCP Servers"
      emptyMessage="No MCP servers configured. Add servers in Resources."
    />
  )
}
