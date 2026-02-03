import { useState, useEffect } from "react"
import { invoke } from "@tauri-apps/api/core"
import { toast } from "sonner"
import {
  Search,
  Download,
  Loader2,
  Package,
  Globe,
  Check,
} from "lucide-react"
import { McpIcon, SkillsIcon } from "@/components/icons/category-icons"
import { Button } from "@/components/ui/Button"
import { Card } from "@/components/ui/Card"
import { Input } from "@/components/ui/Input"
import { ScrollArea } from "@/components/ui/scroll-area"
import { Badge } from "@/components/ui/Badge"
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/Select"
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog"
import {
  AlertDialog,
  AlertDialogAction,
  AlertDialogCancel,
  AlertDialogContent,
  AlertDialogDescription,
  AlertDialogFooter,
  AlertDialogHeader,
  AlertDialogTitle,
} from "@/components/ui/alert-dialog"
import { Label } from "@/components/ui/label"
import { DisabledOverlay } from "./DisabledOverlay"

// Types matching the backend
interface McpPackageInfo {
  registry: string
  name: string
  version: string | null
  runtime: string | null
  license: string | null
}

interface McpRemoteInfo {
  transport_type: string
  url: string
}

interface McpServerListing {
  name: string
  description: string
  source_id: string
  homepage: string | null
  vendor: string | null
  packages: McpPackageInfo[]
  remotes: McpRemoteInfo[]
  available_transports: string[]
  install_hint: string | null
}

interface SkillFileInfo {
  path: string
  url: string
}

interface SkillListing {
  name: string
  description: string | null
  source_id: string
  version: string | null
  author: string | null
  tags: string[]
  source_label: string
  source_repo: string
  source_path: string
  source_branch: string
  skill_md_url: string
  is_multi_file: boolean
  files: SkillFileInfo[]
}

interface McpInstallConfig {
  name: string
  transport: string
  command: string | null
  args: string[]
  url: string | null
  env: Record<string, string>
  auth_type: string
  bearer_token: string | null
  headers: Record<string, string>
}

interface MarketplaceConfig {
  enabled: boolean
  registry_url: string
  skill_sources: { repo_url: string; branch: string; path: string; label: string }[]
}

type ResourceType = "mcp" | "skill"

interface MarketplaceSearchPanelProps {
  type: ResourceType
  onInstallComplete?: () => void
  className?: string
  maxHeight?: string
}

