/**
 * Marketplace View
 *
 * Unified marketplace for browsing and installing both MCP servers and skills.
 * Combines search, settings, and configuration for both resource types.
 */

import { useState, useEffect, useRef, useCallback } from "react"
import { invoke } from "@tauri-apps/api/core"
import { open } from "@tauri-apps/plugin-shell"
import { toast } from "sonner"
import {
  Search,
  Loader2,
  Package,
  Globe,
  Plus,
  RefreshCw,
  Check,
  ChevronRight,
  ChevronDown,
  ExternalLink,
  Trash2,
  RotateCcw,
  User,
  Tag,
  GitBranch,
  File,
  FileText,
} from "lucide-react"
import { McpIcon, SkillsIcon, StoreIcon } from "@/components/icons/category-icons"
import { Button } from "@/components/ui/Button"
import { Card, CardContent, CardHeader, CardTitle, CardDescription } from "@/components/ui/Card"
import { Input } from "@/components/ui/Input"
import { ScrollArea } from "@/components/ui/scroll-area"
import { Badge } from "@/components/ui/Badge"
import { Switch } from "@/components/ui/switch"
import { Tabs, TabsContent, TabsList, TabsTrigger } from "@/components/ui/tabs"
import {
  Dialog,
  DialogContent,
  DialogHeader,
  DialogTitle,
  DialogDescription,
} from "@/components/ui/Modal"
import { DisabledOverlay } from "@/components/add-resource/DisabledOverlay"
import { McpToolDisplay } from "@/components/shared/McpToolDisplay"
import type { McpToolDisplayItem } from "@/components/shared/McpToolDisplay"
import { isValidHttpUrl } from "@/utils/url"
import { cn } from "@/lib/utils"
import { TAB_ICONS, TAB_ICON_CLASS } from "@/constants/tab-icons"

import type { McpServerListing, SkillListing } from "@/components/add-resource"
import type { ToolDefinition } from "@/types/tauri-commands"

interface MarketplaceConfig {
  mcp_enabled: boolean
  skills_enabled: boolean
  registry_url: string
  skill_sources: { repo_url: string; branch: string; path: string; label: string }[]
}

interface CacheStatus {
  mcp_last_refresh: string | null
  skills_last_refresh: string | null
  mcp_cached_queries: number
  skills_cached_sources: number
}

interface SkillInfo {
  name: string
  enabled: boolean
}

type FilterType = "all" | "mcp" | "skill"

interface MarketplaceViewProps {
  activeSubTab: string | null
  onTabChange: (view: string, subTab?: string | null) => void
}

