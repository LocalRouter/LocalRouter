import { useState, useEffect, useCallback } from "react"
import { invoke } from "@tauri-apps/api/core"
import { toast } from "sonner"
import { BookText, RefreshCw, Loader2, Search, Database, AlertTriangle, Info } from "lucide-react"
import { OPTIMIZE_COLORS } from "@/views/optimize-overview/constants"
import { Badge } from "@/components/ui/Badge"
import { Button } from "@/components/ui/Button"
import { Card, CardContent, CardHeader, CardTitle, CardDescription } from "@/components/ui/Card"
import { Input } from "@/components/ui/Input"
import { Tabs, TabsContent, TabsList, TabsTrigger } from "@/components/ui/tabs"
import { ScrollArea } from "@/components/ui/scroll-area"
import {
  ResizablePanelGroup,
  ResizablePanel,
  ResizableHandle,
} from "@/components/ui/resizable"
import { Switch } from "@/components/ui/Toggle"
import { cn } from "@/lib/utils"
import { McpToolDisplay } from "@/components/shared/McpToolDisplay"
import type { McpToolDisplayItem } from "@/components/shared/McpToolDisplay"
import type { ContextManagementConfig, ActiveSessionInfo, CatalogSourceEntry, CatalogCompressionPreview, PreviewCatalogCompressionParams, PreviewServerEntry, ClientInfo } from "@/types/tauri-commands"

// Must match defaults in crates/lr-config/src/types.rs
const DEFAULT_CATALOG_THRESHOLD_BYTES = 1000


interface CatalogCompressionViewProps {
  activeSubTab?: string | null
  onTabChange?: (view: string, subTab?: string | null) => void
}

