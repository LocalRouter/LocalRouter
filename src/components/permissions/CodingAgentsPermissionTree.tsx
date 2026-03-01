import { useState, useEffect, useCallback } from "react"
import { invoke } from "@tauri-apps/api/core"
import { listen } from "@tauri-apps/api/event"
import { toast } from "sonner"
import { PermissionTreeSelector } from "./PermissionTreeSelector"
import { PermissionStateButton } from "./PermissionStateButton"
import type { PermissionState, TreeNode, CodingAgentsPermissions, PermissionTreeProps } from "./types"

interface CodingAgentInfo {
  agentType: string
  displayName: string
  toolPrefix: string
  binaryName: string
  installed: boolean
}

interface CodingAgentsPermissionTreeProps extends PermissionTreeProps {
  permissions: CodingAgentsPermissions
}

export function CodingAgentsPermissionTree({ clientId, permissions, onUpdate }: CodingAgentsPermissionTreeProps) {
  const [agents, setAgents] = useState<CodingAgentInfo[]>([])
  const [loading, setLoading] = useState(true)
  const [saving, setSaving] = useState(false)

  const loadAgents = useCallback(async () => {
    try {
      const agentList = await invoke<CodingAgentInfo[]>("list_coding_agents")
      const installedAgents = agentList.filter((a) => a.installed)
      setAgents(installedAgents)
    } catch (error) {
      console.error("Failed to load coding agents:", error)
    } finally {
      setLoading(false)
    }
  }, [])

  useEffect(() => {
    loadAgents()

    const unsubscribe = listen("coding-agents-changed", () => {
      loadAgents()
    })

    return () => {
      unsubscribe.then((fn) => fn())
    }
  }, [loadAgents])

  const handlePermissionChange = async (key: string, state: PermissionState) => {
    setSaving(true)
    try {
      await invoke("set_client_coding_agents_permission", {
        clientId,
        level: "agent",
        key,
        state,
      })
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
      await invoke("set_client_coding_agents_permission", {
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

  const buildTree = (): TreeNode[] => {
    return agents.map((agent) => ({
      id: agent.toolPrefix,
      label: agent.displayName,
      description: agent.installed ? `${agent.binaryName} (installed)` : `${agent.binaryName} (not found)`,
    }))
  }

  const buildPermissionsMap = (): Record<string, PermissionState> => {
    const map: Record<string, PermissionState> = {}
    if (permissions.agents) {
      for (const [key, state] of Object.entries(permissions.agents)) {
        map[key] = state
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
      renderButton={(props) => <PermissionStateButton {...props} />}
      disabled={saving}
      loading={loading}
      globalLabel="All Coding Agents"
      emptyMessage="No coding agents installed. Install a supported coding agent to manage permissions."
    />
  )
}
