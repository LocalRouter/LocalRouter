import { useState, useEffect } from "react"
import { invoke } from "@tauri-apps/api/core"
import { open } from "@tauri-apps/plugin-dialog"
import { toast } from "sonner"
import { RefreshCw, Folder, Plus, Trash2, FolderOpen, Loader2, Store, FilePlus, Import } from "lucide-react"
import { Button } from "@/components/ui/Button"
import { Input } from "@/components/ui/Input"
import { Label } from "@/components/ui/label"
import { Textarea } from "@/components/ui/textarea"
import { Tabs, TabsList, TabsTrigger, TabsContent } from "@/components/ui/tabs"
import { Dialog, DialogContent, DialogHeader, DialogTitle, DialogDescription } from "@/components/ui/Modal"
import { MarketplaceSearchPanel, type SkillListing } from "@/components/add-resource/MarketplaceSearchPanel"
import type {
  AddSkillSourceParams,
  CreateSkillParams,
  MarketplaceInstallSkillDirectParams,
  RemoveSkillSourceParams,
  SkillInfo,
  SkillsConfig,
} from "@/types/tauri-commands"

export type AddSkillsTab = "paths" | "marketplace" | "new"

interface AddSkillsDialogProps {
  open: boolean
  onOpenChange: (open: boolean) => void
  tab: AddSkillsTab
  onTabChange: (tab: AddSkillsTab) => void
  installedSkillNames: string[]
  /** Called after skills or sources changed */
  onChanged: () => void
}

export function AddSkillsDialog({
  open: isOpen,
  onOpenChange,
  tab,
  onTabChange,
  installedSkillNames,
  onChanged,
}: AddSkillsDialogProps) {
  const [skillPaths, setSkillPaths] = useState<string[]>([])
  const [newPath, setNewPath] = useState("")
  const [addingPath, setAddingPath] = useState(false)
  const [removingPath, setRemovingPath] = useState<string | null>(null)
  const [rescanning, setRescanning] = useState(false)

  const [newSkillName, setNewSkillName] = useState("")
  const [newSkillDescription, setNewSkillDescription] = useState("")
  const [newSkillContent, setNewSkillContent] = useState("")
  const [isCreatingSkill, setIsCreatingSkill] = useState(false)

  const loadSkillPaths = async () => {
    try {
      const config = await invoke<SkillsConfig>("get_skills_config")
      setSkillPaths(config.paths)
    } catch (error) {
      console.error("Failed to load skills config:", error)
    }
  }

  useEffect(() => {
    if (isOpen) loadSkillPaths()
  }, [isOpen])

  const handleRescan = async () => {
    setRescanning(true)
    try {
      const result = await invoke<SkillInfo[]>("rescan_skills")
      toast.success(`Found ${result.length} skill(s)`)
      onChanged()
    } catch (error) {
      console.error("Failed to rescan skills:", error)
      toast.error("Failed to rescan skills")
    } finally {
      setRescanning(false)
    }
  }

  const handleSelectMarketplaceSkill = async (listing: SkillListing) => {
    try {
      await invoke("marketplace_install_skill_direct", {
        sourceUrl: listing.skill_md_url,
        skillName: listing.name,
      } satisfies MarketplaceInstallSkillDirectParams)
      toast.success(`Skill "${listing.name}" installed`)
      onOpenChange(false)
      onChanged()
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
      } satisfies CreateSkillParams)
      toast.success(`Skill "${newSkillName.trim()}" created`)
      setNewSkillName("")
      setNewSkillDescription("")
      setNewSkillContent("")
      onOpenChange(false)
      onChanged()
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
      await invoke("add_skill_source", { path: path.trim() } satisfies AddSkillSourceParams)
      toast.success("Skill path added")
      setNewPath("")
      await loadSkillPaths()
      onChanged()
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
      await invoke("remove_skill_source", { path } satisfies RemoveSkillSourceParams)
      toast.success("Skill path removed")
      await loadSkillPaths()
      onChanged()
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

  return (
    <Dialog open={isOpen} onOpenChange={onOpenChange}>
      <DialogContent className="max-w-2xl max-h-[85vh] flex flex-col overflow-hidden">
        <DialogHeader className="flex-shrink-0">
          <DialogTitle>Add Skills</DialogTitle>
          <DialogDescription>
            Add skills from the marketplace, create new ones, or manage local skill sources.
          </DialogDescription>
        </DialogHeader>

        <Tabs value={tab} onValueChange={(v) => onTabChange(v as AddSkillsTab)} className="flex-1 flex flex-col min-h-0">
          <TabsList className="grid w-full grid-cols-3 flex-shrink-0">
            <TabsTrigger value="new" className="flex items-center gap-1.5">
              <FilePlus className="h-3.5 w-3.5" />
              New
            </TabsTrigger>
            <TabsTrigger value="paths" className="flex items-center gap-1.5">
              <Import className="h-3.5 w-3.5" />
              Import
            </TabsTrigger>
            <TabsTrigger value="marketplace" className="flex items-center gap-1.5">
              <Store className="h-3.5 w-3.5" />
              Marketplace
            </TabsTrigger>
          </TabsList>

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
                  The skill content (markdown body after the YAML frontmatter). This will be saved as a SKILL.md file
                  in the &ldquo;My Skills&rdquo; source.
                </p>
              </div>

              <div className="flex justify-end gap-2 pt-2">
                <Button type="button" variant="outline" onClick={() => onOpenChange(false)}>
                  Cancel
                </Button>
                <Button type="submit" disabled={!newSkillName.trim() || isCreatingSkill}>
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

          {/* Import Tab */}
          <TabsContent value="paths" className="flex-1 flex flex-col min-h-0 mt-4">
            <div className="space-y-4 flex-1 overflow-y-auto">
              <p className="text-sm text-muted-foreground">
                A source is a folder of skills, a single skill folder, or a <code>.zip</code>/<code>.skill</code> file.
                Folders are watched, so skills added to them later appear automatically.
              </p>

              {/* Existing Paths */}
              <div className="space-y-2">
                <p className="text-sm font-medium">Current Sources</p>
                {skillPaths.length === 0 ? (
                  <p className="text-sm text-muted-foreground py-2">No skill sources configured</p>
                ) : (
                  <div className="space-y-1">
                    {skillPaths.map((path) => (
                      <div key={path} className="flex items-center gap-2 p-2 rounded-md bg-muted/50 group">
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
                          title="Remove source"
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
                <p className="text-sm font-medium">Add Source</p>
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
                <Button variant="outline" size="sm" className="w-full" onClick={handleBrowse} disabled={addingPath}>
                  <FolderOpen className="h-4 w-4 mr-2" />
                  Browse...
                </Button>
              </div>
            </div>

            {/* Footer with Rescan */}
            <div className="flex-shrink-0 pt-4 border-t flex justify-between items-center mt-4">
              <Button variant="outline" size="sm" onClick={handleRescan} disabled={rescanning}>
                <RefreshCw className={`h-4 w-4 mr-2 ${rescanning ? "animate-spin" : ""}`} />
                Rescan Skills
              </Button>
              <Button variant="default" size="sm" onClick={() => onOpenChange(false)}>
                Done
              </Button>
            </div>
          </TabsContent>

          {/* Marketplace Tab */}
          <TabsContent value="marketplace" className="flex-1 min-h-0 mt-4">
            <MarketplaceSearchPanel
              type="skill"
              onSelectSkill={handleSelectMarketplaceSkill}
              installedSkillNames={installedSkillNames}
              maxHeight="calc(85vh - 220px)"
            />
          </TabsContent>
        </Tabs>
      </DialogContent>
    </Dialog>
  )
}
