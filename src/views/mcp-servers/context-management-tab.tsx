import { useState, useEffect } from "react"
import { invoke } from "@tauri-apps/api/core"
import { toast } from "sonner"
import { Info } from "lucide-react"
import { Card, CardContent, CardHeader, CardTitle, CardDescription } from "@/components/ui/Card"
import { Switch } from "@/components/ui/Toggle"
import { Input } from "@/components/ui/Input"
import { Button } from "@/components/ui/Button"
import type { ContextManagementConfig, UpdateContextManagementConfigParams } from "@/types/tauri-commands"

export function ContextManagementTab() {
  const [config, setConfig] = useState<ContextManagementConfig | null>(null)
  const [saving, setSaving] = useState(false)
  const [catalogThreshold, setCatalogThreshold] = useState("")
  const [responseThreshold, setResponseThreshold] = useState("")

  useEffect(() => {
    invoke<ContextManagementConfig>("get_context_management_config")
      .then((cfg) => {
        setConfig(cfg)
        setCatalogThreshold(String(cfg.catalog_threshold_bytes))
        setResponseThreshold(String(cfg.response_threshold_bytes))
      })
      .catch((err) => console.error("Failed to load context management config:", err))
  }, [])

  const updateConfig = async (params: UpdateContextManagementConfigParams) => {
    try {
      setSaving(true)
      await invoke("update_context_management_config", { ...params })
      const updated = await invoke<ContextManagementConfig>("get_context_management_config")
      setConfig(updated)
      setCatalogThreshold(String(updated.catalog_threshold_bytes))
      setResponseThreshold(String(updated.response_threshold_bytes))
      toast.success("Context management settings updated")
    } catch (error) {
      toast.error(`Failed to update: ${error}`)
    } finally {
      setSaving(false)
    }
  }

  if (!config) return null

  return (
    <div className="space-y-6 max-w-2xl">
      {/* Overview */}
      <div className="p-4 rounded-lg bg-blue-500/10 border border-blue-600/50">
        <div className="flex items-start gap-3">
          <Info className="h-5 w-5 text-blue-600 dark:text-blue-400 mt-0.5 shrink-0" />
          <div className="space-y-2">
            <p className="text-sm font-medium text-blue-900 dark:text-blue-300">
              How Context Management works
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
              onCheckedChange={(enabled) => updateConfig({ enabled })}
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
              onCheckedChange={(indexingTools) => updateConfig({ indexingTools })}
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
              type="number"
              value={catalogThreshold}
              onChange={(e) => setCatalogThreshold(e.target.value)}
              onKeyDown={(e) => {
                if (e.key === "Enter") {
                  const v = parseInt(catalogThreshold)
                  if (!isNaN(v) && v > 0) updateConfig({ catalogThresholdBytes: v })
                }
              }}
              className="w-40"
              min={1000}
            />
            <span className="text-sm text-muted-foreground">bytes</span>
            <Button
              size="sm"
              variant="outline"
              disabled={saving}
              onClick={() => {
                const v = parseInt(catalogThreshold)
                if (!isNaN(v) && v > 0) updateConfig({ catalogThresholdBytes: v })
              }}
            >
              Save
            </Button>
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
              type="number"
              value={responseThreshold}
              onChange={(e) => setResponseThreshold(e.target.value)}
              onKeyDown={(e) => {
                if (e.key === "Enter") {
                  const v = parseInt(responseThreshold)
                  if (!isNaN(v) && v > 0) updateConfig({ responseThresholdBytes: v })
                }
              }}
              className="w-40"
              min={1000}
            />
            <span className="text-sm text-muted-foreground">bytes</span>
            <Button
              size="sm"
              variant="outline"
              disabled={saving}
              onClick={() => {
                const v = parseInt(responseThreshold)
                if (!isNaN(v) && v > 0) updateConfig({ responseThresholdBytes: v })
              }}
            >
              Save
            </Button>
          </div>
          <p className="text-xs text-muted-foreground mt-2">
            Default: 10,000 bytes. Set higher to compress fewer responses.
          </p>
        </CardContent>
      </Card>
    </div>
  )
}
