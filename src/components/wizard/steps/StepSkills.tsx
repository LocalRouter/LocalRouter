/**
 * Step 4: Select Skills
 *
 * Skills access selection for the client.
 * Supports All / Specific / None access modes.
 */

import { useState, useEffect } from "react"
import { invoke } from "@tauri-apps/api/core"
import { Info } from "lucide-react"
import { Checkbox } from "@/components/ui/checkbox"
import { cn } from "@/lib/utils"

type SkillsAccessMode = "none" | "all" | "specific"

interface SkillInfo {
  name: string
  description: string | null
  source_path: string
  script_count: number
  reference_count: number
  enabled: boolean
}

interface StepSkillsProps {
  accessMode: SkillsAccessMode
  selectedPaths: string[]
  onChange: (mode: SkillsAccessMode, paths: string[]) => void
}

export function StepSkills({ accessMode, selectedPaths, onChange }: StepSkillsProps) {
  const [skills, setSkills] = useState<SkillInfo[]>([])
  const [loading, setLoading] = useState(true)

  useEffect(() => {
    loadSkills()
  }, [])

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

  // Group skills by source_path
  const groupedSkills = skills.reduce<Record<string, SkillInfo[]>>((acc, skill) => {
    const key = skill.source_path
    if (!acc[key]) acc[key] = []
    acc[key].push(skill)
    return acc
  }, {})

  const includeAll = accessMode === "all"
  const selectedSet = new Set(selectedPaths)
  const selectedCount = includeAll
    ? sourcePaths.length
    : Array.from(selectedSet).filter(p => sourcePaths.includes(p)).length
  const isIndeterminate = !includeAll && selectedCount > 0 && selectedCount < sourcePaths.length

  const handleAllToggle = () => {
    if (includeAll) {
      // Switch from All to None
      onChange("none", [])
    } else {
      onChange("all", [])
    }
  }

  const handleSourcePathToggle = (sourcePath: string) => {
    if (includeAll) {
      // Switch from All to Specific, excluding this path
      const otherPaths = sourcePaths.filter(p => p !== sourcePath)
      onChange(otherPaths.length > 0 ? "specific" : "none", otherPaths)
      return
    }

    const newSelected = new Set(selectedSet)
    if (newSelected.has(sourcePath)) {
      newSelected.delete(sourcePath)
    } else {
      newSelected.add(sourcePath)
    }

    const allSelected = sourcePaths.length > 0 && sourcePaths.every(p => newSelected.has(p))
    if (allSelected) {
      onChange("all", [])
    } else {
      const paths = Array.from(newSelected)
      onChange(paths.length > 0 ? "specific" : "none", paths)
    }
  }

  const isPathSelected = (sourcePath: string): boolean => {
    if (includeAll) return true
    return selectedSet.has(sourcePath)
  }

  if (loading) {
    return (
      <div className="flex items-center justify-center h-32 text-muted-foreground text-sm">
        Loading skills...
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
      <div className="border rounded-lg">
        <div className="max-h-[350px] overflow-y-auto">
          {/* All Skills row */}
          <div
            className="flex items-center gap-3 px-4 py-3 border-b bg-background sticky top-0 z-10 cursor-pointer hover:bg-muted/50 transition-colors"
            onClick={handleAllToggle}
          >
            <Checkbox
              checked={includeAll || isIndeterminate}
              onCheckedChange={handleAllToggle}
              className={cn(
                "data-[state=checked]:bg-primary",
                isIndeterminate && "data-[state=checked]:bg-primary/60"
              )}
            />
            <span className="font-semibold text-sm">All Skills</span>
            <span className="text-xs text-muted-foreground ml-auto">
              {includeAll ? (
                <span className="text-primary">All (including future skills)</span>
              ) : (
                `${selectedCount} / ${sourcePaths.length} source${sourcePaths.length !== 1 ? "s" : ""} selected`
              )}
            </span>
          </div>

          {/* Skills grouped by source path */}
          {Object.entries(groupedSkills).map(([sourcePath, groupSkills]) => {
            const isSelected = isPathSelected(sourcePath)

            return (
              <div key={sourcePath}>
                <div
                  className={cn(
                    "flex items-center gap-3 px-4 py-2.5 border-b",
                    "hover:bg-muted/30 transition-colors cursor-pointer",
                    includeAll && "opacity-60"
                  )}
                  onClick={() => handleSourcePathToggle(sourcePath)}
                >
                  <Checkbox
                    checked={isSelected}
                    onCheckedChange={() => handleSourcePathToggle(sourcePath)}
                  />
                  <div className="flex-1 min-w-0">
                    <span className="text-sm font-medium truncate block" title={sourcePath}>
                      {sourcePath}
                    </span>
                    <p className="text-xs text-muted-foreground">
                      {groupSkills.length} skill{groupSkills.length !== 1 ? "s" : ""}
                    </p>
                  </div>
                </div>
                {groupSkills.map(skill => (
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
                  </div>
                ))}
              </div>
            )
          })}
        </div>
      </div>
    </div>
  )
}
