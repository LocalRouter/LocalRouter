
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
  skills_names: string[]
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
  const [selectedSkills, setSelectedSkills] = useState<Set<string>>(
    new Set(client.skills_names)
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
    setSelectedSkills(new Set(client.skills_names))
  }, [client.skills_access_mode, client.skills_names])

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

  const handleAllSkillsToggle = async () => {
    try {
      setSaving(true)
      const newIncludeAll = !includeAllSkills

      if (newIncludeAll) {
        await invoke("set_client_skills_access", {
          clientId: client.client_id,
          mode: "all",
          skillNames: [],
        })
        setIncludeAllSkills(true)
        toast.success("All skills enabled")
      } else {
        const mode = selectedSkills.size > 0 ? "specific" : "none"
        await invoke("set_client_skills_access", {
          clientId: client.client_id,
          mode,
          skillNames: Array.from(selectedSkills),
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

  const handleSkillToggle = async (skillName: string) => {
    if (includeAllSkills) {
      // Switch from All to Specific, excluding this skill
      try {
        setSaving(true)
        const otherSkills = skillNames.filter(n => n !== skillName)

        await invoke("set_client_skills_access", {
          clientId: client.client_id,
          mode: otherSkills.length > 0 ? "specific" : "none",
          skillNames: otherSkills,
        })

        setIncludeAllSkills(false)
        setSelectedSkills(new Set(otherSkills))
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
      const newSelected = new Set(selectedSkills)

      if (newSelected.has(skillName)) {
        newSelected.delete(skillName)
      } else {
        newSelected.add(skillName)
      }

      const allSelected = skillNames.length > 0 && skillNames.every(n => newSelected.has(n))

      if (allSelected) {
        await invoke("set_client_skills_access", {
          clientId: client.client_id,
          mode: "all",
          skillNames: [],
        })
        setIncludeAllSkills(true)
        setSelectedSkills(newSelected)
        toast.success("All skills enabled")
      } else {
        const mode = newSelected.size > 0 ? "specific" : "none"
        await invoke("set_client_skills_access", {
          clientId: client.client_id,
          mode,
          skillNames: Array.from(newSelected),
        })
        setSelectedSkills(newSelected)
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
    ? skillNames.length
    : Array.from(selectedSkills).filter((n) =>
        skillNames.includes(n)
      ).length

  const isIndeterminate = !includeAllSkills && selectedCount > 0 && selectedCount < skillNames.length

  const isSkillSelected = (skillName: string): boolean => {
    if (includeAllSkills) return true
    return selectedSkills.has(skillName)
  }

  return (
    <div className="space-y-6">
      <Card>
        <CardHeader>
          <CardTitle>Skills Access</CardTitle>
          <CardDescription>
            Select which skills this client can access via MCP tools
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
                      `${selectedCount} / ${skillNames.length} skill${skillNames.length !== 1 ? "s" : ""} selected`
                    )}
                  </span>
                </div>

                {/* Individual skills */}
                {skills.map((skill) => {
                  const isSelected = isSkillSelected(skill.name)
                  const canToggle = !saving && skill.enabled

                  return (
                    <div
                      key={skill.name}
                      className={cn(
                        "flex items-center gap-3 px-4 py-2.5 border-b",
                        "hover:bg-muted/30 transition-colors",
                        canToggle ? "cursor-pointer" : "",
                        !skill.enabled && "opacity-40",
                        includeAllSkills && skill.enabled && "opacity-60"
                      )}
                      onClick={() => canToggle && handleSkillToggle(skill.name)}
                    >
                      <Checkbox
                        checked={isSelected}
                        onCheckedChange={() => handleSkillToggle(skill.name)}
                        disabled={!canToggle}
                      />
                      <div className="flex-1 min-w-0">
                        <span className="text-sm font-medium">{skill.name}</span>
                        {skill.description && (
                          <p className="text-xs text-muted-foreground truncate">
                            {skill.description}
                          </p>
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
