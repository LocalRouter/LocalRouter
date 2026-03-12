import { useState, useEffect } from "react"
import { invoke } from "@tauri-apps/api/core"
import { toast } from "sonner"
import { Info } from "lucide-react"
import { TriStateButton } from "@/components/ui/TriStateButton"
import { Card, CardContent, CardHeader, CardTitle, CardDescription } from "@/components/ui/Card"
import { ClientToolsIndexingTree } from "@/components/permissions/ClientToolsIndexingTree"
import type { ContextManagementConfig, ClientMode } from "@/types/tauri-commands"

interface Client {
  id: string
  name: string
  client_id: string
  context_management_enabled: boolean | null
  client_mode?: ClientMode
  template_id?: string | null
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

  const isMcpViaLlm = client.client_mode === "mcp_via_llm"

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
            Enables catalog compression: deferred loading of tools, prompts, and resources combined with
            FTS5 search indexing of welcome messages and tool descriptions. When catalogs exceed the configured
            threshold, descriptions are compressed and low-priority capabilities are deferred.
          </CardDescription>
        </CardHeader>
        <CardContent className="space-y-4">
          {globalConfig && (
            <>
              <p className="text-xs text-muted-foreground mb-1.5">Exposed tools:</p>
              <div className="flex flex-wrap gap-1.5">
                <code className="text-[11px] px-1.5 py-0.5 rounded bg-muted">{globalConfig.search_tool_name}</code>
                <code className="text-[11px] px-1.5 py-0.5 rounded bg-muted">{globalConfig.read_tool_name}</code>
              </div>
            </>
          )}
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

      {/* Client Tools Indexing (MCP via LLM only) */}
      <Card>
        <CardHeader>
          <CardTitle className="text-base">Client Tools Indexing</CardTitle>
          <CardDescription>
            Select which client tool responses get indexed into FTS5 for context search.
          </CardDescription>
        </CardHeader>
        <CardContent>
          {isMcpViaLlm ? (
            globalConfig && (
              <ClientToolsIndexingTree
                clientId={client.id}
                templateId={client.template_id ?? null}
                globalDefault={globalConfig.client_tools_indexing_default}
                onUpdate={onUpdate}
              />
            )
          ) : (
            <div className="p-4 rounded-lg bg-blue-500/10 border border-blue-600/50">
              <div className="flex items-start gap-3">
                <Info className="h-5 w-5 text-blue-600 dark:text-blue-400 mt-0.5 shrink-0" />
                <p className="text-sm text-blue-900 dark:text-blue-400">
                  Client tool indexing requires MCP via LLM mode to be enabled.
                </p>
              </div>
            </div>
          )}
        </CardContent>
      </Card>
    </div>
  )
}