export function MarketplaceView({ activeSubTab, onTabChange }: MarketplaceViewProps) {
  const topTab = activeSubTab === "settings" ? "settings" : activeSubTab === "via-mcp" ? "via-mcp" : "browse"

  // Config
  const [config, setConfig] = useState<MarketplaceConfig | null>(null)
  const [configLoading, setConfigLoading] = useState(true)
  const [cacheStatus, setCacheStatus] = useState<CacheStatus | null>(null)

  // Search state
  const [searchQuery, setSearchQuery] = useState("")
  const [filter, setFilter] = useState<FilterType>("all")
  const hasInitialSearch = useRef(false)

  // MCP results
  const [mcpResults, setMcpResults] = useState<McpServerListing[]>([])
  const [searchingMcp, setSearchingMcp] = useState(false)

  // Skill results
  const [skillResults, setSkillResults] = useState<SkillListing[]>([])
  const [searchingSkills, setSearchingSkills] = useState(false)
  const [installedSkillNames, setInstalledSkillNames] = useState<string[]>([])
  const [installedMcpNames, setInstalledMcpNames] = useState<string[]>([])
  const [selectedSkillSource] = useState<string>("all")

  // Settings state
  const [registryUrl, setRegistryUrl] = useState("")
  const [savingRegistry, setSavingRegistry] = useState(false)
  const [refreshingCache, setRefreshingCache] = useState(false)
  const [clearingMcpCache, setClearingMcpCache] = useState(false)
  const [clearingSkillsCache, setClearingSkillsCache] = useState(false)
  const [settingsSources, setSettingsSources] = useState<{ repo_url: string; branch: string; path: string; label: string }[]>([])
  const [newSourceRepoUrl, setNewSourceRepoUrl] = useState("")
  const [newSourceBranch, setNewSourceBranch] = useState("main")
  const [newSourcePath, setNewSourcePath] = useState("")
  const [newSourceLabel, setNewSourceLabel] = useState("")
  const [addingSource, setAddingSource] = useState(false)
  const [addingDefaultSources, setAddingDefaultSources] = useState(false)

  // Via MCP tools state
  const [marketplaceTools, setMarketplaceTools] = useState<McpToolDisplayItem[]>([])

  // Skill install dialog state
  const [showInstallDialog, setShowInstallDialog] = useState(false)
  const [selectedSkillListing, setSelectedSkillListing] = useState<SkillListing | null>(null)
  const [isInstalling, setIsInstalling] = useState(false)
  const [expandedPreviewFiles, setExpandedPreviewFiles] = useState<Set<string>>(new Set())
  const [previewFileContents, setPreviewFileContents] = useState<Record<string, string>>({})

  // Load config and installed skills
  const loadConfig = useCallback(async () => {
    try {
      const [cfg, cache] = await Promise.all([
        invoke<MarketplaceConfig>("marketplace_get_config"),
        invoke<CacheStatus>("marketplace_get_cache_status").catch(() => null),
      ])
      setConfig(cfg)
      setRegistryUrl(cfg.registry_url)
      setSettingsSources(cfg.skill_sources ?? [])
      setCacheStatus(cache)
    } catch (error) {
      console.error("Failed to load marketplace config:", error)
    } finally {
      setConfigLoading(false)
    }
  }, [])

  const loadInstalledSkills = useCallback(async () => {
    try {
      const skills = await invoke<SkillInfo[]>("list_skills")
      setInstalledSkillNames(skills.map(s => s.name))
    } catch {
      // Skills might not be available
    }
  }, [])

  const loadInstalledMcpServers = useCallback(async () => {
    try {
      const servers = await invoke<{ id: string; name: string }[]>("list_mcp_servers")
      setInstalledMcpNames(servers.map(s => s.name))
    } catch {
      // MCP servers might not be available
    }
  }, [])

  useEffect(() => {
    loadConfig()
    loadInstalledSkills()
    loadInstalledMcpServers()
  }, [loadConfig, loadInstalledSkills, loadInstalledMcpServers])

  // Load marketplace tool definitions for Via MCP tab
  useEffect(() => {
    invoke<ToolDefinition[]>("get_marketplace_tool_definitions")
      .then((defs) =>
        setMarketplaceTools(
          defs.map((d): McpToolDisplayItem => ({
            name: d.name,
            description: d.description,
            inputSchema: d.input_schema,
          }))
        )
      )
      .catch(() => setMarketplaceTools([]))
  }, [])

  // Search functions
  const searchMcp = useCallback(async (query: string) => {
    if (!config?.mcp_enabled) return
    setSearchingMcp(true)
    try {
      const results = await invoke<McpServerListing[]>("marketplace_search_mcp_servers", {
        query: query.trim() || "mcp",
        limit: 20,
      })
      setMcpResults(results)
    } catch (error) {
      console.error("Failed to search MCP servers:", error)
    } finally {
      setSearchingMcp(false)
    }
  }, [config?.mcp_enabled])

  const searchSkills = useCallback(async (query: string) => {
    if (!config?.skills_enabled) return
    setSearchingSkills(true)
    try {
      const results = await invoke<SkillListing[]>("marketplace_search_skills", {
        query: query.trim() || null,
        source: selectedSkillSource === "all" ? null : selectedSkillSource,
      })
      setSkillResults(results)
    } catch (error) {
      console.error("Failed to search skills:", error)
    } finally {
      setSearchingSkills(false)
    }
  }, [config?.skills_enabled, selectedSkillSource])

  const handleSearch = useCallback(() => {
    if (filter === "all" || filter === "mcp") searchMcp(searchQuery)
    if (filter === "all" || filter === "skill") searchSkills(searchQuery)
  }, [filter, searchQuery, searchMcp, searchSkills])

  // Initial search when config loads
  useEffect(() => {
    if (config && !hasInitialSearch.current && topTab === "browse") {
      hasInitialSearch.current = true
      if (config.mcp_enabled) searchMcp("")
      if (config.skills_enabled) searchSkills("")
    }
  }, [config, topTab, searchMcp, searchSkills])

  // Re-search when filter changes
  useEffect(() => {
    if (!hasInitialSearch.current) return
    if (filter === "mcp" && mcpResults.length === 0 && !searchingMcp && config?.mcp_enabled) {
      searchMcp(searchQuery)
    }
    if (filter === "skill" && skillResults.length === 0 && !searchingSkills && config?.skills_enabled) {
      searchSkills(searchQuery)
    }
  }, [filter]) // eslint-disable-line react-hooks/exhaustive-deps

  // MCP click → navigate to mcp-servers view with add template
  const handleMcpClick = (server: McpServerListing) => {
    onTabChange("mcp-servers", `add/${server.name}`)
  }

  // Skill click → open install dialog
  const handleSkillClick = (skill: SkillListing) => {
    setSelectedSkillListing(skill)
    setExpandedPreviewFiles(new Set())
    setPreviewFileContents({})
    setShowInstallDialog(true)
  }

  // Skill install
  const handleInstallSkill = async () => {
    if (!selectedSkillListing) return
    setIsInstalling(true)
    try {
      await invoke("marketplace_install_skill_direct", {
        sourceUrl: selectedSkillListing.skill_md_url,
        skillName: selectedSkillListing.name,
      })
      toast.success(`Skill "${selectedSkillListing.name}" installed`)
      // Trigger rescan so skills view picks it up
      await invoke("rescan_skills").catch(() => {})
      await loadInstalledSkills()
      setShowInstallDialog(false)
      resetInstallDialog()
    } catch (error) {
      console.error("Failed to install skill:", error)
      toast.error(`Failed to install skill: ${error}`)
    } finally {
      setIsInstalling(false)
    }
  }

  const resetInstallDialog = () => {
    setSelectedSkillListing(null)
    setIsInstalling(false)
    setExpandedPreviewFiles(new Set())
    setPreviewFileContents({})
  }

  const togglePreviewFileExpanded = async (file: { path: string; url: string }) => {
    const isExpanded = expandedPreviewFiles.has(file.path)
    if (isExpanded) {
      setExpandedPreviewFiles(prev => {
        const next = new Set(prev)
        next.delete(file.path)
        return next
      })
    } else {
      setExpandedPreviewFiles(prev => new Set(prev).add(file.path))
      if (!previewFileContents[file.path]) {
        try {
          const response = await fetch(file.url)
          if (response.ok) {
            const content = await response.text()
            const lines = content.split('\n')
            const preview = lines.slice(0, 200).join('\n') + (lines.length > 200 ? '\n...(truncated)' : '')
            setPreviewFileContents(prev => ({ ...prev, [file.path]: preview }))
          } else {
            setPreviewFileContents(prev => ({ ...prev, [file.path]: '(Failed to load content)' }))
          }
        } catch {
          setPreviewFileContents(prev => ({ ...prev, [file.path]: '(Failed to load content)' }))
        }
      }
    }
  }

  // Settings handlers
  const handleToggleMcpEnabled = async (enabled: boolean) => {
    try {
      await invoke("marketplace_set_mcp_enabled", { enabled })
      setConfig(prev => prev ? { ...prev, mcp_enabled: enabled } : prev)
      toast.success(enabled ? "MCP marketplace enabled" : "MCP marketplace disabled")
    } catch (error) {
      toast.error(`Failed to update setting: ${error}`)
    }
  }

  const handleToggleSkillsEnabled = async (enabled: boolean) => {
    try {
      await invoke("marketplace_set_skills_enabled", { enabled })
      setConfig(prev => prev ? { ...prev, skills_enabled: enabled } : prev)
      toast.success(enabled ? "Skills marketplace enabled" : "Skills marketplace disabled")
    } catch (error) {
      toast.error(`Failed to update setting: ${error}`)
    }
  }

  const handleSaveRegistryUrl = async () => {
    if (!registryUrl.trim()) return
    setSavingRegistry(true)
    try {
      await invoke("marketplace_set_registry_url", { url: registryUrl.trim() })
      toast.success("Registry URL updated")
    } catch (error) {
      toast.error(`Failed to save: ${error}`)
    } finally {
      setSavingRegistry(false)
    }
  }

  const handleResetRegistryUrl = async () => {
    try {
      const url = await invoke<string>("marketplace_reset_registry_url")
      setRegistryUrl(url)
      toast.success("Registry URL reset to default")
    } catch (error) {
      toast.error(`Failed to reset: ${error}`)
    }
  }

  const handleAddSkillSource = async () => {
    if (!newSourceRepoUrl.trim() || !newSourceLabel.trim()) return
    setAddingSource(true)
    try {
      await invoke("marketplace_add_skill_source", {
        source: {
          repo_url: newSourceRepoUrl.trim(),
          branch: newSourceBranch.trim() || "main",
          path: newSourcePath.trim() || "skills",
          label: newSourceLabel.trim(),
        },
      })
      toast.success("Skill source added")
      setNewSourceRepoUrl("")
      setNewSourceBranch("main")
      setNewSourcePath("")
      setNewSourceLabel("")
      await loadConfig()
    } catch (error) {
      toast.error(`Failed to add source: ${error}`)
    } finally {
      setAddingSource(false)
    }
  }

  const handleRemoveSkillSource = async (repoUrl: string) => {
    try {
      await invoke("marketplace_remove_skill_source", { repoUrl })
      toast.success("Skill source removed")
      await loadConfig()
    } catch (error) {
      toast.error(`Failed to remove source: ${error}`)
    }
  }

  const handleAddDefaultSources = async () => {
    setAddingDefaultSources(true)
    try {
      const count = await invoke<number>("marketplace_add_default_skill_sources")
      if (count > 0) {
        toast.success(`Added ${count} default source(s)`)
        await loadConfig()
      } else {
        toast.info("All default sources already present")
      }
    } catch (error) {
      toast.error(`Failed to add defaults: ${error}`)
    } finally {
      setAddingDefaultSources(false)
    }
  }

  const handleRefreshCache = async () => {
    setRefreshingCache(true)
    try {
      await invoke("marketplace_refresh_cache")
      const cache = await invoke<CacheStatus>("marketplace_get_cache_status").catch(() => null)
      setCacheStatus(cache)
      toast.success("Cache refreshed")
    } catch (error) {
      toast.error(`Failed to refresh: ${error}`)
    } finally {
      setRefreshingCache(false)
    }
  }

  const handleClearMcpCache = async () => {
    setClearingMcpCache(true)
    try {
      await invoke("marketplace_clear_mcp_cache")
      const cache = await invoke<CacheStatus>("marketplace_get_cache_status").catch(() => null)
      setCacheStatus(cache)
      toast.success("MCP cache cleared")
    } catch (error) {
      toast.error(`Failed to clear cache: ${error}`)
    } finally {
      setClearingMcpCache(false)
    }
  }

  const handleClearSkillsCache = async () => {
    setClearingSkillsCache(true)
    try {
      await invoke("marketplace_clear_skills_cache")
      const cache = await invoke<CacheStatus>("marketplace_get_cache_status").catch(() => null)
      setCacheStatus(cache)
      toast.success("Skills cache cleared")
    } catch (error) {
      toast.error(`Failed to clear cache: ${error}`)
    } finally {
      setClearingSkillsCache(false)
    }
  }

  const formatLastRefresh = (date: string | null) => {
    if (!date) return "Never"
    return new Date(date).toLocaleString()
  }

  const handleTopTabChange = (tab: string) => {
    onTabChange("marketplace", tab === "browse" ? null : tab)
  }

  const isSearching = searchingMcp || searchingSkills
  const showMcp = filter === "all" || filter === "mcp"
  const showSkills = filter === "all" || filter === "skill"
  const bothDisabled = !config?.mcp_enabled && !config?.skills_enabled
  const mcpDisabled = !config?.mcp_enabled
  const skillsDisabled = !config?.skills_enabled

  if (configLoading) {
    return (
      <div className="flex items-center justify-center py-12">
        <Loader2 className="h-8 w-8 animate-spin text-muted-foreground" />
      </div>
    )
  }

  return (
    <div className="flex flex-col h-full min-h-0 max-w-5xl">
      <div className="flex-shrink-0 pb-4">
        <h1 className="text-2xl font-bold tracking-tight flex items-center gap-2">
          <StoreIcon className="h-6 w-6" />
          Marketplace
        </h1>
        <p className="text-sm text-muted-foreground">
          Browse and install MCP servers and skills from online registries and sources.
        </p>
      </div>

      <Tabs
        value={topTab}
        onValueChange={handleTopTabChange}
        className="flex flex-col flex-1 min-h-0"
      >
        <TabsList className="flex-shrink-0 w-fit">
          <TabsTrigger value="browse"><TAB_ICONS.browse className={TAB_ICON_CLASS} />Browse</TabsTrigger>
          <TabsTrigger value="via-mcp"><TAB_ICONS.viaMcp className={TAB_ICON_CLASS} />Via MCP</TabsTrigger>
          <TabsTrigger value="settings"><TAB_ICONS.settings className={TAB_ICON_CLASS} />Settings</TabsTrigger>
        </TabsList>

        {/* Browse Tab */}
        <TabsContent value="browse" className="flex-1 min-h-0 mt-4">
          {bothDisabled ? (
            <DisabledOverlay
              title="Marketplace Disabled"
              description="Enable MCP and/or Skills marketplace in Settings to browse and install resources."
              actionLabel="Go to Settings"
              onAction={() => handleTopTabChange("settings")}
            >
              <div className="h-64" />
            </DisabledOverlay>
          ) : (
            <div className="flex flex-col h-full min-h-0">
              {/* Filter toggle + search bar */}
              <div className="flex items-center gap-2 mb-4 flex-shrink-0">
                <div className="flex rounded-md border h-9 shrink-0">
                  <button
                    className={cn(
                      "px-3 text-xs font-medium transition-colors rounded-l-md",
                      filter === "all"
                        ? "bg-accent text-accent-foreground"
                        : "text-muted-foreground hover:bg-accent/50"
                    )}
                    onClick={() => setFilter("all")}
                  >
                    All
                  </button>
                  <button
                    className={cn(
                      "px-3 text-xs font-medium transition-colors border-l",
                      filter === "mcp"
                        ? "bg-accent text-accent-foreground"
                        : "text-muted-foreground hover:bg-accent/50",
                      mcpDisabled && "opacity-50"
                    )}
                    onClick={() => !mcpDisabled && setFilter("mcp")}
                    disabled={mcpDisabled}
                    title={mcpDisabled ? "MCP marketplace disabled" : "Show MCP servers only"}
                  >
                    MCP
                  </button>
                  <button
                    className={cn(
                      "px-3 text-xs font-medium transition-colors border-l rounded-r-md",
                      filter === "skill"
                        ? "bg-accent text-accent-foreground"
                        : "text-muted-foreground hover:bg-accent/50",
                      skillsDisabled && "opacity-50"
                    )}
                    onClick={() => !skillsDisabled && setFilter("skill")}
                    disabled={skillsDisabled}
                    title={skillsDisabled ? "Skills marketplace disabled" : "Show skills only"}
                  >
                    Skills
                  </button>
                </div>
                <div className="relative flex-1">
                  <Search className="absolute left-3 top-1/2 -translate-y-1/2 h-4 w-4 text-muted-foreground" />
                  <Input
                    placeholder={filter === "mcp" ? "Search MCP servers..." : filter === "skill" ? "Search skills..." : "Search MCP servers and skills..."}
                    value={searchQuery}
                    onChange={(e) => setSearchQuery(e.target.value)}
                    onKeyDown={(e) => e.key === "Enter" && handleSearch()}
                    className="pl-9 h-9"
                  />
                </div>
                <Button onClick={handleSearch} disabled={isSearching} className="h-9">
                  {isSearching ? <Loader2 className="h-4 w-4 animate-spin" /> : "Search"}
                </Button>
              </div>

              {/* Combined Results */}
              <div className="flex-1 min-h-0 overflow-hidden">
                <ScrollArea className="h-full">
                  {isSearching ? (
                    <div className="flex items-center justify-center py-12">
                      <Loader2 className="h-6 w-6 animate-spin text-muted-foreground" />
                    </div>
                  ) : (() => {
                    // Build a single combined list of results
                    type ResultItem =
                      | { type: "mcp"; data: McpServerListing }
                      | { type: "skill"; data: SkillListing }
                    const items: ResultItem[] = []
                    if (showMcp && config?.mcp_enabled) {
                      for (const s of mcpResults) items.push({ type: "mcp", data: s })
                    }
                    if (showSkills && config?.skills_enabled) {
                      for (const s of skillResults.slice(0, 20)) items.push({ type: "skill", data: s })
                    }

                    if (items.length === 0) {
                      const hasDisabled = (showMcp && mcpDisabled) || (showSkills && skillsDisabled)
                      return (
                        <div className="flex flex-col items-center justify-center py-12 text-muted-foreground">
                          <StoreIcon className="h-8 w-8 mb-2" />
                          <p className="text-sm">No results found</p>
                          {hasDisabled && (
                            <p className="text-xs mt-2">
                              Some marketplaces are disabled. <button className="underline" onClick={() => handleTopTabChange("settings")}>Enable in Settings</button>.
                            </p>
                          )}
                        </div>
                      )
                    }

                    return (
                      <div className="space-y-3 pr-4 pb-4">
                        {items.map((item) => {
                          if (item.type === "mcp") {
                            const server = item.data
                            const pkg = server.packages[0]
                            return (
                              <Card key={`mcp-${server.name}`} className="p-3">
                                <div className="flex items-start justify-between gap-2">
                                  <div className="flex-1 min-w-0">
                                    <div className="flex items-center gap-2">
                                      <Badge variant="outline" className="text-[10px] shrink-0 px-1.5 py-0">
                                        <McpIcon className="h-3 w-3 mr-0.5" />
                                        MCP
                                      </Badge>
                                      <p className="font-medium text-sm truncate">{server.name}</p>
                                      {pkg?.version && (
                                        <span className="text-xs text-muted-foreground">v{pkg.version}</span>
                                      )}
                                      {installedMcpNames.includes(server.name) && (
                                        <Badge variant="secondary" className="text-xs shrink-0">
                                          <Check className="h-3 w-3 mr-0.5" />
                                          Installed
                                        </Badge>
                                      )}
                                    </div>
                                    <div className="flex items-center gap-2 text-xs text-muted-foreground mt-0.5">
                                      {server.vendor && <span>by {server.vendor}</span>}
                                    </div>
                                    <p className="text-xs text-muted-foreground line-clamp-2 mt-1">
                                      {server.description}
                                    </p>
                                    <div className="flex flex-wrap items-center gap-1 mt-2">
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
                                      {pkg?.license && (
                                        <Badge variant="outline" className="text-xs">{pkg.license}</Badge>
                                      )}
                                      {pkg?.runtime && (
                                        <Badge variant="outline" className="text-xs">{pkg.runtime}</Badge>
                                      )}
                                    </div>
                                    {server.homepage && isValidHttpUrl(server.homepage) && (
                                      <button
                                        className="inline-flex items-center gap-1 text-xs text-primary hover:underline mt-2"
                                        onClick={(e) => { e.stopPropagation(); open(server.homepage!) }}
                                      >
                                        <ExternalLink className="h-3 w-3" />
                                        Source
                                      </button>
                                    )}
                                  </div>
                                  {installedMcpNames.includes(server.name) ? (
                                    <Button size="sm" variant="secondary" onClick={() => onTabChange("mcp-servers", server.name)}>
                                      View
                                      <ChevronRight className="h-4 w-4 ml-1" />
                                    </Button>
                                  ) : (
                                    <Button size="sm" onClick={() => handleMcpClick(server)}>
                                      <Plus className="h-4 w-4 mr-1" />
                                      Add
                                    </Button>
                                  )}
                                </div>
                              </Card>
                            )
                          }

                          const skill = item.data
                          const isInstalled = installedSkillNames.includes(skill.name)
                          return (
                            <Card key={`skill-${skill.source_label}-${skill.name}`} className="p-3">
                              <div className="flex items-start justify-between gap-2">
                                <div className="flex-1 min-w-0">
                                  <div className="flex items-center gap-2">
                                    <Badge variant="outline" className="text-[10px] shrink-0 px-1.5 py-0">
                                      <SkillsIcon className="h-3 w-3 mr-0.5" />
                                      Skill
                                    </Badge>
                                    <p className="font-medium text-sm truncate">{skill.name}</p>
                                    {isInstalled && (
                                      <Badge variant="secondary" className="text-xs shrink-0">
                                        <Check className="h-3 w-3 mr-0.5" />
                                        Installed
                                      </Badge>
                                    )}
                                  </div>
                                  <div className="flex items-center gap-1 text-xs text-muted-foreground mt-0.5">
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
                                        <Badge key={tag} variant="secondary" className="text-xs">{tag}</Badge>
                                      ))}
                                    </div>
                                  )}
                                  {skill.source_repo && isValidHttpUrl(skill.source_repo) && (
                                    <button
                                      className="inline-flex items-center gap-1 text-xs text-primary hover:underline mt-2"
                                      onClick={(e) => { e.stopPropagation(); open(skill.source_repo) }}
                                    >
                                      <ExternalLink className="h-3 w-3" />
                                      Source
                                    </button>
                                  )}
                                </div>
                                {isInstalled ? (
                                  <Button size="sm" variant="secondary" onClick={() => handleSkillClick(skill)}>
                                    <RefreshCw className="h-4 w-4 mr-1" />
                                    Replace
                                  </Button>
                                ) : (
                                  <Button size="sm" onClick={() => handleSkillClick(skill)}>
                                    <Plus className="h-4 w-4 mr-1" />
                                    Add
                                  </Button>
                                )}
                              </div>
                            </Card>
                          )
                        })}
                      </div>
                    )
                  })()}
                </ScrollArea>
              </div>
            </div>
          )}
        </TabsContent>

        {/* Via MCP Tab */}
        <TabsContent value="via-mcp" className="flex-1 min-h-0 mt-4 overflow-y-auto">
          <div className="space-y-6 max-w-2xl">
            <Card>
              <CardHeader>
                <CardTitle>Marketplace via MCP</CardTitle>
                <CardDescription>
                  The marketplace can be accessed programmatically through MCP tools in the Unified MCP Gateway.
                </CardDescription>
              </CardHeader>
              <CardContent className="space-y-4 text-sm text-muted-foreground">
                <p>
                  MCP clients can search for and install MCP servers and skills directly through tool calls &mdash;
                  no UI interaction required. This enables AI agents to discover and install capabilities on demand.
                </p>
                <p>
                  Search is always permitted. Installation requires user approval via a popup
                  when the permission is set to <strong className="text-foreground">Ask</strong>.
                </p>
              </CardContent>
            </Card>

            <Card>
              <CardHeader>
                <CardTitle>Enabling for a Client</CardTitle>
                <CardDescription>
                  Marketplace access via MCP is configured per-client.
                </CardDescription>
              </CardHeader>
              <CardContent className="space-y-3 text-sm text-muted-foreground">
                <p>
                  To enable marketplace tools for a client, go to the client's <strong className="text-foreground">Marketplace</strong> tab and set the permission:
                </p>
                <ul className="list-disc list-inside space-y-1.5 ml-1">
                  <li><strong className="text-foreground">Ask</strong> &mdash; Search is always permitted, installation requires user approval via popup</li>
                  <li><strong className="text-foreground">Off</strong> &mdash; Client has no access to marketplace tools</li>
                </ul>
              </CardContent>
            </Card>

            {marketplaceTools.length > 0 && (
              <Card>
                <CardHeader>
                  <CardTitle>MCP Tools</CardTitle>
                  <CardDescription>
                    When enabled, the client gets access to {marketplaceTools.length} marketplace tools.
                  </CardDescription>
                </CardHeader>
                <CardContent>
                  <McpToolDisplay tools={marketplaceTools} />
                </CardContent>
              </Card>
            )}
          </div>
        </TabsContent>

        {/* Settings Tab */}
        <TabsContent value="settings" className="flex-1 min-h-0 mt-4 overflow-y-auto">
          <div className="space-y-6 max-w-2xl">
            {/* MCP Marketplace */}
            <Card>
              <CardHeader>
                <CardTitle className="flex items-center gap-2">
                  <McpIcon className="h-4 w-4" />
                  MCP Marketplace
                </CardTitle>
                <CardDescription>
                  Browse and install MCP servers from the official registry.
                </CardDescription>
              </CardHeader>
              <CardContent>
                <div className="flex items-center gap-3">
                  <Switch
                    checked={config?.mcp_enabled ?? false}
                    onCheckedChange={handleToggleMcpEnabled}
                  />
                  <span className="text-sm">
                    {config?.mcp_enabled ? "Enabled" : "Disabled"}
                  </span>
                </div>
              </CardContent>
            </Card>

            {/* Registry URL */}
            <Card>
              <CardHeader>
                <CardTitle>Registry URL</CardTitle>
                <CardDescription>
                  The MCP server registry to search for available servers.
                </CardDescription>
              </CardHeader>
              <CardContent className="space-y-3">
                <div className="flex gap-2">
                  <Input
                    value={registryUrl}
                    onChange={(e) => setRegistryUrl(e.target.value)}
                    onKeyDown={(e) => e.key === "Enter" && handleSaveRegistryUrl()}
                    placeholder="https://registry.modelcontextprotocol.io/v0.1/servers"
                    className="flex-1"
                  />
                  <Button onClick={handleSaveRegistryUrl} disabled={savingRegistry} size="sm">
                    {savingRegistry ? <Loader2 className="h-4 w-4 animate-spin" /> : "Save"}
                  </Button>
                  <Button onClick={handleResetRegistryUrl} variant="outline" size="sm" title="Reset to default">
                    <RotateCcw className="h-4 w-4" />
                  </Button>
                </div>
              </CardContent>
            </Card>

            {/* Skills Marketplace */}
            <Card>
              <CardHeader>
                <CardTitle className="flex items-center gap-2">
                  <SkillsIcon className="h-4 w-4" />
                  Skills Marketplace
                </CardTitle>
                <CardDescription>
                  Browse and install skills from configured skill sources.
                </CardDescription>
              </CardHeader>
              <CardContent>
                <div className="flex items-center gap-3">
                  <Switch
                    checked={config?.skills_enabled ?? false}
                    onCheckedChange={handleToggleSkillsEnabled}
                  />
                  <span className="text-sm">
                    {config?.skills_enabled ? "Enabled" : "Disabled"}
                  </span>
                </div>
              </CardContent>
            </Card>

            {/* Skill Sources */}
            <Card>
              <CardHeader>
                <div className="flex items-center justify-between">
                  <div>
                    <CardTitle>Skill Sources</CardTitle>
                    <CardDescription>
                      GitHub repositories to browse for skills.
                    </CardDescription>
                  </div>
                  <Button
                    variant="outline"
                    size="sm"
                    onClick={handleAddDefaultSources}
                    disabled={addingDefaultSources}
                  >
                    {addingDefaultSources ? <Loader2 className="h-4 w-4 mr-2 animate-spin" /> : <RotateCcw className="h-4 w-4 mr-2" />}
                    Add Defaults
                  </Button>
                </div>
              </CardHeader>
              <CardContent className="space-y-4">
                {settingsSources.length === 0 ? (
                  <p className="text-sm text-muted-foreground">No skill sources configured</p>
                ) : (
                  <div className="space-y-2">
                    {settingsSources.map((source) => (
                      <div key={source.repo_url} className="flex items-center gap-2 p-2 rounded-md bg-muted/50 group">
                        <div className="flex-1 min-w-0">
                          <p className="text-sm font-medium truncate">{source.label}</p>
                          <p className="text-xs text-muted-foreground truncate">{source.repo_url}</p>
                        </div>
                        <Button
                          variant="ghost"
                          size="icon"
                          className="h-6 w-6 opacity-0 group-hover:opacity-100 transition-opacity shrink-0"
                          onClick={() => handleRemoveSkillSource(source.repo_url)}
                        >
                          <Trash2 className="h-3.5 w-3.5 text-destructive" />
                        </Button>
                      </div>
                    ))}
                  </div>
                )}

                <div className="border-t pt-4 space-y-3">
                  <p className="text-sm font-medium">Add New Source</p>
                  <div className="grid grid-cols-2 gap-2">
                    <div className="col-span-2">
                      <Input
                        placeholder="Repository URL (e.g., https://github.com/org/repo)"
                        value={newSourceRepoUrl}
                        onChange={(e) => setNewSourceRepoUrl(e.target.value)}
                      />
                    </div>
                    <Input
                      placeholder="Label"
                      value={newSourceLabel}
                      onChange={(e) => setNewSourceLabel(e.target.value)}
                    />
                    <Input
                      placeholder="Branch (default: main)"
                      value={newSourceBranch}
                      onChange={(e) => setNewSourceBranch(e.target.value)}
                    />
                    <Input
                      placeholder="Path (default: skills)"
                      value={newSourcePath}
                      onChange={(e) => setNewSourcePath(e.target.value)}
                    />
                    <Button
                      onClick={handleAddSkillSource}
                      disabled={!newSourceRepoUrl.trim() || !newSourceLabel.trim() || addingSource}
                      size="sm"
                    >
                      {addingSource ? <Loader2 className="h-4 w-4 mr-2 animate-spin" /> : <Plus className="h-4 w-4 mr-2" />}
                      Add Source
                    </Button>
                  </div>
                </div>
              </CardContent>
            </Card>

            {/* Cache */}
            <Card>
              <CardHeader>
                <CardTitle>Cache</CardTitle>
                <CardDescription>
                  Marketplace search results are cached locally for faster access.
                </CardDescription>
              </CardHeader>
              <CardContent className="space-y-4">
                {cacheStatus && (
                  <div className="text-sm text-muted-foreground space-y-1">
                    <p>MCP last refresh: {formatLastRefresh(cacheStatus.mcp_last_refresh)}</p>
                    <p>MCP cached queries: {cacheStatus.mcp_cached_queries}</p>
                    <p>Skills last refresh: {formatLastRefresh(cacheStatus.skills_last_refresh)}</p>
                    <p>Skills cached sources: {cacheStatus.skills_cached_sources}</p>
                  </div>
                )}
                <div className="flex flex-wrap gap-2">
                  <Button onClick={handleRefreshCache} disabled={refreshingCache} variant="outline" size="sm">
                    <RefreshCw className={`h-4 w-4 mr-2 ${refreshingCache ? "animate-spin" : ""}`} />
                    Refresh All
                  </Button>
                  <Button onClick={handleClearMcpCache} disabled={clearingMcpCache} variant="outline" size="sm">
                    {clearingMcpCache ? <Loader2 className="h-4 w-4 mr-2 animate-spin" /> : null}
                    Clear MCP Cache
                  </Button>
                  <Button onClick={handleClearSkillsCache} disabled={clearingSkillsCache} variant="outline" size="sm">
                    {clearingSkillsCache ? <Loader2 className="h-4 w-4 mr-2 animate-spin" /> : null}
                    Clear Skills Cache
                  </Button>
                </div>
              </CardContent>
            </Card>
          </div>
        </TabsContent>
      </Tabs>

      {/* Skill Install Dialog */}
      <Dialog open={showInstallDialog} onOpenChange={(open) => {
        setShowInstallDialog(open)
        if (!open) resetInstallDialog()
      }}>
        <DialogContent className="max-w-lg max-h-[80vh] flex flex-col overflow-hidden">
          <DialogHeader className="flex-shrink-0">
            <DialogTitle>Install Skill</DialogTitle>
            <DialogDescription>
              Preview and install the selected skill.
            </DialogDescription>
          </DialogHeader>

          {selectedSkillListing && (
            <div className="flex-1 flex flex-col min-h-0 overflow-y-auto space-y-4">
              {/* Skill header */}
              <div className="flex items-center gap-3 pb-2 border-b">
                <SkillsIcon className="h-6 w-6 shrink-0" />
                <div className="min-w-0">
                  <p className="text-sm font-medium truncate">{selectedSkillListing.name}</p>
                  <p className="text-xs text-muted-foreground truncate">{selectedSkillListing.description || "No description"}</p>
                </div>
              </div>

              <div className="space-y-4">
                {/* Skill Details */}
                <Card>
                  <CardHeader className="pb-3">
                    <CardTitle className="text-sm">Details</CardTitle>
                  </CardHeader>
                  <CardContent className="space-y-3">
                    <div className="grid grid-cols-2 gap-2 text-sm">
                      {selectedSkillListing.author && (
                        <div className="flex items-center gap-1.5">
                          <User className="h-3.5 w-3.5 text-muted-foreground" />
                          <span className="text-muted-foreground">Author:</span>
                          <span className="font-medium truncate">{selectedSkillListing.author}</span>
                        </div>
                      )}
                      {selectedSkillListing.version && (
                        <div className="flex items-center gap-1.5">
                          <Tag className="h-3.5 w-3.5 text-muted-foreground" />
                          <span className="text-muted-foreground">Version:</span>
                          <span className="font-medium">{selectedSkillListing.version}</span>
                        </div>
                      )}
                      <div className="flex items-center gap-1.5">
                        <GitBranch className="h-3.5 w-3.5 text-muted-foreground" />
                        <span className="text-muted-foreground">Source:</span>
                        <span className="font-medium truncate">{selectedSkillListing.source_label}</span>
                      </div>
                      <div className="flex items-center gap-1.5">
                        <File className="h-3.5 w-3.5 text-muted-foreground" />
                        <span className="text-muted-foreground">Files:</span>
                        <span className="font-medium">{selectedSkillListing.files.length}</span>
                      </div>
                    </div>

                    {selectedSkillListing.tags.length > 0 && (
                      <div className="flex flex-wrap gap-1.5 pt-1">
                        {selectedSkillListing.tags.map((tag) => (
                          <span
                            key={tag}
                            className="text-xs px-2 py-0.5 rounded-full bg-muted text-muted-foreground"
                          >
                            {tag}
                          </span>
                        ))}
                      </div>
                    )}
                  </CardContent>
                </Card>

                {/* Files Preview */}
                {selectedSkillListing.files.length > 0 && (
                  <Card>
                    <CardHeader className="pb-3">
                      <CardTitle className="text-sm">Files to Install</CardTitle>
                      <CardDescription className="text-xs">Click on a file to preview its contents</CardDescription>
                    </CardHeader>
                    <CardContent>
                      <div className="rounded-md border border-border/50 bg-muted/20 py-1 max-h-64 overflow-y-auto">
                        {[
                          { path: 'SKILL.md', url: selectedSkillListing.skill_md_url },
                          ...selectedSkillListing.files
                        ].map((file) => {
                          const isExpanded = expandedPreviewFiles.has(file.path)
                          const content = previewFileContents[file.path]
                          const fileName = file.path.split('/').pop() || file.path
                          const ext = fileName.split('.').pop()?.toLowerCase() ?? ''
                          const isTextFile = ['md', 'txt', 'json', 'yaml', 'yml', 'sh', 'bash', 'py', 'js', 'ts', 'rb', 'pl', 'toml', 'xml', 'html', 'css', 'scss'].includes(ext)
                          const isSkillMd = file.path === 'SKILL.md'

                          return (
                            <div key={file.path}>
                              <button
                                className="w-full flex items-center gap-1.5 py-1 px-3 text-xs hover:bg-muted/50 transition-colors text-left"
                                onClick={() => isTextFile && togglePreviewFileExpanded(file)}
                                disabled={!isTextFile}
                              >
                                {isTextFile ? (
                                  isExpanded
                                    ? <ChevronDown className="h-3 w-3 shrink-0 text-muted-foreground" />
                                    : <ChevronRight className="h-3 w-3 shrink-0 text-muted-foreground" />
                                ) : <div className="w-3" />}
                                <FileText className={cn("h-3.5 w-3.5 shrink-0", isSkillMd ? "text-amber-500" : "text-muted-foreground")} />
                                <span className="truncate" title={file.path}>{file.path}</span>
                                {isSkillMd && (
                                  <span className="text-[9px] px-1 py-0.5 rounded bg-amber-500/10 text-amber-800 dark:text-amber-400 font-medium uppercase ml-1">Definition</span>
                                )}
                              </button>
                              {isExpanded && (
                                <pre className="mx-3 mb-1 px-3 py-2 text-[10px] leading-relaxed bg-muted/30 rounded border border-border/50 overflow-x-auto max-h-48 whitespace-pre-wrap break-words">
                                  {content || 'Loading...'}
                                </pre>
                              )}
                            </div>
                          )
                        })}
                      </div>
                    </CardContent>
                  </Card>
                )}

                {/* Install Actions */}
                {(() => {
                  const isAlreadyInstalled = installedSkillNames.includes(selectedSkillListing.name)
                  return (
                    <div className="flex justify-end gap-2 pt-2">
                      <Button
                        type="button"
                        variant="secondary"
                        onClick={() => {
                          setShowInstallDialog(false)
                          resetInstallDialog()
                        }}
                        disabled={isInstalling}
                      >
                        Cancel
                      </Button>
                      <Button onClick={handleInstallSkill} disabled={isInstalling}>
                        {isInstalling ? (
                          <>
                            <Loader2 className="h-4 w-4 mr-2 animate-spin" />
                            {isAlreadyInstalled ? "Replacing..." : "Installing..."}
                          </>
                        ) : isAlreadyInstalled ? (
                          <>
                            <RefreshCw className="h-4 w-4 mr-2" />
                            Replace
                          </>
                        ) : (
                          <>
                            <Plus className="h-4 w-4 mr-2" />
                            Install
                          </>
                        )}
                      </Button>
                    </div>
                  )
                })()}
              </div>
            </div>
          )}
        </DialogContent>
      </Dialog>
    </div>
  )
}
