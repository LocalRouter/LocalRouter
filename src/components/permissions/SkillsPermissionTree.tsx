import { useState, useEffect, useCallback } from "react"
import { invoke } from "@tauri-apps/api/core"
import { listen } from "@tauri-apps/api/event"
import { toast } from "sonner"
import { PermissionTreeSelector } from "./PermissionTreeSelector"
import type { PermissionState, TreeNode, SkillsPermissions, PermissionTreeProps } from "./types"

interface SkillInfo {
  name: string
  description: string | null
  enabled: boolean
}

interface SkillToolInfo {
  name: string
  description: string | null
}

interface SkillsPermissionTreeProps extends PermissionTreeProps {
  permissions: SkillsPermissions
}

export function SkillsPermissionTree({ clientId, permissions, onUpdate }: SkillsPermissionTreeProps) {
  const [skills, setSkills] = useState<SkillInfo[]>([])
  const [skillTools, setSkillTools] = useState<Record<string, SkillToolInfo[]>>({})
  const [loading, setLoading] = useState(true)
  const [saving, setSaving] = useState(false)

  const loadSkills = useCallback(async () => {
    try {
      const skillList = await invoke<SkillInfo[]>("list_skills")
      const enabledSkills = skillList.filter((s) => s.enabled)
      setSkills(enabledSkills)

      // Eagerly load tools for all enabled skills
      for (const skill of enabledSkills) {
        try {
          const tools = await invoke<SkillToolInfo[]>("get_skill_tools", {
            skillName: skill.name,
          })
          setSkillTools((prev) => ({ ...prev, [skill.name]: tools }))
        } catch (error) {
          console.error(`Failed to load tools for skill ${skill.name}:`, error)
        }
      }
    } catch (error) {
      console.error("Failed to load skills:", error)
    } finally {
      setLoading(false)
    }
  }, [])

  useEffect(() => {
    loadSkills()

    const unsubscribe = listen("skills-changed", () => {
      loadSkills()
    })

    return () => {
      unsubscribe.then((fn) => fn())
    }
  }, [loadSkills])

  const loadSkillTools = async (skillName: string) => {
    if (skillTools[skillName]) return // Already loaded

    try {
      const tools = await invoke<SkillToolInfo[]>("get_skill_tools", {
        skillName,
      })
      setSkillTools((prev) => ({ ...prev, [skillName]: tools }))
    } catch (error) {
      console.error(`Failed to load tools for skill ${skillName}:`, error)
    }
  }

  const handlePermissionChange = async (key: string, state: PermissionState, parentState: PermissionState) => {
    setSaving(true)
    try {
      // If the new state matches the parent, clear the override (inherit from parent)
      // If the new state differs, set an explicit override
      const shouldClear = state === parentState

      // Parse the key to determine the level
      // Format: skill_name or skill_name__tool__tool_name
      const parts = key.split("__")

      if (parts.length === 1) {
        // Skill level - also clear all child permissions (tools)
        await invoke("clear_client_skills_child_permissions", {
          clientId,
          skillName: key,
        })
        await invoke("set_client_skills_permission", {
          clientId,
          level: "skill",
          key,
          state,
          clear: shouldClear,
        })
        // Load tools when skill is enabled
        if (state !== "off") {
          loadSkillTools(key)
        }
      } else if (parts.length === 3 && parts[1] === "tool") {
        // Tool level
        const [skillName, , toolName] = parts
        await invoke("set_client_skills_permission", {
          clientId,
          level: "tool",
          key: `${skillName}__${toolName}`,
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
      await invoke("clear_client_skills_child_permissions", { clientId })
      // Then set the global permission
      await invoke("set_client_skills_permission", {
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

  // Build tree nodes from skills
  const buildTree = (): TreeNode[] => {
    return skills.map((skill) => {
      const tools = skillTools[skill.name]
      const children: TreeNode[] | undefined = tools?.map((tool) => ({
        id: `${skill.name}__tool__${tool.name}`,
        label: tool.name,
        description: tool.description || undefined,
      }))

      return {
        id: skill.name,
        label: skill.name,
        description: skill.description || undefined,
        children: children && children.length > 0 ? children : undefined,
      }
    })
  }

  // Build flat permissions map for the tree
  const buildPermissionsMap = (): Record<string, PermissionState> => {
    const map: Record<string, PermissionState> = {}

    // Skill permissions
    if (permissions.skills) {
      for (const [skillName, state] of Object.entries(permissions.skills)) {
        map[skillName] = state
      }
    }

    // Tool permissions
    if (permissions.tools) {
      for (const [key, state] of Object.entries(permissions.tools)) {
        const [skillName, toolName] = key.split("__")
        map[`${skillName}__tool__${toolName}`] = state
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
      globalLabel="All Skills"
      emptyMessage="No skills discovered. Add skill sources in the Skills view."
    />
  )
}
