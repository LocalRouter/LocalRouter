import { useState, useEffect } from "react"
import { invoke } from "@tauri-apps/api/core"
import { open } from "@tauri-apps/plugin-shell"
import { toast } from "sonner"
import {
  Search,
  Download,
  ExternalLink,
  AlertCircle,
  Loader2,
  Package,
  Globe,
  Check,
  Settings,
  Plus,
  Trash2,
  RefreshCw,
} from "lucide-react"
import { McpIcon, SkillsIcon, StoreIcon } from "@/components/icons/category-icons"
import { Button } from "@/components/ui/Button"
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/Card"
import { Input } from "@/components/ui/Input"
import { ScrollArea } from "@/components/ui/scroll-area"
import { Tabs, TabsContent, TabsList, TabsTrigger } from "@/components/ui/tabs"
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

interface MarketplaceConfig {
  enabled: boolean
  registry_url: string
  skill_sources: MarketplaceSkillSource[]
}

interface MarketplaceSkillSource {
  repo_url: string
  branch: string
  path: string
  label: string
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

interface CacheStatus {
  mcp_last_refresh: string | null
  skills_last_refresh: string | null
  mcp_cached_queries: number
  skills_cached_sources: number
}

interface MarketplaceViewProps {
  activeSubTab: string | null
  onTabChange: (view: string, subTab?: string | null) => void
}

export function MarketplaceView({ activeSubTab: _activeSubTab, onTabChange: _onTabChange }: MarketplaceViewProps) {
  const [config, setConfig] = useState<MarketplaceConfig | null>(null)
  const [loading, setLoading] = useState(true)
  const [showEnableDialog, setShowEnableDialog] = useState(false)

  // MCP Servers tab state
  const [mcpSearch, setMcpSearch] = useState("")
  const [mcpServers, setMcpServers] = useState<McpServerListing[]>([])
  const [searchingMcp, setSearchingMcp] = useState(false)
  const [mcpLimit, setMcpLimit] = useState(20)
  const [mcpHasMore, setMcpHasMore] = useState(false)
  const [loadingMoreMcp, setLoadingMoreMcp] = useState(false)
  const [selectedMcpServer, setSelectedMcpServer] = useState<McpServerListing | null>(null)
  const [showMcpInstallDialog, setShowMcpInstallDialog] = useState(false)
  const [installingMcp, setInstallingMcp] = useState(false)

  // Skills tab state
  const [skillSearch, setSkillSearch] = useState("")
  const [skills, setSkills] = useState<SkillListing[]>([])
  const [searchingSkills, setSearchingSkills] = useState(false)
  const [skillsPage, setSkillsPage] = useState(1)
  const skillsPerPage = 20
  const [selectedSkillSource, setSelectedSkillSource] = useState<string>("all")
  const [selectedSkill, setSelectedSkill] = useState<SkillListing | null>(null)
  const [showSkillInstallDialog, setShowSkillInstallDialog] = useState(false)
  const [installingSkill, setInstallingSkill] = useState(false)

  // MCP Install form state
  const [installTransport, setInstallTransport] = useState<string>("stdio")
  const [installCommand, setInstallCommand] = useState("")
  const [installUrl, setInstallUrl] = useState("")
  const [installEnv, setInstallEnv] = useState<Record<string, string>>({})
  const [installAuthType, setInstallAuthType] = useState<string>("none")
  const [installBearerToken, setInstallBearerToken] = useState("")

  // Settings dialog state
  const [showSettingsDialog, setShowSettingsDialog] = useState(false)
  const [settingsRegistryUrl, setSettingsRegistryUrl] = useState("")
  const [settingsSkillSources, setSettingsSkillSources] = useState<MarketplaceSkillSource[]>([])
  const [savingSettings, setSavingSettings] = useState(false)
  const [cacheStatus, setCacheStatus] = useState<CacheStatus | null>(null)
  const [refreshingCache, setRefreshingCache] = useState(false)
  const [disablingMarketplace, setDisablingMarketplace] = useState(false)
  const [resettingRegistryUrl, setResettingRegistryUrl] = useState(false)
  const [addingDefaultSources, setAddingDefaultSources] = useState(false)
  // New source form
  const [newSourceRepoUrl, setNewSourceRepoUrl] = useState("")
  const [newSourceBranch, setNewSourceBranch] = useState("main")
  const [newSourcePath, setNewSourcePath] = useState("")
  const [newSourceLabel, setNewSourceLabel] = useState("")

  useEffect(() => {
    loadConfig()
  }, [])

  useEffect(() => {
    if (config?.enabled) {
      // Auto-load on enable
      loadMcpServers()
      searchSkills()
    }
  }, [config?.enabled])

  const loadMcpServers = async (limit = 20, append = false) => {
    if (append) {
      setLoadingMoreMcp(true)
    } else {
      setSearchingMcp(true)
      setMcpLimit(20)
    }
    try {
      const results = await invoke<McpServerListing[]>("marketplace_search_mcp_servers", {
        query: mcpSearch.trim() || "mcp",
        limit: limit,
      })
      if (append) {
        setMcpServers(results)
      } else {
        setMcpServers(results)
      }
      // If we got exactly the limit, there might be more
      setMcpHasMore(results.length >= limit)
      setMcpLimit(limit)
    } catch (error) {
      console.error("Failed to load MCP servers:", error)
      toast.error(`Failed to search MCP servers: ${error}`)
    } finally {
      setSearchingMcp(false)
      setLoadingMoreMcp(false)
    }
  }

  const loadMoreMcp = () => {
    const newLimit = mcpLimit + 20
    loadMcpServers(newLimit, true)
  }

  const loadConfig = async (): Promise<MarketplaceConfig | null> => {
    try {
      const cfg = await invoke<MarketplaceConfig>("marketplace_get_config")
      setConfig(cfg)
      if (!cfg.enabled) {
        setShowEnableDialog(true)
      }
      return cfg
    } catch (error) {
      console.error("Failed to load marketplace config:", error)
      toast.error("Failed to load marketplace configuration")
      return null
    } finally {
      setLoading(false)
    }
  }

  const handleEnableMarketplace = async () => {
    try {
      await invoke("marketplace_set_enabled", { enabled: true })
      setShowEnableDialog(false)
      await loadConfig()
      toast.success("Marketplace enabled")
    } catch (error) {
      console.error("Failed to enable marketplace:", error)
      toast.error("Failed to enable marketplace")
    }
  }

  const searchMcpServers = async () => {
    loadMcpServers(20, false)
  }

  // Paginated skills
  const paginatedSkills = skills.slice(0, skillsPage * skillsPerPage)
  const hasMoreSkills = skills.length > skillsPage * skillsPerPage

  const loadMoreSkills = () => {
    setSkillsPage(prev => prev + 1)
  }

  const searchSkills = async () => {
    setSearchingSkills(true)
    setSkillsPage(1) // Reset pagination on new search
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

  const handleMcpInstallClick = (server: McpServerListing) => {
    setSelectedMcpServer(server)
    // Pre-populate install form based on listing
    if (server.available_transports.includes("stdio")) {
      setInstallTransport("stdio")
      // Try to get default command from packages
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
        env: installEnv,
        auth_type: installAuthType,
        bearer_token: installAuthType === "bearer" ? installBearerToken : null,
        headers: {},
      }

      await invoke("marketplace_install_mcp_server_direct", { config: installConfig })
      toast.success(`Installed ${selectedMcpServer.name}`)
      setShowMcpInstallDialog(false)
      resetMcpInstallForm()
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
    setInstallEnv({})
    setInstallAuthType("none")
    setInstallBearerToken("")
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
    } catch (error) {
      console.error("Failed to install skill:", error)
      toast.error(`Failed to install: ${error}`)
    } finally {
      setInstallingSkill(false)
    }
  }

  const openSettings = async () => {
    if (config) {
      setSettingsRegistryUrl(config.registry_url)
      setSettingsSkillSources([...config.skill_sources])
    }
    // Load cache status
    try {
      const status = await invoke<CacheStatus>("marketplace_get_cache_status")
      setCacheStatus(status)
    } catch (error) {
      console.error("Failed to get cache status:", error)
    }
    setShowSettingsDialog(true)
  }

  const refreshCache = async () => {
    setRefreshingCache(true)
    try {
      await invoke("marketplace_refresh_cache")
      toast.success("Cache refreshed")
      // Reload cache status
      const status = await invoke<CacheStatus>("marketplace_get_cache_status")
      setCacheStatus(status)
      // Reload data
      loadMcpServers()
      searchSkills()
    } catch (error) {
      console.error("Failed to refresh cache:", error)
      toast.error(`Failed to refresh cache: ${error}`)
    } finally {
      setRefreshingCache(false)
    }
  }

  const handleDisableMarketplace = async () => {
    setDisablingMarketplace(true)
    try {
      // Clear caches first
      await invoke("marketplace_clear_mcp_cache")
      await invoke("marketplace_clear_skills_cache")
      // Disable marketplace
      await invoke("marketplace_set_enabled", { enabled: false })
      toast.success("Marketplace disabled")
      setShowSettingsDialog(false)
      // Reload config to show disabled state
      await loadConfig()
    } catch (error) {
      console.error("Failed to disable marketplace:", error)
      toast.error(`Failed to disable marketplace: ${error}`)
    } finally {
      setDisablingMarketplace(false)
    }
  }

  const handleResetRegistryUrl = async () => {
    setResettingRegistryUrl(true)
    try {
      const defaultUrl = await invoke<string>("marketplace_reset_registry_url")
      setSettingsRegistryUrl(defaultUrl)
      toast.success("Registry URL reset to default")
    } catch (error) {
      console.error("Failed to reset registry URL:", error)
      toast.error(`Failed to reset registry URL: ${error}`)
    } finally {
      setResettingRegistryUrl(false)
    }
  }

  const handleAddDefaultSources = async () => {
    setAddingDefaultSources(true)
    try {
      const addedCount = await invoke<number>("marketplace_add_default_skill_sources")
      if (addedCount > 0) {
        toast.success(`Added ${addedCount} default source${addedCount > 1 ? 's' : ''}`)
        // Reload config to get updated sources
        const updatedConfig = await loadConfig()
        if (updatedConfig) {
          setSettingsSkillSources([...updatedConfig.skill_sources])
        }
      } else {
        toast.info("All default sources are already added")
      }
    } catch (error) {
      console.error("Failed to add default sources:", error)
      toast.error(`Failed to add default sources: ${error}`)
    } finally {
      setAddingDefaultSources(false)
    }
  }

  const formatLastRefresh = (timestamp: string | null): string => {
    if (!timestamp) return "Never"
    try {
      const date = new Date(timestamp)
      return date.toLocaleString()
    } catch {
      return "Unknown"
    }
  }

  const addSkillSource = () => {
    if (!newSourceRepoUrl.trim() || !newSourceLabel.trim()) {
      toast.error("Repository URL and label are required")
      return
    }
    const newSource: MarketplaceSkillSource = {
      repo_url: newSourceRepoUrl.trim(),
      branch: newSourceBranch.trim() || "main",
      path: newSourcePath.trim(),
      label: newSourceLabel.trim(),
    }
    setSettingsSkillSources([...settingsSkillSources, newSource])
    // Reset form
    setNewSourceRepoUrl("")
    setNewSourceBranch("main")
    setNewSourcePath("")
    setNewSourceLabel("")
  }

  const removeSkillSource = (index: number) => {
    setSettingsSkillSources(settingsSkillSources.filter((_, i) => i !== index))
  }

  const saveSettings = async () => {
    setSavingSettings(true)
    try {
      // Update registry URL
      await invoke("marketplace_set_registry_url", { url: settingsRegistryUrl })

      // Update skill sources - remove all then add
      if (config) {
        for (const source of config.skill_sources) {
          await invoke("marketplace_remove_skill_source", { repoUrl: source.repo_url })
        }
      }
      for (const source of settingsSkillSources) {
        await invoke("marketplace_add_skill_source", { source })
      }

      toast.success("Settings saved")
      setShowSettingsDialog(false)
      await loadConfig()
      // Refresh skills with new sources
      searchSkills()
    } catch (error) {
      console.error("Failed to save settings:", error)
      toast.error(`Failed to save settings: ${error}`)
    } finally {
      setSavingSettings(false)
    }
  }

  if (loading) {
    return (
      <div className="flex items-center justify-center h-full">
        <Loader2 className="h-8 w-8 animate-spin text-muted-foreground" />
      </div>
    )
  }

  if (!config?.enabled) {
    return (
      <>
        <div className="flex flex-col items-center justify-center h-full gap-4 p-8">
          <AlertCircle className="h-12 w-12 text-muted-foreground" />
          <h2 className="text-xl font-semibold">Marketplace Disabled</h2>
          <p className="text-muted-foreground text-center max-w-md">
            The marketplace allows you to browse and install MCP servers and skills from online registries.
            Enable it to get started.
          </p>
          <Button onClick={() => setShowEnableDialog(true)}>
            Enable Marketplace
          </Button>
        </div>

        <AlertDialog open={showEnableDialog} onOpenChange={setShowEnableDialog}>
          <AlertDialogContent>
            <AlertDialogHeader>
              <AlertDialogTitle>Enable Marketplace?</AlertDialogTitle>
              <AlertDialogDescription>
                This will allow LocalRouter to connect to online registries to browse and install MCP servers and skills.
                Your data stays local - only search queries are sent to the registry.
              </AlertDialogDescription>
            </AlertDialogHeader>
            <AlertDialogFooter>
              <AlertDialogCancel>Cancel</AlertDialogCancel>
              <AlertDialogAction onClick={handleEnableMarketplace}>
                Enable
              </AlertDialogAction>
            </AlertDialogFooter>
          </AlertDialogContent>
        </AlertDialog>
      </>
    )
  }

  return (
    <div className="flex flex-col h-full min-h-0">
      <div className="flex-shrink-0 pb-4">
        <div className="flex items-center justify-between">
          <div>
            <h1 className="text-2xl font-bold tracking-tight flex items-center gap-2">
              <StoreIcon className="h-6 w-6" />
              Marketplace
            </h1>
            <p className="text-sm text-muted-foreground">
              Browse and install MCP servers and skills
            </p>
          </div>
          <Button variant="outline" size="sm" onClick={openSettings}>
            <Settings className="h-4 w-4 mr-2" />
            Settings
          </Button>
        </div>
      </div>

      <Tabs defaultValue="mcp-servers" className="flex-1 flex flex-col min-h-0">
        <TabsList className="flex-shrink-0 w-fit">
          <TabsTrigger value="mcp-servers" className="gap-2">
            <McpIcon className="h-4 w-4" />
            MCP
          </TabsTrigger>
          <TabsTrigger value="skills" className="gap-2">
            <SkillsIcon className="h-4 w-4" />
            Skills
          </TabsTrigger>
        </TabsList>

        <TabsContent value="mcp-servers" className="flex-1 overflow-hidden mt-4">
          <div className="flex flex-col h-full rounded-lg border">
            {/* Search bar */}
            <div className="flex-shrink-0 p-4 border-b">
              <div className="flex gap-2">
                <div className="relative flex-1">
                  <Search className="absolute left-3 top-1/2 -translate-y-1/2 h-4 w-4 text-muted-foreground" />
                  <Input
                    placeholder="Search MCP servers..."
                    value={mcpSearch}
                    onChange={(e) => setMcpSearch(e.target.value)}
                    onKeyDown={(e) => e.key === "Enter" && searchMcpServers()}
                    className="pl-9"
                  />
                </div>
                <Button onClick={searchMcpServers} disabled={searchingMcp}>
                  {searchingMcp ? (
                    <Loader2 className="h-4 w-4 animate-spin" />
                  ) : (
                    "Search"
                  )}
                </Button>
              </div>
            </div>

            {/* Results */}
            <ScrollArea className="flex-1 p-4">
              {searchingMcp ? (
                <div className="flex items-center justify-center py-12">
                  <Loader2 className="h-8 w-8 animate-spin text-muted-foreground" />
                </div>
              ) : mcpServers.length === 0 ? (
                <div className="flex flex-col items-center justify-center py-12 text-muted-foreground">
                  <McpIcon className="h-8 w-8 mb-2" />
                  <p>No MCP servers found. Try a different search.</p>
                </div>
              ) : (
                <div className="space-y-4">
                  <div className="grid gap-4">
                    {mcpServers.map((server) => (
                      <Card key={server.name}>
                        <CardHeader className="pb-2">
                          <div className="flex items-start justify-between">
                            <div>
                              <CardTitle className="text-base">{server.name}</CardTitle>
                              {server.vendor && (
                                <p className="text-xs text-muted-foreground">by {server.vendor}</p>
                              )}
                            </div>
                            <div className="flex gap-1">
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
                        </CardHeader>
                        <CardContent>
                          <p className="text-sm text-muted-foreground mb-3">{server.description}</p>
                          <div className="flex items-center justify-between">
                            <div className="flex gap-2">
                              {server.homepage && (
                                <Button
                                  variant="ghost"
                                  size="sm"
                                  onClick={() => open(server.homepage!)}
                                >
                                  <ExternalLink className="h-4 w-4 mr-1" />
                                  Homepage
                                </Button>
                              )}
                            </div>
                            <Button size="sm" onClick={() => handleMcpInstallClick(server)}>
                              <Download className="h-4 w-4 mr-1" />
                              Install
                            </Button>
                          </div>
                        </CardContent>
                      </Card>
                    ))}
                  </div>
                  {mcpHasMore && (
                    <div className="flex justify-center pt-4">
                      <Button variant="outline" onClick={loadMoreMcp} disabled={loadingMoreMcp}>
                        {loadingMoreMcp ? (
                          <>
                            <Loader2 className="h-4 w-4 mr-2 animate-spin" />
                            Loading...
                          </>
                        ) : (
                          `Load More (showing ${mcpServers.length})`
                        )}
                      </Button>
                    </div>
                  )}
                </div>
              )}
            </ScrollArea>
          </div>
        </TabsContent>

        <TabsContent value="skills" className="flex-1 overflow-hidden mt-4">
          <div className="flex flex-col h-full rounded-lg border">
            {/* Search bar with source filter */}
            <div className="flex-shrink-0 p-4 border-b">
              <div className="flex gap-2">
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
                  <SelectTrigger className="w-[180px]">
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
                <Button onClick={searchSkills} disabled={searchingSkills}>
                  {searchingSkills ? (
                    <Loader2 className="h-4 w-4 animate-spin" />
                  ) : (
                    "Search"
                  )}
                </Button>
              </div>
            </div>

            {/* Results */}
            <ScrollArea className="flex-1 p-4">
              {searchingSkills ? (
                <div className="flex items-center justify-center py-12">
                  <Loader2 className="h-8 w-8 animate-spin text-muted-foreground" />
                </div>
              ) : skills.length === 0 ? (
                <div className="flex flex-col items-center justify-center py-12 text-muted-foreground">
                  <SkillsIcon className="h-8 w-8 mb-2" />
                  <p>No skills found. Try a different search or source.</p>
                </div>
              ) : (
                <div className="space-y-4">
                  <div className="grid gap-4">
                    {paginatedSkills.map((skill) => (
                      <Card key={`${skill.source_label}-${skill.name}`}>
                        <CardHeader className="pb-2">
                          <div className="flex items-start justify-between">
                            <div>
                              <CardTitle className="text-base">{skill.name}</CardTitle>
                              <div className="flex items-center gap-2 text-xs text-muted-foreground">
                                {skill.author && <span>by {skill.author}</span>}
                                {skill.version && <span>v{skill.version}</span>}
                                <Badge variant="outline" className="text-xs">
                                  {skill.source_label}
                                </Badge>
                              </div>
                            </div>
                          </div>
                        </CardHeader>
                        <CardContent>
                          <p className="text-sm text-muted-foreground mb-3">
                            {skill.description || "No description available"}
                          </p>
                          {skill.tags.length > 0 && (
                            <div className="flex flex-wrap gap-1 mb-3">
                              {skill.tags.map((tag) => (
                                <Badge key={tag} variant="secondary" className="text-xs">
                                  {tag}
                                </Badge>
                              ))}
                            </div>
                          )}
                          <div className="flex items-center justify-between">
                            <Button
                              variant="ghost"
                              size="sm"
                              onClick={() => open(skill.source_repo)}
                            >
                              <ExternalLink className="h-4 w-4 mr-1" />
                              Source
                            </Button>
                            <Button size="sm" onClick={() => handleSkillInstallClick(skill)}>
                              <Download className="h-4 w-4 mr-1" />
                              Install
                            </Button>
                          </div>
                        </CardContent>
                      </Card>
                    ))}
                  </div>
                  {hasMoreSkills && (
                    <div className="flex justify-center pt-4">
                      <Button variant="outline" onClick={loadMoreSkills}>
                        Load More (showing {paginatedSkills.length} of {skills.length})
                      </Button>
                    </div>
                  )}
                </div>
              )}
            </ScrollArea>
          </div>
        </TabsContent>
      </Tabs>

      {/* MCP Server Install Dialog */}
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

      {/* Skill Install Confirmation Dialog */}
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

      {/* Settings Dialog */}
      <Dialog open={showSettingsDialog} onOpenChange={setShowSettingsDialog}>
        <DialogContent className="max-w-lg max-h-[80vh] overflow-hidden flex flex-col">
          <DialogHeader>
            <DialogTitle>Marketplace Settings</DialogTitle>
            <DialogDescription>
              Configure the marketplace, MCP server registry, and skill sources
            </DialogDescription>
          </DialogHeader>

          <Tabs defaultValue="general" className="flex-1 flex flex-col overflow-hidden">
            <TabsList className="grid w-full grid-cols-3">
              <TabsTrigger value="general">General</TabsTrigger>
              <TabsTrigger value="mcp">MCP</TabsTrigger>
              <TabsTrigger value="skills">Skills</TabsTrigger>
            </TabsList>

            <TabsContent value="general" className="flex-1 overflow-y-auto space-y-6 mt-4">
              {/* Cache Status */}
              <div className="space-y-3">
                <div className="flex items-center justify-between">
                  <Label>Cache</Label>
                  <Button
                    variant="outline"
                    size="sm"
                    onClick={refreshCache}
                    disabled={refreshingCache}
                  >
                    {refreshingCache ? (
                      <Loader2 className="h-4 w-4 animate-spin" />
                    ) : (
                      <>
                        <RefreshCw className="h-4 w-4 mr-2" />
                        Refresh
                      </>
                    )}
                  </Button>
                </div>
                <p className="text-xs text-muted-foreground">
                  Data is cached for 7 days. Click refresh to fetch latest data.
                </p>
                {cacheStatus && (
                  <div className="text-xs space-y-1 text-muted-foreground">
                    <p>MCP Servers: {formatLastRefresh(cacheStatus.mcp_last_refresh)} ({cacheStatus.mcp_cached_queries} queries cached)</p>
                    <p>Skills: {formatLastRefresh(cacheStatus.skills_last_refresh)} ({cacheStatus.skills_cached_sources} sources cached)</p>
                  </div>
                )}
              </div>

              {/* Disable Marketplace */}
              <div className="space-y-3 pt-4 border-t">
                <Label>Disable Marketplace</Label>
                <p className="text-xs text-muted-foreground">
                  Disabling the marketplace will clear the cache but will not remove any MCP servers or skills you've already installed.
                </p>
                <Button
                  variant="destructive"
                  size="sm"
                  onClick={handleDisableMarketplace}
                  disabled={disablingMarketplace}
                >
                  {disablingMarketplace ? (
                    <>
                      <Loader2 className="h-4 w-4 mr-2 animate-spin" />
                      Disabling...
                    </>
                  ) : (
                    "Disable Marketplace"
                  )}
                </Button>
              </div>
            </TabsContent>

            <TabsContent value="mcp" className="flex-1 overflow-y-auto space-y-6 mt-4">
              {/* MCP Registry URL */}
              <div className="space-y-2">
                <Label>Registry URL</Label>
                <Input
                  value={settingsRegistryUrl}
                  onChange={(e) => setSettingsRegistryUrl(e.target.value)}
                  placeholder="https://registry.modelcontextprotocol.io/v0.1/servers"
                />
                <p className="text-xs text-muted-foreground">
                  The URL of the MCP server registry to search. Change this to use a custom or self-hosted registry.
                </p>
                <Button
                  variant="outline"
                  size="sm"
                  onClick={handleResetRegistryUrl}
                  disabled={resettingRegistryUrl}
                >
                  {resettingRegistryUrl ? (
                    <Loader2 className="h-4 w-4 animate-spin" />
                  ) : (
                    "Reset to Default"
                  )}
                </Button>
              </div>
            </TabsContent>

            <TabsContent value="skills" className="flex-1 overflow-y-auto space-y-4 mt-4">
              {/* Skill Sources */}
              <div className="space-y-3">
                <div>
                  <Label>Skill Sources</Label>
                  <p className="text-xs text-muted-foreground mt-1">
                    GitHub repositories containing skills with SKILL.md files
                  </p>
                </div>

                {/* Existing sources */}
                {settingsSkillSources.length > 0 && (
                  <div className="space-y-2">
                    {settingsSkillSources.map((source, index) => (
                      <div key={index} className="flex items-center gap-2 p-2 border rounded-md">
                        <div className="flex-1 min-w-0">
                          <p className="text-sm font-medium truncate">{source.label}</p>
                          <p className="text-xs text-muted-foreground truncate">{source.repo_url}</p>
                          <p className="text-xs text-muted-foreground">
                            Branch: {source.branch} | Path: {source.path || "/"}
                          </p>
                        </div>
                        <Button
                          variant="ghost"
                          size="sm"
                          onClick={() => removeSkillSource(index)}
                        >
                          <Trash2 className="h-4 w-4 text-destructive" />
                        </Button>
                      </div>
                    ))}
                  </div>
                )}

                {settingsSkillSources.length === 0 && (
                  <p className="text-sm text-muted-foreground py-4 text-center border rounded-md">
                    No skill sources configured
                  </p>
                )}

                {/* Add default sources button */}
                <Button
                  variant="outline"
                  size="sm"
                  onClick={handleAddDefaultSources}
                  disabled={addingDefaultSources}
                  className="w-full"
                >
                  {addingDefaultSources ? (
                    <>
                      <Loader2 className="h-4 w-4 mr-2 animate-spin" />
                      Adding...
                    </>
                  ) : (
                    <>
                      <Plus className="h-4 w-4 mr-2" />
                      Add Default Sources
                    </>
                  )}
                </Button>

                {/* Add new source form */}
                <div className="border rounded-md p-3 space-y-3">
                  <p className="text-sm font-medium">Add New Source</p>
                  <div className="grid grid-cols-2 gap-2">
                    <div className="col-span-2">
                      <Label className="text-xs">Repository URL</Label>
                      <Input
                        value={newSourceRepoUrl}
                        onChange={(e) => setNewSourceRepoUrl(e.target.value)}
                        placeholder="https://github.com/owner/repo"
                        className="text-sm"
                      />
                    </div>
                    <div>
                      <Label className="text-xs">Label</Label>
                      <Input
                        value={newSourceLabel}
                        onChange={(e) => setNewSourceLabel(e.target.value)}
                        placeholder="My Skills"
                        className="text-sm"
                      />
                    </div>
                    <div>
                      <Label className="text-xs">Branch</Label>
                      <Input
                        value={newSourceBranch}
                        onChange={(e) => setNewSourceBranch(e.target.value)}
                        placeholder="main"
                        className="text-sm"
                      />
                    </div>
                    <div className="col-span-2">
                      <Label className="text-xs">Path (optional)</Label>
                      <Input
                        value={newSourcePath}
                        onChange={(e) => setNewSourcePath(e.target.value)}
                        placeholder="skills"
                        className="text-sm"
                      />
                    </div>
                  </div>
                  <Button
                    variant="outline"
                    size="sm"
                    onClick={addSkillSource}
                    className="w-full"
                  >
                    <Plus className="h-4 w-4 mr-2" />
                    Add Source
                  </Button>
                </div>
              </div>
            </TabsContent>
          </Tabs>

          <DialogFooter className="mt-4">
            <Button variant="outline" onClick={() => setShowSettingsDialog(false)}>
              Cancel
            </Button>
            <Button onClick={saveSettings} disabled={savingSettings}>
              {savingSettings ? (
                <>
                  <Loader2 className="h-4 w-4 mr-2 animate-spin" />
                  Saving...
                </>
              ) : (
                "Save Settings"
              )}
            </Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>
    </div>
  )
}
