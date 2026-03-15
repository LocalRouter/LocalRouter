import { useState, useEffect, useCallback } from "react"
import { invoke } from "@tauri-apps/api/core"
import { listen } from "@tauri-apps/api/event"
import { Zap, ArrowRight, CheckCircle2, XCircle, Loader2, Download } from "lucide-react"
import { Badge } from "@/components/ui/Badge"
import { Button } from "@/components/ui/Button"
import { Card, CardContent, CardHeader, CardTitle, CardDescription } from "@/components/ui/Card"
import { Switch } from "@/components/ui/Toggle"
import { GatewayIndexingTree } from "@/components/permissions/GatewayIndexingTree"
import { IndexingStateButton } from "@/components/permissions/IndexingStateButton"
import type { JsonRepairConfig, PromptCompressionConfig, CompressionStatus, RouteLLMStatus, RouteLLMState, ContextManagementConfig, IndexingState } from "@/types/tauri-commands"
import { ROUTELLM_REQUIREMENTS } from "@/components/routellm/types"
import { OptimizeDiagram } from "./OptimizeDiagram"
import { FEATURES } from "@/constants/features"

interface OptimizeOverviewProps {
  activeSubTab?: string | null
  onTabChange?: (view: string, subTab?: string | null) => void
}

const getRouteLLMStateInfo = (state: RouteLLMState) => {
  switch (state) {
    case "not_downloaded":
      return { label: "Not Downloaded", variant: "secondary" as const }
    case "downloading":
      return { label: "Downloading...", variant: "default" as const }
    case "downloaded_not_running":
      return { label: "Downloaded", variant: "outline" as const }
    case "initializing":
      return { label: "Loading...", variant: "default" as const }
    case "started":
      return { label: "Ready", variant: "success" as const }
    default:
      return { label: "Unknown", variant: "secondary" as const }
  }
}

