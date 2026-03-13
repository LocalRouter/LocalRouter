import { useState, useEffect } from "react"
import { invoke } from "@tauri-apps/api/core"
import { listen } from "@tauri-apps/api/event"
import { open } from "@tauri-apps/plugin-dialog"

import { toast } from "sonner"
import { RefreshCw, ExternalLink, ChevronDown, ChevronRight, FileText, FileCode, Image, Folder, FlaskConical, Play, BookOpen, Plus, Trash2, FolderOpen, Loader2, Store, FilePlus } from "lucide-react"
import { SkillsIcon } from "@/components/icons/category-icons"
import { Button } from "@/components/ui/Button"
import { Card, CardContent, CardHeader, CardTitle, CardDescription } from "@/components/ui/Card"
import { MarketplaceSearchPanel, type SkillListing } from "@/components/add-resource/MarketplaceSearchPanel"
import { Textarea } from "@/components/ui/textarea"
import { Label } from "@/components/ui/label"
import { Switch } from "@/components/ui/switch"
import { Input } from "@/components/ui/Input"
import { ScrollArea } from "@/components/ui/scroll-area"
import { Tabs, TabsList, TabsTrigger, TabsContent } from "@/components/ui/tabs"
import {
  ResizablePanelGroup,
  ResizablePanel,
  ResizableHandle,
} from "@/components/ui/resizable"
import {
  Dialog,
  DialogContent,
  DialogHeader,
  DialogTitle,
  DialogDescription,
} from "@/components/ui/Modal"
import {
  AlertDialog,
  AlertDialogAction,
  AlertDialogCancel,
  AlertDialogContent,
  AlertDialogDescription,
  AlertDialogFooter,
  AlertDialogHeader,
  AlertDialogTitle,
  AlertDialogTrigger,
} from "@/components/ui/alert-dialog"
import { McpTab } from "@/views/try-it-out/mcp-tab"
import { cn } from "@/lib/utils"

interface SkillInfo {
  name: string
  description: string | null
  version: string | null
  author: string | null
  tags: string[]
  extra: Record<string, unknown>
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

interface SkillsConfig {
  paths: string[]
  disabled_skills: string[]
  async_enabled: boolean
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
  const [search, setSearch] = useState("")
  const [detailTab, setDetailTab] = useState("info")

  // Add skills dialog state
  const [showAddDialog, setShowAddDialog] = useState(false)
  const [addDialogTab, setAddDialogTab] = useState<"paths" | "marketplace" | "new">("paths")
  const [skillPaths, setSkillPaths] = useState<string[]>([])
  const [newPath, setNewPath] = useState("")
  const [addingPath, setAddingPath] = useState(false)
  const [removingPath, setRemovingPath] = useState<string | null>(null)
  const [isMarketplaceSkill, setIsMarketplaceSkill] = useState(false)
  const [isUserCreatedSkill, setIsUserCreatedSkill] = useState(false)
  const [isDeleting, setIsDeleting] = useState(false)

  // New skill form state
  const [newSkillName, setNewSkillName] = useState("")
  const [newSkillDescription, setNewSkillDescription] = useState("")
  const [newSkillContent, setNewSkillContent] = useState("")
  const [isCreatingSkill, setIsCreatingSkill] = useState(false)

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

  // Reset state when selected skill changes
  useEffect(() => {
    if (selectedSkill) {
      loadSkillFiles(selectedSkill)
    } else {
      setSkillFiles([])
      setExpandedFiles(new Set())
      setIsMarketplaceSkill(false)
      setIsUserCreatedSkill(false)
    }
    setDetailTab("info")
  }, [selectedSkill])

  // Check marketplace/user-created status when skills list updates (separate to avoid resetting tab)
  useEffect(() => {
    if (selectedSkill) {
      const skillInfo = skills.find(s => s.name === selectedSkill)
      if (skillInfo) {
        invoke<boolean>("marketplace_is_skill_from_marketplace", {
          skillPath: skillInfo.source_path,
        }).then(setIsMarketplaceSkill).catch(() => setIsMarketplaceSkill(false))
        invoke<boolean>("is_user_created_skill", {
          skillPath: skillInfo.source_path,
        }).then(setIsUserCreatedSkill).catch(() => setIsUserCreatedSkill(false))
      }
    }
  }, [selectedSkill, skills])

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