export function CatalogCompressionView({ activeSubTab, onTabChange }: CatalogCompressionViewProps) {
  const [config, setConfig] = useState<ContextManagementConfig | null>(null)
  const [sessions, setSessions] = useState<ActiveSessionInfo[]>([])
  const [selectedSessionId, setSelectedSessionId] = useState<string | null>(null)
  const [, setSaving] = useState(false)
  const [sessionDetailTab, setSessionDetailTab] = useState<string>("info")
  const [catalogSources, setCatalogSources] = useState<CatalogSourceEntry[]>([])
  const [catalogSourcesLoading, setCatalogSourcesLoading] = useState(false)
  const [searchQuery, setSearchQuery] = useState("")
  const [searchResults, setSearchResults] = useState<string | null>(null)
  const [searchLoading, setSearchLoading] = useState(false)

  const tab = activeSubTab || "info"

  const handleTabChange = (newTab: string) => {
    onTabChange?.("catalog-compression", newTab)
  }

  const loadSessions = useCallback(async () => {
    try {
      const data = await invoke<ActiveSessionInfo[]>("list_active_sessions")
      setSessions(data)
    } catch (err) {
      console.error("Failed to load sessions:", err)
    }
  }, [])

  const loadCatalogSources = useCallback(async (sessionId: string) => {
    setCatalogSourcesLoading(true)
    try {
      const data = await invoke<CatalogSourceEntry[]>("get_session_context_sources", { sessionId })
      setCatalogSources(data)
    } catch (err) {
      console.error("Failed to load catalog sources:", err)
      setCatalogSources([])
    } finally {
      setCatalogSourcesLoading(false)
    }
  }, [])

  const runSearch = useCallback(async (sessionId: string) => {
    if (!searchQuery.trim()) return
    setSearchLoading(true)
    try {
      const result = await invoke<{ content?: Array<{ text?: string }> }>("query_session_context_index", { sessionId, query: searchQuery })
      const text = result?.content?.map((c) => c.text || "").join("\n") || "No results"
      setSearchResults(text)
    } catch (err) {
      setSearchResults(`Error: ${err}`)
    } finally {
      setSearchLoading(false)
    }
  }, [searchQuery])

  // Load detail data when session or detail tab changes
  useEffect(() => {
    if (!selectedSessionId) return
    const session = sessions.find((s) => s.session_id === selectedSessionId)
    if (!session?.context_management_enabled) return

    if (sessionDetailTab === "index") {
      loadCatalogSources(selectedSessionId)
    }
  }, [selectedSessionId, sessionDetailTab, sessions, loadCatalogSources])

  // Reset detail tab when session changes
  useEffect(() => {
    setSessionDetailTab("info")
    setCatalogSources([])
    setSearchResults(null)
    setSearchQuery("")
  }, [selectedSessionId])

  useEffect(() => {
    let ignore = false

    invoke<ContextManagementConfig>("get_context_management_config")
      .then((cfg) => { if (!ignore) setConfig(cfg) })
      .catch((err) => console.error("Failed to load context management config:", err))
    loadSessions()

    const interval = setInterval(loadSessions, 5000)
    return () => {
      ignore = true
      clearInterval(interval)
    }
  }, [loadSessions])

  // Clear selected session if it disappears
  useEffect(() => {
    if (selectedSessionId) {
      const stillExists = sessions.some((s) => s.session_id === selectedSessionId)
      if (!stillExists) {
        setSelectedSessionId(null)
      }
    }
  }, [sessions, selectedSessionId])

  const updateField = async (field: string, value: unknown) => {
    try {
      setSaving(true)
      await invoke("update_context_management_config", { [field]: value })
      const updated = await invoke<ContextManagementConfig>("get_context_management_config")
      setConfig(updated)
    } catch (error) {
      toast.error(`Failed to update: ${error}`)
    } finally {
      setSaving(false)
    }
  }

  const selectedSession = selectedSessionId
    ? sessions.find((s) => s.session_id === selectedSessionId) ?? null
    : null

  return (
    <div className="flex flex-col h-full min-h-0 max-w-5xl">
      <div className="flex-shrink-0 pb-4">
        <div className="flex items-center gap-2">
          <h1 className="text-2xl font-bold tracking-tight flex items-center gap-2">
            <BookText className={`h-6 w-6 ${OPTIMIZE_COLORS.catalogCompression}`} />
            MCP Catalog Compression
          </h1>
          <Badge variant="outline" className="bg-purple-500/10 text-purple-900 dark:text-purple-400">EXPERIMENTAL</Badge>
        </div>
        <p className="text-sm text-muted-foreground">
          Compress MCP catalogs using deferred loading and FTS5 search indexing
        </p>
      </div>

      <Tabs
        value={tab}
        onValueChange={handleTabChange}
        className="flex flex-col flex-1 min-h-0"
      >
        <TabsList className="flex-shrink-0 w-fit">
          <TabsTrigger value="info">Info</TabsTrigger>
          <TabsTrigger value="preview">Try it out</TabsTrigger>
          <TabsTrigger value="sessions">
            Sessions
            {sessions.length > 0 && (
              <Badge variant="secondary" className="ml-1.5 text-[10px] px-1 py-0">
                {sessions.length}
              </Badge>
            )}
          </TabsTrigger>
          <TabsTrigger value="settings">Settings</TabsTrigger>
        </TabsList>

        {/* Info Tab */}
        <TabsContent value="info" className="flex-1 min-h-0 mt-4">
          <div className="space-y-4 max-w-2xl">
            {config && (
              <>
                <Card>
                  <CardHeader>
                    <div className="flex items-center justify-between">
                      <CardTitle className="text-base">Default: Catalog Compression</CardTitle>
                      <Switch
                        checked={config.catalog_compression}
                        onCheckedChange={(value) => updateField("catalogCompression", value)}
                      />
                    </div>
                    <CardDescription>
                      When enabled, tools, prompts, and resources are deferred from the initial catalog
                      and loaded on-demand via FTS5 search indexing. This reduces the context window
                      usage for clients with many MCP servers.
                    </CardDescription>
                  </CardHeader>
                </Card>

                <div className="flex items-center gap-2 text-sm text-muted-foreground">
                  <span>Exposed tool names:</span>
                  <code className="text-xs bg-muted px-1.5 py-0.5 rounded">{config.search_tool_name}</code>
                  <code className="text-xs bg-muted px-1.5 py-0.5 rounded">{config.read_tool_name}</code>
                </div>
              </>
            )}
          </div>
        </TabsContent>

        {/* Sessions Tab */}
        <TabsContent value="sessions" className="flex-1 min-h-0 mt-4">
          {sessions.length === 0 ? (
            <div className="flex flex-col items-center justify-center h-full text-muted-foreground gap-4 border rounded-lg">
              <BookText className="h-12 w-12 opacity-30" />
              <div className="text-center">
                <p className="font-medium">No active sessions</p>
                <p className="text-sm mt-1">
                  Sessions will appear here when MCP clients connect through the gateway.
                </p>
              </div>
            </div>
          ) : (
            <ResizablePanelGroup direction="horizontal" className="flex-1 min-h-0 rounded-lg border">
              {/* Session List */}
              <ResizablePanel defaultSize={21} minSize={15}>
                <div className="flex flex-col h-full">
                  <div className="p-4 border-b">
                    <div className="flex items-center justify-between">
                      <p className="text-sm font-medium">{sessions.length} session{sessions.length !== 1 ? "s" : ""}</p>
                      <Button
                        variant="ghost"
                        size="sm"
                        onClick={loadSessions}
                        className="h-7 w-7 p-0"
                      >
                        <RefreshCw className="h-3.5 w-3.5" />
                      </Button>
                    </div>
                  </div>
                  <ScrollArea className="flex-1">
                    <div className="p-2 space-y-1">
                      {sessions.map((s) => (
                        <div
                          key={s.session_id}
                          onClick={() => setSelectedSessionId(s.session_id)}
                          className={cn(
                            "flex flex-col gap-1 p-3 rounded-md cursor-pointer",
                            selectedSessionId === s.session_id
                              ? "bg-accent"
                              : "hover:bg-muted"
                          )}
                        >
                          <div className="flex items-center justify-between gap-2">
                            <p className="text-sm font-medium truncate flex-1">
                              {s.client_name || s.client_id}
                            </p>
                            <span className="text-[10px] text-muted-foreground shrink-0">
                              {formatDuration(s.duration_secs)}
                            </span>
                          </div>
                          <p className="text-xs text-muted-foreground truncate">
                            {s.initialized_servers} server{s.initialized_servers !== 1 ? "s" : ""}
                            {s.failed_servers > 0 ? ` (${s.failed_servers} failed)` : ""}
                            {s.context_management_enabled
                              ? <>{" "}&middot; {s.cm_activated_tools}/{s.cm_total_tools} tools exposed</>
                              : <>{" "}&middot; {s.total_tools} tools</>
                            }
                          </p>
                        </div>
                      ))}
                    </div>
                  </ScrollArea>
                </div>
              </ResizablePanel>

              <ResizableHandle withHandle />

              {/* Session Detail */}
              <ResizablePanel defaultSize={79}>
                {selectedSession ? (
                    <div className="flex flex-col h-full">
                      <div className="flex-shrink-0 p-4 pb-0">
                        <div className="flex items-center gap-2 mb-3">
                          <h2 className="text-lg font-bold truncate">
                            {selectedSession.client_name || selectedSession.client_id}
                          </h2>
                        </div>
                        {selectedSession.context_management_enabled ? (
                          <Tabs value={sessionDetailTab} onValueChange={setSessionDetailTab}>
                            <TabsList className="w-fit">
                              <TabsTrigger value="info">
                                <Info className="h-3.5 w-3.5 mr-1" />
                                Info
                              </TabsTrigger>
                              <TabsTrigger value="index">
                                <Database className="h-3.5 w-3.5 mr-1" />
                                Index
                              </TabsTrigger>
                              <TabsTrigger value="query">
                                <Search className="h-3.5 w-3.5 mr-1" />
                                Query
                              </TabsTrigger>
                            </TabsList>
                          </Tabs>
                        ) : null}
                      </div>

                      <ScrollArea className="flex-1 min-h-0">
                        <div className="p-4 space-y-4">
                          {/* Info sub-tab (or only content when CM is off) */}
                          {(sessionDetailTab === "info" || !selectedSession.context_management_enabled) && (
                            <>
                              {selectedSession.client_name && (
                                <p className="text-sm text-muted-foreground">
                                  {selectedSession.client_id}
                                </p>
                              )}
                              <Card>
                                <CardHeader className="pb-3">
                                  <CardTitle className="text-sm">Session Info</CardTitle>
                                </CardHeader>
                                <CardContent>
                                  <div className="grid grid-cols-2 gap-y-3 gap-x-6 text-sm">
                                    <div>
                                      <p className="text-muted-foreground text-xs">Duration</p>
                                      <p className="font-medium">{formatDuration(selectedSession.duration_secs)}</p>
                                    </div>
                                    <div>
                                      <p className="text-muted-foreground text-xs">Connected MCP Servers</p>
                                      <p className="font-medium">
                                        {selectedSession.initialized_servers}
                                        {selectedSession.failed_servers > 0 && (
                                          <span className="text-destructive"> ({selectedSession.failed_servers} failed)</span>
                                        )}
                                      </p>
                                    </div>
                                  </div>
                                </CardContent>
                              </Card>

                              {selectedSession.context_management_enabled ? (
                                <Card>
                                  <CardHeader className="pb-3">
                                    <CardTitle className="text-sm">Tool Compression</CardTitle>
                                    <CardDescription>
                                      Tools exceeding the catalog threshold are hidden from the client and discoverable via search.
                                    </CardDescription>
                                  </CardHeader>
                                  <CardContent>
                                    <div className="grid grid-cols-2 gap-y-3 gap-x-6 text-sm">
                                      <div>
                                        <p className="text-muted-foreground text-xs">Exposed to Client</p>
                                        <p className="font-medium">
                                          {selectedSession.cm_activated_tools} of {selectedSession.cm_total_tools} tools
                                        </p>
                                      </div>
                                      <div>
                                        <p className="text-muted-foreground text-xs">Hidden (Deferred)</p>
                                        <p className="font-medium">
                                          {selectedSession.cm_total_tools - selectedSession.cm_activated_tools} tools
                                        </p>
                                      </div>
                                      <div>
                                        <p className="text-muted-foreground text-xs">Catalog Threshold</p>
                                        <p className="font-medium">
                                          {formatBytes(selectedSession.cm_catalog_threshold_bytes)}
                                        </p>
                                      </div>
                                      <div>
                                        <p className="text-muted-foreground text-xs">Indexed Sources</p>
                                        <p className="font-medium">{selectedSession.cm_indexed_sources}</p>
                                      </div>
                                    </div>
                                  </CardContent>
                                </Card>
                              ) : (
                                <Card>
                                  <CardHeader className="pb-3">
                                    <CardTitle className="text-sm">Tool Compression</CardTitle>
                                  </CardHeader>
                                  <CardContent>
                                    <p className="text-sm text-muted-foreground">
                                      Disabled for this session. All {selectedSession.total_tools} tools are sent to the client without compression.
                                    </p>
                                  </CardContent>
                                </Card>
                              )}
                            </>
                          )}

                          {/* Index sub-tab */}
                          {sessionDetailTab === "index" && selectedSession.context_management_enabled && (
                            <Card>
                              <CardHeader className="pb-3">
                                <div className="flex items-center justify-between">
                                  <CardTitle className="text-sm">Catalog Index</CardTitle>
                                  <Button
                                    variant="ghost"
                                    size="sm"
                                    onClick={() => loadCatalogSources(selectedSession.session_id)}
                                    className="h-7 w-7 p-0"
                                    disabled={catalogSourcesLoading}
                                  >
                                    {catalogSourcesLoading ? (
                                      <Loader2 className="h-3.5 w-3.5 animate-spin" />
                                    ) : (
                                      <RefreshCw className="h-3.5 w-3.5" />
                                    )}
                                  </Button>
                                </div>
                                <CardDescription>
                                  {catalogSources.length} source{catalogSources.length !== 1 ? "s" : ""} indexed
                                  {" "}&middot; {catalogSources.filter((s) => s.activated).length} activated
                                </CardDescription>
                              </CardHeader>
                              <CardContent>
                                {catalogSourcesLoading && catalogSources.length === 0 ? (
                                  <div className="flex items-center gap-2 text-sm text-muted-foreground">
                                    <Loader2 className="h-4 w-4 animate-spin" />
                                    Loading index...
                                  </div>
                                ) : catalogSources.length === 0 ? (
                                  <p className="text-sm text-muted-foreground">No sources indexed yet.</p>
                                ) : (
                                  <>
                                    <div className="space-y-1">
                                      {catalogSources.map((source) => (
                                        <div
                                          key={source.source_label}
                                          className="flex items-center justify-between gap-2 py-1.5 px-2 rounded-md text-sm hover:bg-muted"
                                        >
                                          <code className="text-xs truncate min-w-0">
                                            {source.source_label}
                                          </code>
                                          <div className="flex items-center gap-2 shrink-0">
                                            <Badge variant="outline" className="text-[10px] px-1.5 py-0">
                                              {source.item_type}
                                            </Badge>
                                            {source.activated ? (
                                              <Badge variant="success" className="text-[10px] px-1.5 py-0">
                                                active
                                              </Badge>
                                            ) : (
                                            <Badge variant="secondary" className="text-[10px] px-1.5 py-0">
                                              deferred
                                            </Badge>
                                          )}
                                          <Button
                                            variant="ghost"
                                            size="sm"
                                            className="h-6 w-6 p-0"
                                            title={`Search in ${source.source_label}`}
                                            onClick={() => {
                                              setSearchQuery(source.source_label)
                                              setSearchResults(null)
                                              setSessionDetailTab("query")
                                            }}
                                          >
                                            <Search className="h-3 w-3" />
                                          </Button>
                                        </div>
                                      </div>
                                    ))}
                                        </div>
                                      </>
                                )}
                              </CardContent>
                            </Card>
                          )}

                          {/* Query sub-tab */}
                          {sessionDetailTab === "query" && selectedSession.context_management_enabled && (
                            <div className="space-y-4">
                              <Card>
                                <CardHeader className="pb-3">
                                  <CardTitle className="text-sm">Search Index</CardTitle>
                                  <CardDescription>
                                    Search the FTS5 index for this session using batch execute.
                                  </CardDescription>
                                </CardHeader>
                                <CardContent className="space-y-3">
                                  <div className="flex gap-2">
                                    <Input
                                      placeholder="Search query..."
                                      value={searchQuery}
                                      onChange={(e) => setSearchQuery(e.target.value)}
                                      onKeyDown={(e) => {
                                        if (e.key === "Enter") runSearch(selectedSession.session_id)
                                      }}
                                      className="flex-1"
                                    />
                                    <Button
                                      size="sm"
                                      onClick={() => runSearch(selectedSession.session_id)}
                                      disabled={searchLoading || !searchQuery.trim()}
                                    >
                                      {searchLoading ? (
                                        <Loader2 className="h-3.5 w-3.5 animate-spin mr-1" />
                                      ) : (
                                        <Search className="h-3.5 w-3.5 mr-1" />
                                      )}
                                      Search
                                    </Button>
                                  </div>
                                </CardContent>
                              </Card>

                              {searchResults !== null && (
                                <Card>
                                  <CardHeader className="pb-3">
                                    <CardTitle className="text-sm">Results</CardTitle>
                                  </CardHeader>
                                  <CardContent>
                                    <pre className="text-xs bg-muted/50 rounded-md p-3 max-h-[400px] overflow-y-auto whitespace-pre-wrap break-words font-mono">
                                      {searchResults}
                                    </pre>
                                  </CardContent>
                                </Card>
                              )}
                            </div>
                          )}

                      </div>
                    </ScrollArea>
                  </div>
                ) : (
                  <div className="flex flex-col items-center justify-center h-full text-muted-foreground gap-4">
                    <BookText className="h-12 w-12 opacity-30" />
                    <div className="text-center">
                      <p className="font-medium">Select a session to view details</p>
                    </div>
                  </div>
                )}
              </ResizablePanel>
            </ResizablePanelGroup>
          )}
        </TabsContent>

        {/* Settings Tab */}
        <TabsContent value="settings" className="flex-1 min-h-0 mt-4">
          {config && (
            <div className="space-y-4 max-w-2xl">
              {/* Catalog Threshold */}
              <CatalogThresholdSlider
                config={config}
                updateField={updateField}
              />

              {/* Tool Names */}
              <Card>
                <CardHeader>
                  <CardTitle className="text-base">Tool Names</CardTitle>
                  <CardDescription>
                    Configurable names for the injected context management tools.
                    Change to avoid clashing with client tools (e.g. Claude Code&apos;s Read).
                  </CardDescription>
                </CardHeader>
                <CardContent className="space-y-3">
                  <div className="flex gap-4 items-center">
                    <label className="text-sm text-muted-foreground w-24">Search tool:</label>
                    <Input
                      defaultValue={config.search_tool_name}
                      key={`search-name-${config.search_tool_name}`}
                      onBlur={(e) => {
                        const v = e.target.value.trim()
                        if (v && v !== config.search_tool_name) {
                          updateField("searchToolName", v)
                        }
                      }}
                      onKeyDown={(e) => {
                        if (e.key === "Enter") (e.target as HTMLInputElement).blur()
                      }}
                      className="w-48"
                    />
                  </div>
                  <div className="flex gap-4 items-center">
                    <label className="text-sm text-muted-foreground w-24">Read tool:</label>
                    <Input
                      defaultValue={config.read_tool_name}
                      key={`read-name-${config.read_tool_name}`}
                      onBlur={(e) => {
                        const v = e.target.value.trim()
                        if (v && v !== config.read_tool_name) {
                          updateField("readToolName", v)
                        }
                      }}
                      onKeyDown={(e) => {
                        if (e.key === "Enter") (e.target as HTMLInputElement).blur()
                      }}
                      className="w-48"
                    />
                  </div>
                  <p className="text-xs text-muted-foreground">
                    Changes apply to new sessions only. Active sessions keep the names they started with.
                  </p>
                </CardContent>
              </Card>
            </div>
          )}
        </TabsContent>

        {/* Preview Tab */}
        <TabsContent value="preview" className="flex-1 min-h-0 mt-4">
          <CompressionPreview
            initialThreshold={config?.catalog_threshold_bytes ?? 1000}
          />
        </TabsContent>
      </Tabs>
    </div>
  )
}

