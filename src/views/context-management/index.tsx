import { useState, useEffect, useRef, useCallback } from "react"
import { invoke } from "@tauri-apps/api/core"
import { toast } from "sonner"
import { BookText, Info, RefreshCw } from "lucide-react"
import { Badge } from "@/components/ui/Badge"
import { Button } from "@/components/ui/Button"
import { Card, CardContent, CardHeader, CardTitle, CardDescription } from "@/components/ui/Card"
import { Switch } from "@/components/ui/Toggle"
import { Input } from "@/components/ui/Input"
import type { ContextManagementConfig, ActiveSessionInfo } from "@/types/tauri-commands"

export function ContextManagementView() {
  const [config, setConfig] = useState<ContextManagementConfig | null>(null)
  const [sessions, setSessions] = useState<ActiveSessionInfo[]>([])
  const [saving, setSaving] = useState(false)
  const catalogRef = useRef<HTMLInputElement>(null)
  const responseRef = useRef<HTMLInputElement>(null)

  const loadSessions = useCallback(async () => {
    try {
      const data = await invoke<ActiveSessionInfo[]>("list_active_sessions")
      setSessions(data)
    } catch (err) {
      console.error("Failed to load sessions:", err)
    }
  }, [])

  useEffect(() => {
    invoke<ContextManagementConfig>("get_context_management_config")
      .then(setConfig)
      .catch((err) => console.error("Failed to load context management config:", err))
    loadSessions()
    const interval = setInterval(loadSessions, 5000)
    return () => clearInterval(interval)
  }, [loadSessions])

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

  if (!config) return null

  return (
    <div className="space-y-4">
      <div>
        <div className="flex items-center gap-2">
          <h1 className="text-2xl font-bold tracking-tight flex items-center gap-2">
            <BookText className="h-6 w-6" />Context Management
          </h1>
          <Badge variant="outline" className="bg-purple-500/10 text-purple-900 dark:text-purple-400">EXPERIMENTAL</Badge>
        </div>
        <p className="text-sm text-muted-foreground">
          Compress MCP catalogs and tool responses using FTS5 search indexing
        </p>
      </div>

      <div className="space-y-6 max-w-2xl">
        {/* Overview */}
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
              Expose <code className="px-1 py-0.5 rounded bg-muted text-xs">ctx_execute</code> and{" "}
              <code className="px-1 py-0.5 rounded bg-muted text-xs">ctx_search</code> tools
              so clients can run code and search indexed content directly.
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
                ref={catalogRef}
                type="number"
                defaultValue={config.catalog_threshold_bytes}
                onBlur={(e) => {
                  const v = parseInt(e.target.value)
                  if (!isNaN(v) && v > 0 && v !== config.catalog_threshold_bytes) {
                    updateField("catalogThresholdBytes", v)
                  }
                }}
                onKeyDown={(e) => {
                  if (e.key === "Enter") {
                    (e.target as HTMLInputElement).blur()
                  }
                }}
                className="w-40"
                min={1000}
              />
              <span className="text-sm text-muted-foreground">bytes</span>
            </div>
            <p className="text-xs text-muted-foreground mt-2">
              Default: 50,000 bytes. Lower values compress more aggressively.
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
                ref={responseRef}
                type="number"
                defaultValue={config.response_threshold_bytes}
                onBlur={(e) => {
                  const v = parseInt(e.target.value)
                  if (!isNaN(v) && v > 0 && v !== config.response_threshold_bytes) {
                    updateField("responseThresholdBytes", v)
                  }
                }}
                onKeyDown={(e) => {
                  if (e.key === "Enter") {
                    (e.target as HTMLInputElement).blur()
                  }
                }}
                className="w-40"
                min={1000}
              />
              <span className="text-sm text-muted-foreground">bytes</span>
            </div>
            <p className="text-xs text-muted-foreground mt-2">
              Default: 10,000 bytes. Set higher to compress fewer responses.
            </p>
          </CardContent>
        </Card>

        {/* Active Sessions */}
        <Card>
          <CardHeader>
            <div className="flex items-center justify-between">
              <CardTitle className="text-base">Active Sessions</CardTitle>
              <Button
                variant="ghost"
                size="sm"
                onClick={loadSessions}
                className="h-7 w-7 p-0"
              >
                <RefreshCw className="h-3.5 w-3.5" />
              </Button>
            </div>
            <CardDescription>
              Live MCP gateway sessions and their context management state.
            </CardDescription>
          </CardHeader>
          <CardContent>
            {sessions.length === 0 ? (
              <p className="text-sm text-muted-foreground">No active sessions</p>
            ) : (
              <div className="space-y-3">
                {sessions.map((s) => (
                  <div
                    key={s.client_id}
                    className="flex items-center justify-between p-3 rounded-lg border bg-muted/30"
                  >
                    <div className="space-y-1">
                      <div className="flex items-center gap-2">
                        <span className="text-sm font-medium">{s.client_name || s.client_id}</span>
                        {s.context_management_enabled ? (
                          <Badge variant="outline" className="text-[10px] bg-green-500/10 text-green-700 dark:text-green-400">
                            CM active
                          </Badge>
                        ) : (
                          <Badge variant="outline" className="text-[10px]">
                            CM off
                          </Badge>
                        )}
                      </div>
                      <div className="flex gap-4 text-xs text-muted-foreground">
                        <span>{formatDuration(s.duration_secs)}</span>
                        <span>{s.initialized_servers} server{s.initialized_servers !== 1 ? "s" : ""}{s.failed_servers > 0 ? ` (${s.failed_servers} failed)` : ""}</span>
                        <span>{s.total_tools} tools</span>
                      </div>
                    </div>
                    {s.context_management_enabled && (
                      <div className="text-right text-xs text-muted-foreground space-y-0.5">
                        <div>{s.cm_indexed_sources} indexed</div>
                        <div>{s.cm_activated_tools}/{s.cm_total_tools} activated</div>
                      </div>
                    )}
                  </div>
                ))}
              </div>
            )}
          </CardContent>
        </Card>
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
