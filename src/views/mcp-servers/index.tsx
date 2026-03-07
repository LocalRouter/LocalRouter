/**
 * MCP Servers View
 *
 * Standalone view for managing MCP (Model Context Protocol) servers.
 */

import { useState, useEffect, useCallback } from "react"
import { invoke } from "@tauri-apps/api/core"
import { listen } from "@tauri-apps/api/event"
import { toast } from "sonner"
import { RefreshCw, Loader2, RotateCcw } from "lucide-react"
import { Tabs, TabsContent, TabsList, TabsTrigger } from "@/components/ui/tabs"
import { Card, CardContent, CardHeader, CardTitle, CardDescription } from "@/components/ui/Card"
import { Switch } from "@/components/ui/switch"
import { Input } from "@/components/ui/Input"
import { Button } from "@/components/ui/Button"
import { McpIcon } from "@/components/icons/category-icons"
import { SamplePopupButton } from "@/components/shared/SamplePopupButton"
import { McpServersPanel, McpHealthStatus, McpHealthCheckEvent } from "../resources/mcp-servers-panel"
import { MarketplaceSearchPanel, type McpServerListing } from "@/components/add-resource"

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

interface McpServersViewProps {
  activeSubTab: string | null
  onTabChange: (view: string, subTab?: string | null) => void
}