interface CatalogThresholdSliderProps {
  config: ContextManagementConfig
  updateField: (field: string, value: unknown) => Promise<void>
}

function CatalogThresholdSlider({ config, updateField }: CatalogThresholdSliderProps) {
  const [sliderValue, setSliderValue] = useState(config.catalog_threshold_bytes)

  // Sync slider when config changes externally
  useEffect(() => {
    setSliderValue(config.catalog_threshold_bytes)
  }, [config.catalog_threshold_bytes])

  // Debounced save when slider value changes
  useEffect(() => {
    if (sliderValue === config.catalog_threshold_bytes) return
    const timer = setTimeout(() => {
      updateField("catalogThresholdBytes", sliderValue)
    }, 300)
    return () => clearTimeout(timer)
  }, [sliderValue, config.catalog_threshold_bytes, updateField])

  return (
    <Card>
      <CardHeader>
        <CardTitle className="text-base">Catalog Compression Threshold</CardTitle>
        <CardDescription>
          When the total catalog size exceeds this threshold (in bytes), tool descriptions
          are progressively compressed and deferred to the search index.
        </CardDescription>
      </CardHeader>
      <CardContent className="space-y-4">
        <div className="space-y-2">
          <div className="flex items-center justify-between text-sm">
            <span className="text-muted-foreground">Threshold</span>
            <span className="font-mono">{sliderValue >= 102400 ? "No limit" : formatBytes(sliderValue)}</span>
          </div>
          <input
            type="range"
            value={sliderValue}
            onChange={(e) => setSliderValue(Number(e.target.value))}
            min={0}
            max={102400}
            step={100}
            className="w-full"
          />
          <div className="flex justify-between text-xs text-muted-foreground">
            <span>0 (max compression)</span>
            <span>No limit</span>
          </div>
        </div>
        {sliderValue !== DEFAULT_CATALOG_THRESHOLD_BYTES && (
          <Button
            variant="ghost"
            size="sm"
            onClick={() => setSliderValue(DEFAULT_CATALOG_THRESHOLD_BYTES)}
          >
            Reset to default ({formatBytes(DEFAULT_CATALOG_THRESHOLD_BYTES)})
          </Button>
        )}
      </CardContent>
    </Card>
  )
}

