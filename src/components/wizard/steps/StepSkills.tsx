/**
 * Step 4: Select Skills
 *
 * Skills permission selection using Allow/Ask/Off states.
 * Supports hierarchical permissions for skills and their tools.
 */

import { useState, useEffect, useCallback } from "react"
import { invoke } from "@tauri-apps/api/core"
import { Loader2, Info } from "lucide-react"
import { PermissionTreeSelector } from "@/components/permissions/PermissionTreeSelector"
import { PermissionStateButton } from "@/components/permissions/PermissionStateButton"
import type { PermissionState, TreeNode, SkillsPermissions } from "@/components/permissions/types"

interface SkillInfo {
  name: string
  description: string | null
  enabled: boolean
}

interface SkillToolInfo {
  name: string
  description: string | null
}

interface StepSkillsProps {
  permissions: SkillsPermissions
  onChange: (permissions: SkillsPermissions) => void
}

export function StepSkills({ permissions, onChange }: StepSkillsProps) {
  const [skills, setSkills] = useState<SkillInfo[]>([])
  const [skillTools, setSkillTools] = useState<Record<string, SkillToolInfo[]>>({})
  const [loading, setLoading] = useState(true)

  const loadSkills = useCallback(async () => {
    try {
      setLoading(true)
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
      setSkills([])
    } finally {
      setLoading(false)
    }
  }, [])

  useEffect(() => {
    loadSkills()
  }, [loadSkills])

  // Handle permission changes
  const handlePermissionChange = (key: string, state: PermissionState, parentState: PermissionState) => {
    // If the new state matches the parent, remove the override (inherit from parent)
    // Otherwise, set an explicit override
    const shouldClear = state === parentState

    // Parse the key to determine the level
    // Format: skill_name or skill_name__tool__tool_name
    const parts = key.split("__")

    const newPermissions = { ...permissions }

    if (parts.length === 1) {
      // Skill level
      const newSkills = { ...permissions.skills }
      if (shouldClear) {
        delete newSkills[key]
      } else {
        newSkills[key] = state
      }
      newPermissions.skills = newSkills
    } else if (parts.length === 3 && parts[1] === "tool") {
      // Tool level
      const [skillName, , toolName] = parts
      const compositeKey = `${skillName}__${toolName}`

      const newTools = { ...permissions.tools }
      if (shouldClear) {
        delete newTools[compositeKey]
      } else {
        newTools[compositeKey] = state
      }
      newPermissions.tools = newTools
    }

    onChange(newPermissions)
  }

  const handleGlobalChange = (state: PermissionState) => {
    // Clear all child customizations when global changes
    onChange({
      global: state,
      skills: {},
      tools: {},
    })
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
    for (const [skillName, state] of Object.entries(permissions.skills)) {
      map[skillName] = state
    }

    // Tool permissions
    for (const [key, state] of Object.entries(permissions.tools)) {
      const [skillName, toolName] = key.split("__")
      map[`${skillName}__tool__${toolName}`] = state
    }

    return map
  }

  if (loading) {
    return (
      <div className="flex items-center justify-center py-12">
        <Loader2 className="h-6 w-6 animate-spin text-muted-foreground" />
      </div>
    )
  }

  if (skills.length === 0) {
    return (
      <div className="space-y-4">
        <div className="flex items-start gap-3 p-4 rounded-lg bg-muted/50">
          <Info className="h-5 w-5 text-muted-foreground mt-0.5 shrink-0" />
          <div className="text-sm text-muted-foreground">
            <p className="font-medium text-foreground mb-1">No skills configured yet</p>
            <p>
              Skills can be added from the Skills page after client creation.
              You can skip this step for now.
            </p>
          </div>
        </div>
      </div>
    )
  }

  return (
    <div className="space-y-4">
      <p className="text-sm text-muted-foreground">
        Configure skills access for this client. Use Allow, Ask, or Off for each skill.
      </p>

      <PermissionTreeSelector
        nodes={buildTree()}
        permissions={buildPermissionsMap()}
        globalPermission={permissions.global}
        onPermissionChange={handlePermissionChange}
        onGlobalChange={handleGlobalChange}
        renderButton={(props) => <PermissionStateButton {...props} />}
        globalLabel="All Skills"
        emptyMessage="No skills discovered"
      />

      <p className="text-xs text-muted-foreground">
        Skills provide domain-specific tools and capabilities to LLM applications.
      </p>
    </div>
  )
}
