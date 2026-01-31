
import { useState, useEffect } from "react"
import { invoke } from "@tauri-apps/api/core"
import { listen } from "@tauri-apps/api/event"
import { toast } from "sonner"
import { Card, CardContent, CardHeader, CardTitle, CardDescription } from "@/components/ui/Card"
import { Checkbox } from "@/components/ui/checkbox"
import { cn } from "@/lib/utils"

interface Client {
  id: string
  name: string
  client_id: string
  skills_access_mode: "none" | "all" | "specific"
  skills_paths: string[]
}

interface SkillInfo {
  name: string
  description: string | null
  version: string | null
  author: string | null
  tags: string[]
  source_path: string
  script_count: number
  reference_count: number
  asset_count: number
  enabled: boolean
}

interface SkillsTabProps {
  client: Client
  onUpdate: () => void
}

export function ClientSkillsTab({ client, onUpdate }: SkillsTabProps) {
  const [skills, setSkills] = useState<SkillInfo[]>([])
  const [loading, setLoading] = useState(true)
  const [saving, setSaving] = useState(false)

  const [includeAllSkills, setIncludeAllSkills] = useState(client.skills_access_mode === "all")
  const [selectedPaths, setSelectedPaths] = useState<Set<string>>(
    new Set(client.skills_paths)
  )

  useEffect(() => {
    loadSkills()

    const unsubscribe = listen("skills-changed", () => {
      loadSkills()
    })

    return () => {
      unsubscribe.then((fn) => fn())
    }
  }, [])

  useEffect(() => {
    setIncludeAllSkills(client.skills_access_mode === "all")
    setSelectedPaths(new Set(client.skills_paths))
  }, [client.skills_access_mode, client.skills_paths])

  const loadSkills = async () => {
    try {
      const skillList = await invoke<SkillInfo[]>("list_skills")
      setSkills(skillList)
    } catch (error) {
      console.error("Failed to load skills:", error)
    } finally {
      setLoading(false)
    }
  }

  // Get unique source paths from discovered skills
  const sourcePaths = [...new Set(skills.map(s => s.source_path))]

  const handleAllSkillsToggle = async () => {
    try {
      setSaving(true)
      const newIncludeAll = !includeAllSkills

      if (newIncludeAll) {
        await invoke("set_client_skills_access", {
          clientId: client.client_id,
          mode: "all",
          paths: [],
        })
        setIncludeAllSkills(true)
        toast.success("All skills enabled")
      } else {
        const mode = selectedPaths.size > 0 ? "specific" : "none"
        await invoke("set_client_skills_access", {
          clientId: client.client_id,
          mode,
          paths: Array.from(selectedPaths),
        })
        setIncludeAllSkills(false)
        toast.success("Switched to specific skill selection")
      }
      onUpdate()
    } catch (error) {
      console.error("Failed to update skills access:", error)
      toast.error("Failed to update skills settings")
    } finally {
      setSaving(false)
    }
  }

  const handleSourcePathToggle = async (sourcePath: string) => {
    if (includeAllSkills) {
      // Switch from All to Specific, excluding this path
      try {
        setSaving(true)
        const otherPaths = sourcePaths.filter(p => p !== sourcePath)

        await invoke("set_client_skills_access", {
          clientId: client.client_id,
          mode: otherPaths.length > 0 ? "specific" : "none",
          paths: otherPaths,
        })

        setIncludeAllSkills(false)
        setSelectedPaths(new Set(otherPaths))
        toast.success("Skill access updated")
        onUpdate()
      } catch (error) {
        console.error("Failed to update skill:", error)
        toast.error("Failed to update skill access")
      } finally {
        setSaving(false)
      }
      return
    }

    try {
      setSaving(true)
      const newSelected = new Set(selectedPaths)

      if (newSelected.has(sourcePath)) {
        newSelected.delete(sourcePath)
      } else {
        newSelected.add(sourcePath)
      }

      const allSelected = sourcePaths.length > 0 && sourcePaths.every(p => newSelected.has(p))

      if (allSelected) {
        await invoke("set_client_skills_access", {
          clientId: client.client_id,
          mode: "all",
          paths: [],
        })
        setIncludeAllSkills(true)
        setSelectedPaths(newSelected)
        toast.success("All skills enabled")
      } else {
        const mode = newSelected.size > 0 ? "specific" : "none"
        await invoke("set_client_skills_access", {
          clientId: client.client_id,
          mode,
          paths: Array.from(newSelected),
        })
        setSelectedPaths(newSelected)
        toast.success("Skill access updated")
      }

      onUpdate()
    } catch (error) {
      console.error("Failed to update skill:", error)
      toast.error("Failed to update skill access")
    } finally {
      setSaving(false)
    }
  }

  const selectedCount = includeAllSkills
    ? sourcePaths.length
    : Array.from(selectedPaths).filter((p) =>
        sourcePaths.includes(p)
      ).length

  const isIndeterminate = !includeAllSkills && selectedCount > 0 && selectedCount < sourcePaths.length

  const isPathSelected = (sourcePath: string): boolean => {
    if (includeAllSkills) return true
    return selectedPaths.has(sourcePath)
  }

  // Group skills by source_path
  const groupedSkills = skills.reduce<Record<string, SkillInfo[]>>((acc, skill) => {
    const key = skill.source_path
    if (!acc[key]) acc[key] = []
    acc[key].push(skill)
    return acc
  }, {})

  return (
    <div className="space-y-6">
      <Card>
        <CardHeader>
          <CardTitle>Skills Access</CardTitle>
          <CardDescription>
            Select which skill sources this client can access via MCP tools
          </CardDescription>
        </CardHeader>
        <CardContent>
          {loading ? (
            <div className="p-8 text-center text-muted-foreground text-sm">
              Loading skills...
            </div>
          ) : skills.length === 0 ? (
            <div className="p-8 text-center text-muted-foreground text-sm">
              No skills discovered. Add skill sources in the Skills view.
            </div>
          ) : (
            <div className="border rounded-lg">
              <div className="max-h-[400px] overflow-y-auto">
                {/* All Skills row */}
                <div
                  className="flex items-center gap-3 px-4 py-3 border-b bg-background sticky top-0 z-10 cursor-pointer hover:bg-muted/50 transition-colors"
                  onClick={() => !saving && handleAllSkillsToggle()}
                >
                  <Checkbox
                    checked={includeAllSkills || isIndeterminate}
                    onCheckedChange={handleAllSkillsToggle}
                    disabled={saving}
                    className={cn(
                      "data-[state=checked]:bg-primary",
                      isIndeterminate && "data-[state=checked]:bg-primary/60"
                    )}
                  />
                  <span className="font-semibold text-sm">
                    All Skills
                  </span>
                  <span className="text-xs text-muted-foreground ml-auto">
                    {includeAllSkills ? (
                      <span className="text-primary">All (including future skills)</span>
                    ) : (
                      `${selectedCount} / ${sourcePaths.length} source${sourcePaths.length !== 1 ? "s" : ""} selected`
                    )}
                  </span>
                </div>

                {/* Skills grouped by source path */}
                {Object.entries(groupedSkills).map(([sourcePath, groupSkills]) => {
                  const isSelected = isPathSelected(sourcePath)
                  const canToggle = !saving
                  const hasDisabled = groupSkills.some(s => !s.enabled)

                  return (
                    <div key={sourcePath}>
                      <div
                        className={cn(
                          "flex items-center gap-3 px-4 py-2.5 border-b",
                          "hover:bg-muted/30 transition-colors",
                          canToggle ? "cursor-pointer" : "",
                          includeAllSkills && "opacity-60"
                        )}
                        onClick={() => canToggle && handleSourcePathToggle(sourcePath)}
                      >
                        <Checkbox
                          checked={isSelected}
                          onCheckedChange={() => handleSourcePathToggle(sourcePath)}
                          disabled={!canToggle}
                        />
                        <div className="flex-1 min-w-0">
                          <span className="text-sm font-medium truncate block" title={sourcePath}>
                            {sourcePath}
                          </span>
                          <p className="text-xs text-muted-foreground">
                            {groupSkills.length} skill{groupSkills.length !== 1 ? "s" : ""}
                            {hasDisabled && " (some disabled globally)"}
                          </p>
                        </div>
                      </div>
                      {/* Show individual skills under this source (non-interactive, info only) */}
                      {groupSkills.map((skill) => (
                        <div
                          key={skill.name}
                          className={cn(
                            "flex items-center gap-3 px-4 py-1.5 border-b border-border/50",
                            !skill.enabled && "opacity-40"
                          )}
                          style={{ paddingLeft: "3rem" }}
                        >
                          <div className="flex-1 min-w-0">
                            <span className="text-xs font-medium">{skill.name}</span>
                            {skill.description && (
                              <span className="text-xs text-muted-foreground ml-2 truncate">
                                {skill.description}
                              </span>
                            )}
                          </div>
                          <div className="flex items-center gap-1.5">
                            {!skill.enabled && (
                              <span className="text-[10px] px-1.5 py-0.5 rounded bg-muted text-muted-foreground">
                                disabled
                              </span>
                            )}
                            {skill.script_count > 0 && (
                              <span className="text-[10px] px-1.5 py-0.5 rounded bg-blue-500/10 text-blue-600 dark:text-blue-400">
                                scripts
                              </span>
                            )}
                            {skill.reference_count > 0 && (
                              <span className="text-[10px] px-1.5 py-0.5 rounded bg-green-500/10 text-green-600 dark:text-green-400">
                                refs
                              </span>
                            )}
                          </div>
                        </div>
                      ))}
                    </div>
                  )
                })}
              </div>
            </div>
          )}
        </CardContent>
      </Card>
    </div>
  )
}