export function OptimizeOverviewView({ onTabChange }: OptimizeOverviewProps) {
  // LLM optimization state
  const [jsonRepairConfig, setJsonRepairConfig] = useState<JsonRepairConfig | null>(null)
  const [compressionConfig, setCompressionConfig] = useState<PromptCompressionConfig | null>(null)
  const [compressionStatus, setCompressionStatus] = useState<CompressionStatus | null>(null)
  const [routellmStatus, setRoutellmStatus] = useState<RouteLLMStatus | null>(null)
  const [savingJsonRepair, setSavingJsonRepair] = useState(false)
  const [savingCompression, setSavingCompression] = useState(false)

  // Context management state
  const [cmConfig, setCmConfig] = useState<ContextManagementConfig | null>(null)

  const loadJsonRepairConfig = useCallback(async () => {
    try {
      const data = await invoke<JsonRepairConfig>("get_json_repair_config")
      setJsonRepairConfig(data)
    } catch (err) {
      console.error("Failed to load JSON repair config:", err)
    }
  }, [])

  const loadCompressionConfig = useCallback(async () => {
    try {
      const data = await invoke<PromptCompressionConfig>("get_compression_config")
      setCompressionConfig(data)
    } catch (err) {
      console.error("Failed to load compression config:", err)
    }
  }, [])

  const loadCompressionStatus = useCallback(async () => {
    try {
      const data = await invoke<CompressionStatus>("get_compression_status")
      setCompressionStatus(data)
    } catch (err) {
      console.error("Failed to load compression status:", err)
    }
  }, [])

  const loadRoutellmStatus = useCallback(async () => {
    try {
      const data = await invoke<RouteLLMStatus>("routellm_get_status")
      setRoutellmStatus(data)
    } catch (err) {
      console.error("Failed to load RouteLLM status:", err)
    }
  }, [])

  const loadCmConfig = useCallback(async () => {
    try {
      const data = await invoke<ContextManagementConfig>("get_context_management_config")
      setCmConfig(data)
    } catch (err) {
      console.error("Failed to load context management config:", err)
    }
  }, [])

  useEffect(() => {
    loadJsonRepairConfig()
    loadCompressionConfig()
    loadCompressionStatus()
    loadRoutellmStatus()
    loadCmConfig()

    const unlistenConfig = listen('config-changed', () => {
      loadJsonRepairConfig()
      loadCompressionConfig()
      loadCmConfig()
    })

    return () => {
      unlistenConfig.then(fn => fn())
    }
  }, [loadJsonRepairConfig, loadCompressionConfig, loadCompressionStatus, loadRoutellmStatus, loadCmConfig])

  const updateJsonRepairEnabled = async (enabled: boolean) => {
    if (!jsonRepairConfig) return
    setSavingJsonRepair(true)
    const newConfig = { ...jsonRepairConfig, enabled }
    try {
      await invoke("update_json_repair_config", { configJson: JSON.stringify(newConfig) })
      setJsonRepairConfig(newConfig)
    } catch (err) {
      console.error("Failed to update JSON repair config:", err)
    } finally {
      setSavingJsonRepair(false)
    }
  }

  const updateCompressionEnabled = async (enabled: boolean) => {
    if (!compressionConfig) return
    setSavingCompression(true)
    try {
      await invoke("update_compression_config", { configJson: JSON.stringify({ ...compressionConfig, enabled }) })
      setCompressionConfig({ ...compressionConfig, enabled })
    } catch (err) {
      console.error("Failed to update compression config:", err)
    } finally {
      setSavingCompression(false)
    }
  }

  const updateCmField = async (field: string, value: unknown) => {
    try {
      await invoke("update_context_management_config", { [field]: value })
      const updated = await invoke<ContextManagementConfig>("get_context_management_config")
      setCmConfig(updated)
    } catch (err) {
      console.error("Failed to update context management config:", err)
    }
  }

  const navigateTo = (view: string, subTab?: string) => {
    onTabChange?.(view, subTab ?? null)
  }

  return (
    <div className="flex flex-col h-full min-h-0 gap-4 max-w-5xl">
      <div className="flex-shrink-0">
        <div className="flex items-center gap-3 mb-1">
          <h1 className="text-2xl font-bold tracking-tight flex items-center gap-2">
            <Zap className="h-6 w-6" />
            Optimize
          </h1>
        </div>
        <p className="text-sm text-muted-foreground">
          Optimize LLM and MCP requests with safety scanning, JSON repair, prompt compression, intelligent routing, and context management
        </p>
      </div>

      <div className="space-y-4 max-w-2xl overflow-y-auto flex-1 min-h-0">
        {/* Architecture Diagram */}
        <OptimizeDiagram />

        {/* GuardRails Section */}
        <Card>
          <CardHeader className="pb-3">
            <div className="flex items-center gap-2">
              <FEATURES.guardrails.icon className={`h-4 w-4 ${FEATURES.guardrails.color}`} />
              <CardTitle className="text-base">GuardRails</CardTitle>
            </div>
            <CardDescription>
              LLM-based content safety scanning. Requests are checked against safety categories
              before being sent to the provider. Configure safety models and default category actions.
            </CardDescription>
          </CardHeader>
          <CardContent className="pt-0">
            <Button variant="ghost" size="sm" className="gap-1.5 -ml-2" onClick={() => navigateTo("guardrails")}>
              Configure
              <ArrowRight className="h-3 w-3" />
            </Button>
          </CardContent>
        </Card>

        {/* Secret Scanning Section */}
        <Card>
          <CardHeader className="pb-3">
            <div className="flex items-center gap-2">
              <FEATURES.secretScanning.icon className={`h-4 w-4 ${FEATURES.secretScanning.color}`} />
              <CardTitle className="text-base">Secret Scanning</CardTitle>
            </div>
            <CardDescription>
              Detect potential secrets (API keys, tokens, passwords) in outbound requests
              before they reach providers. Regex-based detection with entropy filtering.
            </CardDescription>
          </CardHeader>
          <CardContent className="pt-0">
            <Button variant="ghost" size="sm" className="gap-1.5 -ml-2" onClick={() => navigateTo("secret-scanning")}>
              Configure
              <ArrowRight className="h-3 w-3" />
            </Button>
          </CardContent>
        </Card>

        {/* JSON Repair Section */}
        <Card>
          <CardHeader className="pb-3">
            <div className="flex items-center justify-between">
              <div className="flex items-center gap-2">
                <FEATURES.jsonRepair.icon className={`h-4 w-4 ${FEATURES.jsonRepair.color}`} />
                <CardTitle className="text-base">Default: Enable JSON Repair</CardTitle>
              </div>
              {jsonRepairConfig && (
                <Switch
                  checked={jsonRepairConfig.enabled}
                  onCheckedChange={updateJsonRepairEnabled}
                  disabled={savingJsonRepair}
                />
              )}
            </div>
            <CardDescription>
              Automatically fix malformed JSON responses from LLMs. Includes syntax repair and schema coercion
              for requests with JSON response format.
            </CardDescription>
          </CardHeader>
          <CardContent className="pt-0">
            <Button variant="ghost" size="sm" className="gap-1.5 -ml-2" onClick={() => navigateTo("json-repair")}>
              Configure
              <ArrowRight className="h-3 w-3" />
            </Button>
          </CardContent>
        </Card>

        {/* Compression Section */}
        <Card>
          <CardHeader className="pb-3">
            <div className="flex items-center justify-between">
              <div className="flex items-center gap-2">
                <FEATURES.compression.icon className={`h-4 w-4 ${FEATURES.compression.color}`} />
                <CardTitle className="text-base">Default: Enable Prompt Compression</CardTitle>
              </div>
              {compressionConfig && (
                <Switch
                  checked={compressionConfig.enabled}
                  onCheckedChange={updateCompressionEnabled}
                  disabled={savingCompression}
                />
              )}
            </div>
            <CardDescription>
              LLMLingua-2 token-level compression reduces input tokens for chat completion requests.
              Extractive compression keeps exact original tokens — no hallucination possible.
            </CardDescription>
          </CardHeader>
          <CardContent className="pt-0">
            <div className="flex items-center justify-between">
              <Button variant="ghost" size="sm" className="gap-1.5 -ml-2" onClick={() => navigateTo("compression")}>
                Configure
                <ArrowRight className="h-3 w-3" />
              </Button>
              {compressionStatus && (
                <div className="flex items-center gap-2 text-xs text-muted-foreground">
                  {compressionStatus.model_loaded ? (
                    <>
                      <CheckCircle2 className="h-3 w-3 text-green-600 dark:text-green-400" />
                      Model loaded
                    </>
                  ) : compressionStatus.model_downloaded ? (
                    <>
                      <Download className="h-3 w-3" />
                      Model downloaded (not loaded)
                    </>
                  ) : (
                    <>
                      <XCircle className="h-3 w-3" />
                      Model not downloaded
                    </>
                  )}
                </div>
              )}
            </div>
          </CardContent>
        </Card>

        {/* Strong/Weak Section */}
        <Card>
          <CardHeader className="pb-3">
            <div className="flex items-center gap-2">
              <FEATURES.routing.icon className={`h-4 w-4 ${FEATURES.routing.color}`} />
              <CardTitle className="text-base">Strong/Weak Routing</CardTitle>
              <Badge variant="outline" className="bg-purple-500/10 text-purple-900 dark:text-purple-400 text-[10px]">EXPERIMENTAL</Badge>
            </div>
            <CardDescription>
              Intelligent routing that analyzes complexity to select the most cost-effective model — typically
              saving 30-60% on costs. Requires a {ROUTELLM_REQUIREMENTS.DISK_GB} GB model download.
              Configured per-client in their strategy settings.
            </CardDescription>
          </CardHeader>
          <CardContent className="pt-0">
            <div className="flex items-center justify-between">
              <Button variant="ghost" size="sm" className="gap-1.5 -ml-2" onClick={() => navigateTo("strong-weak")}>
                Configure
                <ArrowRight className="h-3 w-3" />
              </Button>
              {routellmStatus ? (
                <div className="flex items-center gap-2 text-xs">
                  <Badge variant={getRouteLLMStateInfo(routellmStatus.state).variant} className="text-[10px]">
                    {getRouteLLMStateInfo(routellmStatus.state).label}
                  </Badge>
                </div>
              ) : (
                <Loader2 className="h-3 w-3 animate-spin text-muted-foreground" />
              )}
            </div>
          </CardContent>
        </Card>

        {/* Catalog Compression Section */}
        <Card>
          <CardHeader className="pb-3">
            <div className="flex items-center justify-between">
              <div className="flex items-center gap-2">
                <FEATURES.catalogCompression.icon className={`h-4 w-4 ${FEATURES.catalogCompression.color}`} />
                <CardTitle className="text-base">Default: Catalog Compression</CardTitle>
              </div>
              {cmConfig && (
                <Switch
                  checked={cmConfig.catalog_compression}
                  onCheckedChange={(v) => updateCmField("catalogCompression", v)}
                />
              )}
            </div>
            <CardDescription>
              Deferred loading of tools, prompts, and resources. When catalogs exceed the configured
              threshold, capabilities are compressed and a search tool lets the AI discover them on demand.
            </CardDescription>
          </CardHeader>
          <CardContent className="pt-0">
            <Button variant="ghost" size="sm" className="gap-1.5 -ml-2" onClick={() => navigateTo("catalog-compression")}>
              Configure
              <ArrowRight className="h-3 w-3" />
            </Button>
          </CardContent>
        </Card>

        {/* Response RAG Section */}
        <Card>
          <CardHeader className="pb-3">
            <div className="flex items-center gap-2">
              <FEATURES.responseRag.icon className={`h-4 w-4 ${FEATURES.responseRag.color}`} />
              <CardTitle className="text-base">Response RAG</CardTitle>
            </div>
            <CardDescription>
              FTS5 search indexing of tool responses. Control which gateway and client tool
              responses get indexed for context search.
            </CardDescription>
          </CardHeader>
          <CardContent className="space-y-3 pt-0">
            {cmConfig && (
              <div className="border rounded-lg overflow-hidden">
                {/* Gateway tools tree (strip its own border) */}
                <div className="[&>div]:border-0 [&>div]:rounded-none">
                  <GatewayIndexingTree
                    permissions={cmConfig.gateway_indexing}
                    onUpdate={async () => {
                      const updated = await invoke<ContextManagementConfig>("get_context_management_config")
                      setCmConfig(updated)
                    }}
                  />
                </div>

                {/* Client Tools - same level as "All Gateway Tools" */}
                <div className="flex items-center gap-2 px-3 py-3 border-t bg-background">
                  <span className="font-semibold text-sm flex-1 min-w-0 truncate">Client Tools</span>
                  <div className="shrink-0">
                    <IndexingStateButton
                      value={cmConfig.client_tools_indexing_default}
                      onChange={(state: IndexingState) => updateCmField("clientToolsIndexingDefault", state)}
                      size="sm"
                    />
                  </div>
                </div>
                <div className="flex items-center gap-2 py-2 border-t border-border/50 hover:bg-muted/30 transition-colors text-sm" style={{ paddingLeft: "28px", paddingRight: "12px" }}>
                  <div className="w-5" />
                  <span className="text-xs text-muted-foreground">
                    Per-client overrides are configured in each client&apos;s settings.
                  </span>
                </div>
              </div>
            )}
            <Button variant="ghost" size="sm" className="gap-1.5 -ml-2" onClick={() => navigateTo("response-rag")}>
              Configure
              <ArrowRight className="h-3 w-3" />
            </Button>
          </CardContent>
        </Card>
      </div>
    </div>
  )
}
