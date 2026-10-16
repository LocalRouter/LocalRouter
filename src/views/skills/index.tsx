import { useState, useEffect, useMemo, useCallback } from "react"
import { invoke } from "@tauri-apps/api/core"
import { listenSafe } from "@/hooks/useTauriListener"
import { toast } from "sonner"
import { ChevronDown, ChevronRight, FilePlus, Import, Plus, RefreshCw, Search, Store, TriangleAlert } from "lucide-react"
import { SkillsIcon } from "@/components/icons/category-icons"
import { Button } from "@/components/ui/Button"
import { Input } from "@/components/ui/Input"
import { Card, CardContent, CardHeader, CardTitle, CardDescription } from "@/components/ui/Card"
import { ScrollArea } from "@/components/ui/scroll-area"
import { ResizablePanelGroup, ResizablePanel, ResizableHandle } from "@/components/ui/resizable"
import { cn } from "@/lib/utils"
import { groupSkillsBySource, splitMarketplaceGroups } from "@/utils/skill-sources"
import type { SkillInfo, SkillSourceInfo, SkillSourceKind, SkillsConfig, UpdateSkillsToolNamesParams } from "@/types/tauri-commands"
import { AddSkillsDialog, type AddSkillsTab } from "./add-skills-dialog"
import { SkillDetail } from "./skill-detail"
import { SourceDetail } from "./source-detail"
import { SkillSourceIcon } from "./source-icon"
import { matchesSkill } from "./skill-search"

interface SkillsViewProps {
  activeSubTab: string | null
  onTabChange: (view: string, subTab?: string | null) => void
}

/** Sub-tab values: a skill name, `source:<path>`, or the marketplace group */
const SOURCE_SELECTION_PREFIX = "source:"
const MARKETPLACE_SELECTION = "group:marketplace"

type ListMode = "sources" | "all"

interface SidebarGroup {
  /** Selection value for the group */
  id: string
  kind: SkillSourceKind | "marketplace_group"
  label: string
  /** Full path, shown as a tooltip and matched by search */
  path: string
  missing: boolean
  skills: SkillInfo[]
}

