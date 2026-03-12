import { useState, useEffect, useCallback } from "react"
import { invoke } from "@tauri-apps/api/core"
import { toast } from "sonner"
import { BookText, Info, RefreshCw, Loader2, Search, Database, BarChart3, AlertTriangle } from "lucide-react"
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
import { cn } from "@/lib/utils"
import Markdown from "react-markdown"
import remarkGfm from "remark-gfm"
import { ToolList } from "@/components/shared/ToolList"
import type { ToolListItem } from "@/components/shared/ToolList"
import { GatewayIndexingTree } from "@/components/permissions/GatewayIndexingTree"
import { IndexingStateButton } from "@/components/permissions/IndexingStateButton"
import type { ContextManagementConfig, ActiveSessionInfo, CatalogSourceEntry, CatalogCompressionPreview, PreviewCatalogCompressionParams, PreviewServerEntry, ClientInfo, IndexingState } from "@/types/tauri-commands"

// Must match defaults in crates/lr-config/src/types.rs
const DEFAULT_CATALOG_THRESHOLD_BYTES = 1000
const DEFAULT_RESPONSE_THRESHOLD_BYTES = 200


interface ContextManagementViewProps {
  activeSubTab?: string | null
  onTabChange?: (view: string, subTab?: string | null) => void
}

