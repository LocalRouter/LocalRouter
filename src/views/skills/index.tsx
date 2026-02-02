import { useState, useEffect } from "react"
import { invoke } from "@tauri-apps/api/core"
import { listen } from "@tauri-apps/api/event"

import { toast } from "sonner"
import { RefreshCw, ExternalLink, ChevronDown, ChevronRight, FileText, FileCode, Image, Folder } from "lucide-react"
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

interface SkillFile {
  name: string
  category: string
  content_preview: string | null
}

interface SkillsViewProps {
  activeSubTab: string | null
  onTabChange: (view: string, subTab?: string | null) => void
}

export function SkillsView({ activeSubTab, onTabChange }: SkillsViewProps) {
  const [skills, setSkills] = useState<SkillInfo[]>([])
  const [loading, setLoading] = useState(true)
  const [rescanning, setRescanning] = useState(false)
  const [selectedSkill, setSelectedSkill] = useState<string | null>(activeSubTab)
  const [skillFiles, setSkillFiles] = useState<SkillFile[]>([])
  const [loadingFiles, setLoadingFiles] = useState(false)
  const [expandedFiles, setExpandedFiles] = useState<Set<string>>(new Set())

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

  useEffect(() => {
    if (selectedSkill) {
      loadSkillFiles(selectedSkill)
    } else {
      setSkillFiles([])
      setExpandedFiles(new Set())
    }
  }, [selectedSkill])

  const loadData = async () => {
    try {
      const skillList = await invoke<SkillInfo[]>("list_skills")
      setSkills(skillList)
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

  const handleOpenPath = async (path: string) => {
    try {
      await invoke("open_path", { path })
    } catch (error) {
      console.error("Failed to open path:", error)
      toast.error("Failed to open in file explorer")
    }
  }

  const loadSkillFiles = async (skillName: string) => {
    setLoadingFiles(true)
    try {
      const files = await invoke<SkillFile[]>("get_skill_files", { skillName })
      setSkillFiles(files)
    } catch (error) {
      console.error("Failed to load skill files:", error)
      setSkillFiles([])
    } finally {
      setLoadingFiles(false)
    }
  }

  const toggleFileExpanded = (fileName: string) => {
    setExpandedFiles(prev => {
      const next = new Set(prev)
      if (next.has(fileName)) {
        next.delete(fileName)
      } else {
        next.add(fileName)
      }
      return next
    })
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
        </div>
      </div>

      <div className="flex gap-6">
        {/* Left: Skills list */}
        <div className="w-[35%] space-y-4">
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
                  {skills.map((skill) => (
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

                {/* File Tree */}
                {loadingFiles ? (
                  <div className="text-xs text-muted-foreground border-t pt-3">
                    Loading files...
                  </div>
                ) : skillFiles.length > 0 && (
                  <div className="border-t pt-3">
                    <h4 className="text-xs font-medium text-muted-foreground mb-2">Files</h4>
                    <div className="rounded-md border border-border/50 bg-muted/20 py-1">
                      {(() => {
                        // Build a nested tree from flat relative paths
                        interface TreeNode {
                          name: string
                          fullPath: string
                          file?: SkillFile
                          children: Record<string, TreeNode>
                        }

                        const root: TreeNode = { name: "", fullPath: "", children: {} }
                        for (const file of skillFiles) {
                          const parts = file.name.split("/")
                          let node = root
                          for (let i = 0; i < parts.length; i++) {
                            const part = parts[i]
                            const partialPath = parts.slice(0, i + 1).join("/")
                            if (!node.children[part]) {
                              node.children[part] = { name: part, fullPath: partialPath, children: {} }
                            }
                            node = node.children[part]
                          }
                          node.file = file
                        }

                        const getFileIcon = (name: string) => {
                          const ext = name.split(".").pop()?.toLowerCase() ?? ""
                          if (name === "SKILL.md") return <FileText className="h-3.5 w-3.5 text-amber-500" />
                          if (["sh", "bash", "py", "js", "ts", "rb", "pl"].includes(ext)) return <FileCode className="h-3.5 w-3.5 text-blue-500" />
                          if (["md", "txt", "json", "yaml", "yml", "toml", "xml", "csv", "html", "css"].includes(ext)) return <FileText className="h-3.5 w-3.5 text-green-500" />
                          if (["png", "jpg", "jpeg", "gif", "svg", "webp", "ico"].includes(ext)) return <Image className="h-3.5 w-3.5 text-purple-500" />
                          return <FileText className="h-3.5 w-3.5 text-muted-foreground" />
                        }

                        const renderNode = (node: TreeNode, depth: number): React.ReactNode[] => {
                          const entries = Object.values(node.children)
                          // Sort: directories first, then files, alphabetically within each
                          entries.sort((a, b) => {
                            const aIsDir = Object.keys(a.children).length > 0 && !a.file
                            const bIsDir = Object.keys(b.children).length > 0 && !b.file
                            if (aIsDir !== bIsDir) return aIsDir ? -1 : 1
                            return a.name.localeCompare(b.name)
                          })

                          const results: React.ReactNode[] = []
                          const padLeft = 12 + depth * 16

                          for (const entry of entries) {
                            const isDir = Object.keys(entry.children).length > 0 && !entry.file
                            if (isDir) {
                              // Count total files under this directory
                              const countFiles = (n: TreeNode): number => {
                                let c = n.file ? 1 : 0
                                for (const child of Object.values(n.children)) c += countFiles(child)
                                return c
                              }
                              const fileCount = countFiles(entry)
                              const dirKey = `dir:${entry.fullPath}`
                              const isDirExpanded = expandedFiles.has(dirKey)

                              results.push(
                                <div key={dirKey}>
                                  <button
                                    className="w-full flex items-center gap-1.5 py-1 text-xs hover:bg-muted/50 transition-colors"
                                    style={{ paddingLeft: padLeft }}
                                    onClick={() => toggleFileExpanded(dirKey)}
                                  >
                                    {isDirExpanded
                                      ? <ChevronDown className="h-3 w-3 shrink-0 text-muted-foreground" />
                                      : <ChevronRight className="h-3 w-3 shrink-0 text-muted-foreground" />}
                                    <Folder className="h-3.5 w-3.5 text-muted-foreground" />
                                    <span className="font-medium">{entry.name}/</span>
                                    <span className="text-muted-foreground">({fileCount})</span>
                                  </button>
                                  {isDirExpanded && renderNode(entry, depth + 1)}
                                </div>
                              )
                            } else if (entry.file) {
                              const file = entry.file
                              results.push(
                                <div key={file.name}>
                                  <button
                                    className="w-full flex items-center gap-1.5 py-1 pr-3 text-xs hover:bg-muted/50 transition-colors"
                                    style={{ paddingLeft: padLeft }}
                                    onClick={() => file.content_preview && toggleFileExpanded(file.name)}
                                  >
                                    {file.content_preview ? (
                                      expandedFiles.has(file.name)
                                        ? <ChevronDown className="h-3 w-3 shrink-0 text-muted-foreground" />
                                        : <ChevronRight className="h-3 w-3 shrink-0 text-muted-foreground" />
                                    ) : <div className="w-3" />}
                                    {getFileIcon(entry.name)}
                                    <span>{entry.name}</span>
                                  </button>
                                  {expandedFiles.has(file.name) && file.content_preview && (
                                    <pre
                                      className="mr-3 mb-1 px-3 py-2 text-[10px] leading-relaxed bg-muted/30 rounded border border-border/50 overflow-x-auto max-h-48 whitespace-pre-wrap break-words"
                                      style={{ marginLeft: padLeft + 18 }}
                                    >
                                      {file.content_preview}
                                    </pre>
                                  )}
                                </div>
                              )
                            }
                          }
                          return results
                        }

                        return renderNode(root, 0)
                      })()}
                    </div>
                  </div>
                )}

                {/* Source path */}
                <div className="flex items-center justify-between text-xs text-muted-foreground border-t pt-3">
                  <span className="truncate" title={selectedSkillInfo.source_path}>
                    Source: {selectedSkillInfo.source_path}
                  </span>
                  <Button
                    variant="ghost"
                    size="sm"
                    className="h-6 text-xs shrink-0"
                    onClick={() => handleOpenPath(selectedSkillInfo.source_path)}
                  >
                    <ExternalLink className="h-3 w-3 mr-1" />
                    Open folder
                  </Button>
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