interface CompressionPreviewProps {
  initialThreshold: number
}

function CompressionPreview({ initialThreshold }: CompressionPreviewProps) {
  const [threshold, setThreshold] = useState(initialThreshold)
  const [source, setSource] = useState<string | null>(null)
  const [preview, setPreview] = useState<CatalogCompressionPreview | null>(null)
  const [loading, setLoading] = useState(false)
  const [clients, setClients] = useState<ClientInfo[]>([])
  const [error, setError] = useState<string | null>(null)

  // Load clients for the dropdown and set default source
  useEffect(() => {
    invoke<ClientInfo[]>("list_clients")
      .then((loaded) => {
        setClients(loaded)
        const firstEnabled = loaded.find((c) => c.enabled)
        setSource(firstEnabled ? `client:${firstEnabled.client_id}` : "mock")
      })
      .catch((e) => {
        console.error("Failed to load clients:", e)
        setSource("mock")
      })
  }, [])

  const fetchPreview = useCallback(async (bytes: number, src: string) => {
    setLoading(true)
    setError(null)
    try {
      const result = await invoke<CatalogCompressionPreview>("preview_catalog_compression", {
        catalogThresholdBytes: bytes,
        source: src,
      } satisfies PreviewCatalogCompressionParams)
      setPreview(result)
    } catch (e) {
      const msg = String(e)
      setError(msg)
      console.error("Failed to load compression preview:", e)
    } finally {
      setLoading(false)
    }
  }, [])

  // Debounced fetch on slider/source change (also serves as initial load once source is set)
  useEffect(() => {
    if (source === null) return
    const timer = setTimeout(() => {
      fetchPreview(threshold, source)
    }, 300)
    return () => clearTimeout(timer)
  }, [threshold, source, fetchPreview])

  return (
    <div className="space-y-4">
      {/* Controls */}
      <Card>
        <CardHeader>
          <CardTitle className="text-base">Compression Preview</CardTitle>
          <CardDescription>
            See how different thresholds affect the welcome message and tool catalog.
          </CardDescription>
        </CardHeader>
        <CardContent className="space-y-4">
          {/* Client dropdown */}
          <div className="space-y-1.5">
            <label className="text-sm text-muted-foreground">Client</label>
            <select
              value={source ?? ""}
              onChange={(e) => setSource(e.target.value)}
              className="w-full h-9 rounded-md border border-input bg-background px-3 text-sm ring-offset-background focus:outline-none focus:ring-2 focus:ring-ring focus:ring-offset-2"
            >
              {clients.filter((c) => c.enabled).map((c) => (
                <option key={c.id} value={`client:${c.client_id}`}>
                  {c.name}
                </option>
              ))}
              <option value="mock">Example Client</option>
            </select>
          </div>

          {/* Slider */}
          <div className="space-y-2">
            <div className="flex items-center justify-between text-sm">
              <span className="text-muted-foreground">Threshold</span>
              <span className="font-mono">{threshold >= 102400 ? "No limit" : formatBytes(threshold)}</span>
            </div>
            <input
              type="range"
              value={threshold}
              onChange={(e) => setThreshold(Number(e.target.value))}
              min={0}
              max={102400}
              step={100}
              className="w-full"
            />
            <div className="flex justify-between text-xs text-muted-foreground">
              <span>0 (max compression)</span>
              <span>No limit</span>
            </div>
          </div>

          {/* Stats bar */}
          {preview && (
            <div className="flex flex-wrap gap-4 text-sm">
              <div className="flex items-center gap-1.5">
                <span className="text-muted-foreground">Before:</span>
                <span className="font-mono">{formatBytes(preview.uncompressed_size + preview.tool_definitions_size)}</span>
              </div>
              <div className="flex items-center gap-1.5">
                <span className="text-muted-foreground">After:</span>
                <span className="font-mono">{formatBytes(preview.compressed_size + preview.compressed_tool_definitions_size)}</span>
              </div>
            </div>
          )}

          {error && (
            <div className="flex items-center gap-2 text-sm text-destructive">
              <AlertTriangle className="h-3.5 w-3.5 shrink-0" />
              <span>{error}</span>
            </div>
          )}

          {loading && (
            <div className="flex items-center gap-2 text-sm text-muted-foreground">
              <Loader2 className="h-3 w-3 animate-spin" />
              Computing...
            </div>
          )}
        </CardContent>
      </Card>

      {/* Side-by-side welcome message */}
      {preview && (
        <ResizablePanelGroup direction="horizontal" className="rounded-md border min-h-[400px]">
          <ResizablePanel defaultSize={50} minSize={20}>
            <div className="h-full flex flex-col">
              <div className="px-3 py-2 border-b bg-muted/50 text-xs font-medium text-muted-foreground flex items-center justify-between">
                <span>Uncompressed</span>
                <span className="font-mono">{formatBytes(preview.uncompressed_size)}</span>
              </div>
              <ScrollArea className="flex-1 p-3">
                <pre className="text-xs whitespace-pre-wrap font-mono leading-relaxed">{preview.welcome_message_uncompressed}</pre>
              </ScrollArea>
            </div>
          </ResizablePanel>
          <ResizableHandle withHandle />
          <ResizablePanel defaultSize={50} minSize={20}>
            <div className="h-full flex flex-col">
              <div className="px-3 py-2 border-b bg-muted/50 text-xs font-medium text-muted-foreground flex items-center justify-between">
                <span>Compressed</span>
                <span className="font-mono">{formatBytes(preview.compressed_size)}</span>
              </div>
              <ScrollArea className="flex-1 p-3">
                <pre className="text-xs whitespace-pre-wrap font-mono leading-relaxed">{preview.welcome_message}</pre>
              </ScrollArea>
            </div>
          </ResizablePanel>
        </ResizablePanelGroup>
      )}

      {/* Tools / Resources / Prompts -- side-by-side detail */}
      {preview && preview.servers.length > 0 && (
        <ResizablePanelGroup direction="horizontal" className="rounded-md border min-h-[300px]">
          <ResizablePanel defaultSize={50} minSize={20}>
            <div className="h-full flex flex-col">
              <div className="px-3 py-2 border-b bg-muted/50 text-xs font-medium text-muted-foreground flex items-center justify-between">
                <span>Uncompressed</span>
                <span className="font-mono">{formatBytes(preview.tool_definitions_size)}</span>
              </div>
              <ScrollArea className="flex-1">
                <div className="p-3 space-y-4">
                  {preview.servers.map((server) => (
                    <ServerCatalogBlock key={server.name} server={server} mode="full" />
                  ))}
                </div>
              </ScrollArea>
            </div>
          </ResizablePanel>
          <ResizableHandle withHandle />
          <ResizablePanel defaultSize={50} minSize={20}>
            <div className="h-full flex flex-col">
              <div className="px-3 py-2 border-b bg-muted/50 text-xs font-medium text-muted-foreground flex items-center justify-between">
                <span>Compressed</span>
                <span className="font-mono">{formatBytes(preview.compressed_tool_definitions_size)}</span>
              </div>
              <ScrollArea className="flex-1">
                <div className="p-3 space-y-4">
                  {preview.servers.map((server) => (
                    <ServerCatalogBlock key={server.name} server={server} mode="compressed" />
                  ))}
                </div>
              </ScrollArea>
            </div>
          </ResizablePanel>
        </ResizablePanelGroup>
      )}
    </div>
  )
}

