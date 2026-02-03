import { useState, useEffect, useRef } from "react"
import { invoke } from "@tauri-apps/api/core"
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
  ExternalLink,
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
import { DisabledOverlay } from "./DisabledOverlay"

// Types matching the backend - exported for use by parent components
export interface McpPackageInfo {
  registry: string
  name: string
  version: string | null
  runtime: string | null
  license: string | null
}

export interface McpRemoteInfo {
  transport_type: string
  url: string
}

export interface McpServerListing {
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

export interface SkillFileInfo {
  path: string
  url: string
}

export interface SkillListing {
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
  skill_sources: { repo_url: string; branch: string; path: string; label: string }[]
}

type ResourceType = "mcp" | "skill"

interface MarketplaceSearchPanelProps {
  type: ResourceType
  /** Callback when an MCP server is selected for installation (type="mcp" only) */
  onSelectMcp?: (item: McpServerListing) => void
  /** Callback when a skill is selected for installation (type="skill" only) */
  onSelectSkill?: (item: SkillListing) => void
  /** @deprecated Use onSelectMcp or onSelectSkill instead. Legacy callback for direct install. */
  onInstallComplete?: () => void
  /** List of already installed skill names (for showing "Replace" instead of "Add") */
  installedSkillNames?: string[]
  className?: string
  maxHeight?: string
}

export function MarketplaceSearchPanel({
  type,
  onSelectMcp,
  onSelectSkill,
  onInstallComplete: _onInstallComplete,
  installedSkillNames = [],
  className,
  maxHeight = "400px",
}: MarketplaceSearchPanelProps) {
  const [config, setConfig] = useState<MarketplaceConfig | null>(null)
  const [loading, setLoading] = useState(true)
  const hasInitialSearch = useRef(false)

  // MCP state
  const [mcpSearch, setMcpSearch] = useState("")
  const [mcpServers, setMcpServers] = useState<McpServerListing[]>([])
  const [searchingMcp, setSearchingMcp] = useState(false)

  // Skill state
  const [skillSearch, setSkillSearch] = useState("")
  const [skills, setSkills] = useState<SkillListing[]>([])
  const [searchingSkills, setSearchingSkills] = useState(false)
  const [selectedSkillSource, setSelectedSkillSource] = useState<string>("all")

  useEffect(() => {
    loadConfig()
  }, [])

  // Trigger initial search when config is loaded and enabled
  useEffect(() => {
    if (config?.enabled && !hasInitialSearch.current) {
      hasInitialSearch.current = true
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

  const handleMcpClick = (server: McpServerListing) => {
    if (onSelectMcp) {
      onSelectMcp(server)
    }
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

  const handleSkillClick = (skill: SkillListing) => {
    if (onSelectSkill) {
      onSelectSkill(skill)
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
        <div className="flex-1 min-h-0 overflow-hidden" style={{ maxHeight }}>
          <ScrollArea className="h-full">
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
              <div className="space-y-3 pr-4">
                {mcpServers.map((server) => {
                  const pkg = server.packages[0]
                  return (
                    <Card key={server.name} className="p-3">
                      <div className="flex items-start justify-between gap-2">
                        <div className="flex-1 min-w-0">
                          <div className="flex items-center gap-2">
                            <p className="font-medium text-sm truncate">{server.name}</p>
                            {pkg?.version && (
                              <span className="text-xs text-muted-foreground">v{pkg.version}</span>
                            )}
                          </div>
                          {server.vendor && (
                            <p className="text-xs text-muted-foreground">by {server.vendor}</p>
                          )}
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
                              <Badge variant="outline" className="text-xs">
                                {pkg.license}
                              </Badge>
                            )}
                            {pkg?.runtime && (
                              <Badge variant="outline" className="text-xs">
                                {pkg.runtime}
                              </Badge>
                            )}
                          </div>
                          {server.homepage && (
                            <a
                              href={server.homepage}
                              target="_blank"
                              rel="noopener noreferrer"
                              className="inline-flex items-center gap-1 text-xs text-primary hover:underline mt-2"
                              onClick={(e) => e.stopPropagation()}
                            >
                              <ExternalLink className="h-3 w-3" />
                              Source
                            </a>
                          )}
                        </div>
                        <Button size="sm" variant="secondary" onClick={() => handleMcpClick(server)}>
                          View
                          <ChevronRight className="h-4 w-4 ml-1" />
                        </Button>
                      </div>
                    </Card>
                  )
                })}
              </div>
            )}
          </ScrollArea>
        </div>
      </div>
    )
  }

  // Skill Search Panel
  return (
    <div className={className}>
      {/* Search bar */}
      <div className="flex items-center gap-2 mb-4">
        <div className="relative flex-1">
          <Search className="absolute left-3 top-1/2 -translate-y-1/2 h-4 w-4 text-muted-foreground" />
          <Input
            placeholder="Search skills..."
            value={skillSearch}
            onChange={(e) => setSkillSearch(e.target.value)}
            onKeyDown={(e) => e.key === "Enter" && searchSkills()}
            className="pl-9 h-9"
          />
        </div>
        <Select value={selectedSkillSource} onValueChange={setSelectedSkillSource}>
          <SelectTrigger className="w-[140px] h-9">
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
        <Button onClick={searchSkills} disabled={searchingSkills} className="h-9">
          {searchingSkills ? <Loader2 className="h-4 w-4 animate-spin" /> : "Search"}
        </Button>
      </div>

      {/* Results */}
      <div className="flex-1 min-h-0 overflow-hidden" style={{ maxHeight }}>
        <ScrollArea className="h-full">
          {searchingSkills ? (
            <div className="flex items-center justify-center py-12">
              <Loader2 className="h-6 w-6 animate-spin text-muted-foreground" />
            </div>
          ) : skills.length === 0 ? (
            <div className="flex flex-col items-center justify-center py-8 text-muted-foreground">
              <SkillsIcon className="h-8 w-8 mb-2" />
              <p className="text-sm">No skills found</p>
              {(!config?.skill_sources || config.skill_sources.length === 0) && (
                <p className="text-xs mt-2 text-center max-w-[280px]">
                  No skill sources configured. Add sources in Settings → Marketplace to browse available skills.
                </p>
              )}
            </div>
          ) : (
            <div className="space-y-3 pr-4">
              {skills.slice(0, 20).map((skill) => {
                const isInstalled = installedSkillNames.includes(skill.name)
                return (
                <Card key={`${skill.source_label}-${skill.name}`} className="p-3">
                  <div className="flex items-start justify-between gap-2">
                    <div className="flex-1 min-w-0">
                      <div className="flex items-center gap-2">
                        <p className="font-medium text-sm truncate">{skill.name}</p>
                        {isInstalled && (
                          <Badge variant="secondary" className="text-xs shrink-0">
                            <Check className="h-3 w-3 mr-0.5" />
                            Installed
                          </Badge>
                        )}
                      </div>
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
              )})}
            </div>
          )}
        </ScrollArea>
      </div>
    </div>
  )
}
