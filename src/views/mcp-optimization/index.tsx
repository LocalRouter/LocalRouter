import { useState, useEffect, useCallback } from "react"
import { invoke } from "@tauri-apps/api/core"
import { listen } from "@tauri-apps/api/event"
import { Zap, BookText, ArrowRight, CheckCircle2, XCircle } from "lucide-react"
import { Button } from "@/components/ui/Button"
import { Card, CardContent, CardHeader, CardTitle, CardDescription } from "@/components/ui/Card"
import { Switch } from "@/components/ui/Toggle"
import type { ContextManagementConfig, ContextModeInfo } from "@/types/tauri-commands"

interface McpOptimizationViewProps {
  activeSubTab?: string | null
  onTabChange?: (view: string, subTab?: string | null) => void
}

export function McpOptimizationView({ onTabChange }: McpOptimizationViewProps) {
  const [config, setConfig] = useState<ContextManagementConfig | null>(null)
  const [modeInfo, setModeInfo] = useState<ContextModeInfo | null>(null)
  const [saving, setSaving] = useState(false)

  const loadConfig = useCallback(async () => {
    try {
      const data = await invoke<ContextManagementConfig>("get_context_management_config")
      setConfig(data)
    } catch (err) {
      console.error("Failed to load context management config:", err)
    }
  }, [])

  const loadModeInfo = useCallback(async () => {
    try {
      const info = await invoke<ContextModeInfo>("get_context_mode_info")
      setModeInfo(info)
    } catch (err) {
      console.error("Failed to load context mode info:", err)
    }
  }, [])

  useEffect(() => {
    loadConfig()
    loadModeInfo()

    const unlistenConfig = listen('config-changed', () => {
      loadConfig()
    })

    return () => {
      unlistenConfig.then(fn => fn())
    }
  }, [loadConfig, loadModeInfo])

  const updateField = async (field: string, value: boolean) => {
    setSaving(true)
    try {
      await invoke("update_context_management_config", { [field]: value })
      await loadConfig()
    } catch (err) {
      console.error("Failed to update context management config:", err)
    } finally {
      setSaving(false)
    }
  }

  const navigateTo = (view: string, subTab?: string) => {
    onTabChange?.(view, subTab ?? null)
  }

  return (
    <div className="flex flex-col h-full min-h-0 gap-4 max-w-5xl">
      <div className="flex-shrink-0">
        <div className="flex items-center gap-3 mb-1">
          <h1 className="text-xl font-semibold flex items-center gap-2">
            <Zap className="h-6 w-6" />
            MCP Optimization
          </h1>
        </div>
        <p className="text-sm text-muted-foreground">
          Optimize MCP gateway context with catalog compression and indexing tools
        </p>
      </div>

      <div className="space-y-4 max-w-2xl overflow-y-auto flex-1 min-h-0">
        {/* Catalog Compression Section */}
        <Card>
          <CardHeader className="pb-3">
            <div className="flex items-center justify-between">
              <div className="flex items-center gap-2">
                <BookText className="h-4 w-4 text-muted-foreground" />
                <CardTitle className="text-base">Default: Enable Catalog Compression</CardTitle>
              </div>
              {config && (
                <Switch
                  checked={config.enabled}
                  onCheckedChange={(enabled) => updateField("enabled", enabled)}
                  disabled={saving}
                />
              )}
            </div>
            <CardDescription>
              Uses deferred loading of tools, prompts, and resources combined with indexing.
              When catalogs exceed the configured threshold, capabilities are hidden and a{" "}
              <code className="px-1 py-0.5 rounded bg-muted text-xs">ctx_search</code>{" "}
              tool lets the AI discover and unhide them on demand.
            </CardDescription>
          </CardHeader>
          <CardContent className="pt-0">
            <div className="flex items-center justify-between">
              <Button variant="ghost" size="sm" className="gap-1.5 -ml-2" onClick={() => navigateTo("context-management")}>
                Configure
                <ArrowRight className="h-3 w-3" />
              </Button>
              {modeInfo && (
                <div className="flex items-center gap-2 text-xs text-muted-foreground">
                  {modeInfo.contextModeVersion ? (
                    <>
                      <CheckCircle2 className="h-3 w-3 text-green-600 dark:text-green-400" />
                      context-mode v{modeInfo.contextModeVersion}
                    </>
                  ) : (
                    <>
                      <XCircle className="h-3 w-3" />
                      context-mode not installed
                    </>
                  )}
                </div>
              )}
            </div>
          </CardContent>
        </Card>

        {/* Indexing Tools Section */}
        <Card>
          <CardHeader className="pb-3">
            <div className="flex items-center justify-between">
              <div className="flex items-center gap-2">
                <BookText className="h-4 w-4 text-muted-foreground" />
                <CardTitle className="text-base">Default: Indexing Tools</CardTitle>
              </div>
              {config && (
                <Switch
                  checked={config.indexing_tools}
                  onCheckedChange={(v) => updateField("indexingTools", v)}
                  disabled={saving}
                />
              )}
            </div>
            <CardDescription>
              Enables indexing tools that reduce context window usage for Bash, Read, WebFetch, Grep,
              and Task calls. Tool outputs are indexed and searchable rather than returned directly
              into the context window.
            </CardDescription>
          </CardHeader>
          <CardContent className="pt-0">
            <Button variant="ghost" size="sm" className="gap-1.5 -ml-2" onClick={() => navigateTo("context-management", "settings")}>
              Configure
              <ArrowRight className="h-3 w-3" />
            </Button>
          </CardContent>
        </Card>
      </div>
    </div>
  )
}