  const handleDeleteSkill = async (skillName: string, skillPath: string) => {
    setIsDeleting(true)
    try {
      if (isUserCreatedSkill) {
        await invoke("delete_user_skill", { skillName, skillPath })
      } else {
        await invoke("marketplace_delete_skill", { skillName, skillPath })
      }
      toast.success(`Skill "${skillName}" deleted`)
      setSelectedSkill(null)
      onTabChange("skills", null)
      await loadData()
    } catch (error) {
      console.error("Failed to delete skill:", error)
      toast.error(`Failed to delete skill: ${error}`)
    } finally {
      setIsDeleting(false)
    }
  }

  const loadSkillsConfig = async () => {
    try {
      const config = await invoke<SkillsConfig>("get_skills_config")
      setSkillPaths(config.paths)
    } catch (error) {
      console.error("Failed to load skills config:", error)
    }
  }

  const handleOpenAddDialog = async (tab?: "paths" | "marketplace" | "new") => {
    await loadSkillsConfig()
    if (tab) setAddDialogTab(tab)
    setShowAddDialog(true)
  }

  const handleSelectMarketplaceSkill = async (listing: SkillListing) => {
    try {
      await invoke("marketplace_install_skill_direct", {
        sourceUrl: listing.skill_md_url,
        skillName: listing.name,
      })
      toast.success(`Skill "${listing.name}" installed`)
      setShowAddDialog(false)
      await loadData()
    } catch (error) {
      console.error("Failed to install skill:", error)
      toast.error(`Failed to install skill: ${error}`)
    }
  }

  const handleCreateSkill = async (e: React.FormEvent) => {
    e.preventDefault()
    if (!newSkillName.trim()) {
      toast.error("Skill name is required")
      return
    }

    setIsCreatingSkill(true)
    try {
      await invoke("create_skill", {
        name: newSkillName.trim(),
        description: newSkillDescription.trim() || null,
        content: newSkillContent,
      })
      toast.success(`Skill "${newSkillName.trim()}" created`)
      setNewSkillName("")
      setNewSkillDescription("")
      setNewSkillContent("")
      setShowAddDialog(false)
      await loadData()
    } catch (error) {
      console.error("Failed to create skill:", error)
      toast.error(`Failed to create skill: ${error}`)
    } finally {
      setIsCreatingSkill(false)
    }
  }

  const handleAddPath = async (path: string) => {
    if (!path.trim()) return

    setAddingPath(true)
    try {
      await invoke("add_skill_source", { path: path.trim() })
      toast.success("Skill path added")
      setNewPath("")
      await loadSkillsConfig()
    } catch (error) {
      console.error("Failed to add skill path:", error)
      toast.error("Failed to add skill path")
    } finally {
      setAddingPath(false)
    }
  }

  const handleRemovePath = async (path: string) => {
    setRemovingPath(path)
    try {
      await invoke("remove_skill_source", { path })
      toast.success("Skill path removed")
      await loadSkillsConfig()
    } catch (error) {
      console.error("Failed to remove skill path:", error)
      toast.error("Failed to remove skill path")
    } finally {
      setRemovingPath(null)
    }
  }

  const handleBrowse = async () => {
    try {
      const selected = await open({
        directory: true,
        multiple: false,
        title: "Select Skill Folder or File",
      })
      if (selected && typeof selected === "string") {
        await handleAddPath(selected)
      }
    } catch (error) {
      console.error("Failed to open browse dialog:", error)
    }
  }

  const selectedSkillInfo = skills.find(s => s.name === selectedSkill)

  const filteredSkills = skills
    .filter((s) =>
      s.name.toLowerCase().includes(search.toLowerCase()) ||
      (s.description && s.description.toLowerCase().includes(search.toLowerCase()))
    )
    .sort((a, b) => a.name.toLowerCase().localeCompare(b.name.toLowerCase()))