export function ContextManagementView({ activeSubTab, onTabChange }: ContextManagementViewProps) {
  const [config, setConfig] = useState<ContextManagementConfig | null>(null)
  const [sessions, setSessions] = useState<ActiveSessionInfo[]>([])
  const [selectedSessionId, setSelectedSessionId] = useState<string | null>(null)
  const [, setSaving] = useState(false)
  const [sessionDetailTab, setSessionDetailTab] = useState<string>("info")
  const [catalogSources, setCatalogSources] = useState<CatalogSourceEntry[]>([])
  const [catalogSourcesLoading, setCatalogSourcesLoading] = useState(false)
  const [contextStats, setContextStats] = useState<string | null>(null)
  const [contextStatsLoading, setContextStatsLoading] = useState(false)
  const [contextStatsLoaded, setContextStatsLoaded] = useState<string | null>(null)
  const [searchQuery, setSearchQuery] = useState("")
  const [searchResults, setSearchResults] = useState<string | null>(null)
  const [searchLoading, setSearchLoading] = useState(false)

  const tab = activeSubTab || "info"

  const handleTabChange = (newTab: string) => {
    onTabChange?.("context-management", newTab)
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

  const loadContextStats = useCallback(async (sessionId: string) => {
    setContextStatsLoading(true)
    try {
      const result = await invoke<{ content?: Array<{ text?: string }> }>("get_session_context_stats", { sessionId })
      const text = result?.content?.map((c) => c.text || "").join("\n") || "No stats available"
      setContextStats(text)
      setContextStatsLoaded(sessionId)
    } catch (err) {
      setContextStats(`Error: ${err}`)
    } finally {
      setContextStatsLoading(false)
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
    } else if (sessionDetailTab === "stats" && contextStatsLoaded !== selectedSessionId) {
      loadContextStats(selectedSessionId)
    }
  }, [selectedSessionId, sessionDetailTab, sessions, loadCatalogSources, loadContextStats, contextStatsLoaded])

  // Reset detail tab when session changes
  useEffect(() => {
    setSessionDetailTab("info")
    setCatalogSources([])
    setContextStats(null)
    setContextStatsLoaded(null)
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
            <BookText className="h-6 w-6" />
            Context Management
          </h1>
          <Badge variant="outline" className="bg-purple-500/10 text-purple-900 dark:text-purple-400">EXPERIMENTAL</Badge>
        </div>
        <p className="text-sm text-muted-foreground">
          Compress MCP catalogs and tool responses using FTS5 search indexing
        </p>
      </div>

      <Tabs
        value={tab}
        onValueChange={handleTabChange}
        className="flex flex-col flex-1 min-h-0"
      >
        <TabsList className="flex-shrink-0 w-fit">
          <TabsTrigger value="info">Info</TabsTrigger>
          <TabsTrigger value="preview">Preview</TabsTrigger>
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
            {/* Enable Context Management */}
            {config && (
              <Card>
                <CardHeader>
                  <div className="flex items-center justify-between">
                    <CardTitle className="text-base">Default: Enable Catalog Compression</CardTitle>
                  </div>
                  <CardDescription>
                    Uses deferred loading of tools, prompts, and resources combined with
                    FTS5 search indexing of welcome messages and tool descriptions. When catalogs exceed the
                    configured threshold, capabilities are hidden and a search
                    tool lets the AI discover and unhide them on demand.
                  </CardDescription>
                  <p className="text-xs text-muted-foreground mt-1">
                    Clients can override this setting individually in their Context tab.
                    Requires client support for{" "}
                    <code className="px-1 py-0.5 rounded bg-muted text-xs">tools/listChanged</code> notifications.
                  </p>
                </CardHeader>
                <CardContent>
                  <p className="text-xs text-muted-foreground mb-1.5">Exposed tools:</p>
                  <div className="flex flex-wrap gap-1.5">
                    <code className="text-[11px] px-1.5 py-0.5 rounded bg-muted">{config.search_tool_name}</code>
                    <code className="text-[11px] px-1.5 py-0.5 rounded bg-muted">{config.read_tool_name}</code>
                  </div>
                </CardContent>
              </Card>
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
                              <TabsTrigger value="stats">
                                <BarChart3 className="h-3.5 w-3.5 mr-1" />
                                Stats
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
                                    <div className="prose prose-sm dark:prose-invert max-w-none bg-muted/50 rounded-md p-3 max-h-[400px] overflow-y-auto [&_pre]:bg-muted [&_pre]:p-2 [&_pre]:rounded [&_code]:text-xs [&_p]:my-1 [&_h1]:text-base [&_h2]:text-sm [&_h3]:text-sm [&_ul]:my-1 [&_ol]:my-1 [&_li]:my-0">
                                      <Markdown remarkPlugins={[remarkGfm]}>{searchResults}</Markdown>
                                    </div>
                                  </CardContent>
                                </Card>
                              )}
                            </div>
                          )}

                          {/* Stats sub-tab */}
                          {sessionDetailTab === "stats" && selectedSession.context_management_enabled && (
                            <Card>
                              <CardHeader className="pb-3">
                                <div className="flex items-center justify-between">
                                  <CardTitle className="text-sm">Context-Mode Stats</CardTitle>
                                  <Button
                                    variant="ghost"
                                    size="sm"
                                    onClick={() => loadContextStats(selectedSession.session_id)}
                                    className="h-7 w-7 p-0"
                                    disabled={contextStatsLoading}
                                  >
                                    {contextStatsLoading ? (
                                      <Loader2 className="h-3.5 w-3.5 animate-spin" />
                                    ) : (
                                      <RefreshCw className="h-3.5 w-3.5" />
                                    )}
                                  </Button>
                                </div>
                                <CardDescription>
                                  Each refresh calls ctx_stats which counts towards the session stats.
                                </CardDescription>
                              </CardHeader>
                              <CardContent>
                                {contextStatsLoading && !contextStats ? (
                                  <div className="flex items-center gap-2 text-sm text-muted-foreground">
                                    <Loader2 className="h-4 w-4 animate-spin" />
                                    Loading stats...
                                  </div>
                                ) : contextStats ? (
                                  <div className="prose prose-sm dark:prose-invert max-w-none bg-muted/50 rounded-md p-3 [&_pre]:bg-muted [&_pre]:p-2 [&_pre]:rounded [&_code]:text-xs [&_p]:my-1 [&_h1]:text-base [&_h2]:text-sm [&_h3]:text-sm [&_ul]:my-1 [&_ol]:my-1 [&_li]:my-0">
                                    <Markdown remarkPlugins={[remarkGfm]}>{contextStats}</Markdown>
                                  </div>
                                ) : (
                                  <p className="text-sm text-muted-foreground">No stats available.</p>
                                )}
                              </CardContent>
                            </Card>
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
              <Card>
                <CardHeader>
                  <CardTitle className="text-base">Catalog Compression Threshold</CardTitle>
                  <CardDescription>
                    When the total catalog size exceeds this threshold (in bytes), tool descriptions
                    are progressively compressed and deferred to the search index.
                  </CardDescription>
                </CardHeader>
                <CardContent>
                  <div className="flex gap-2 items-center">
                    <Input
                      type="number"
                      defaultValue={config.catalog_threshold_bytes}
                      key={`catalog-${config.catalog_threshold_bytes}`}
                      onBlur={(e) => {
                        const v = parseInt(e.target.value)
                        if (!isNaN(v) && v >= 0 && v !== config.catalog_threshold_bytes) {
                          updateField("catalogThresholdBytes", v)
                        }
                      }}
                      onKeyDown={(e) => {
                        if (e.key === "Enter") {
                          (e.target as HTMLInputElement).blur()
                        }
                      }}
                      className="w-40"
                      min={0}
                    />
                    <span className="text-sm text-muted-foreground">bytes</span>
                    {config.catalog_threshold_bytes !== DEFAULT_CATALOG_THRESHOLD_BYTES && (
                      <Button
                        variant="ghost"
                        size="sm"
                        onClick={() => updateField("catalogThresholdBytes", DEFAULT_CATALOG_THRESHOLD_BYTES)}
                      >
                        Reset to default
                      </Button>
                    )}
                  </div>
                  <p className="text-xs text-muted-foreground mt-2">
                    Default: {DEFAULT_CATALOG_THRESHOLD_BYTES.toLocaleString()} bytes. Lower values compress more aggressively. Set to 0 to always compress.
                  </p>
                </CardContent>
              </Card>

              {/* Response Threshold */}
              <Card>
                <CardHeader>
                  <CardTitle className="text-base">Response Compression Threshold</CardTitle>
                  <CardDescription>
                    Tool responses larger than this threshold are indexed and replaced with a
                    truncated preview and a search hint.
                  </CardDescription>
                </CardHeader>
                <CardContent>
                  <div className="flex gap-2 items-center">
                    <Input
                      type="number"
                      defaultValue={config.response_threshold_bytes}
                      key={`response-${config.response_threshold_bytes}`}
                      onBlur={(e) => {
                        const v = parseInt(e.target.value)
                        if (!isNaN(v) && v >= 0 && v !== config.response_threshold_bytes) {
                          updateField("responseThresholdBytes", v)
                        }
                      }}
                      onKeyDown={(e) => {
                        if (e.key === "Enter") {
                          (e.target as HTMLInputElement).blur()
                        }
                      }}
                      className="w-40"
                      min={0}
                    />
                    <span className="text-sm text-muted-foreground">bytes</span>
                    {config.response_threshold_bytes !== DEFAULT_RESPONSE_THRESHOLD_BYTES && (
                      <Button
                        variant="ghost"
                        size="sm"
                        onClick={() => updateField("responseThresholdBytes", DEFAULT_RESPONSE_THRESHOLD_BYTES)}
                      >
                        Reset to default
                      </Button>
                    )}
                  </div>
                  <p className="text-xs text-muted-foreground mt-2">
                    Default: {DEFAULT_RESPONSE_THRESHOLD_BYTES.toLocaleString()} bytes. Set higher to compress fewer responses. Set to 0 to always compress.
                  </p>
                </CardContent>
              </Card>

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

              {/* Gateway Indexing Picker */}
              <Card>
                <CardHeader>
                  <CardTitle className="text-base">Gateway Tool Indexing</CardTitle>
                  <CardDescription>
                    Control which gateway tools get their responses indexed into FTS5.
                    Applies to all clients in any mode.
                  </CardDescription>
                </CardHeader>
                <CardContent>
                  <GatewayIndexingTree
                    permissions={config.gateway_indexing}
                    onUpdate={async () => {
                      const updated = await invoke<ContextManagementConfig>("get_context_management_config")
                      setConfig(updated)
                    }}
                  />
                </CardContent>
              </Card>

              {/* Client Tools Indexing Default */}
              <Card>
                <CardHeader>
                  <div className="flex items-center justify-between">
                    <div>
                      <CardTitle className="text-base">Client Tools Indexing</CardTitle>
                      <CardDescription className="mt-1.5">
                        Default for client tool response indexing across all MCP via LLM clients.
                        Per-client overrides in client settings.
                      </CardDescription>
                    </div>
                    <IndexingStateButton
                      value={config.client_tools_indexing_default}
                      onChange={(state: IndexingState) => updateField("clientToolsIndexingDefault", state)}
                    />
                  </div>
                </CardHeader>
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

  const savings = preview
    ? Math.round((1 - preview.compressed_size / preview.uncompressed_size) * 100)
    : 0

  return (
    <div className="space-y-4">
      {/* Controls */}
      <Card>
        <CardHeader>
          <CardTitle className="text-base">Compression Preview</CardTitle>
          <CardDescription>
            See how different thresholds affect the welcome message and tool catalog.
            Select mock servers or a connected client.
          </CardDescription>
        </CardHeader>
        <CardContent className="space-y-4">
          {/* Source dropdown */}
          <div className="space-y-1.5">
            <label className="text-sm text-muted-foreground">Source</label>
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
              <option value="mock">Mock Servers (GitHub, Atlassian, Filesystem, PostgreSQL, Slack)</option>
            </select>
          </div>

          {/* Slider */}
          <div className="space-y-2">
            <div className="flex items-center justify-between text-sm">
              <span className="text-muted-foreground">Try it out</span>
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
            <div className="flex flex-wrap gap-3 text-sm">
              <div className="flex items-center gap-1.5">
                <span className="text-muted-foreground">Welcome:</span>
                <span className="font-mono">{formatBytes(preview.welcome_size)}</span>
              </div>
              <div className="flex items-center gap-1.5">
                <span className="text-muted-foreground">Tool defs:</span>
                <span className="font-mono">{formatBytes(preview.tool_definitions_size)}</span>
              </div>
              <div className="flex items-center gap-1.5">
                <span className="text-muted-foreground">Total:</span>
                <span className="font-mono">{formatBytes(preview.uncompressed_size)}</span>
              </div>
              <div className="flex items-center gap-1.5">
                <span className="text-muted-foreground">Compressed:</span>
                <span className="font-mono">{formatBytes(preview.compressed_size)}</span>
              </div>
              {savings > 0 && (
                <Badge variant="outline" className="text-green-600 border-green-600/30">
                  {savings}% saved
                </Badge>
              )}
              {preview.indexed_welcomes_count > 0 && (
                <Badge variant="outline" className="text-yellow-600 border-yellow-600/30">
                  P1: {preview.indexed_welcomes_count} indexed
                </Badge>
              )}
              {preview.deferred_servers_count > 0 && (
                <Badge variant="outline" className="text-orange-600 border-orange-600/30">
                  P2: {preview.deferred_servers_count} deferred
                </Badge>
              )}
              {preview.welcome_toc_dropped_count > 0 && (
                <Badge variant="outline" className="text-red-600 border-red-600/30">
                  P3: {preview.welcome_toc_dropped_count} TOC dropped
                </Badge>
              )}
              {preview.batch_toc_dropped_count > 0 && (
                <Badge variant="outline" className="text-red-600 border-red-600/30">
                  P4: {preview.batch_toc_dropped_count} batch TOC dropped
                </Badge>
              )}
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

      {/* Tools / Resources / Prompts — side-by-side detail */}
      {preview && preview.servers.length > 0 && (
        <Card>
          <CardHeader className="pb-3">
            <CardTitle className="text-base">Tools / Resources / Prompts</CardTitle>
            <CardDescription>
              Left: full catalog. Right: after compression (compressed descriptions shortened, deferred items omitted).
            </CardDescription>
          </CardHeader>
          <CardContent>
            <ResizablePanelGroup direction="horizontal" className="rounded-md border min-h-[300px]">
              {/* Left: full catalog */}
              <ResizablePanel defaultSize={50} minSize={20}>
                <div className="h-full flex flex-col">
                  <div className="px-3 py-2 border-b bg-muted/50 text-xs font-medium text-muted-foreground">
                    Full Catalog
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
              {/* Right: compressed catalog */}
              <ResizablePanel defaultSize={50} minSize={20}>
                <div className="h-full flex flex-col">
                  <div className="px-3 py-2 border-b bg-muted/50 text-xs font-medium text-muted-foreground">
                    After Compression
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
          </CardContent>
        </Card>
      )}
    </div>
  )
}

/** Renders a single server's catalog block for the compression preview. */
function ServerCatalogBlock({ server, mode }: { server: PreviewServerEntry; mode: "full" | "compressed" }) {
  const isDeferred = server.compression_state === "deferred"
  const isTruncated = server.compression_state === "truncated"
  const isCompressed = server.compression_state === "compressed"

  // In compressed mode, deferred servers are completely omitted
  if (mode === "compressed" && isDeferred) {
    return null
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

  // Build unified list of all items for ToolList
  const allItems: ToolListItem[] = [
    // Tools with full details
    ...tools.map((t): ToolListItem => ({
      name: t.name,
      description: t.description,
      inputSchema: t.input_schema as Record<string, unknown> | null,
      itemType: "tool",
    })),
    // Tools with names only (no full details available)
    ...server.tool_names
      .filter((name) => !tools.some((t) => t.name === name))
      .map((name): ToolListItem => ({ name, itemType: "tool" })),
    // Resources with full details
    ...resources.map((res): ToolListItem => ({
      name: res.name,
      description: res.description,
      itemType: "resource",
    })),
    // Resources with names only
    ...server.resource_names
      .filter((name) => !resources.some((r) => r.name === name))
      .map((name): ToolListItem => ({ name, itemType: "resource" })),
    // Prompts with full details
    ...prompts.map((p): ToolListItem => ({
      name: p.name,
      description: p.description,
      itemType: "prompt",
    })),
    // Prompts with names only
    ...server.prompt_names
      .filter((name) => !prompts.some((p) => p.name === name))
      .map((name): ToolListItem => ({ name, itemType: "prompt" })),
  ]

  return (
    <div>
      <div className="flex items-center gap-2 mb-1">
        <span className="text-xs font-semibold">{server.name}</span>
        {mode === "compressed" && isCompressed && (
          <Badge variant="outline" className="text-[10px] px-1 py-0 text-yellow-600 border-yellow-600/30">compressed</Badge>
        )}
        <span className="text-[10px] text-muted-foreground">{totalItems} items</span>
      </div>

      <div className="ml-2">
        <ToolList tools={allItems} compact />
      </div>

      {mode === "compressed" && isCompressed && (
        <div className="ml-2 mt-1.5 text-[10px] text-yellow-600/80 italic border-l-2 border-yellow-600/20 pl-2">
          [compressed] ctx_search(source=&quot;catalog:{server.name.toLowerCase().replace(/ /g, "-")}&quot;) to view full details
        </div>
      )}
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
