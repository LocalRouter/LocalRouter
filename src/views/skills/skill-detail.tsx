import { useState, useEffect } from "react"
import { invoke } from "@tauri-apps/api/core"
import { toast } from "sonner"
import { ExternalLink, ChevronDown, ChevronRight, FileText, FileCode, Image, Folder, FolderOpen, FlaskConical, Play, BookOpen, Loader2 } from "lucide-react"
import { TAB_ICONS, TAB_ICON_CLASS } from "@/constants/tab-icons"
import { Button } from "@/components/ui/Button"
import { Card, CardContent, CardHeader, CardTitle, CardDescription } from "@/components/ui/Card"
import { Switch } from "@/components/ui/switch"
import { ScrollArea } from "@/components/ui/scroll-area"
import { Tabs, TabsList, TabsTrigger, TabsContent } from "@/components/ui/tabs"
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
import { SkillSourceIcon } from "./source-icon"
import type {
  DeleteUserSkillParams,
  GetSkillFilesParams,
  IsUserCreatedSkillParams,
  MarketplaceDeleteSkillParams,
  MarketplaceIsSkillFromMarketplaceParams,
  OpenPathParams,
  RevealPathParams,
  SetSkillEnabledParams,
  SkillFileInfo,
  SkillInfo,
  SkillSourceInfo,
} from "@/types/tauri-commands"

interface SkillDetailProps {
  skill: SkillInfo
  source: SkillSourceInfo
  /** Show the source in the breadcrumb as the marketplace group */
  inMarketplaceGroup: boolean
  onSelectSource: () => void
  /** Called after the skill changed */
  onChanged: () => void
  onDeleted: () => void
}

