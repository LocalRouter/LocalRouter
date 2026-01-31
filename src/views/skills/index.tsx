import { useState, useEffect } from "react"
import { invoke } from "@tauri-apps/api/core"
import { listen } from "@tauri-apps/api/event"
import { toast } from "sonner"
import { Plus, FolderOpen, RefreshCw, Trash2 } from "lucide-react"
import { Button } from "@/components/ui/Button"
import { Card, CardContent, CardHeader, CardTitle, CardDescription } from "@/components/ui/Card"
import { Switch } from "@/components/ui/switch"

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

interface SkillsConfig {
  paths: string[]
  disabled_skills: string[]
}

interface SkillsViewProps {
  activeSubTab: string | null
  onTabChange: (view: string, subTab?: string | null) => void
}

export function SkillsView({ activeSubTab, onTabChange }: SkillsViewProps) {
  const [skills, setSkills] = useState<SkillInfo[]>([])
  const [config, setConfig] = useState<SkillsConfig | null>(null)
  const [loading, setLoading] = useState(true)
  const [rescanning, setRescanning] = useState(false)
  const [selectedSkill, setSelectedSkill] = useState<string | null>(activeSubTab)
  const [addMode, setAddMode] = useState(false)
  const [newPath, setNewPath] = useState("")

  useEffect(() => {
    loadData()

    const unsubscribe = listen("skills-changed", () => {
      loadData()
    })

    return () => {
      unsubscribe.then((fn) => fn())
    }
  }, [])

  useEffect(() => {
    setSelectedSkill(activeSubTab)
  }, [activeSubTab])

  const loadData = async () => {
    try {
      const [skillList, skillsConfig] = await Promise.all([
        invoke<SkillInfo[]>("list_skills"),
        invoke<SkillsConfig>("get_skills_config"),
      ])
      setSkills(skillList)
      setConfig(skillsConfig)
    } catch (error) {
      console.error("Failed to load skills:", error)
    } finally {
      setLoading(false)
    }
  }

  const handleRescan = async () => {
    setRescanning(true)
    try {
      const result = await invoke<SkillInfo[]>("rescan_skills")
      setSkills(result)
      toast.success(`Found ${result.length} skill(s)`)
    } catch (error) {
      console.error("Failed to rescan skills:", error)
      toast.error("Failed to rescan skills")
    } finally {
      setRescanning(false)
    }
  }

  const handleAddPath = async () => {
    const trimmed = newPath.trim()
    if (!trimmed) return

    try {
      await invoke("add_skill_source", { path: trimmed })
      toast.success("Skill source added")
      setNewPath("")
      setAddMode(false)
      loadData()
    } catch (error) {
      console.error("Failed to add path:", error)
      toast.error(`Failed to add source: ${error}`)
    }
  }

  const handleRemoveSource = async (path: string) => {
    try {
      await invoke("remove_skill_source", { path })
      toast.success("Skill source removed")
      loadData()
    } catch (error) {
      console.error("Failed to remove skill source:", error)
      toast.error("Failed to remove source")
    }
  }

  const handleToggleEnabled = async (skillName: string, enabled: boolean) => {
    try {
      await invoke("set_skill_enabled", { skillName, enabled })
      toast.success(enabled ? "Skill enabled" : "Skill disabled")
      loadData()
    } catch (error) {
      console.error("Failed to toggle skill:", error)
      toast.error("Failed to update skill")
    }
  }

  // Group skills by source_path
  const groupedSkills = skills.reduce<Record<string, SkillInfo[]>>((acc, skill) => {
    const key = skill.source_path
    if (!acc[key]) acc[key] = []
    acc[key].push(skill)
    return acc
  }, {})

  const selectedSkillInfo = skills.find(s => s.name === selectedSkill)

  if (loading) {
    return (
      <div className="flex items-center justify-center h-64">
        <p className="text-muted-foreground">Loading skills...</p>
      </div>
    )
  }

  return (
    <div className="space-y-6">
      {/* Header */}
      <div className="flex items-center justify-between">
        <div>
          <h1 className="text-2xl font-bold">Skills</h1>
          <p className="text-sm text-muted-foreground">
            Manage AgentSkills.io skill packages
          </p>
        </div>
        <div className="flex items-center gap-2">
          <Button
            variant="outline"
            size="sm"
            onClick={handleRescan}
            disabled={rescanning}
          >
            <RefreshCw className={`h-4 w-4 mr-2 ${rescanning ? "animate-spin" : ""}`} />
            Rescan
          </Button>
          <Button size="sm" onClick={() => { setAddMode(true); setNewPath("") }}>
            <Plus className="h-4 w-4 mr-2" />
            Add Skill Source
          </Button>
        </div>
      </div>

      {/* Add path input */}
      {addMode && (
        <Card>
          <CardContent className="pt-4">
            <div className="flex items-center gap-2">
              <label className="text-sm font-medium shrink-0">
                Source Path:
              </label>
              <input
                type="text"
                value={newPath}
                onChange={(e) => setNewPath(e.target.value)}
                onKeyDown={(e) => { if (e.key === "Enter") handleAddPath(); if (e.key === "Escape") setAddMode(false) }}
                placeholder="/path/to/skills/directory or /path/to/skill.zip"
                className="flex-1 h-8 rounded-md border border-input bg-background px-3 py-1 text-sm"
                autoFocus
              />
              <Button size="sm" onClick={handleAddPath} disabled={!newPath.trim()}>
                Add
              </Button>
              <Button size="sm" variant="outline" onClick={() => setAddMode(false)}>
                Cancel
              </Button>
            </div>
            <p className="text-xs text-muted-foreground mt-1.5">
              Path to a skill directory (with SKILL.md), a directory of skills, or a .zip/.skill file
            </p>
          </CardContent>
        </Card>
      )}

      <div className="flex gap-6">
        {/* Left: Skills list */}
        <div className="w-[35%] space-y-4">
          {/* Configured paths */}
          {config && config.paths.length > 0 && (
            <Card>
              <CardHeader className="pb-3">
                <CardTitle className="text-sm">Configured Sources</CardTitle>
              </CardHeader>
              <CardContent className="space-y-2">
                {config.paths.map((p) => (
                  <div key={p} className="flex items-center justify-between text-xs group">
                    <div className="flex items-center gap-1.5 min-w-0">
                      <FolderOpen className="h-3 w-3 text-muted-foreground shrink-0" />
                      <span className="truncate text-muted-foreground" title={p}>{p}</span>
                    </div>
                    <Button
                      variant="ghost"
                      size="icon"
                      className="h-5 w-5 opacity-0 group-hover:opacity-100"
                      onClick={() => handleRemoveSource(p)}
                    >
                      <Trash2 className="h-3 w-3" />
                    </Button>
                  </div>
                ))}
              </CardContent>
            </Card>
          )}

          {/* Skills list */}
          <Card>
            <CardHeader className="pb-3">
              <CardTitle className="text-sm">Discovered Skills ({skills.length})</CardTitle>
            </CardHeader>
            <CardContent>
              {skills.length === 0 ? (
                <p className="text-sm text-muted-foreground text-center py-4">
                  No skills found. Add a skill source to get started.
                </p>
              ) : (
                <div className="space-y-1">
                  {Object.entries(groupedSkills).map(([sourcePath, groupSkills]) => (
                    <div key={sourcePath}>
                      {Object.keys(groupedSkills).length > 1 && (
                        <div className="text-[10px] text-muted-foreground font-medium px-2 py-1 truncate" title={sourcePath}>
                          {sourcePath}
                        </div>
                      )}
                      {groupSkills.map((skill) => (
                        <button
                          key={skill.name}
                          onClick={() => {
                            setSelectedSkill(skill.name)
                            onTabChange("skills", skill.name)
                          }}
                          className={`w-full text-left px-3 py-2 rounded-md text-sm transition-colors ${
                            selectedSkill === skill.name
                              ? "bg-accent text-accent-foreground"
                              : "hover:bg-muted/50"
                          } ${!skill.enabled ? "opacity-50" : ""}`}
                        >
                          <div className="font-medium">{skill.name}</div>
                          {skill.description && (
                            <div className="text-xs text-muted-foreground truncate">
                              {skill.description}
                            </div>
                          )}
                        </button>
                      ))}
                    </div>
                  ))}
                </div>
              )}
            </CardContent>
          </Card>
        </div>

        {/* Right: Skill detail */}
        <div className="flex-1">
          {selectedSkillInfo ? (
            <Card>
              <CardHeader>
                <div className="flex items-center justify-between">
                  <div>
                    <CardTitle>{selectedSkillInfo.name}</CardTitle>
                    {selectedSkillInfo.description && (
                      <CardDescription>{selectedSkillInfo.description}</CardDescription>
                    )}
                  </div>
                  <div className="flex items-center gap-2">
                    <span className="text-xs text-muted-foreground">
                      {selectedSkillInfo.enabled ? "Enabled" : "Disabled"}
                    </span>
                    <Switch
                      checked={selectedSkillInfo.enabled}
                      onCheckedChange={(checked) => handleToggleEnabled(selectedSkillInfo.name, checked)}
                    />
                  </div>
                </div>
              </CardHeader>
              <CardContent className="space-y-4">
                {/* Metadata */}
                <div className="grid grid-cols-2 gap-3 text-sm">
                  {selectedSkillInfo.version && (
                    <div>
                      <span className="text-muted-foreground">Version:</span>{" "}
                      <span className="font-medium">{selectedSkillInfo.version}</span>
                    </div>
                  )}
                  {selectedSkillInfo.author && (
                    <div>
                      <span className="text-muted-foreground">Author:</span>{" "}
                      <span className="font-medium">{selectedSkillInfo.author}</span>
                    </div>
                  )}
                </div>

                {/* Tags */}
                {selectedSkillInfo.tags.length > 0 && (
                  <div className="flex flex-wrap gap-1.5">
                    {selectedSkillInfo.tags.map((tag) => (
                      <span
                        key={tag}
                        className="text-xs px-2 py-0.5 rounded-full bg-muted text-muted-foreground"
                      >
                        {tag}
                      </span>
                    ))}
                  </div>
                )}

                {/* Capabilities */}
                <div className="flex gap-2">
                  {selectedSkillInfo.script_count > 0 && (
                    <span className="text-xs px-2 py-1 rounded bg-blue-500/10 text-blue-600 dark:text-blue-400">
                      {selectedSkillInfo.script_count} Script{selectedSkillInfo.script_count > 1 ? "s" : ""}
                    </span>
                  )}
                  {selectedSkillInfo.reference_count > 0 && (
                    <span className="text-xs px-2 py-1 rounded bg-green-500/10 text-green-600 dark:text-green-400">
                      {selectedSkillInfo.reference_count} Reference{selectedSkillInfo.reference_count > 1 ? "s" : ""}
                    </span>
                  )}
                  {selectedSkillInfo.asset_count > 0 && (
                    <span className="text-xs px-2 py-1 rounded bg-purple-500/10 text-purple-600 dark:text-purple-400">
                      {selectedSkillInfo.asset_count} Asset{selectedSkillInfo.asset_count > 1 ? "s" : ""}
                    </span>
                  )}
                </div>

                {/* Source path */}
                <div className="text-xs text-muted-foreground border-t pt-3">
                  Source: {selectedSkillInfo.source_path}
                </div>
              </CardContent>
            </Card>
          ) : (
            <div className="flex items-center justify-center h-64 text-muted-foreground text-sm">
              Select a skill to view details
            </div>
          )}
        </div>
      </div>
    </div>
  )
}
