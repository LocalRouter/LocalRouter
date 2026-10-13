import { useState, useEffect, useCallback } from "react"
import { invoke } from "@tauri-apps/api/core"
import { listenSafe } from "@/hooks/useTauriListener"
import { toast } from "sonner"
import { PermissionTreeSelector } from "./PermissionTreeSelector"
import { PermissionStateButton } from "./PermissionStateButton"
import type { PermissionState, TreeNode, SkillsPermissions, PermissionTreeProps } from "./types"
import type {
  SkillInfo,
  SkillSourceInfo,
  SkillToolInfo,
  ClearClientSkillsChildPermissionsParams,
  SetClientSkillsPermissionParams,
} from "@/types/tauri-commands"
import { groupSkillsBySource, splitMarketplaceGroups } from "@/utils/skill-sources"

interface SkillsPermissionTreeProps extends PermissionTreeProps {
  permissions: SkillsPermissions
}

/** Node id prefix for source rows; source paths may contain "__" */
const SOURCE_PREFIX = "source:"
/** Display-only group of marketplace installs (each is its own one-skill source) */
const MARKETPLACE_GROUP_ID = "group:marketplace"

const pluralSkills = (n: number) => `${n} skill${n === 1 ? "" : "s"}`

export function SkillsPermissionTree({ clientId, permissions, onUpdate }: SkillsPermissionTreeProps) {
  const [skills, setSkills] = useState<SkillInfo[]>([])
  const [sources, setSources] = useState<SkillSourceInfo[]>([])
  const [skillTools, setSkillTools] = useState<Record<string, SkillToolInfo[]>>({})
  const [loading, setLoading] = useState(true)
  const [saving, setSaving] = useState(false)

  const loadSkills = useCallback(async () => {
    try {
      const [skillList, sourceList] = await Promise.all([
        invoke<SkillInfo[]>("list_skills"),
        invoke<SkillSourceInfo[]>("list_skill_sources"),
      ])
      const enabledSkills = skillList.filter((s) => s.enabled)
      setSkills(enabledSkills)
      setSources(sourceList)

      const entries = await Promise.all(
        enabledSkills.map(async (skill) => {
          try {
            const tools = await invoke<SkillToolInfo[]>("get_skill_tools", { skillName: skill.name })
            return [skill.name, tools] as const
          } catch (error) {
            console.error(`Failed to load tools for skill ${skill.name}:`, error)
            return [skill.name, []] as const
          }
        })
      )
      setSkillTools(Object.fromEntries(entries))
    } catch (error) {
      console.error("Failed to load skills:", error)
    } finally {
      setLoading(false)
    }
  }, [])

  useEffect(() => {
    loadSkills()

    const l = listenSafe("skills-changed", () => {
      loadSkills()
    })

    return () => {
      l.cleanup()
    }
  }, [loadSkills])

  const handlePermissionChange = async (key: string, state: PermissionState, parentState: PermissionState) => {
    setSaving(true)
    try {
      // If the new state matches the parent, clear the override (inherit from parent)
      // If the new state differs, set an explicit override
      const shouldClear = state === parentState

      if (key.startsWith(SOURCE_PREFIX)) {
        // Source level - also clear the overrides of its skills and their tools
        const sourcePath = key.slice(SOURCE_PREFIX.length)
        await invoke("clear_client_skills_child_permissions", {
          clientId,
          sourcePath,
        } satisfies ClearClientSkillsChildPermissionsParams)
        await invoke("set_client_skills_permission", {
          clientId,
          level: "source",
          key: sourcePath,
          state,
          clear: shouldClear,
        } satisfies SetClientSkillsPermissionParams)
        onUpdate()
        return
      }

      // Format: skill_name or skill_name__tool__tool_name
      const parts = key.split("__")

      if (parts.length === 1) {
        // Skill level - also clear all child permissions (tools)
        await invoke("clear_client_skills_child_permissions", {
          clientId,
          skillName: key,
        } satisfies ClearClientSkillsChildPermissionsParams)
        await invoke("set_client_skills_permission", {
          clientId,
          level: "skill",
          key,
          state,
          clear: shouldClear,
        } satisfies SetClientSkillsPermissionParams)
      } else if (parts.length === 3 && parts[1] === "tool") {
        // Tool level
        const [skillName, , toolName] = parts
        await invoke("set_client_skills_permission", {
          clientId,
          level: "tool",
          key: `${skillName}__${toolName}`,
          state,
          clear: shouldClear,
        } satisfies SetClientSkillsPermissionParams)
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
      await invoke("clear_client_skills_child_permissions", { clientId } satisfies ClearClientSkillsChildPermissionsParams)
      // Then set the global permission
      await invoke("set_client_skills_permission", {
        clientId,
        level: "global",
        key: null,
        state,
      } satisfies SetClientSkillsPermissionParams)
      onUpdate()
    } catch (error) {
      console.error("Failed to set global permission:", error)
      toast.error("Failed to update permission")
    } finally {
      setSaving(false)
    }
  }

  const skillNode = (skill: SkillInfo): TreeNode => {
    const children: TreeNode[] | undefined = skillTools[skill.name]?.map((tool) => ({
      id: `${skill.name}__tool__${tool.name}`,
      label: tool.name,
      description: tool.description || undefined,
    }))
    return {
      id: skill.name,
      label: skill.name,
      description: skill.description || undefined,
      searchText: `${skill.tags.join(" ")} ${skill.source_path}`,
      children: children && children.length > 0 ? children : undefined,
    }
  }

  // Build tree nodes: source -> skill -> tool, marketplace installs grouped
  const buildTree = (): TreeNode[] => {
    const { sourceGroups, marketplaceSkills } = splitMarketplaceGroups(groupSkillsBySource(skills, sources))
    const nodes: TreeNode[] = sourceGroups.map(({ source, skills: sourceSkills }) => ({
      id: `${SOURCE_PREFIX}${source.path}`,
      label: source.label,
      description: `${source.path} · ${pluralSkills(sourceSkills.length)}`,
      searchText: source.path,
      children: sourceSkills.map(skillNode),
    }))
    if (marketplaceSkills.length > 0) {
      nodes.push({
        id: MARKETPLACE_GROUP_ID,
        label: "Marketplace",
        description: `Installed from the marketplace · ${pluralSkills(marketplaceSkills.length)}`,
        isGroup: true,
        children: marketplaceSkills.map(skillNode),
      })
    }
    return nodes
  }

  // Build flat permissions map for the tree
  const buildPermissionsMap = (): Record<string, PermissionState> => {
    const map: Record<string, PermissionState> = {}

    for (const [sourcePath, state] of Object.entries(permissions.sources ?? {})) {
      map[`${SOURCE_PREFIX}${sourcePath}`] = state
    }

    for (const [skillName, state] of Object.entries(permissions.skills ?? {})) {
      map[skillName] = state
    }

    for (const [key, state] of Object.entries(permissions.tools ?? {})) {
      const [skillName, toolName] = key.split("__")
      map[`${skillName}__tool__${toolName}`] = state
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
      globalLabel="All Skills"
      emptyMessage="No skills discovered. Add skill sources in the Skills view."
      searchable
      searchPlaceholder="Search skills or sources..."
    />
  )
}
