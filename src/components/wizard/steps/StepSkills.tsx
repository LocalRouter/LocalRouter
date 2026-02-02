/**
 * Step 4: Select Skills
 *
 * Skills access selection for the client.
 * Supports All / Specific / None access modes with per-skill selection.
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
  selectedSkills: string[]
  onChange: (mode: SkillsAccessMode, skills: string[]) => void
}

export function StepSkills({ accessMode, selectedSkills, onChange }: StepSkillsProps) {
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

  const enabledSkills = skills.filter(s => s.enabled)
  const skillNames = enabledSkills.map(s => s.name)

  const includeAll = accessMode === "all"
  const selectedSet = new Set(selectedSkills)
  const selectedCount = includeAll
    ? skillNames.length
    : Array.from(selectedSet).filter(n => skillNames.includes(n)).length
  const isIndeterminate = !includeAll && selectedCount > 0 && selectedCount < skillNames.length

  const handleAllToggle = () => {
    if (includeAll) {
      // Switch from All to None
      onChange("none", [])
    } else {
      onChange("all", [])
    }
  }

  const handleSkillToggle = (skillName: string) => {
    if (includeAll) {
      // Switch from All to Specific, excluding this skill
      const otherSkills = skillNames.filter(n => n !== skillName)
      onChange(otherSkills.length > 0 ? "specific" : "none", otherSkills)
      return
    }

    const newSelected = new Set(selectedSet)
    if (newSelected.has(skillName)) {
      newSelected.delete(skillName)
    } else {
      newSelected.add(skillName)
    }

    const allSelected = skillNames.length > 0 && skillNames.every(n => newSelected.has(n))
    if (allSelected) {
      onChange("all", [])
    } else {
      const names = Array.from(newSelected)
      onChange(names.length > 0 ? "specific" : "none", names)
    }
  }

  const isSkillSelected = (skillName: string): boolean => {
    if (includeAll) return true
    return selectedSet.has(skillName)
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
                `${selectedCount} / ${skillNames.length} skill${skillNames.length !== 1 ? "s" : ""} selected`
              )}
            </span>
          </div>

          {/* Individual skills */}
          {skills.map((skill) => {
            const isSelected = isSkillSelected(skill.name)

            return (
              <div
                key={skill.name}
                className={cn(
                  "flex items-center gap-3 px-4 py-2.5 border-b",
                  "hover:bg-muted/30 transition-colors",
                  skill.enabled ? "cursor-pointer" : "",
                  !skill.enabled && "opacity-40",
                  includeAll && skill.enabled && "opacity-60"
                )}
                onClick={() => skill.enabled && handleSkillToggle(skill.name)}
              >
                <Checkbox
                  checked={isSelected}
                  onCheckedChange={() => handleSkillToggle(skill.name)}
                  disabled={!skill.enabled}
                />
                <div className="flex-1 min-w-0">
                  <span className="text-sm font-medium">{skill.name}</span>
                  {skill.description && (
                    <p className="text-xs text-muted-foreground truncate">
                      {skill.description}
                    </p>
                  )}
                </div>
              </div>
            )
          })}
        </div>
      </div>
    </div>
  )
}
