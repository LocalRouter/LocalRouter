import { useState } from "react"
import { invoke } from "@tauri-apps/api/core"
import { toast } from "sonner"
import { ExternalLink, FilePlus, Loader2, Store, Trash2, TriangleAlert } from "lucide-react"
import { Button } from "@/components/ui/Button"
import { Input } from "@/components/ui/Input"
import { Switch } from "@/components/ui/switch"
import { ScrollArea } from "@/components/ui/scroll-area"
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
import { cn } from "@/lib/utils"
import { SkillSourceIcon, SOURCE_KIND_LABEL } from "./source-icon"
import { matchesSkill } from "./skill-search"
import type {
  OpenPathParams,
  RemoveSkillSourceParams,
  SetSkillEnabledParams,
  SetSkillsEnabledParams,
  SkillInfo,
  SkillSourceInfo,
} from "@/types/tauri-commands"

interface SourceDetailProps {
  /** The source, or null for the marketplace group */
  source: SkillSourceInfo | null
  skills: SkillInfo[]
  onSelectSkill: (name: string) => void
  onChanged: () => void
  onRemoved: () => void
  onCreateSkill: () => void
  onBrowseMarketplace: () => void
}

export function SourceDetail({
  source,
  skills,
  onSelectSkill,
  onChanged,
  onRemoved,
  onCreateSkill,
  onBrowseMarketplace,
}: SourceDetailProps) {
  const [filter, setFilter] = useState("")
  const [bulkUpdating, setBulkUpdating] = useState(false)
  const [removing, setRemoving] = useState(false)

  const enabledCount = skills.filter((s) => s.enabled).length
  const visibleSkills = skills.filter((s) => matchesSkill(s, filter.trim().toLowerCase()))
  const title = source ? source.label : "Marketplace"

  const setAllEnabled = async (enabled: boolean) => {
    setBulkUpdating(true)
    try {
      await invoke("set_skills_enabled", {
        skillNames: skills.map((s) => s.name),
        enabled,
      } satisfies SetSkillsEnabledParams)
      toast.success(enabled ? `Enabled ${skills.length} skill(s)` : `Disabled ${skills.length} skill(s)`)
      onChanged()
    } catch (error) {
      console.error("Failed to update skills:", error)
      toast.error("Failed to update skills")
    } finally {
      setBulkUpdating(false)
    }
  }

  const toggleSkill = async (skill: SkillInfo, enabled: boolean) => {
    try {
      await invoke("set_skill_enabled", { skillName: skill.name, enabled } satisfies SetSkillEnabledParams)
      onChanged()
    } catch (error) {
      console.error("Failed to toggle skill:", error)
      toast.error("Failed to update skill")
    }
  }

  const openPath = async (path: string) => {
    try {
      await invoke("open_path", { path } satisfies OpenPathParams)
    } catch (error) {
      console.error("Failed to open path:", error)
      toast.error("Failed to open in file explorer")
    }
  }

  const removeSource = async () => {
    if (!source) return
    setRemoving(true)
    try {
      await invoke("remove_skill_source", { path: source.path } satisfies RemoveSkillSourceParams)
      toast.success(`Removed source "${source.label}"`)
      onRemoved()
    } catch (error) {
      console.error("Failed to remove source:", error)
      toast.error("Failed to remove source")
    } finally {
      setRemoving(false)
    }
  }

  return (
    <ScrollArea className="h-full">
      <div className="p-6 space-y-6">
        {/* Header */}
        <div className="flex items-start gap-4">
          <div className="h-11 w-11 rounded-lg bg-muted flex items-center justify-center shrink-0">
            <SkillSourceIcon kind={source ? source.kind : "marketplace_group"} className="h-5 w-5 text-muted-foreground" />
          </div>
          <div className="flex-1 min-w-0 space-y-1">
            <div className="flex items-center gap-2 flex-wrap">
              <h2 className="text-xl font-bold truncate">{title}</h2>
              <span className="text-xs px-2 py-0.5 rounded-full bg-muted text-muted-foreground">
                {source ? SOURCE_KIND_LABEL[source.kind] : "Installed skills"}
              </span>
              {source && !source.exists && (
                <span className="text-xs px-2 py-0.5 rounded-full bg-amber-500/10 text-amber-700 dark:text-amber-400 inline-flex items-center gap-1">
                  <TriangleAlert className="h-3 w-3" />
                  Missing
                </span>
              )}
            </div>
            {source ? (
              <p className="text-xs font-mono text-muted-foreground break-all">{source.path}</p>
            ) : (
              <p className="text-sm text-muted-foreground">
                Each marketplace skill is installed as its own source.
              </p>
            )}
            <p className="text-sm text-muted-foreground">
              {skills.length} skill{skills.length === 1 ? "" : "s"} · {enabledCount} enabled
            </p>
          </div>
        </div>

        {/* Actions */}
        <div className="flex flex-wrap items-center gap-2">
          {source?.exists && (
            <Button variant="outline" size="sm" onClick={() => openPath(source.path)}>
              <ExternalLink className="h-4 w-4 mr-1" />
              Open folder
            </Button>
          )}
          {source?.kind === "user" && (
            <Button variant="outline" size="sm" onClick={onCreateSkill}>
              <FilePlus className="h-4 w-4 mr-1" />
              New skill
            </Button>
          )}
          {!source && (
            <Button variant="outline" size="sm" onClick={onBrowseMarketplace}>
              <Store className="h-4 w-4 mr-1" />
              Browse marketplace
            </Button>
          )}
          <Button
            variant="outline"
            size="sm"
            onClick={() => setAllEnabled(true)}
            disabled={bulkUpdating || skills.length === 0 || enabledCount === skills.length}
          >
            Enable all
          </Button>
          <Button
            variant="outline"
            size="sm"
            onClick={() => setAllEnabled(false)}
            disabled={bulkUpdating || enabledCount === 0}
          >
            Disable all
          </Button>
          {bulkUpdating && <Loader2 className="h-4 w-4 animate-spin text-muted-foreground" />}
          {source && (
            <AlertDialog>
              <AlertDialogTrigger asChild>
                <Button variant="ghost" size="sm" className="ml-auto text-destructive hover:text-destructive" disabled={removing}>
                  <Trash2 className="h-4 w-4 mr-1" />
                  Remove source
                </Button>
              </AlertDialogTrigger>
              <AlertDialogContent>
                <AlertDialogHeader>
                  <AlertDialogTitle>Remove source "{source.label}"?</AlertDialogTitle>
                  <AlertDialogDescription>
                    LocalRouter stops loading the {skills.length} skill{skills.length === 1 ? "" : "s"} from{" "}
                    <span className="font-mono break-all">{source.path}</span>. Nothing is deleted from disk; you can
                    add the source again later.
                  </AlertDialogDescription>
                </AlertDialogHeader>
                <AlertDialogFooter>
                  <AlertDialogCancel>Cancel</AlertDialogCancel>
                  <AlertDialogAction
                    onClick={removeSource}
                    className="bg-destructive text-destructive-foreground hover:bg-destructive/90"
                  >
                    Remove
                  </AlertDialogAction>
                </AlertDialogFooter>
              </AlertDialogContent>
            </AlertDialog>
          )}
        </div>

        {/* Skills */}
        <div className="space-y-2">
          {skills.length > 8 && (
            <Input
              placeholder={`Filter ${skills.length} skills...`}
              value={filter}
              onChange={(e) => setFilter(e.target.value)}
            />
          )}
          {skills.length === 0 ? (
            <div className="rounded-lg border border-dashed p-8 text-center text-sm text-muted-foreground">
              {source && !source.exists
                ? "This path no longer exists. Restore it or remove the source."
                : "No skills found in this source. A skill is a folder containing a SKILL.md file."}
            </div>
          ) : visibleSkills.length === 0 ? (
            <p className="text-sm text-muted-foreground p-4 text-center">No skills match &ldquo;{filter.trim()}&rdquo;</p>
          ) : (
            <div className="rounded-lg border divide-y">
              {visibleSkills.map((skill) => (
                <div
                  key={skill.name}
                  onClick={() => onSelectSkill(skill.name)}
                  className="flex items-center gap-3 px-4 py-3 cursor-pointer hover:bg-muted/40 transition-colors"
                >
                  <div className={cn("flex-1 min-w-0", !skill.enabled && "opacity-50")}>
                    <p className="font-medium truncate">{skill.name}</p>
                    {skill.description && (
                      <p className="text-xs text-muted-foreground line-clamp-2">{skill.description}</p>
                    )}
                    {skill.tags.length > 0 && (
                      <div className="flex flex-wrap gap-1 mt-1">
                        {skill.tags.slice(0, 5).map((tag) => (
                          <span key={tag} className="text-[10px] px-1.5 py-0.5 rounded-full bg-muted text-muted-foreground">
                            {tag}
                          </span>
                        ))}
                      </div>
                    )}
                  </div>
                  <div onClick={(e) => e.stopPropagation()} className="shrink-0">
                    <Switch
                      checked={skill.enabled}
                      onCheckedChange={(checked) => toggleSkill(skill, checked)}
                      aria-label={`${skill.enabled ? "Disable" : "Enable"} ${skill.name}`}
                    />
                  </div>
                </div>
              ))}
            </div>
          )}
        </div>
      </div>
    </ScrollArea>
  )
}
