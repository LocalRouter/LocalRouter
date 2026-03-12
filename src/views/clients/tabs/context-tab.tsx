import { useState, useEffect } from "react"
import { invoke } from "@tauri-apps/api/core"
import { toast } from "sonner"
import { Info } from "lucide-react"
import { TriStateButton } from "@/components/ui/TriStateButton"
import { Card, CardContent, CardHeader, CardTitle, CardDescription } from "@/components/ui/Card"
import type { ContextManagementConfig } from "@/types/tauri-commands"

interface Client {
  id: string
  name: string
  client_id: string
  context_management_enabled: boolean | null
}

interface ContextTabProps {
  client: Client
  onUpdate: () => void
  onViewChange?: (view: string, subTab?: string | null) => void
}

export function ClientContextTab({ client, onUpdate, onViewChange }: ContextTabProps) {
  const [saving, setSaving] = useState(false)
  const [contextManagement, setContextManagement] = useState<boolean | null>(client.context_management_enabled)
  const [globalConfig, setGlobalConfig] = useState<ContextManagementConfig | null>(null)

  useEffect(() => {
    setContextManagement(client.context_management_enabled)
  }, [client.context_management_enabled])

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
            tool lets the AI discover and unhide them on demand.
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
    </div>
  )
}
