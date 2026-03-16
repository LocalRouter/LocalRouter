import { useState, useEffect } from "react"
import { invoke } from "@tauri-apps/api/core"
import { toast } from "sonner"
import { CheckCircle2, ChevronRight, Download, Loader2 } from "lucide-react"
import { FEATURES, INDEXING_CHILDREN } from "@/constants/features"
import { ExperimentalBadge } from "@/components/shared/ExperimentalBadge"
import { TAB_ICONS, TAB_ICON_CLASS } from "@/constants/tab-icons"
import { Button } from "@/components/ui/Button"
import { Card, CardContent, CardHeader, CardTitle, CardDescription } from "@/components/ui/Card"
import { Tabs, TabsContent, TabsList, TabsTrigger } from "@/components/ui/tabs"
import type { EmbeddingStatus, ContextManagementConfig, MemoryConfig, UpdateContextManagementConfigParams } from "@/types/tauri-commands"

interface IndexingViewProps {
  activeSubTab?: string | null
  onTabChange?: (view: string, subTab?: string | null) => void
}

export function IndexingView({ activeSubTab, onTabChange }: IndexingViewProps) {
  const [embeddingStatus, setEmbeddingStatus] = useState<EmbeddingStatus | null>(null)
  const [isDownloading, setIsDownloading] = useState(false)
  const [ctxConfig, setCtxConfig] = useState<ContextManagementConfig | null>(null)
  const [memConfig, setMemConfig] = useState<MemoryConfig | null>(null)

  const tab = activeSubTab || "info"
  const handleTabChange = (newTab: string) => onTabChange?.("indexing", newTab)

  useEffect(() => {
    invoke<EmbeddingStatus>("get_embedding_status").then(setEmbeddingStatus).catch(() => {})
    invoke<ContextManagementConfig>("get_context_management_config").then(setCtxConfig).catch(() => {})
    invoke<MemoryConfig>("get_memory_config").then(setMemConfig).catch(() => {})
  }, [])

  const downloadEmbeddingModel = async () => {
    setIsDownloading(true)
    try {
      await invoke("install_embedding_model")
      toast.success("Embedding model downloaded and loaded")
      const status = await invoke<EmbeddingStatus>("get_embedding_status")
      setEmbeddingStatus(status)
    } catch (error: any) {
      toast.error(`Download failed: ${error.message || error}`)
    } finally {
      setIsDownloading(false)
    }
  }

  const updateField = async (field: string, value: unknown) => {
    try {
      await invoke("update_context_management_config", { [field]: value } satisfies Partial<UpdateContextManagementConfigParams>)
      const updated = await invoke<ContextManagementConfig>("get_context_management_config")
      setCtxConfig(updated)
    } catch (error) {
      toast.error(`Failed to update: ${error}`)
    }
  }

  const navigateTo = (viewId: string) => {
    onTabChange?.(viewId, null)
  }

  return (
    <div className="flex flex-col h-full min-h-0 max-w-5xl">
      <div className="flex-shrink-0 pb-4">
        <h1 className="text-2xl font-bold tracking-tight flex items-center gap-2">
          <FEATURES.indexing.icon className={`h-6 w-6 ${FEATURES.indexing.color}`} />
          Indexing
        </h1>
        <p className="text-sm text-muted-foreground">
          FTS5 search engine powering MCP Catalog Indexing, Tool Responses Indexing, and Conversation Memory
        </p>
      </div>

      <Tabs
        value={tab}
        onValueChange={handleTabChange}
        className="flex flex-col flex-1 min-h-0"
      >
        <TabsList className="flex-shrink-0 w-fit">
          <TabsTrigger value="info"><TAB_ICONS.info className={TAB_ICON_CLASS} />Info</TabsTrigger>
          <TabsTrigger value="settings"><TAB_ICONS.settings className={TAB_ICON_CLASS} />Settings</TabsTrigger>
        </TabsList>

        {/* Info Tab */}
        <TabsContent value="info" className="flex-1 min-h-0 mt-4 overflow-y-auto">
          <div className="space-y-4 max-w-2xl">
            {/* How It Works */}
            <Card>
              <CardHeader className="pb-3">
                <CardTitle className="text-sm">How It Works</CardTitle>
              </CardHeader>
              <CardContent className="space-y-2 text-sm text-muted-foreground">
                <p>
                  All three indexing features share the same native FTS5 full-text search engine.
                  Content is automatically chunked (markdown, JSON, or plain text), indexed into SQLite FTS5,
                  and searchable via a <strong>Search + Read</strong> tool pattern:
                </p>
                <ol className="list-decimal list-inside space-y-1 ml-2">
                  <li>Content is chunked by structure (headings, code blocks, paragraphs)</li>
                  <li>Chunks are indexed with Porter stemming, trigram matching, and fuzzy correction</li>
                  <li>The LLM searches with a <code>Search</code> tool &mdash; results include source labels and line numbers</li>
                  <li>The LLM reads full context with a <code>Read</code> tool &mdash; supports offset/limit pagination</li>
                </ol>
              </CardContent>
            </Card>

            {/* Semantic Search */}
            <Card>
              <CardHeader className="pb-3">
                <CardTitle className="text-sm">Semantic Search (Optional)</CardTitle>
                <CardDescription>
                  Download a small local embedding model (~80MB) to enable hybrid search across all three features.
                  FTS5 keyword search works without it.
                </CardDescription>
              </CardHeader>
              <CardContent className="space-y-3">
                {embeddingStatus?.downloaded ? (
                  <div className="flex items-center gap-2 text-sm">
                    <CheckCircle2 className="h-4 w-4 text-green-600 dark:text-green-400 shrink-0" />
                    <span className="font-medium">{embeddingStatus.model_name}</span>
                    <span className="text-muted-foreground text-xs">
                      {embeddingStatus.model_size_mb != null && `(${embeddingStatus.model_size_mb.toFixed(0)} MB)`}
                      {embeddingStatus.loaded ? " — loaded" : " — downloaded"}
                    </span>
                  </div>
                ) : (
                  <Button size="sm" onClick={downloadEmbeddingModel} disabled={isDownloading}>
                    {isDownloading ? (
                      <><Loader2 className="h-3.5 w-3.5 mr-1.5 animate-spin" />Downloading...</>
                    ) : (
                      <><Download className="h-3.5 w-3.5 mr-1.5" />Download all-MiniLM-L6-v2 (~80MB)</>
                    )}
                  </Button>
                )}
                <p className="text-xs text-muted-foreground">
                  Enables semantic search: &ldquo;SQL database for login&rdquo; finds
                  &ldquo;We chose PostgreSQL for authentication.&rdquo;
                  Runs locally via Metal/CUDA/CPU &mdash; no external API calls.
                </p>

                {/* Latency benchmarks — shown when model is downloaded */}
                {embeddingStatus?.downloaded && <div className="pt-2 border-t space-y-2">
                  <p className="text-xs font-medium text-foreground">Performance (Apple Silicon, all-MiniLM-L6-v2)</p>
                  <div className="grid grid-cols-2 gap-3">
                    <div>
                      <p className="text-[11px] font-medium text-muted-foreground mb-1">Index Latency</p>
                      <table className="text-[11px] text-muted-foreground w-full">
                        <thead>
                          <tr className="border-b">
                            <th className="text-left font-medium pb-0.5">Size</th>
                            <th className="text-right font-medium pb-0.5">FTS5</th>
                            <th className="text-right font-medium pb-0.5">+ Vector</th>
                          </tr>
                        </thead>
                        <tbody>
                          <tr><td>1 KB</td><td className="text-right">0.6 ms</td><td className="text-right">29 ms</td></tr>
                          <tr><td>10 KB</td><td className="text-right">1.5 ms</td><td className="text-right">237 ms</td></tr>
                          <tr><td>100 KB</td><td className="text-right">11 ms</td><td className="text-right">2.3 s</td></tr>
                        </tbody>
                      </table>
                    </div>
                    <div>
                      <p className="text-[11px] font-medium text-muted-foreground mb-1">Search Latency</p>
                      <table className="text-[11px] text-muted-foreground w-full">
                        <thead>
                          <tr className="border-b">
                            <th className="text-left font-medium pb-0.5">Size</th>
                            <th className="text-right font-medium pb-0.5">FTS5</th>
                            <th className="text-right font-medium pb-0.5">+ Vector</th>
                          </tr>
                        </thead>
                        <tbody>
                          <tr><td>1 KB</td><td className="text-right">44 µs</td><td className="text-right">7.1 ms</td></tr>
                          <tr><td>10 KB</td><td className="text-right">97 µs</td><td className="text-right">7.3 ms</td></tr>
                          <tr><td>100 KB</td><td className="text-right">465 µs</td><td className="text-right">7.8 ms</td></tr>
                        </tbody>
                      </table>
                    </div>
                  </div>
                  <p className="text-[11px] text-muted-foreground italic">
                    Search adds a fixed ~7 ms for query embedding. Model cold start: 32 ms (memory-mapped).
                  </p>
                </div>}
              </CardContent>
            </Card>

            {/* Feature Cards */}
            {INDEXING_CHILDREN.map((key) => {
              const feature = FEATURES[key]
              const Icon = feature.icon
              return (
                <Card key={key} className="group">
                  <CardHeader className="pb-2">
                    <CardTitle className="text-sm flex items-center gap-2">
                      <Icon className={`h-4 w-4 ${feature.color}`} />
                      {feature.name}
                      {feature.experimental && <ExperimentalBadge />}
                    </CardTitle>
                  </CardHeader>
                  <CardContent className="space-y-2">
                    <FeatureStatus
                      featureKey={key}
                      ctxConfig={ctxConfig}
                      memConfig={memConfig}
                    />
                    <Button
                      variant="ghost"
                      size="sm"
                      className="gap-1 -ml-2 text-xs"
                      onClick={() => navigateTo(feature.viewId)}
                    >
                      Configure
                      <ChevronRight className="h-3 w-3" />
                    </Button>
                  </CardContent>
                </Card>
              )
            })}
          </div>
        </TabsContent>

        {/* Settings Tab */}
        <TabsContent value="settings" className="flex-1 min-h-0 mt-4 overflow-y-auto">
          {ctxConfig && (
            <div className="space-y-4 max-w-2xl">
              {/* Vector Search */}
              <Card>
                <CardHeader className="pb-3">
                  <CardTitle className="text-sm">Semantic Vector Search</CardTitle>
                  <CardDescription>
                    When enabled and the embedding model is downloaded, all content stores use hybrid
                    FTS5 + vector search. Disable to use FTS5 keyword search only.
                  </CardDescription>
                </CardHeader>
                <CardContent>
                  <div className="flex items-center gap-3">
                    <button
                      onClick={() => updateField("vectorSearchEnabled", !ctxConfig.vector_search_enabled)}
                      className={`relative inline-flex h-5 w-9 items-center rounded-full transition-colors ${
                        ctxConfig.vector_search_enabled ? "bg-primary" : "bg-muted-foreground/30"
                      }`}
                    >
                      <span
                        className={`inline-block h-3.5 w-3.5 transform rounded-full bg-background transition-transform ${
                          ctxConfig.vector_search_enabled ? "translate-x-[18px]" : "translate-x-[3px]"
                        }`}
                      />
                    </button>
                    <span className="text-sm">
                      {ctxConfig.vector_search_enabled ? "Enabled" : "Disabled"}
                    </span>
                    {ctxConfig.vector_search_enabled && !embeddingStatus?.downloaded && (
                      <span className="text-xs text-muted-foreground">
                        (embedding model not yet downloaded)
                      </span>
                    )}
                  </div>
                </CardContent>
              </Card>
            </div>
          )}
        </TabsContent>
      </Tabs>
    </div>
  )
}

function FeatureStatus({
  featureKey,
  ctxConfig,
  memConfig,
}: {
  featureKey: string
  ctxConfig: ContextManagementConfig | null
  memConfig: MemoryConfig | null
}) {
  switch (featureKey) {
    case 'catalogCompression':
      return (
        <p className="text-xs text-muted-foreground">
          {ctxConfig?.catalog_compression
            ? `Enabled — catalog deferred above ${ctxConfig.catalog_threshold_bytes.toLocaleString()} bytes`
            : "Disabled"}
        </p>
      )
    case 'responseRag':
      return (
        <p className="text-xs text-muted-foreground">
          {ctxConfig
            ? `Response threshold: ${ctxConfig.response_threshold_bytes.toLocaleString()} bytes`
            : "Loading..."}
        </p>
      )
    case 'memory':
      return (
        <p className="text-xs text-muted-foreground">
          {memConfig?.compaction_model
            ? `Compaction: ${memConfig.compaction_model}`
            : "Compaction disabled (raw transcripts)"}
          {" — "}search top-k: {memConfig?.search_top_k ?? 5}
        </p>
      )
    default:
      return null
  }
}
