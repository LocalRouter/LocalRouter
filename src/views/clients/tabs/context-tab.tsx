import { useState, useEffect } from "react"
import { invoke } from "@tauri-apps/api/core"
import { toast } from "sonner"
import { Info } from "lucide-react"
import { FEATURES } from "@/constants/features"
import { InfoTooltip } from "@/components/ui/info-tooltip"
import { TriStateButton } from "@/components/ui/TriStateButton"
import { Card, CardContent, CardHeader, CardTitle, CardDescription } from "@/components/ui/Card"
import { ClientToolsIndexingTree } from "@/components/permissions/ClientToolsIndexingTree"
import type { ContextManagementConfig, ClientMode } from "@/types/tauri-commands"

interface Client {
  id: string
  name: string
  client_id: string
  context_management_enabled: boolean | null
  catalog_compression_enabled: boolean | null
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
  const [catalogCompression, setCatalogCompression] = useState<boolean | null>(client.catalog_compression_enabled)
  const [globalConfig, setGlobalConfig] = useState<ContextManagementConfig | null>(null)

  useEffect(() => {
    setContextManagement(client.context_management_enabled)
  }, [client.context_management_enabled])

  useEffect(() => {
    setCatalogCompression(client.catalog_compression_enabled)
  }, [client.catalog_compression_enabled])

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
      toast.success(FEATURES.responseRag.name + " " + label)
      onUpdate()
    } catch (error) {
      console.error("Failed to update context management:", error)
      toast.error("Failed to update settings")
    } finally {
      setSaving(false)
    }
  }

  const handleCatalogCompressionChange = async (value: boolean | null) => {
    try {
      setSaving(true)
      await invoke("toggle_client_catalog_compression", {
        clientId: client.client_id,
        enabled: value,
      })
      setCatalogCompression(value)
      const label = value === null ? "inheriting global" : value ? "enabled" : "disabled"
      toast.success(FEATURES.catalogCompression.name + " " + label)
      onUpdate()
    } catch (error) {
      console.error("Failed to update catalog compression:", error)
      toast.error("Failed to update settings")
    } finally {
      setSaving(false)
    }
  }

  const isMcpViaLlm = client.client_mode === "mcp_via_llm"

  // Context management is implicitly enabled when any indexing is configured (mirrors Rust is_enabled())
  const isGloballyEnabled = globalConfig != null && (
    globalConfig.gateway_indexing.global === "enable" ||
    Object.values(globalConfig.gateway_indexing.servers).some(s => s === "enable") ||
    Object.values(globalConfig.gateway_indexing.tools).some(s => s === "enable") ||
    globalConfig.client_tools_indexing_default === "enable"
  )

  const isGlobalCatalogCompressionEnabled = globalConfig?.catalog_compression ?? true

  return (
    <div className="space-y-6">
      {/* Context Management */}
      <Card>
        <CardHeader>
          <div className="flex items-center justify-between">
            <div className="flex items-center gap-2">
              <FEATURES.responseRag.icon className={`h-5 w-5 ${FEATURES.responseRag.color}`} />
              <CardTitle className="text-base">{FEATURES.responseRag.name}</CardTitle>
            </div>
            <div className="flex items-center gap-1">
              <InfoTooltip content="Indexes MCP tool descriptions for full-text search, allowing the router to serve only relevant tools per request instead of the full catalog." />
              <TriStateButton
                value={contextManagement}
                onChange={handleContextManagementChange}
                disabled={saving}
                defaultLabel={`Default (${isGloballyEnabled ? "On" : "Off"})`}
                onLabel="On"
                offLabel="Off"
              />
            </div>
          </div>
          <CardDescription>
            Enables context management: FTS5 search indexing of welcome messages and tool descriptions.
            Requires client support for{" "}
            <code className="px-1 py-0.5 rounded bg-muted text-xs">tools/listChanged</code> notifications.
          </CardDescription>
        </CardHeader>
        {globalConfig && isMcpViaLlm && (contextManagement === true || (contextManagement === null && isGloballyEnabled)) && (
          <CardContent>
            <p className="text-xs text-muted-foreground mb-1.5">Client tools to index:</p>
            <ClientToolsIndexingTree
              clientId={client.id}
              templateId={client.template_id ?? null}
              globalDefault={globalConfig.client_tools_indexing_default}
              onUpdate={onUpdate}
            />
          </CardContent>
        )}
      </Card>

      {/* Catalog Compression */}
      <Card>
        <CardHeader>
          <div className="flex items-center justify-between">
            <div className="flex items-center gap-2">
              <FEATURES.catalogCompression.icon className={`h-5 w-5 ${FEATURES.catalogCompression.color}`} />
              <CardTitle className="text-base">{FEATURES.catalogCompression.name}</CardTitle>
            </div>
            <div className="flex items-center gap-1">
              <InfoTooltip content="Defers tool/prompt/resource catalogs behind FTS5 search indexing, reducing initial context size for clients with large tool sets." />
              <TriStateButton
                value={catalogCompression}
                onChange={handleCatalogCompressionChange}
                disabled={saving}
                defaultLabel={`Default (${isGlobalCatalogCompressionEnabled ? "On" : "Off"})`}
                onLabel="On"
                offLabel="Off"
              />
            </div>
          </div>
          <CardDescription>
            Deferred loading of tools, prompts, and resources. When catalogs exceed the configured
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
                      onClick={() => onViewChange("catalog-compression")}
                    >
                      Global settings
                    </button>
                  ) : (
                    <span>Global settings</span>
                  )}{" "}
                  control thresholds, compression levels, and defaults.
                </p>
              </div>
            </div>
          </div>
        </CardContent>
      </Card>

    </div>
  )
}