export function SkillsView({ activeSubTab, onTabChange }: SkillsViewProps) {
  const [skills, setSkills] = useState<SkillInfo[]>([])
  const [sources, setSources] = useState<SkillSourceInfo[]>([])
  const [skillsConfig, setSkillsConfig] = useState<SkillsConfig | null>(null)
  const [loading, setLoading] = useState(true)
  const [rescanning, setRescanning] = useState(false)
  const [selection, setSelection] = useState<string | null>(activeSubTab)
  const [search, setSearch] = useState("")
  const [listMode, setListMode] = useState<ListMode>("sources")
  const [expandedGroups, setExpandedGroups] = useState<Set<string> | null>(null)
  const [showAddDialog, setShowAddDialog] = useState(false)
  const [addDialogTab, setAddDialogTab] = useState<AddSkillsTab>("new")

  const loadData = useCallback(async () => {
    try {
      const [skillList, sourceList, config] = await Promise.all([
        invoke<SkillInfo[]>("list_skills"),
        invoke<SkillSourceInfo[]>("list_skill_sources"),
        invoke<SkillsConfig>("get_skills_config"),
      ])
      setSkills(skillList)
      setSources(sourceList)
      setSkillsConfig(config)
    } catch (error) {
      console.error("Failed to load skills:", error)
    } finally {
      setLoading(false)
    }
  }, [])

  useEffect(() => {
    loadData()

    const l = listenSafe("skills-changed", () => {
      loadData()
    })

    return () => {
      l.cleanup()
    }
  }, [loadData])

  useEffect(() => {
    setSelection(activeSubTab)
  }, [activeSubTab])

  const select = (value: string | null) => {
    setSelection(value)
    onTabChange("skills", value)
  }

  const groups = useMemo((): SidebarGroup[] => {
    const { sourceGroups, marketplaceSkills } = splitMarketplaceGroups(groupSkillsBySource(skills, sources, true))
    const result: SidebarGroup[] = sourceGroups.map(({ source, skills: groupSkills }) => ({
      id: `${SOURCE_SELECTION_PREFIX}${source.path}`,
      kind: source.kind,
      label: source.label,
      path: source.path,
      missing: !source.exists,
      skills: groupSkills,
    }))
    if (sources.some((s) => s.kind === "marketplace")) {
      result.push({
        id: MARKETPLACE_SELECTION,
        kind: "marketplace_group",
        label: "Marketplace",
        path: "",
        missing: false,
        skills: marketplaceSkills,
      })
    }
    return result
  }, [skills, sources])

  // Start with a lone group open; with several, start collapsed for an overview
  useEffect(() => {
    if (expandedGroups === null && !loading) {
      setExpandedGroups(new Set(groups.length === 1 ? [groups[0].id] : []))
    }
  }, [groups, loading, expandedGroups])

  const query = search.trim().toLowerCase()

  const selectedSkill = skills.find((s) => s.name === selection) ?? null
  const selectedGroup = groups.find((g) => g.id === selection) ?? null
  const groupOfSkill = (skill: SkillInfo) => groups.find((g) => g.skills.some((s) => s.name === skill.name))

  const visibleGroups = groups
    .map((group) => {
      const groupMatches =
        !!query && (group.label.toLowerCase().includes(query) || group.path.toLowerCase().includes(query))
      return { ...group, skills: groupMatches ? group.skills : group.skills.filter((s) => matchesSkill(s, query)) }
    })
    .filter((group) => !query || group.skills.length > 0 || group.label.toLowerCase().includes(query))

  const flatSkills = skills
    .filter((s) => matchesSkill(s, query))
    .sort((a, b) => a.name.toLowerCase().localeCompare(b.name.toLowerCase()))

  const isExpanded = (group: SidebarGroup) =>
    !!query || expandedGroups?.has(group.id) || (selectedSkill !== null && groupOfSkill(selectedSkill)?.id === group.id)

  const toggleGroup = (id: string) => {
    setExpandedGroups((prev) => {
      const next = new Set(prev ?? [])
      if (next.has(id)) next.delete(id)
      else next.add(id)
      return next
    })
  }

  const handleRescan = async () => {
    setRescanning(true)
    try {
      const result = await invoke<SkillInfo[]>("rescan_skills")
      toast.success(`Found ${result.length} skill(s)`)
      await loadData()
    } catch (error) {
      console.error("Failed to rescan skills:", error)
      toast.error("Failed to rescan skills")
    } finally {
      setRescanning(false)
    }
  }

  const openAddDialog = (tab: AddSkillsTab) => {
    setAddDialogTab(tab)
    setShowAddDialog(true)
  }

  const enabledCount = skills.filter((s) => s.enabled).length
  const skillToolName = skillsConfig?.tool_name ?? "SkillRead"

  const renderSkillRow = (skill: SkillInfo, indent: boolean, sourceLabel?: string) => (
    <div
      key={skill.name}
      onClick={() => select(skill.name)}
      className={cn(
        "flex items-center gap-2 py-2 pr-3 rounded-md cursor-pointer",
        indent ? "pl-9" : "pl-3",
        selection === skill.name ? "bg-accent" : "hover:bg-muted",
        !skill.enabled && "opacity-50"
      )}
    >
      <div className="flex-1 min-w-0">
        <p className="text-sm font-medium truncate">{skill.name}</p>
        {(sourceLabel || skill.description) && (
          <p className="text-xs text-muted-foreground truncate">{sourceLabel ?? skill.description}</p>
        )}
      </div>
      {!skill.enabled && <span className="text-xs text-muted-foreground shrink-0">Off</span>}
    </div>
  )

  const renderDetail = () => {
    if (selectedSkill) {
      const group = groupOfSkill(selectedSkill)
      const source =
        sources.find((s) => s.path === selectedSkill.source_path) ??
        ({ path: selectedSkill.source_path, label: selectedSkill.source_path, kind: "directory", exists: true, skill_count: 1 } as SkillSourceInfo)
      return (
        <SkillDetail
          key={selectedSkill.name}
          skill={selectedSkill}
          source={source}
          inMarketplaceGroup={group?.id === MARKETPLACE_SELECTION}
          onSelectSource={() => select(group?.id ?? `${SOURCE_SELECTION_PREFIX}${source.path}`)}
          onChanged={loadData}
          onDeleted={() => {
            select(group?.id ?? null)
            loadData()
          }}
        />
      )
    }

    if (selectedGroup) {
      const source = selectedGroup.id === MARKETPLACE_SELECTION
        ? null
        : sources.find((s) => `${SOURCE_SELECTION_PREFIX}${s.path}` === selectedGroup.id) ?? null
      return (
        <SourceDetail
          key={selectedGroup.id}
          source={source}
          skills={selectedGroup.skills}
          onSelectSkill={select}
          onChanged={loadData}
          onRemoved={() => {
            select(null)
            loadData()
          }}
          onCreateSkill={() => openAddDialog("new")}
          onBrowseMarketplace={() => openAddDialog("marketplace")}
        />
      )
    }

    return (
      <ScrollArea className="h-full">
        <div className="p-6 space-y-6">
          <div className="grid grid-cols-3 gap-3">
            {[
              { label: "Skills", value: skills.length },
              { label: "Enabled", value: enabledCount },
              { label: "Sources", value: sources.length },
            ].map((stat) => (
              <div key={stat.label} className="rounded-lg border p-4">
                <p className="text-2xl font-semibold tabular-nums">{stat.value}</p>
                <p className="text-xs text-muted-foreground">{stat.label}</p>
              </div>
            ))}
          </div>

          {groups.length === 0 ? (
            <div className="rounded-lg border border-dashed p-8 text-center space-y-4">
              <SkillsIcon className="h-10 w-10 mx-auto opacity-30" />
              <div>
                <p className="font-medium">No skills yet</p>
                <p className="text-sm text-muted-foreground">
                  Create a skill, import a folder of skills, or install one from the marketplace.
                </p>
              </div>
              <div className="flex justify-center gap-2">
                <Button variant="outline" size="sm" onClick={() => openAddDialog("new")}>
                  <FilePlus className="h-4 w-4 mr-1" /> New skill
                </Button>
                <Button variant="outline" size="sm" onClick={() => openAddDialog("paths")}>
                  <Import className="h-4 w-4 mr-1" /> Import folder
                </Button>
                <Button variant="outline" size="sm" onClick={() => openAddDialog("marketplace")}>
                  <Store className="h-4 w-4 mr-1" /> Marketplace
                </Button>
              </div>
            </div>
          ) : (
            <div className="space-y-2">
              <h3 className="text-sm font-medium">Sources</h3>
              <div className="grid grid-cols-1 md:grid-cols-2 gap-3">
                {groups.map((group) => {
                  const enabled = group.skills.filter((s) => s.enabled).length
                  return (
                    <button
                      key={group.id}
                      type="button"
                      onClick={() => select(group.id)}
                      className="text-left rounded-lg border p-4 hover:bg-muted/40 transition-colors flex items-start gap-3"
                    >
                      <div className="h-9 w-9 rounded-md bg-muted flex items-center justify-center shrink-0">
                        <SkillSourceIcon kind={group.kind} className="h-4 w-4 text-muted-foreground" />
                      </div>
                      <div className="min-w-0 flex-1">
                        <p className="font-medium truncate flex items-center gap-1.5">
                          {group.label}
                          {group.missing && <TriangleAlert className="h-3.5 w-3.5 text-amber-500 shrink-0" />}
                        </p>
                        {group.path && (
                          <p className="text-xs font-mono text-muted-foreground truncate" title={group.path}>
                            {group.path}
                          </p>
                        )}
                        <p className="text-xs text-muted-foreground mt-1">
                          {group.skills.length} skill{group.skills.length === 1 ? "" : "s"}
                          {group.skills.length > 0 && ` · ${enabled} enabled`}
                        </p>
                      </div>
                    </button>
                  )
                })}
              </div>
            </div>
          )}

          <Card>
            <CardHeader>
              <CardTitle className="text-sm">Tool Names</CardTitle>
              <CardDescription>
                Rename tools to resolve conflicts with other tools the LLM client already uses. Applies to all skills; changes apply to new sessions only.
              </CardDescription>
            </CardHeader>
            <CardContent>
              <div className="flex gap-4 items-center">
                <label className="text-sm text-muted-foreground w-28 shrink-0">Skill read tool:</label>
                <Input
                  defaultValue={skillToolName}
                  key={`skill-tool-name-${skillToolName}`}
                  onBlur={async (e) => {
                    const v = e.target.value.trim()
                    if (v && v !== skillToolName) {
                      try {
                        await invoke("update_skills_tool_names", { toolName: v } satisfies UpdateSkillsToolNamesParams)
                        toast.success("Skill read tool name updated")
                        loadData()
                      } catch (error) {
                        toast.error("Failed to update skill read tool name")
                      }
                    }
                  }}
                  onKeyDown={(e) => { if (e.key === "Enter") (e.target as HTMLInputElement).blur() }}
                  className="flex-1"
                />
              </div>
            </CardContent>
          </Card>
        </div>
      </ScrollArea>
    )
  }

  return (
    <div className="flex flex-col h-full min-h-0 max-w-5xl">
      <div className="flex-shrink-0 pb-4 flex items-start justify-between gap-4">
        <div>
          <h1 className="text-2xl font-bold tracking-tight flex items-center gap-2">
            <SkillsIcon className="h-6 w-6" />
            Skills
          </h1>
          <p className="text-sm text-muted-foreground">
            Install and manage skill packages from AgentSkills.io. Skills are exposed as MCP tools and prompts through the unified MCP gateway.
          </p>
        </div>
        <div className="flex items-center gap-2 shrink-0">
          <Button variant="outline" size="icon" onClick={handleRescan} disabled={rescanning} title="Rescan skill sources">
            <RefreshCw className={cn("h-4 w-4", rescanning && "animate-spin")} />
          </Button>
          <Button onClick={() => openAddDialog("new")}>
            <Plus className="h-4 w-4 mr-1" />
            Add Skills
          </Button>
        </div>
      </div>

      <div className="flex-1 min-h-0 mt-4">
        <ResizablePanelGroup direction="horizontal" className="flex-1 min-h-0 rounded-lg border">
          {/* List Panel */}
          <ResizablePanel defaultSize={28} minSize={18}>
            <div className="flex flex-col h-full">
              <div className="p-3 border-b space-y-2">
                <div className="relative">
                  <Search className="absolute left-2.5 top-1/2 -translate-y-1/2 h-4 w-4 text-muted-foreground pointer-events-none" />
                  <Input
                    placeholder="Search skills or sources..."
                    value={search}
                    onChange={(e) => setSearch(e.target.value)}
                    className="pl-8"
                  />
                </div>
                <div className="grid grid-cols-2 rounded-md bg-muted p-0.5 text-xs">
                  {(["sources", "all"] as const).map((mode) => (
                    <button
                      key={mode}
                      type="button"
                      onClick={() => setListMode(mode)}
                      className={cn(
                        "rounded px-2 py-1 transition-colors",
                        listMode === mode ? "bg-background shadow-sm font-medium" : "text-muted-foreground hover:text-foreground"
                      )}
                    >
                      {mode === "sources" ? "By source" : `All skills (${skills.length})`}
                    </button>
                  ))}
                </div>
              </div>
              {/* Radix lays the viewport content out as a table; force block so rows truncate */}
              <ScrollArea className="flex-1 [&_[data-radix-scroll-area-viewport]>div]:!block">
                <div className="p-2 space-y-0.5">
                  {loading ? (
                    <p className="text-sm text-muted-foreground p-4">Loading...</p>
                  ) : listMode === "all" ? (
                    flatSkills.length === 0 ? (
                      <p className="text-sm text-muted-foreground p-4">No skills found</p>
                    ) : (
                      flatSkills.map((skill) => renderSkillRow(skill, false, groupOfSkill(skill)?.label))
                    )
                  ) : visibleGroups.length === 0 ? (
                    <p className="text-sm text-muted-foreground p-4">
                      {groups.length === 0 ? "No skill sources yet" : "No skills found"}
                    </p>
                  ) : (
                    visibleGroups.map((group) => {
                      const open = isExpanded(group)
                      return (
                        <div key={group.id}>
                          <div
                            onClick={() => {
                              select(group.id)
                              if (!open) toggleGroup(group.id)
                            }}
                            className={cn(
                              "flex items-center gap-1.5 py-2 pr-3 pl-1 rounded-md cursor-pointer",
                              selection === group.id ? "bg-accent" : "hover:bg-muted"
                            )}
                            title={group.path || undefined}
                          >
                            <button
                              type="button"
                              onClick={(e) => {
                                e.stopPropagation()
                                toggleGroup(group.id)
                              }}
                              disabled={!!query}
                              className="p-0.5 rounded hover:bg-muted-foreground/10"
                              aria-label={open ? "Collapse" : "Expand"}
                            >
                              {open ? (
                                <ChevronDown className="h-4 w-4 text-muted-foreground" />
                              ) : (
                                <ChevronRight className="h-4 w-4 text-muted-foreground" />
                              )}
                            </button>
                            <SkillSourceIcon kind={group.kind} className="h-4 w-4 text-muted-foreground shrink-0" />
                            <div className="min-w-0 flex-1">
                              <p className="text-sm font-semibold truncate">{group.label}</p>
                              {group.path && (
                                <p className="text-xs font-mono text-muted-foreground truncate">{group.path}</p>
                              )}
                            </div>
                            {group.missing && <TriangleAlert className="h-3.5 w-3.5 text-amber-500 shrink-0" />}
                            <span className="text-xs text-muted-foreground tabular-nums shrink-0">
                              {group.skills.length}
                            </span>
                          </div>
                          {open && (
                            group.skills.length === 0 ? (
                              <p className="text-xs text-muted-foreground pl-9 py-1.5">No skills</p>
                            ) : (
                              group.skills.map((skill) => renderSkillRow(skill, true))
                            )
                          )}
                        </div>
                      )
                    })
                  )}
                </div>
              </ScrollArea>
            </div>
          </ResizablePanel>

          <ResizableHandle withHandle />

          {/* Detail Panel */}
          <ResizablePanel defaultSize={72}>{renderDetail()}</ResizablePanel>
        </ResizablePanelGroup>

        <AddSkillsDialog
          open={showAddDialog}
          onOpenChange={setShowAddDialog}
          tab={addDialogTab}
          onTabChange={setAddDialogTab}
          installedSkillNames={skills.map((s) => s.name)}
          onChanged={loadData}
        />
      </div>
    </div>
  )
}