export function SkillDetail({
  skill,
  source,
  inMarketplaceGroup,
  onSelectSource,
  onChanged,
  onDeleted,
}: SkillDetailProps) {
  const [skillFiles, setSkillFiles] = useState<SkillFileInfo[]>([])
  const [loadingFiles, setLoadingFiles] = useState(false)
  const [expandedFiles, setExpandedFiles] = useState<Set<string>>(new Set())
  const [detailTab, setDetailTab] = useState("info")
  const [isMarketplaceSkill, setIsMarketplaceSkill] = useState(false)
  const [isUserCreatedSkill, setIsUserCreatedSkill] = useState(false)
  const [isDeleting, setIsDeleting] = useState(false)

  useEffect(() => {
    let cancelled = false
    setLoadingFiles(true)
    invoke<SkillFileInfo[]>("get_skill_files", { skillName: skill.name } satisfies GetSkillFilesParams)
      .then((files) => !cancelled && setSkillFiles(files))
      .catch((error) => {
        console.error("Failed to load skill files:", error)
        if (!cancelled) setSkillFiles([])
      })
      .finally(() => !cancelled && setLoadingFiles(false))
    return () => {
      cancelled = true
    }
  }, [skill.name])

  useEffect(() => {
    invoke<boolean>("marketplace_is_skill_from_marketplace", {
      skillPath: skill.source_path,
    } satisfies MarketplaceIsSkillFromMarketplaceParams)
      .then(setIsMarketplaceSkill)
      .catch(() => setIsMarketplaceSkill(false))
    invoke<boolean>("is_user_created_skill", {
      skillPath: skill.source_path,
    } satisfies IsUserCreatedSkillParams)
      .then(setIsUserCreatedSkill)
      .catch(() => setIsUserCreatedSkill(false))
  }, [skill.source_path])

  const toggleFileExpanded = (fileName: string) => {
    setExpandedFiles((prev) => {
      const next = new Set(prev)
      if (next.has(fileName)) {
        next.delete(fileName)
      } else {
        next.add(fileName)
      }
      return next
    })
  }

  const handleOpenPath = async (path: string) => {
    try {
      await invoke("open_path", { path } satisfies OpenPathParams)
    } catch (error) {
      console.error("Failed to open path:", error)
      toast.error("Failed to open in file explorer")
    }
  }

  const handleRevealPath = async (path: string) => {
    try {
      await invoke("reveal_path", { path } satisfies RevealPathParams)
    } catch (error) {
      console.error("Failed to reveal path:", error)
      toast.error("Failed to show in file explorer")
    }
  }

  const handleToggleEnabled = async (enabled: boolean) => {
    try {
      await invoke("set_skill_enabled", { skillName: skill.name, enabled } satisfies SetSkillEnabledParams)
      toast.success(enabled ? "Skill enabled" : "Skill disabled")
      onChanged()
    } catch (error) {
      console.error("Failed to toggle skill:", error)
      toast.error("Failed to update skill")
    }
  }

  const handleDeleteSkill = async () => {
    setIsDeleting(true)
    try {
      const params = { skillName: skill.name, skillPath: skill.source_path }
      if (isUserCreatedSkill) {
        await invoke("delete_user_skill", params satisfies DeleteUserSkillParams)
      } else {
        await invoke("marketplace_delete_skill", params satisfies MarketplaceDeleteSkillParams)
      }
      toast.success(`Skill "${skill.name}" deleted`)
      onDeleted()
    } catch (error) {
      console.error("Failed to delete skill:", error)
      toast.error(`Failed to delete skill: ${error}`)
    } finally {
      setIsDeleting(false)
    }
  }

  return (
    <ScrollArea className="h-full">
      <div className="p-6 space-y-6">
        {/* Header */}
        <div className="space-y-2">
          <button
            type="button"
            onClick={onSelectSource}
            className="inline-flex items-center gap-1.5 text-xs text-muted-foreground hover:text-foreground transition-colors"
            title={source.path}
          >
            <SkillSourceIcon kind={inMarketplaceGroup ? "marketplace_group" : source.kind} className="h-3.5 w-3.5" />
            {inMarketplaceGroup ? "Marketplace" : source.label}
            <ChevronRight className="h-3 w-3" />
          </button>
          <div className="flex items-start justify-between gap-4">
            <div className="min-w-0">
              <h2 className="text-xl font-bold flex items-center gap-2">
                {skill.name}
                {!skill.enabled && (
                  <span className="text-xs font-normal px-2 py-0.5 rounded-full bg-muted text-muted-foreground">
                    Disabled
                  </span>
                )}
              </h2>
              {skill.description && <p className="text-sm text-muted-foreground">{skill.description}</p>}
            </div>
            <Button variant="outline" size="sm" onClick={() => setDetailTab("try-it-out")}>
              <FlaskConical className="h-4 w-4 mr-1" />
              Try It Out
            </Button>
          </div>
        </div>

        <Tabs value={detailTab} onValueChange={setDetailTab}>
          <TabsList>
            <TabsTrigger value="info"><TAB_ICONS.info className={TAB_ICON_CLASS} />Info</TabsTrigger>
            <TabsTrigger value="try-it-out"><TAB_ICONS.tryItOut className={TAB_ICON_CLASS} />Try It Out</TabsTrigger>
            <TabsTrigger value="settings"><TAB_ICONS.settings className={TAB_ICON_CLASS} />Settings</TabsTrigger>
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
                    {skill.author && (
                      <div>
                        <span className="text-muted-foreground">Author:</span>{" "}
                        <span className="font-medium">{skill.author}</span>
                      </div>
                    )}
                    {skill.version && (
                      <div>
                        <span className="text-muted-foreground">Version:</span>{" "}
                        <span className="font-medium">{skill.version}</span>
                      </div>
                    )}
                    {Object.entries(skill.extra)
                      .sort(([a], [b]) => a.localeCompare(b))
                      .map(([key, value]) => (
                        <div key={key}>
                          <span className="text-muted-foreground">{key}:</span>{" "}
                          <span className="font-medium">{typeof value === "object" ? JSON.stringify(value) : String(value)}</span>
                        </div>
                      ))}
                  </div>

                  {/* Tags */}
                  {skill.tags.length > 0 && (
                    <div className="flex flex-wrap gap-1.5">
                      {skill.tags.map((tag) => (
                        <span key={tag} className="text-xs px-2 py-0.5 rounded-full bg-muted text-muted-foreground">
                          {tag}
                        </span>
                      ))}
                    </div>
                  )}

                  {/* Capabilities */}
                  <div className="flex gap-2">
                    {skill.script_count > 0 && (
                      <span className="text-xs px-2 py-1 rounded bg-blue-500/10 text-blue-800 dark:text-blue-400 flex items-center gap-1">
                        <Play className="h-3 w-3" />
                        {skill.script_count} Executable{skill.script_count > 1 ? "s" : ""}
                      </span>
                    )}
                    {(skill.reference_count > 0 || skill.asset_count > 0) && (
                      <span className="text-xs px-2 py-1 rounded bg-green-500/10 text-green-800 dark:text-green-400 flex items-center gap-1">
                        <BookOpen className="h-3 w-3" />
                        {skill.reference_count + skill.asset_count} Resource{skill.reference_count + skill.asset_count > 1 ? "s" : ""}
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
                  <CardHeader className="pb-3 flex-row items-center justify-between space-y-0 gap-3">
                    <div className="min-w-0">
                      <CardTitle className="text-sm">Files</CardTitle>
                      <p className="text-xs font-mono text-muted-foreground truncate mt-1" title={skill.skill_dir}>
                        {skill.skill_dir}
                      </p>
                    </div>
                    <Button
                      variant="outline"
                      size="sm"
                      className="h-7 text-xs shrink-0"
                      onClick={() => handleOpenPath(skill.skill_dir)}
                    >
                      <ExternalLink className="h-3 w-3 mr-1" />
                      Open folder
                    </Button>
                  </CardHeader>
                  <CardContent>
                    <div className="rounded-md border border-border/50 bg-muted/20 py-1">
                      <FileTree
                        files={skillFiles}
                        expanded={expandedFiles}
                        onToggle={toggleFileExpanded}
                        onReveal={handleRevealPath}
                      />
                    </div>
                  </CardContent>
                </Card>
              )}

            </div>
          </TabsContent>

          <TabsContent value="try-it-out">
            <McpTab
              initialMode="direct"
              initialDirectTarget={`skill:${skill.name}`}
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
                    <Switch checked={skill.enabled} onCheckedChange={handleToggleEnabled} />
                  </div>
                  {(isMarketplaceSkill || isUserCreatedSkill) && (
                    <div className="flex items-center justify-between pt-4 border-t">
                      <div>
                        <p className="text-sm font-medium">Delete this skill</p>
                        <p className="text-sm text-muted-foreground">Permanently delete "{skill.name}" and all its files</p>
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
                            <AlertDialogTitle>Delete "{skill.name}"?</AlertDialogTitle>
                            <AlertDialogDescription>
                              This will permanently delete this skill and all its files.
                              This action cannot be undone.
                            </AlertDialogDescription>
                          </AlertDialogHeader>
                          <AlertDialogFooter>
                            <AlertDialogCancel>Cancel</AlertDialogCancel>
                            <AlertDialogAction
                              onClick={handleDeleteSkill}
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
  )
}

interface FileTreeNode {
  name: string
  fullPath: string
  file?: SkillFileInfo
  children: Record<string, FileTreeNode>
}

function getFileIcon(name: string, category: string) {
  if (category === "skill_md") return <FileText className="h-3.5 w-3.5 text-amber-500" />
  if (category === "script") return <Play className="h-3.5 w-3.5 text-blue-500" />
  if (category === "reference") return <BookOpen className="h-3.5 w-3.5 text-green-500" />
  if (category === "asset") return <Image className="h-3.5 w-3.5 text-purple-500" />
  const ext = name.split(".").pop()?.toLowerCase() ?? ""
  if (["sh", "bash", "py", "js", "ts", "rb", "pl"].includes(ext)) return <FileCode className="h-3.5 w-3.5 text-muted-foreground" />
  if (["png", "jpg", "jpeg", "gif", "svg", "webp", "ico"].includes(ext)) return <Image className="h-3.5 w-3.5 text-muted-foreground" />
  return <FileText className="h-3.5 w-3.5 text-muted-foreground" />
}

function getCategoryBadge(category: string) {
  if (category === "skill_md") return <span className="text-[9px] px-1 py-0.5 rounded bg-amber-500/10 text-amber-800 dark:text-amber-400 font-medium uppercase">Definition</span>
  if (category === "script") return <span className="text-[9px] px-1 py-0.5 rounded bg-blue-500/10 text-blue-800 dark:text-blue-400 font-medium uppercase">Executable</span>
  if (category === "reference") return <span className="text-[9px] px-1 py-0.5 rounded bg-green-500/10 text-green-800 dark:text-green-400 font-medium uppercase">Resource</span>
  if (category === "asset") return <span className="text-[9px] px-1 py-0.5 rounded bg-purple-500/10 text-purple-800 dark:text-purple-400 font-medium uppercase">Resource</span>
  return null
}

function countFiles(node: FileTreeNode): number {
  let c = node.file ? 1 : 0
  for (const child of Object.values(node.children)) c += countFiles(child)
  return c
}

function FileTree({
  files,
  expanded,
  onToggle,
  onReveal,
}: {
  files: SkillFileInfo[]
  expanded: Set<string>
  onToggle: (key: string) => void
  onReveal: (path: string) => void
}) {
  const root: FileTreeNode = { name: "", fullPath: "", children: {} }
  for (const file of files) {
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

  const renderNode = (node: FileTreeNode, depth: number): React.ReactNode[] => {
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
        const dirKey = `dir:${entry.fullPath}`
        const isDirExpanded = expanded.has(dirKey)

        results.push(
          <div key={dirKey}>
            <button
              className="w-full flex items-center gap-1.5 py-1 text-xs hover:bg-muted/50 transition-colors"
              style={{ paddingLeft: padLeft }}
              onClick={() => onToggle(dirKey)}
            >
              {isDirExpanded
                ? <ChevronDown className="h-3 w-3 shrink-0 text-muted-foreground" />
                : <ChevronRight className="h-3 w-3 shrink-0 text-muted-foreground" />}
              <Folder className="h-3.5 w-3.5 text-muted-foreground" />
              <span className="font-medium">{entry.name}/</span>
              <span className="text-muted-foreground">({countFiles(entry)})</span>
            </button>
            {isDirExpanded && renderNode(entry, depth + 1)}
          </div>
        )
      } else if (entry.file) {
        const file = entry.file
        results.push(
          <div key={file.name}>
            <div className="group flex items-center hover:bg-muted/50 transition-colors">
              <button
                className="flex-1 min-w-0 flex items-center gap-1.5 py-1 text-xs"
                style={{ paddingLeft: padLeft }}
                onClick={() => file.content_preview && onToggle(file.name)}
              >
                {file.content_preview ? (
                  expanded.has(file.name)
                    ? <ChevronDown className="h-3 w-3 shrink-0 text-muted-foreground" />
                    : <ChevronRight className="h-3 w-3 shrink-0 text-muted-foreground" />
                ) : <div className="w-3" />}
                {getFileIcon(entry.name, file.category)}
                <span className="truncate">{entry.name}</span>
                {getCategoryBadge(file.category)}
              </button>
              <button
                className="mr-2 p-1 rounded text-muted-foreground hover:text-foreground opacity-0 group-hover:opacity-100 focus-visible:opacity-100 transition-opacity"
                title="Show in file explorer"
                aria-label={`Show ${entry.name} in file explorer`}
                onClick={() => onReveal(file.path)}
              >
                <FolderOpen className="h-3.5 w-3.5" />
              </button>
            </div>
            {expanded.has(file.name) && file.content_preview && (
              <pre
                className="mr-3 mb-1 px-3 py-2 text-[10px] leading-relaxed bg-muted/30 rounded border border-border/50 overflow-auto max-h-96 whitespace-pre-wrap break-words"
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

  return <>{renderNode(root, 0)}</>
}
