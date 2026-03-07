import { useState, useEffect, useCallback } from "react"
import { invoke } from "@tauri-apps/api/core"
import { toast } from "sonner"
import { BookText, Info, RefreshCw, CheckCircle2, XCircle, Loader2, Download } from "lucide-react"
import { Badge } from "@/components/ui/Badge"
import { Button } from "@/components/ui/Button"
import { Card, CardContent, CardHeader, CardTitle, CardDescription } from "@/components/ui/Card"
import { Switch } from "@/components/ui/Toggle"
import { Input } from "@/components/ui/Input"
import { Tabs, TabsContent, TabsList, TabsTrigger } from "@/components/ui/tabs"
import { ScrollArea } from "@/components/ui/scroll-area"
import {
  ResizablePanelGroup,
  ResizablePanel,
  ResizableHandle,
} from "@/components/ui/resizable"
import { cn } from "@/lib/utils"
import type { ContextManagementConfig, ActiveSessionInfo, ContextModeInfo } from "@/types/tauri-commands"

interface ContextManagementViewProps {
  activeSubTab?: string | null
  onTabChange?: (view: string, subTab?: string | null) => void
}

export function ContextManagementView({ activeSubTab, onTabChange }: ContextManagementViewProps) {
  const [config, setConfig] = useState<ContextManagementConfig | null>(null)
  const [sessions, setSessions] = useState<ActiveSessionInfo[]>([])
  const [selectedSessionId, setSelectedSessionId] = useState<string | null>(null)
  const [saving, setSaving] = useState(false)
  const [modeInfo, setModeInfo] = useState<ContextModeInfo | null>(null)
  const [modeInfoLoading, setModeInfoLoading] = useState(true)
  const [installing, setInstalling] = useState(false)

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

  const loadModeInfo = useCallback(async () => {
    setModeInfoLoading(true)
    try {
      const info = await invoke<ContextModeInfo>("get_context_mode_info")
      setModeInfo(info)
    } catch (err) {
      console.error("Failed to load context-mode info:", err)
    } finally {
      setModeInfoLoading(false)
    }
  }, [])

  useEffect(() => {
    invoke<ContextManagementConfig>("get_context_management_config")
      .then(setConfig)
      .catch((err) => console.error("Failed to load context management config:", err))
    loadSessions()
    loadModeInfo()
    const interval = setInterval(loadSessions, 5000)
    return () => clearInterval(interval)
  }, [loadSessions, loadModeInfo])

  // Clear selected session if it disappears
  useEffect(() => {
    if (selectedSessionId) {
      const stillExists = sessions.some((s) => s.client_id === selectedSessionId)
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
    ? sessions.find((s) => s.client_id === selectedSessionId) ?? null
    : null

  return (
    <div className="flex flex-col h-full min-h-0">
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
            {/* How it works */}
            <div className="p-4 rounded-lg bg-blue-500/10 border border-blue-600/50">
              <div className="flex items-start gap-3">
                <Info className="h-5 w-5 text-blue-600 dark:text-blue-400 mt-0.5 shrink-0" />
                <div className="space-y-2">
                  <p className="text-sm font-medium text-blue-900 dark:text-blue-300">
                    How it works
                  </p>
                  <p className="text-sm text-blue-900 dark:text-blue-400">
                    Context management uses FTS5 full-text search to intelligently compress MCP
                    catalogs and tool responses. Large catalogs are indexed and replaced with a
                    compact summary, and a search tool lets clients discover capabilities on demand.
                    Tool responses exceeding the threshold are indexed and replaced with a preview.
                  </p>
                  <p className="text-sm text-blue-900 dark:text-blue-400">
                    Clients can override this setting individually in their MCP tab.
                    Requires client support for{" "}
                    <code className="px-1 py-0.5 rounded bg-blue-500/20 text-xs">tools/listChanged</code>.
                  </p>
                </div>
              </div>
            </div>

            {/* Installation Status */}
            <Card>
              <CardHeader className="pb-3">
                <div className="flex items-center justify-between">
                  <CardTitle className="text-base">Installation Status</CardTitle>
                  <Button
                    variant="ghost"
                    size="sm"
                    onClick={loadModeInfo}
                    className="h-7 w-7 p-0"
                    disabled={modeInfoLoading}
                  >
                    {modeInfoLoading ? (
                      <Loader2 className="h-3.5 w-3.5 animate-spin" />
                    ) : (
                      <RefreshCw className="h-3.5 w-3.5" />
                    )}
                  </Button>
                </div>
                <CardDescription>
                  Context-mode runs via <code className="px-1 py-0.5 rounded bg-muted text-xs">npx context-mode</code> as
                  a per-session STDIO process.
                </CardDescription>
              </CardHeader>
              <CardContent className="space-y-3">
                {modeInfoLoading && !modeInfo ? (
                  <div className="flex items-center gap-2 text-sm text-muted-foreground">
                    <Loader2 className="h-4 w-4 animate-spin" />
                    Checking installation...
                  </div>
                ) : modeInfo ? (
                  <div className="space-y-3">
                    {/* npx */}
                    <div className={cn("flex items-center gap-2.5", !modeInfo.npxAvailable && "opacity-45")}>
                      {modeInfo.npxAvailable ? (
                        <CheckCircle2 className="h-4 w-4 text-green-600 dark:text-green-400 shrink-0" />
                      ) : (
                        <XCircle className="h-4 w-4 text-destructive shrink-0" />
                      )}
                      <div className="flex-1 min-w-0">
                        <div className="flex items-center gap-2">
                          <p className="text-sm font-medium">npx</p>
                          {modeInfo.npxAvailable ? (
                            <Badge variant="success" className="text-[10px] px-1 py-0">available</Badge>
                          ) : (
                            <Badge variant="destructive" className="text-[10px] px-1 py-0">not found</Badge>
                          )}
                        </div>
                        {modeInfo.npxPath && (
                          <p className="text-xs text-muted-foreground truncate">
                            {modeInfo.npxPath}
                            {modeInfo.npxVersion && ` (v${modeInfo.npxVersion})`}
                          </p>
                        )}
                        {!modeInfo.npxAvailable && (
                          <p className="text-xs text-muted-foreground">
                            Install Node.js to get npx, which is required for context-mode.
                          </p>
                        )}
                      </div>
                    </div>

                    {/* context-mode */}
                    <div className={cn("flex items-center gap-2.5", !modeInfo.contextModeVersion && "opacity-45")}>
                      {modeInfo.contextModeVersion ? (
                        <CheckCircle2 className="h-4 w-4 text-green-600 dark:text-green-400 shrink-0" />
                      ) : (
                        <XCircle className="h-4 w-4 text-muted-foreground shrink-0" />
                      )}
                      <div className="flex-1 min-w-0">
                        <div className="flex items-center gap-2">
                          <p className="text-sm font-medium">context-mode</p>
                          {modeInfo.contextModeVersion ? (
                            <Badge variant="success" className="text-[10px] px-1 py-0">
                              v{modeInfo.contextModeVersion}
                            </Badge>
                          ) : (
                            <Badge variant="secondary" className="text-[10px] px-1 py-0">not installed</Badge>
                          )}
                        </div>
                        <p className="text-xs text-muted-foreground">
                          {modeInfo.contextModeVersion
                            ? "Installed and ready. Will be spawned per-session when enabled."
                            : modeInfo.npxAvailable
                              ? "Not yet installed. Install now or it will be auto-installed on first use."
                              : "Requires npx to be available."}
                        </p>
                      </div>
                      {!modeInfo.contextModeVersion && modeInfo.npxAvailable && (
                        <Button
                          variant="outline"
                          size="sm"
                          className="shrink-0 ml-2"
                          disabled={installing}
                          onClick={async () => {
                            setInstalling(true)
                            try {
                              const version = await invoke<string>("install_context_mode")
                              toast.success(`context-mode v${version} installed`)
                              await loadModeInfo()
                            } catch (err) {
                              toast.error(`Install failed: ${err}`)
                            } finally {
                              setInstalling(false)
                            }
                          }}
                        >
                          {installing ? (
                            <Loader2 className="h-3.5 w-3.5 animate-spin mr-1.5" />
                          ) : (
                            <Download className="h-3.5 w-3.5 mr-1.5" />
                          )}
                          {installing ? "Installing..." : "Install"}
                        </Button>
                      )}
                    </div>
                  </div>
                ) : null}
              </CardContent>
            </Card>

            {/* Global Status */}
            {config && (
              <Card>
                <CardHeader className="pb-3">
                  <CardTitle className="text-base">Global Status</CardTitle>
                </CardHeader>
                <CardContent>
                  <div className="grid grid-cols-2 gap-3 text-sm">
                    <div>
                      <span className="text-muted-foreground">Context Management:</span>{" "}
                      <Badge variant={config.enabled ? "success" : "secondary"} className="text-[10px]">
                        {config.enabled ? "Enabled" : "Disabled"}
                      </Badge>
                    </div>
                    <div>
                      <span className="text-muted-foreground">Indexing Tools:</span>{" "}
                      <Badge variant={config.indexing_tools ? "success" : "secondary"} className="text-[10px]">
                        {config.indexing_tools ? "Enabled" : "Disabled"}
                      </Badge>
                    </div>
                    <div>
                      <span className="text-muted-foreground">Catalog Threshold:</span>{" "}
                      <span className="font-medium">{config.catalog_threshold_bytes.toLocaleString()} bytes</span>
                    </div>
                    <div>
                      <span className="text-muted-foreground">Response Threshold:</span>{" "}
                      <span className="font-medium">{config.response_threshold_bytes.toLocaleString()} bytes</span>
                    </div>
                    <div>
                      <span className="text-muted-foreground">Active Sessions:</span>{" "}
                      <span className="font-medium">
                        {sessions.filter((s) => s.context_management_enabled).length} / {sessions.length}
                        {sessions.length > 0 && (
                          <span className="text-muted-foreground font-normal"> with CM</span>
                        )}
                      </span>
                    </div>
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
              <ResizablePanel defaultSize={35} minSize={25}>
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
                          key={s.client_id}
                          onClick={() => setSelectedSessionId(s.client_id)}
                          className={cn(
                            "flex flex-col gap-1 p-3 rounded-md cursor-pointer",
                            selectedSessionId === s.client_id
                              ? "bg-accent"
                              : "hover:bg-muted"
                          )}
                        >
                          <div className="flex items-center justify-between gap-2">
                            <p className="text-sm font-medium truncate flex-1">
                              {s.client_name || s.client_id}
                            </p>
                            {s.context_management_enabled ? (
                              <Badge variant="success" className="text-[10px] px-1.5 py-0 shrink-0">
                                CM on
                              </Badge>
                            ) : (
                              <Badge variant="secondary" className="text-[10px] px-1.5 py-0 shrink-0">
                                CM off
                              </Badge>
                            )}
                          </div>
                          <div className="flex items-center justify-between gap-2">
                            <p className="text-xs text-muted-foreground truncate">
                              {s.initialized_servers} server{s.initialized_servers !== 1 ? "s" : ""}
                              {s.failed_servers > 0 ? ` (${s.failed_servers} failed)` : ""}
                              {" "}&middot; {s.total_tools} tools
                            </p>
                            <span className="text-[10px] text-muted-foreground shrink-0">
                              {formatDuration(s.duration_secs)}
                            </span>
                          </div>
                        </div>
                      ))}
                    </div>
                  </ScrollArea>
                </div>
              </ResizablePanel>

              <ResizableHandle withHandle />

              {/* Session Detail */}
              <ResizablePanel defaultSize={65}>
                {selectedSession ? (
                  <ScrollArea className="h-full">
                    <div className="p-6 space-y-4">
                      <div>
                        <div className="flex items-center gap-2">
                          <h2 className="text-lg font-bold">
                            {selectedSession.client_name || selectedSession.client_id}
                          </h2>
                          {selectedSession.context_management_enabled ? (
                            <Badge variant="success">CM Active</Badge>
                          ) : (
                            <Badge variant="secondary">CM Off</Badge>
                          )}
                        </div>
                        {selectedSession.client_name && (
                          <p className="text-sm text-muted-foreground mt-1">
                            {selectedSession.client_id}
                          </p>
                        )}
                      </div>

                      <Card>
                        <CardHeader className="pb-3">
                          <CardTitle className="text-sm">Session Info</CardTitle>
                        </CardHeader>
                        <CardContent>
                          <div className="grid grid-cols-2 gap-3 text-sm">
                            <div>
                              <span className="text-muted-foreground">Duration:</span>{" "}
                              <span className="font-medium">{formatDuration(selectedSession.duration_secs)}</span>
                            </div>
                            <div>
                              <span className="text-muted-foreground">Total Tools:</span>{" "}
                              <span className="font-medium">{selectedSession.total_tools}</span>
                            </div>
                            <div>
                              <span className="text-muted-foreground">Servers:</span>{" "}
                              <span className="font-medium">
                                {selectedSession.initialized_servers}
                                {selectedSession.failed_servers > 0 && (
                                  <span className="text-destructive"> ({selectedSession.failed_servers} failed)</span>
                                )}
                              </span>
                            </div>
                            <div>
                              <span className="text-muted-foreground">Context Management:</span>{" "}
                              <span className="font-medium">
                                {selectedSession.context_management_enabled ? "Enabled" : "Disabled"}
                              </span>
                            </div>
                          </div>
                        </CardContent>
                      </Card>

                      {selectedSession.context_management_enabled && (
                        <Card>
                          <CardHeader className="pb-3">
                            <CardTitle className="text-sm">Context Management Stats</CardTitle>
                          </CardHeader>
                          <CardContent>
                            <div className="grid grid-cols-2 gap-3 text-sm">
                              <div>
                                <span className="text-muted-foreground">Indexed Sources:</span>{" "}
                                <span className="font-medium">{selectedSession.cm_indexed_sources}</span>
                              </div>
                              <div>
                                <span className="text-muted-foreground">Activated Tools:</span>{" "}
                                <span className="font-medium">
                                  {selectedSession.cm_activated_tools} / {selectedSession.cm_total_tools}
                                </span>
                              </div>
                            </div>
                          </CardContent>
                        </Card>
                      )}
                    </div>
                  </ScrollArea>
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
              {/* Enable/Disable */}
              <Card>
                <CardHeader>
                  <div className="flex items-center justify-between">
                    <CardTitle className="text-base">Enable Context Management</CardTitle>
                    <Switch
                      checked={config.enabled}
                      onCheckedChange={(enabled) => updateField("enabled", enabled)}
                      disabled={saving}
                    />
                  </div>
                  <CardDescription>
                    Enable context management globally for all clients that don't have a per-client override.
                  </CardDescription>
                </CardHeader>
              </Card>

              {/* Indexing Tools */}
              <Card>
                <CardHeader>
                  <div className="flex items-center justify-between">
                    <CardTitle className="text-base">Indexing Tools</CardTitle>
                    <Switch
                      checked={config.indexing_tools}
                      onCheckedChange={(v) => updateField("indexingTools", v)}
                      disabled={saving}
                    />
                  </div>
                  <CardDescription>
                    Expose additional tools to clients:{" "}
                    <code className="px-1 py-0.5 rounded bg-muted text-xs">ctx_execute</code> lets
                    clients run code in a sandboxed environment to process large outputs without
                    flooding their context window, and{" "}
                    <code className="px-1 py-0.5 rounded bg-muted text-xs">ctx_search</code> lets
                    clients search through previously indexed catalogs and compressed tool responses.
                  </CardDescription>
                </CardHeader>
              </Card>

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
                  </div>
                  <p className="text-xs text-muted-foreground mt-2">
                    Default: 50,000 bytes. Lower values compress more aggressively. Set to 0 to always compress.
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
                  </div>
                  <p className="text-xs text-muted-foreground mt-2">
                    Default: 10,000 bytes. Set higher to compress fewer responses. Set to 0 to always compress.
                  </p>
                </CardContent>
              </Card>
            </div>
          )}
        </TabsContent>
      </Tabs>
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