export function McpServersView({ activeSubTab, onTabChange }: McpServersViewProps) {
  // Parse subTab to determine top-level tab and server selection
  // Format: "try-it-out" or "try-it-out/init/..." or "marketplace" or "settings" or "server-id" or "add/template-id"
  const parseSubTab = (subTab: string | null) => {
    if (!subTab) return { topTab: "servers", selectedId: null, addTemplateId: null }
    if (subTab === "marketplace") return { topTab: "marketplace", selectedId: null, addTemplateId: null }
if (subTab === "settings") return { topTab: "settings", selectedId: null, addTemplateId: null }
    if (subTab.startsWith("add/")) {
      return { topTab: "servers", selectedId: null, addTemplateId: subTab.slice(4) }
    }
    return { topTab: "servers", selectedId: subTab, addTemplateId: null }
  }

  const { topTab, selectedId, addTemplateId } = parseSubTab(activeSubTab)

  // Lifted health status state - persists across interactions
  const [healthStatus, setHealthStatus] = useState<Record<string, McpHealthStatus>>({})
  const [healthInitialized, setHealthInitialized] = useState(false)

  // Settings state
  const [marketplaceConfig, setMarketplaceConfig] = useState<MarketplaceConfig | null>(null)
  const [cacheStatus, setCacheStatus] = useState<CacheStatus | null>(null)
  const [registryUrl, setRegistryUrl] = useState("")
  const [savingRegistry, setSavingRegistry] = useState(false)
  const [refreshingCache, setRefreshingCache] = useState(false)
  const [clearingCache, setClearingCache] = useState(false)

  // Start health checks for all servers (called once on mount)
  const startHealthChecks = useCallback(async (serverIds: string[]) => {
    setHealthStatus((prev) => {
      const updated = { ...prev }
      for (const id of serverIds) {
        if (!updated[id]) {
          updated[id] = { status: "pending" }
        }
      }
      return updated
    })

    try {
      await invoke("start_mcp_health_checks")
    } catch (error) {
      console.error("Failed to start MCP health checks:", error)
    }
  }, [])

  // Refresh health for a single server
  const refreshHealth = useCallback(async (serverId: string) => {
    setHealthStatus((prev) => ({
      ...prev,
      [serverId]: { status: "pending" },
    }))
    await invoke("check_single_mcp_health", { serverId })
  }, [])

  // Listen for health check events
  useEffect(() => {
    const unsubHealth = listen<McpHealthCheckEvent>("mcp-health-check", (event) => {
      const { server_id, status, latency_ms, error } = event.payload
      setHealthStatus((prev) => ({
        ...prev,
        [server_id]: {
          status: status as McpHealthStatus["status"],
          latency_ms,
          error,
        },
      }))
    })

    interface ItemHealth {
      name: string
      status: string
      latency_ms?: number
      error?: string
    }
    interface HealthCacheState {
      mcp_servers: Record<string, ItemHealth>
    }
    const unsubCacheChanged = listen<HealthCacheState>("health-status-changed", (event) => {
      const { mcp_servers } = event.payload
      if (!mcp_servers) return
      setHealthStatus((prev) => {
        const updated = { ...prev }
        for (const [id, health] of Object.entries(mcp_servers)) {
          updated[id] = {
            status: health.status as McpHealthStatus["status"],
            latency_ms: health.latency_ms,
            error: health.error,
          }
        }
        return updated
      })
    })

    return () => {
      unsubHealth.then((fn) => fn())
      unsubCacheChanged.then((fn) => fn())
    }
  }, [])

  // Load marketplace config and cache status for settings tab
  const loadSettingsData = useCallback(async () => {
    try {
      const [cfg, cache] = await Promise.all([
        invoke<MarketplaceConfig>("marketplace_get_config"),
        invoke<CacheStatus>("marketplace_get_cache_status").catch(() => null),
      ])
      setMarketplaceConfig(cfg)
      setRegistryUrl(cfg.registry_url)
      setCacheStatus(cache)
    } catch (error) {
      console.error("Failed to load settings:", error)
    }
  }, [])

  useEffect(() => {
    if (topTab === "settings" || topTab === "marketplace") {
      loadSettingsData()
    }
  }, [topTab, loadSettingsData])

  const handleSelect = (id: string | null) => {
    onTabChange("mcp-servers", id)
  }

  const handleTopTabChange = (tab: string) => {
    onTabChange("mcp-servers", tab === "servers" ? null : tab)
  }

  // Marketplace: navigate to servers tab with the selected listing for installation
  const handleSelectMcp = (listing: McpServerListing) => {
    onTabChange("mcp-servers", `add/${listing.name}`)
  }

  // Settings handlers
  const handleToggleMcpEnabled = async (enabled: boolean) => {
    try {
      await invoke("marketplace_set_mcp_enabled", { enabled })
      setMarketplaceConfig(prev => prev ? { ...prev, mcp_enabled: enabled } : prev)
      toast.success(enabled ? "MCP marketplace enabled" : "MCP marketplace disabled")
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
    setClearingCache(true)
    try {
      await invoke("marketplace_clear_mcp_cache")
      const cache = await invoke<CacheStatus>("marketplace_get_cache_status").catch(() => null)
      setCacheStatus(cache)
      toast.success("MCP cache cleared")
    } catch (error) {
      toast.error(`Failed to clear cache: ${error}`)
    } finally {
      setClearingCache(false)
    }
  }

  const formatLastRefresh = (date: string | null) => {
    if (!date) return "Never"
    return new Date(date).toLocaleString()
  }

  return (
    <div className="flex flex-col h-full min-h-0">
      <div className="flex-shrink-0 pb-4">
        <h1 className="text-2xl font-bold tracking-tight flex items-center gap-2"><McpIcon className="h-6 w-6" />MCP</h1>
        <div className="flex items-center gap-2">
          <p className="text-sm text-muted-foreground">
            Connect to external MCP servers and aggregate their tools, prompts, and resources into the unified MCP gateway that clients connect to.
          </p>
          <SamplePopupButton popupType="mcp_tool" />
        </div>
      </div>

      <Tabs
        value={topTab}
        onValueChange={handleTopTabChange}
        className="flex flex-col flex-1 min-h-0"
      >
        <TabsList className="flex-shrink-0 w-fit">
          <TabsTrigger value="servers">Servers</TabsTrigger>
          <TabsTrigger value="marketplace">Marketplace</TabsTrigger>
<TabsTrigger value="settings">Settings</TabsTrigger>
        </TabsList>

        <TabsContent value="servers" className="flex-1 min-h-0 mt-4">
          <McpServersPanel
            selectedId={selectedId}
            onSelect={handleSelect}
            healthStatus={healthStatus}
            onHealthInit={(serverIds) => {
              if (!healthInitialized) {
                setHealthInitialized(true)
                startHealthChecks(serverIds)
              }
            }}
            onRefreshHealth={refreshHealth}
            initialAddTemplateId={addTemplateId}
            onViewChange={onTabChange}
          />
        </TabsContent>

        <TabsContent value="marketplace" className="flex-1 min-h-0 mt-4">
          <MarketplaceSearchPanel
            type="mcp"
            onSelectMcp={handleSelectMcp}
            maxHeight="100%"
          />
        </TabsContent>

        <TabsContent value="settings" className="flex-1 min-h-0 mt-4 overflow-y-auto">
          <div className="space-y-6 max-w-2xl">
            {/* MCP Marketplace Toggle */}
            <Card>
              <CardHeader>
                <CardTitle>MCP Marketplace</CardTitle>
                <CardDescription>
                  Browse and install MCP servers from the official registry.
                </CardDescription>
              </CardHeader>
              <CardContent>
                <div className="flex items-center gap-3">
                  <Switch
                    checked={marketplaceConfig?.mcp_enabled ?? false}
                    onCheckedChange={handleToggleMcpEnabled}
                  />
                  <span className="text-sm">
                    {marketplaceConfig?.mcp_enabled ? "Enabled" : "Disabled"}
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
                  <Button
                    onClick={handleSaveRegistryUrl}
                    disabled={savingRegistry}
                    size="sm"
                  >
                    {savingRegistry ? <Loader2 className="h-4 w-4 animate-spin" /> : "Save"}
                  </Button>
                  <Button
                    onClick={handleResetRegistryUrl}
                    variant="outline"
                    size="sm"
                    title="Reset to default"
                  >
                    <RotateCcw className="h-4 w-4" />
                  </Button>
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
              <CardContent className="space-y-3">
                {cacheStatus && (
                  <div className="text-sm text-muted-foreground space-y-1">
                    <p>Last refresh: {formatLastRefresh(cacheStatus.mcp_last_refresh)}</p>
                    <p>Cached queries: {cacheStatus.mcp_cached_queries}</p>
                  </div>
                )}
                <div className="flex gap-2">
                  <Button
                    onClick={handleRefreshCache}
                    disabled={refreshingCache}
                    variant="outline"
                    size="sm"
                  >
                    <RefreshCw className={`h-4 w-4 mr-2 ${refreshingCache ? "animate-spin" : ""}`} />
                    Refresh
                  </Button>
                  <Button
                    onClick={handleClearMcpCache}
                    disabled={clearingCache}
                    variant="outline"
                    size="sm"
                  >
                    {clearingCache ? <Loader2 className="h-4 w-4 mr-2 animate-spin" /> : null}
                    Clear MCP Cache
                  </Button>
                </div>
              </CardContent>
            </Card>
          </div>
        </TabsContent>

      </Tabs>
    </div>
  )
}
