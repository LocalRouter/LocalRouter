import { useState, useEffect, useRef } from "react"
import { invoke } from "@tauri-apps/api/core"
import { toast } from "sonner"
import { BookText, Info } from "lucide-react"
import { Card, CardContent, CardHeader, CardTitle, CardDescription } from "@/components/ui/Card"
import { Switch } from "@/components/ui/Toggle"
import { Input } from "@/components/ui/Input"
import type { ContextManagementConfig } from "@/types/tauri-commands"

export function ContextManagementView() {
  const [config, setConfig] = useState<ContextManagementConfig | null>(null)
  const [saving, setSaving] = useState(false)
  const catalogRef = useRef<HTMLInputElement>(null)
  const responseRef = useRef<HTMLInputElement>(null)

  useEffect(() => {
    invoke<ContextManagementConfig>("get_context_management_config")
      .then(setConfig)
      .catch((err) => console.error("Failed to load context management config:", err))
  }, [])

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
        <h1 className="text-2xl font-bold tracking-tight flex items-center gap-2">
          <BookText className="h-6 w-6" />Context Management
        </h1>
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
              <div className="flex items-center gap-2">
                <CardTitle className="text-base">Enable Context Management</CardTitle>
                <span className="text-xs px-1.5 py-0.5 rounded bg-purple-500/20 text-purple-900 dark:text-purple-300 font-medium">
                  EXPERIMENTAL
                </span>
              </div>
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
      </div>
    </div>
  )
}