export function MarketplaceSearchPanel({
  type,
  onInstallComplete,
  className,
  maxHeight = "400px",
}: MarketplaceSearchPanelProps) {
  const [config, setConfig] = useState<MarketplaceConfig | null>(null)
  const [loading, setLoading] = useState(true)

  // MCP state
  const [mcpSearch, setMcpSearch] = useState("")
  const [mcpServers, setMcpServers] = useState<McpServerListing[]>([])
  const [searchingMcp, setSearchingMcp] = useState(false)
  const [selectedMcpServer, setSelectedMcpServer] = useState<McpServerListing | null>(null)
  const [showMcpInstallDialog, setShowMcpInstallDialog] = useState(false)
  const [installingMcp, setInstallingMcp] = useState(false)

  // MCP Install form state
  const [installTransport, setInstallTransport] = useState<string>("stdio")
  const [installCommand, setInstallCommand] = useState("")
  const [installUrl, setInstallUrl] = useState("")
  const [installAuthType, setInstallAuthType] = useState<string>("none")
  const [installBearerToken, setInstallBearerToken] = useState("")

  // Skill state
  const [skillSearch, setSkillSearch] = useState("")
  const [skills, setSkills] = useState<SkillListing[]>([])
  const [searchingSkills, setSearchingSkills] = useState(false)
  const [selectedSkillSource, setSelectedSkillSource] = useState<string>("all")
  const [selectedSkill, setSelectedSkill] = useState<SkillListing | null>(null)
  const [showSkillInstallDialog, setShowSkillInstallDialog] = useState(false)
  const [installingSkill, setInstallingSkill] = useState(false)

  useEffect(() => {
    loadConfig()
  }, [])

  useEffect(() => {
    if (config?.enabled) {
      if (type === "mcp") {
        loadMcpServers()
      } else {
        searchSkills()
      }
    }
  }, [config?.enabled, type])

  const loadConfig = async () => {
    try {
      const cfg = await invoke<MarketplaceConfig>("marketplace_get_config")
      setConfig(cfg)
    } catch (error) {
      console.error("Failed to load marketplace config:", error)
    } finally {
      setLoading(false)
    }
  }

  const handleEnableMarketplace = async () => {
    try {
      await invoke("marketplace_set_enabled", { enabled: true })
      await loadConfig()
      toast.success("Marketplace enabled")
    } catch (error) {
      console.error("Failed to enable marketplace:", error)
      toast.error("Failed to enable marketplace")
    }
  }

  // MCP functions
  const loadMcpServers = async () => {
    setSearchingMcp(true)
    try {
      const results = await invoke<McpServerListing[]>("marketplace_search_mcp_servers", {
        query: mcpSearch.trim() || "mcp",
        limit: 20,
      })
      setMcpServers(results)
    } catch (error) {
      console.error("Failed to load MCP servers:", error)
      toast.error(`Failed to search MCP servers: ${error}`)
    } finally {
      setSearchingMcp(false)
    }
  }

  const handleMcpInstallClick = (server: McpServerListing) => {
    setSelectedMcpServer(server)
    if (server.available_transports.includes("stdio")) {
      setInstallTransport("stdio")
      if (server.packages.length > 0) {
        const pkg = server.packages[0]
        if (pkg.runtime === "node" || pkg.registry === "npm") {
          setInstallCommand(`npx -y ${pkg.name}`)
        } else if (pkg.runtime === "python" || pkg.registry === "pypi") {
          setInstallCommand(`uvx ${pkg.name}`)
        }
      }
    } else if (server.remotes.length > 0) {
      setInstallTransport("http_sse")
      setInstallUrl(server.remotes[0].url)
    }
    setShowMcpInstallDialog(true)
  }

  const handleMcpInstall = async () => {
    if (!selectedMcpServer) return

    setInstallingMcp(true)
    try {
      const installConfig: McpInstallConfig = {
        name: selectedMcpServer.name,
        transport: installTransport,
        command: installTransport === "stdio" ? installCommand : null,
        args: [],
        url: installTransport === "http_sse" ? installUrl : null,
        env: {},
        auth_type: installAuthType,
        bearer_token: installAuthType === "bearer" ? installBearerToken : null,
        headers: {},
      }

      await invoke("marketplace_install_mcp_server_direct", { config: installConfig })
      toast.success(`Installed ${selectedMcpServer.name}`)
      setShowMcpInstallDialog(false)
      resetMcpInstallForm()
      onInstallComplete?.()
    } catch (error) {
      console.error("Failed to install MCP server:", error)
      toast.error(`Failed to install: ${error}`)
    } finally {
      setInstallingMcp(false)
    }
  }

  const resetMcpInstallForm = () => {
    setSelectedMcpServer(null)
    setInstallTransport("stdio")
    setInstallCommand("")
    setInstallUrl("")
    setInstallAuthType("none")
    setInstallBearerToken("")
  }

  // Skill functions
  const searchSkills = async () => {
    setSearchingSkills(true)
    try {
      const results = await invoke<SkillListing[]>("marketplace_search_skills", {
        query: skillSearch.trim() || null,
        source: selectedSkillSource === "all" ? null : selectedSkillSource,
      })
      setSkills(results)
    } catch (error) {
      console.error("Failed to search skills:", error)
      toast.error(`Failed to search skills: ${error}`)
    } finally {
      setSearchingSkills(false)
    }
  }

  const handleSkillInstallClick = (skill: SkillListing) => {
    setSelectedSkill(skill)
    setShowSkillInstallDialog(true)
  }

  const handleSkillInstall = async () => {
    if (!selectedSkill) return

    setInstallingSkill(true)
    try {
      await invoke("marketplace_install_skill_direct", {
        sourceUrl: selectedSkill.skill_md_url,
        skillName: selectedSkill.name,
      })
      toast.success(`Installed ${selectedSkill.name}`)
      setShowSkillInstallDialog(false)
      setSelectedSkill(null)
      onInstallComplete?.()
    } catch (error) {
      console.error("Failed to install skill:", error)
      toast.error(`Failed to install: ${error}`)
    } finally {
      setInstallingSkill(false)
    }
  }

  if (loading) {
    return (
      <div className="flex items-center justify-center py-12">
        <Loader2 className="h-8 w-8 animate-spin text-muted-foreground" />
      </div>
    )
  }

  if (!config?.enabled) {
    return (
      <DisabledOverlay
        title="Marketplace Disabled"
        description="Enable the marketplace to browse and install resources from online registries."
        actionLabel="Enable Marketplace"
        onAction={handleEnableMarketplace}
        className={className}
      >
        <div className="h-64" />
      </DisabledOverlay>
    )
  }

  // MCP Search Panel
  if (type === "mcp") {
    return (
      <div className={className}>
        {/* Search bar */}
        <div className="flex gap-2 mb-4">
          <div className="relative flex-1">
            <Search className="absolute left-3 top-1/2 -translate-y-1/2 h-4 w-4 text-muted-foreground" />
            <Input
              placeholder="Search MCP servers..."
              value={mcpSearch}
              onChange={(e) => setMcpSearch(e.target.value)}
              onKeyDown={(e) => e.key === "Enter" && loadMcpServers()}
              className="pl-9"
            />
          </div>
          <Button onClick={loadMcpServers} disabled={searchingMcp} size="sm">
            {searchingMcp ? <Loader2 className="h-4 w-4 animate-spin" /> : "Search"}
          </Button>
        </div>

        {/* Results */}
        <ScrollArea style={{ maxHeight }}>
          {searchingMcp ? (
            <div className="flex items-center justify-center py-12">
              <Loader2 className="h-6 w-6 animate-spin text-muted-foreground" />
            </div>
          ) : mcpServers.length === 0 ? (
            <div className="flex flex-col items-center justify-center py-8 text-muted-foreground">
              <McpIcon className="h-8 w-8 mb-2" />
              <p className="text-sm">No MCP servers found</p>
            </div>
          ) : (
            <div className="space-y-3">
              {mcpServers.map((server) => (
                <Card key={server.name} className="p-3">
                  <div className="flex items-start justify-between gap-2">
                    <div className="flex-1 min-w-0">
                      <p className="font-medium text-sm truncate">{server.name}</p>
                      {server.vendor && (
                        <p className="text-xs text-muted-foreground">by {server.vendor}</p>
                      )}
                      <p className="text-xs text-muted-foreground line-clamp-2 mt-1">
                        {server.description}
                      </p>
                      <div className="flex gap-1 mt-2">
                        {server.available_transports.includes("stdio") && (
                          <Badge variant="secondary" className="text-xs">
                            <Package className="h-3 w-3 mr-1" />
                            stdio
                          </Badge>
                        )}
                        {server.remotes.length > 0 && (
                          <Badge variant="secondary" className="text-xs">
                            <Globe className="h-3 w-3 mr-1" />
                            remote
                          </Badge>
                        )}
                      </div>
                    </div>
                    <Button size="sm" onClick={() => handleMcpInstallClick(server)}>
                      <Download className="h-4 w-4" />
                    </Button>
                  </div>
                </Card>
              ))}
            </div>
          )}
        </ScrollArea>

        {/* MCP Install Dialog */}
        <Dialog open={showMcpInstallDialog} onOpenChange={setShowMcpInstallDialog}>
          <DialogContent className="max-w-md">
            <DialogHeader>
              <DialogTitle>Install {selectedMcpServer?.name}</DialogTitle>
              <DialogDescription>
                Configure how to connect to this MCP server
              </DialogDescription>
            </DialogHeader>

            <div className="space-y-4">
              <div className="space-y-2">
                <Label>Transport</Label>
                <Select value={installTransport} onValueChange={setInstallTransport}>
                  <SelectTrigger>
                    <SelectValue />
                  </SelectTrigger>
                  <SelectContent>
                    {selectedMcpServer?.available_transports.includes("stdio") && (
                      <SelectItem value="stdio">Stdio (Local Process)</SelectItem>
                    )}
                    {(selectedMcpServer?.remotes.length ?? 0) > 0 && (
                      <SelectItem value="http_sse">HTTP+SSE (Remote)</SelectItem>
                    )}
                  </SelectContent>
                </Select>
              </div>

              {installTransport === "stdio" && (
                <div className="space-y-2">
                  <Label>Command</Label>
                  <Input
                    value={installCommand}
                    onChange={(e) => setInstallCommand(e.target.value)}
                    placeholder="npx -y @modelcontextprotocol/server-name"
                  />
                  {selectedMcpServer?.install_hint && (
                    <p className="text-xs text-muted-foreground">{selectedMcpServer.install_hint}</p>
                  )}
                </div>
              )}

              {installTransport === "http_sse" && (
                <>
                  <div className="space-y-2">
                    <Label>URL</Label>
                    <Input
                      value={installUrl}
                      onChange={(e) => setInstallUrl(e.target.value)}
                      placeholder="https://server.example.com/mcp"
                    />
                  </div>

                  <div className="space-y-2">
                    <Label>Authentication</Label>
                    <Select value={installAuthType} onValueChange={setInstallAuthType}>
                      <SelectTrigger>
                        <SelectValue />
                      </SelectTrigger>
                      <SelectContent>
                        <SelectItem value="none">None</SelectItem>
                        <SelectItem value="bearer">Bearer Token</SelectItem>
                      </SelectContent>
                    </Select>
                  </div>

                  {installAuthType === "bearer" && (
                    <div className="space-y-2">
                      <Label>Bearer Token</Label>
                      <Input
                        type="password"
                        value={installBearerToken}
                        onChange={(e) => setInstallBearerToken(e.target.value)}
                        placeholder="Enter your API token"
                      />
                    </div>
                  )}
                </>
              )}
            </div>

            <DialogFooter>
              <Button variant="outline" onClick={() => setShowMcpInstallDialog(false)}>
                Cancel
              </Button>
              <Button onClick={handleMcpInstall} disabled={installingMcp}>
                {installingMcp ? (
                  <>
                    <Loader2 className="h-4 w-4 mr-2 animate-spin" />
                    Installing...
                  </>
                ) : (
                  <>
                    <Check className="h-4 w-4 mr-2" />
                    Install
                  </>
                )}
              </Button>
            </DialogFooter>
          </DialogContent>
        </Dialog>
      </div>
    )
  }

  // Skill Search Panel
  return (
    <div className={className}>
      {/* Search bar */}
      <div className="flex gap-2 mb-4">
        <div className="relative flex-1">
          <Search className="absolute left-3 top-1/2 -translate-y-1/2 h-4 w-4 text-muted-foreground" />
          <Input
            placeholder="Search skills..."
            value={skillSearch}
            onChange={(e) => setSkillSearch(e.target.value)}
            onKeyDown={(e) => e.key === "Enter" && searchSkills()}
            className="pl-9"
          />
        </div>
        <Select value={selectedSkillSource} onValueChange={setSelectedSkillSource}>
          <SelectTrigger className="w-[140px]">
            <SelectValue placeholder="Source" />
          </SelectTrigger>
          <SelectContent>
            <SelectItem value="all">All Sources</SelectItem>
            {config?.skill_sources.map((source) => (
              <SelectItem key={source.repo_url} value={source.label}>
                {source.label}
              </SelectItem>
            ))}
          </SelectContent>
        </Select>
        <Button onClick={searchSkills} disabled={searchingSkills} size="sm">
          {searchingSkills ? <Loader2 className="h-4 w-4 animate-spin" /> : "Search"}
        </Button>
      </div>

      {/* Results */}
      <ScrollArea style={{ maxHeight }}>
        {searchingSkills ? (
          <div className="flex items-center justify-center py-12">
            <Loader2 className="h-6 w-6 animate-spin text-muted-foreground" />
          </div>
        ) : skills.length === 0 ? (
          <div className="flex flex-col items-center justify-center py-8 text-muted-foreground">
            <SkillsIcon className="h-8 w-8 mb-2" />
            <p className="text-sm">No skills found</p>
          </div>
        ) : (
          <div className="space-y-3">
            {skills.slice(0, 20).map((skill) => (
              <Card key={`${skill.source_label}-${skill.name}`} className="p-3">
                <div className="flex items-start justify-between gap-2">
                  <div className="flex-1 min-w-0">
                    <p className="font-medium text-sm truncate">{skill.name}</p>
                    <div className="flex items-center gap-1 text-xs text-muted-foreground">
                      {skill.author && <span>by {skill.author}</span>}
                      <Badge variant="outline" className="text-xs ml-1">
                        {skill.source_label}
                      </Badge>
                    </div>
                    <p className="text-xs text-muted-foreground line-clamp-2 mt-1">
                      {skill.description || "No description available"}
                    </p>
                    {skill.tags.length > 0 && (
                      <div className="flex flex-wrap gap-1 mt-2">
                        {skill.tags.slice(0, 3).map((tag) => (
                          <Badge key={tag} variant="secondary" className="text-xs">
                            {tag}
                          </Badge>
                        ))}
                      </div>
                    )}
                  </div>
                  <Button size="sm" onClick={() => handleSkillInstallClick(skill)}>
                    <Download className="h-4 w-4" />
                  </Button>
                </div>
              </Card>
            ))}
          </div>
        )}
      </ScrollArea>

      {/* Skill Install Dialog */}
      <AlertDialog open={showSkillInstallDialog} onOpenChange={setShowSkillInstallDialog}>
        <AlertDialogContent>
          <AlertDialogHeader>
            <AlertDialogTitle>Install {selectedSkill?.name}?</AlertDialogTitle>
            <AlertDialogDescription>
              This will download the skill files from GitHub and add them to your local skills directory.
              {selectedSkill?.files.length ? ` (${selectedSkill.files.length + 1} files)` : ""}
            </AlertDialogDescription>
          </AlertDialogHeader>
          <AlertDialogFooter>
            <AlertDialogCancel disabled={installingSkill}>Cancel</AlertDialogCancel>
            <AlertDialogAction onClick={handleSkillInstall} disabled={installingSkill}>
              {installingSkill ? (
                <>
                  <Loader2 className="h-4 w-4 mr-2 animate-spin" />
                  Installing...
                </>
              ) : (
                "Install"
              )}
            </AlertDialogAction>
          </AlertDialogFooter>
        </AlertDialogContent>
      </AlertDialog>
    </div>
  )
}
