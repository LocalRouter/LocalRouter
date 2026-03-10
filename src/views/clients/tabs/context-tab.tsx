import { useState, useEffect } from "react"
import { invoke } from "@tauri-apps/api/core"
import { toast } from "sonner"
import { Info, AlertTriangle } from "lucide-react"
import { TriStateButton } from "@/components/ui/TriStateButton"
import { Card, CardContent, CardHeader, CardTitle, CardDescription } from "@/components/ui/Card"
import type { ContextManagementConfig } from "@/types/tauri-commands"

interface Client {
  id: string
  name: string
  client_id: string
  context_management_enabled: boolean | null
  indexing_tools_enabled: boolean | null
}

interface ContextTabProps {
  client: Client
  onUpdate: () => void
  onViewChange?: (view: string, subTab?: string | null) => void
}

export function ClientContextTab({ client, onUpdate, onViewChange }: ContextTabProps) {
  const [saving, setSaving] = useState(false)
  const [contextManagement, setContextManagement] = useState<boolean | null>(client.context_management_enabled)
  const [indexingTools, setIndexingTools] = useState<boolean | null>(client.indexing_tools_enabled)
  const [globalConfig, setGlobalConfig] = useState<ContextManagementConfig | null>(null)

  useEffect(() => {
    setContextManagement(client.context_management_enabled)
    setIndexingTools(client.indexing_tools_enabled)
  }, [client.context_management_enabled, client.indexing_tools_enabled])

  useEffect(() => {
    invoke<ContextManagementConfig>("get_context_management_config")
      .then(setGlobalConfig)
      .catch(() => {})
  }, [])

  const handleContextManagementChange = async (value: boolean | null) => {
    try {
      setSaving(true)
      await invoke("toggle_client_context_management", {
        clientId: client.client_id,
        enabled: value,
      })
      setContextManagement(value)
      const label = value === null ? "inheriting global" : value ? "enabled" : "disabled"
      toast.success("Context management " + label)
      onUpdate()
    } catch (error) {
      console.error("Failed to update context management:", error)
      toast.error("Failed to update settings")
    } finally {
      setSaving(false)
    }
  }

  const handleIndexingToolsChange = async (value: boolean | null) => {
    try {
      setSaving(true)
      await invoke("toggle_client_indexing_tools", {
        clientId: client.client_id,
        enabled: value,
      })
      setIndexingTools(value)
      const label = value === null ? "inheriting global" : value ? "enabled" : "disabled"
      toast.success("Indexing tools " + label)
      onUpdate()
    } catch (error) {
      console.error("Failed to update indexing tools:", error)
      toast.error("Failed to update settings")
    } finally {
      setSaving(false)
    }
  }

  return (
    <div className="space-y-6">
      {/* Context Management */}
      <Card>
        <CardHeader>
          <div className="flex items-center justify-between">
            <div className="flex items-center gap-2">
              <CardTitle className="text-base">Catalog Compression</CardTitle>
              <span className="text-xs px-1.5 py-0.5 rounded bg-purple-500/20 text-purple-900 dark:text-purple-300 font-medium">
                EXPERIMENTAL
              </span>
            </div>
            <TriStateButton
              value={contextManagement}
              onChange={handleContextManagementChange}
              disabled={saving}
              defaultLabel={`Default (${globalConfig?.catalog_compression ? "On" : "Off"})`}
              onLabel="On"
              offLabel="Off"
            />
          </div>
          <CardDescription>
            Enables catalog compression: deferred loading of tools, prompts, and resources combined with{" "}
            <a href="https://github.com/mksglu/context-mode" target="_blank" rel="noopener noreferrer" className="text-blue-500 hover:underline">context-mode</a>{" "}
            indexing of welcome messages and tool descriptions. When catalogs exceed the configured
            threshold, descriptions are compressed and low-priority capabilities are deferred. A{" "}
            <code className="px-1 py-0.5 rounded bg-muted text-xs">ctx_search</code>{" "}
            tool lets the AI discover and unhide them on demand. This exposes only the search
            capability &mdash; to also give AI clients the full indexing tools, enable Indexing Tools below.
          </CardDescription>
        </CardHeader>
        <CardContent className="space-y-4">
          <p className="text-xs text-muted-foreground mb-1.5">Exposed tools:</p>
          <div className="flex flex-wrap gap-1.5">
            <code className="text-[11px] px-1.5 py-0.5 rounded bg-muted">ctx_search</code>
          </div>
          <div className="p-4 rounded-lg bg-blue-500/10 border border-blue-600/50">
            <div className="flex items-start gap-3">
              <Info className="h-5 w-5 text-blue-600 dark:text-blue-400 mt-0.5 shrink-0" />
              <div className="space-y-2">
                <p className="text-sm text-blue-900 dark:text-blue-400">
                  {onViewChange ? (
                    <button
                      className="text-blue-500 hover:underline"
                      onClick={() => onViewChange("context-management")}
                    >
                      Global settings
                    </button>
                  ) : (
                    <span>Global settings</span>
                  )}{" "}
                  control thresholds, compression levels, and defaults.
                  Requires client support for{" "}
                  <code className="px-1 py-0.5 rounded bg-blue-500/20 text-xs">
                    tools/listChanged
                  </code>{" "}
                  notifications.
                </p>
              </div>
            </div>
          </div>
        </CardContent>
      </Card>

      {/* Indexing Tools */}
      <Card>
        <CardHeader>
          <div className="flex items-center justify-between">
            <CardTitle className="text-base">Indexing Tools</CardTitle>
            <TriStateButton
              value={indexingTools}
              onChange={handleIndexingToolsChange}
              disabled={saving}
              defaultLabel={`Default (${globalConfig?.indexing_tools ? "On" : "Off"})`}
              onLabel="On"
              offLabel="Off"
            />
          </div>
          <CardDescription>
            Enables the{" "}
            <a href="https://github.com/mksglu/context-mode" target="_blank" rel="noopener noreferrer" className="text-blue-500 hover:underline">context-mode</a>{" "}
            indexing tools that reduce context window usage for Bash, Read, WebFetch, Grep, and Task
            calls. Tool outputs are indexed and searchable rather than returned directly into the
            context window, freeing space for the AI to work with larger results.
          </CardDescription>
        </CardHeader>
        <CardContent className="space-y-3">
          <p className="text-xs text-muted-foreground mb-1.5">Exposed tools:</p>
          <div className="flex flex-wrap gap-1.5">
            <code className="text-[11px] px-1.5 py-0.5 rounded bg-muted">ctx_execute</code>
            <code className="text-[11px] px-1.5 py-0.5 rounded bg-muted">ctx_execute_file</code>
            <code className="text-[11px] px-1.5 py-0.5 rounded bg-muted">ctx_batch_execute</code>
            <code className="text-[11px] px-1.5 py-0.5 rounded bg-muted">ctx_index</code>
            <code className="text-[11px] px-1.5 py-0.5 rounded bg-muted">ctx_fetch_and_index</code>
            <code className="text-[11px] px-1.5 py-0.5 rounded bg-muted">ctx_search</code>
          </div>
          <div className="p-3 rounded-lg border border-amber-600/50 bg-amber-500/10">
            <div className="flex gap-2 items-start">
              <AlertTriangle className="h-4 w-4 text-amber-600 dark:text-amber-400 mt-0.5 shrink-0" />
              <p className="text-sm text-amber-900 dark:text-amber-400">
                Indexing tools give this client the ability to read any file on the system
                (by indexing it) and run arbitrary scripts on disk (in a sandbox, indexing the output).
                Only enable for clients you trust with file system access.
              </p>
            </div>
          </div>
        </CardContent>
      </Card>
    </div>
  )
}