/** Renders a single server's catalog block for the compression preview. */
function ServerCatalogBlock({ server, mode }: { server: PreviewServerEntry; mode: "full" | "compressed" }) {
  const isDeferred = server.compression_state === "deferred"
  const isTruncated = server.compression_state === "truncated"

  // In compressed mode, deferred servers show a summary instead of full tool list
  if (mode === "compressed" && isDeferred) {
    return (
      <div className="opacity-50">
        <div className="flex items-center gap-2 mb-1">
          <span className="text-xs font-semibold">{server.name}</span>
          <Badge variant="outline" className="text-[10px] px-1 py-0">deferred</Badge>
        </div>
        <p className="text-[10px] text-muted-foreground italic">
          {server.tool_names.length} tools, {server.resource_names.length} resources, {server.prompt_names.length} prompts — discoverable via search index
        </p>
      </div>
    )
  }

  // In compressed mode, truncated servers show only counts
  if (mode === "compressed" && isTruncated) {
    return (
      <div className="opacity-50">
        <div className="flex items-center gap-2 mb-1">
          <span className="text-xs font-semibold">{server.name}</span>
          <Badge variant="outline" className="text-[10px] px-1 py-0 text-red-600 border-red-600/30">truncated</Badge>
        </div>
        <p className="text-[10px] text-muted-foreground italic">
          {server.tool_names.length} tools, {server.resource_names.length} resources, {server.prompt_names.length} prompts — ctx_search to explore
        </p>
      </div>
    )
  }

  const tools = server.tools ?? []
  const resources = server.resources ?? []
  const prompts = server.prompts ?? []
  const totalItems = server.tool_names.length + server.resource_names.length + server.prompt_names.length

  // Build unified list of all items for McpToolDisplay
  const allItems: McpToolDisplayItem[] = [
    // Tools with full details
    ...tools.map((t): McpToolDisplayItem => ({
      name: t.name,
      description: t.description,
      inputSchema: t.input_schema as Record<string, unknown> | null,
      itemType: "tool",
    })),
    // Tools with names only (no full details available)
    ...server.tool_names
      .filter((name) => !tools.some((t) => t.name === name))
      .map((name): McpToolDisplayItem => ({ name, itemType: "tool" })),
    // Resources with full details
    ...resources.map((res): McpToolDisplayItem => ({
      name: res.name,
      description: res.description,
      itemType: "resource",
    })),
    // Resources with names only
    ...server.resource_names
      .filter((name) => !resources.some((r) => r.name === name))
      .map((name): McpToolDisplayItem => ({ name, itemType: "resource" })),
    // Prompts with full details
    ...prompts.map((p): McpToolDisplayItem => ({
      name: p.name,
      description: p.description,
      itemType: "prompt",
    })),
    // Prompts with names only
    ...server.prompt_names
      .filter((name) => !prompts.some((p) => p.name === name))
      .map((name): McpToolDisplayItem => ({ name, itemType: "prompt" })),
  ]

  return (
    <div>
      <div className="flex items-center gap-2 mb-1">
        <span className="text-xs font-semibold">{server.name}</span>
        <span className="text-[10px] text-muted-foreground">{totalItems} items</span>
      </div>

      <div className="ml-2">
        <McpToolDisplay tools={allItems} compact />
      </div>
    </div>
  )
}

function formatDuration(secs: number): string {
  if (secs < 60) return `${secs}s`
  const mins = Math.floor(secs / 60)
  if (mins < 60) return `${mins}m`
  const hrs = Math.floor(mins / 60)
  const remainMins = mins % 60
  return `${hrs}h ${remainMins}m`
}

function formatBytes(bytes: number): string {
  if (bytes === 0) return "0 B"
  if (bytes < 1024) return `${bytes} B`
  const kb = bytes / 1024
  if (kb < 1024) return `${kb.toFixed(kb < 10 ? 1 : 0)} KB`
  const mb = kb / 1024
  return `${mb.toFixed(1)} MB`
}