  return (
    <div className="flex flex-col h-full min-h-0 max-w-5xl">
      <div className="flex-shrink-0 pb-4">
        <h1 className="text-2xl font-bold tracking-tight flex items-center gap-2">
          <SkillsIcon className="h-6 w-6" />
          Skills
        </h1>
        <p className="text-sm text-muted-foreground">
          Install and manage skill packages from AgentSkills.io. Skills are exposed as MCP tools and prompts through the unified MCP gateway.
        </p>
      </div>

      <div className="flex-1 min-h-0 mt-4">
      <ResizablePanelGroup direction="horizontal" className="flex-1 min-h-0 rounded-lg border">
        {/* List Panel */}
        <ResizablePanel defaultSize={21} minSize={15}>
          <div className="flex flex-col h-full">
            <div className="p-4 border-b">
              <div className="flex items-center gap-2">
                <Input
                  placeholder="Search skills..."
                  value={search}
                  onChange={(e) => setSearch(e.target.value)}
                  className="flex-1"
                />
                <Button size="icon" onClick={() => handleOpenAddDialog()} title="Add skill">
                  <Plus className="h-4 w-4" />
                </Button>
              </div>
            </div>
            <ScrollArea className="flex-1">
              <div className="p-2 space-y-1">
                {loading ? (
                  <p className="text-sm text-muted-foreground p-4">Loading...</p>
                ) : filteredSkills.length === 0 ? (
                  <p className="text-sm text-muted-foreground p-4">No skills found</p>
                ) : (
                  filteredSkills.map((skill) => (
                    <div
                      key={skill.name}
                      onClick={() => {
                        setSelectedSkill(skill.name)
                        onTabChange("skills", skill.name)
                      }}
                      className={cn(
                        "flex items-center gap-3 p-3 rounded-md cursor-pointer",
                        selectedSkill === skill.name
                          ? "bg-accent"
                          : "hover:bg-muted",
                        !skill.enabled && "opacity-50"
                      )}
                    >
                      <div className="flex-1 min-w-0">
                        <p className="font-medium truncate">{skill.name}</p>
                        {skill.description && (
                          <p className="text-xs text-muted-foreground truncate">
                            {skill.description}
                          </p>
                        )}
                      </div>
                      {!skill.enabled && (
                        <span className="text-xs text-muted-foreground shrink-0">Disabled</span>
                      )}
                    </div>
                  ))
                )}
              </div>
            </ScrollArea>
          </div>
        </ResizablePanel>

        <ResizableHandle withHandle />

        {/* Detail Panel */}
        <ResizablePanel defaultSize={79}>
          {selectedSkillInfo ? (
            <ScrollArea className="h-full">
              <div className="p-6 space-y-6">
                {/* Header */}
                <div className="flex items-start justify-between">
                  <div>
                    <h2 className="text-xl font-bold">{selectedSkillInfo.name}</h2>
                    {selectedSkillInfo.description && (
                      <p className="text-sm text-muted-foreground">{selectedSkillInfo.description}</p>
                    )}
                  </div>
                  <div className="flex items-center gap-2">
                    <Button
                      variant="outline"
                      size="sm"
                      onClick={() => setDetailTab("try-it-out")}
                    >
                      <FlaskConical className="h-4 w-4 mr-1" />
                      Try It Out
                    </Button>
                  </div>
                </div>

                <Tabs value={detailTab} onValueChange={setDetailTab}>
                  <TabsList>
                    <TabsTrigger value="info">Info</TabsTrigger>
                    <TabsTrigger value="try-it-out">Try It Out</TabsTrigger>
                    <TabsTrigger value="settings">Settings</TabsTrigger>
                  </TabsList>

                  <TabsContent value="info">
                    <div className="space-y-6">
                      {/* Metadata */}
                      <Card>
                        <CardHeader className="pb-3">
                          <CardTitle className="text-sm">Details</CardTitle>
                        </CardHeader>
                        <CardContent className="space-y-4">
                          <div className="grid grid-cols-2 gap-3 text-sm">
                            {selectedSkillInfo.author && (
                              <div>
                                <span className="text-muted-foreground">Author:</span>{" "}
                                <span className="font-medium">{selectedSkillInfo.author}</span>
                              </div>
                            )}
                            {selectedSkillInfo.version && (
                              <div>
                                <span className="text-muted-foreground">Version:</span>{" "}
                                <span className="font-medium">{selectedSkillInfo.version}</span>
                              </div>
                            )}
                            {Object.entries(selectedSkillInfo.extra)
                              .sort(([a], [b]) => a.localeCompare(b))
                              .map(([key, value]) => (
                              <div key={key}>
                                <span className="text-muted-foreground">{key}:</span>{" "}
                                <span className="font-medium">{typeof value === "object" ? JSON.stringify(value) : String(value)}</span>
                              </div>
                            ))}
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
                              <span className="text-xs px-2 py-1 rounded bg-blue-500/10 text-blue-800 dark:text-blue-400 flex items-center gap-1">
                                <Play className="h-3 w-3" />
                                {selectedSkillInfo.script_count} Executable{selectedSkillInfo.script_count > 1 ? "s" : ""}
                              </span>
                            )}
                            {(selectedSkillInfo.reference_count > 0 || selectedSkillInfo.asset_count > 0) && (
                              <span className="text-xs px-2 py-1 rounded bg-green-500/10 text-green-800 dark:text-green-400 flex items-center gap-1">
                                <BookOpen className="h-3 w-3" />
                                {selectedSkillInfo.reference_count + selectedSkillInfo.asset_count} Resource{selectedSkillInfo.reference_count + selectedSkillInfo.asset_count > 1 ? "s" : ""}
                              </span>
                            )}
                          </div>
                        </CardContent>
                      </Card>

                      {/* File Tree */}
                      {loadingFiles ? (
                        <Card>
                          <CardContent className="py-4">
                            <p className="text-xs text-muted-foreground">Loading files...</p>
                          </CardContent>
                        </Card>
                      ) : skillFiles.length > 0 && (
                        <Card>
                          <CardHeader className="pb-3">
                            <CardTitle className="text-sm">Files</CardTitle>
                          </CardHeader>
                          <CardContent>
                            <div className="rounded-md border border-border/50 bg-muted/20 py-1">
                              {(() => {
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

                                const getFileIcon = (name: string, category: string) => {
                                  if (category === "skill_md") return <FileText className="h-3.5 w-3.5 text-amber-500" />
                                  if (category === "script") return <Play className="h-3.5 w-3.5 text-blue-500" />
                                  if (category === "reference") return <BookOpen className="h-3.5 w-3.5 text-green-500" />
                                  if (category === "asset") return <Image className="h-3.5 w-3.5 text-purple-500" />
                                  const ext = name.split(".").pop()?.toLowerCase() ?? ""
                                  if (["sh", "bash", "py", "js", "ts", "rb", "pl"].includes(ext)) return <FileCode className="h-3.5 w-3.5 text-muted-foreground" />
                                  if (["png", "jpg", "jpeg", "gif", "svg", "webp", "ico"].includes(ext)) return <Image className="h-3.5 w-3.5 text-muted-foreground" />
                                  return <FileText className="h-3.5 w-3.5 text-muted-foreground" />
                                }

                                const getCategoryBadge = (category: string) => {
                                  if (category === "skill_md") return <span className="text-[9px] px-1 py-0.5 rounded bg-amber-500/10 text-amber-800 dark:text-amber-400 font-medium uppercase">Definition</span>
                                  if (category === "script") return <span className="text-[9px] px-1 py-0.5 rounded bg-blue-500/10 text-blue-800 dark:text-blue-400 font-medium uppercase">Executable</span>
                                  if (category === "reference") return <span className="text-[9px] px-1 py-0.5 rounded bg-green-500/10 text-green-800 dark:text-green-400 font-medium uppercase">Resource</span>
                                  if (category === "asset") return <span className="text-[9px] px-1 py-0.5 rounded bg-purple-500/10 text-purple-800 dark:text-purple-400 font-medium uppercase">Resource</span>
                                  return null
                                }

                                const renderNode = (node: TreeNode, depth: number): React.ReactNode[] => {
                                  const entries = Object.values(node.children)
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
                                      const badge = getCategoryBadge(file.category)
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
                                            {getFileIcon(entry.name, file.category)}
                                            <span>{entry.name}</span>
                                            {badge}
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
                          </CardContent>
                        </Card>
                      )}

                      {/* Source path */}
                      <div className="flex items-center justify-between text-xs text-muted-foreground">
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
                    </div>
                  </TabsContent>

                  <TabsContent value="try-it-out">
                    <McpTab
                      initialMode="direct"
                      initialDirectTarget={`skill:${selectedSkillInfo.name}`}
                      hideModeSwitcher
                      hideDirectTargetSelector
                      innerPath={null}
                      onPathChange={() => {}}
                    />
                  </TabsContent>

                  <TabsContent value="settings">
                    <div className="space-y-6">
                      {/* Danger Zone */}
                      <Card className="border-red-200 dark:border-red-900">
                        <CardHeader>
                          <CardTitle className="text-red-600 dark:text-red-400">Danger Zone</CardTitle>
                          <CardDescription>Irreversible and destructive actions for this skill</CardDescription>
                        </CardHeader>
                        <CardContent className="space-y-4">
                          <div className="flex items-center justify-between">
                            <div>
                              <p className="text-sm font-medium">Enable skill</p>
                              <p className="text-sm text-muted-foreground">
                                When disabled, this skill will not be available to clients
                              </p>
                            </div>
                            <Switch
                              checked={selectedSkillInfo.enabled}
                              onCheckedChange={(checked) => handleToggleEnabled(selectedSkillInfo.name, checked)}
                            />
                          </div>
                          {(isMarketplaceSkill || isUserCreatedSkill) && (
                            <div className="flex items-center justify-between pt-4 border-t">
                              <div>
                                <p className="text-sm font-medium">Delete this skill</p>
                                <p className="text-sm text-muted-foreground">Permanently delete "{selectedSkillInfo.name}" and all its files</p>
                              </div>
                              <AlertDialog>
                                <AlertDialogTrigger asChild>
                                  <Button variant="destructive" disabled={isDeleting}>
                                    {isDeleting ? (
                                      <>
                                        <Loader2 className="h-4 w-4 mr-2 animate-spin" />
                                        Deleting...
                                      </>
                                    ) : (
                                      "Delete Skill"
                                    )}
                                  </Button>
                                </AlertDialogTrigger>
                                <AlertDialogContent>
                                  <AlertDialogHeader>
                                    <AlertDialogTitle>Delete "{selectedSkillInfo.name}"?</AlertDialogTitle>
                                    <AlertDialogDescription>
                                      This will permanently delete this skill and all its files.
                                      This action cannot be undone.
                                    </AlertDialogDescription>
                                  </AlertDialogHeader>
                                  <AlertDialogFooter>
                                    <AlertDialogCancel>Cancel</AlertDialogCancel>
                                    <AlertDialogAction
                                      onClick={() => handleDeleteSkill(selectedSkillInfo.name, selectedSkillInfo.source_path)}
                                      className="bg-destructive text-destructive-foreground hover:bg-destructive/90"
                                    >
                                      Delete
                                    </AlertDialogAction>
                                  </AlertDialogFooter>
                                </AlertDialogContent>
                              </AlertDialog>
                            </div>
                          )}
                        </CardContent>
                      </Card>
                    </div>
                  </TabsContent>
                </Tabs>
              </div>
            </ScrollArea>
          ) : (
            <div className="flex flex-col items-center justify-center h-full text-muted-foreground gap-4">
              <SkillsIcon className="h-12 w-12 opacity-30" />
              <div className="text-center">
                <p className="font-medium">Select a skill to view details</p>
                <p className="text-sm">
                  Click + to browse the marketplace, create a new skill, or add skill paths
                </p>
              </div>
            </div>
          )}
        </ResizablePanel>
      </ResizablePanelGroup>

      {/* Add Skills Dialog */}
      <Dialog open={showAddDialog} onOpenChange={setShowAddDialog}>
        <DialogContent className="max-w-2xl max-h-[85vh] flex flex-col overflow-hidden">
          <DialogHeader className="flex-shrink-0">
            <DialogTitle>Add Skills</DialogTitle>
            <DialogDescription>
              Add skills from the marketplace, create new ones, or manage local skill paths.
            </DialogDescription>
          </DialogHeader>

          <Tabs value={addDialogTab} onValueChange={(v) => setAddDialogTab(v as typeof addDialogTab)} className="flex-1 flex flex-col min-h-0">
            <TabsList className="grid w-full grid-cols-3 flex-shrink-0">
              <TabsTrigger value="paths" className="flex items-center gap-1.5">
                <FolderOpen className="h-3.5 w-3.5" />
                Paths
              </TabsTrigger>
              <TabsTrigger value="marketplace" className="flex items-center gap-1.5">
                <Store className="h-3.5 w-3.5" />
                Marketplace
              </TabsTrigger>
              <TabsTrigger value="new" className="flex items-center gap-1.5">
                <FilePlus className="h-3.5 w-3.5" />
                New
              </TabsTrigger>
            </TabsList>

            {/* Marketplace Tab */}
            <TabsContent value="marketplace" className="flex-1 min-h-0 mt-4">
              <MarketplaceSearchPanel
                type="skill"
                onSelectSkill={handleSelectMarketplaceSkill}
                installedSkillNames={skills.map(s => s.name)}
                maxHeight="calc(85vh - 220px)"
              />
            </TabsContent>

            {/* New Skill Tab */}
            <TabsContent value="new" className="flex-1 min-h-0 mt-4">
              <form onSubmit={handleCreateSkill} className="space-y-4">
                <div className="space-y-2">
                  <Label htmlFor="skill-name">Name</Label>
                  <Input
                    id="skill-name"
                    placeholder="my-skill"
                    value={newSkillName}
                    onChange={(e) => setNewSkillName(e.target.value)}
                  />
                </div>

                <div className="space-y-2">
                  <Label htmlFor="skill-description">Description</Label>
                  <Input
                    id="skill-description"
                    placeholder="What does this skill do?"
                    value={newSkillDescription}
                    onChange={(e) => setNewSkillDescription(e.target.value)}
                  />
                </div>

                <div className="space-y-2">
                  <Label htmlFor="skill-content">Content</Label>
                  <Textarea
                    id="skill-content"
                    placeholder="Skill instructions in markdown..."
                    value={newSkillContent}
                    onChange={(e) => setNewSkillContent(e.target.value)}
                    rows={10}
                    className="font-mono text-sm"
                  />
                  <p className="text-xs text-muted-foreground">
                    The skill content (markdown body after the YAML frontmatter). This will be saved as a SKILL.md file.
                  </p>
                </div>

                <div className="flex justify-end gap-2 pt-2">
                  <Button
                    type="button"
                    variant="outline"
                    onClick={() => setShowAddDialog(false)}
                  >
                    Cancel
                  </Button>
                  <Button
                    type="submit"
                    disabled={!newSkillName.trim() || isCreatingSkill}
                  >
                    {isCreatingSkill ? (
                      <>
                        <Loader2 className="h-4 w-4 mr-2 animate-spin" />
                        Creating...
                      </>
                    ) : (
                      "Create Skill"
                    )}
                  </Button>
                </div>
              </form>
            </TabsContent>

            {/* Paths Tab */}
            <TabsContent value="paths" className="flex-1 flex flex-col min-h-0 mt-4">
              <div className="space-y-4 flex-1 overflow-y-auto">
                {/* Existing Paths */}
                <div className="space-y-2">
                  <p className="text-sm font-medium">Current Paths</p>
                  {skillPaths.length === 0 ? (
                    <p className="text-sm text-muted-foreground py-2">No skill paths configured</p>
                  ) : (
                    <div className="space-y-1">
                      {skillPaths.map((path) => (
                        <div
                          key={path}
                          className="flex items-center gap-2 p-2 rounded-md bg-muted/50 group"
                        >
                          <Folder className="h-4 w-4 text-muted-foreground shrink-0" />
                          <span className="text-sm flex-1 truncate" title={path}>
                            {path}
                          </span>
                          <Button
                            variant="ghost"
                            size="icon"
                            className="h-6 w-6 opacity-0 group-hover:opacity-100 transition-opacity"
                            onClick={() => handleRemovePath(path)}
                            disabled={removingPath === path}
                          >
                            <Trash2 className="h-3.5 w-3.5 text-destructive" />
                          </Button>
                        </div>
                      ))}
                    </div>
                  )}
                </div>

                {/* Add New Path */}
                <div className="space-y-2">
                  <p className="text-sm font-medium">Add New Path</p>
                  <div className="flex gap-2">
                    <Input
                      placeholder="Enter path or browse..."
                      value={newPath}
                      onChange={(e) => setNewPath(e.target.value)}
                      onKeyDown={(e) => {
                        if (e.key === "Enter" && newPath.trim()) {
                          handleAddPath(newPath)
                        }
                      }}
                      className="flex-1"
                    />
                    <Button
                      variant="outline"
                      size="icon"
                      onClick={() => handleAddPath(newPath)}
                      disabled={!newPath.trim() || addingPath}
                      title="Add path"
                    >
                      <Plus className="h-4 w-4" />
                    </Button>
                  </div>
                  <Button
                    variant="outline"
                    size="sm"
                    className="w-full"
                    onClick={handleBrowse}
                    disabled={addingPath}
                  >
                    <FolderOpen className="h-4 w-4 mr-2" />
                    Browse...
                  </Button>
                </div>
              </div>

              {/* Footer with Rescan */}
              <div className="flex-shrink-0 pt-4 border-t flex justify-between items-center mt-4">
                <Button
                  variant="outline"
                  size="sm"
                  onClick={handleRescan}
                  disabled={rescanning}
                >
                  <RefreshCw className={`h-4 w-4 mr-2 ${rescanning ? "animate-spin" : ""}`} />
                  Rescan Skills
                </Button>
                <Button
                  variant="default"
                  size="sm"
                  onClick={() => setShowAddDialog(false)}
                >
                  Done
                </Button>
              </div>
            </TabsContent>
          </Tabs>
        </DialogContent>
      </Dialog>
      </div>
    </div>
  )
}
