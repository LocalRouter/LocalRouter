import { useState, useEffect } from "react"
import { invoke } from "@tauri-apps/api/core"
import { toast } from "sonner"
import { CheckCircle2, ChevronRight, Download, Loader2 } from "lucide-react"
import { FEATURES, INDEXING_CHILDREN } from "@/constants/features"
import { ExperimentalBadge } from "@/components/shared/ExperimentalBadge"
import { Button } from "@/components/ui/Button"
import { Card, CardContent, CardHeader, CardTitle, CardDescription } from "@/components/ui/Card"
import type { EmbeddingStatus, ContextManagementConfig, MemoryConfig } from "@/types/tauri-commands"

interface IndexingViewProps {
  activeSubTab?: string | null
  onTabChange?: (view: string, subTab?: string | null) => void
}

export function IndexingView({ onTabChange }: IndexingViewProps) {
  const [embeddingStatus, setEmbeddingStatus] = useState<EmbeddingStatus | null>(null)
  const [isDownloading, setIsDownloading] = useState(false)
  const [ctxConfig, setCtxConfig] = useState<ContextManagementConfig | null>(null)
  const [memConfig, setMemConfig] = useState<MemoryConfig | null>(null)

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

  const navigateTo = (viewId: string) => {
    onTabChange?.(viewId, null)
  }

  return (
    <div className="flex flex-col h-full min-h-0 gap-4 max-w-5xl overflow-y-auto">
      <div className="flex-shrink-0">
        <h1 className="text-2xl font-bold tracking-tight flex items-center gap-2">
          <FEATURES.indexing.icon className={`h-6 w-6 ${FEATURES.indexing.color}`} />
          Indexing
        </h1>
        <p className="text-sm text-muted-foreground">
          FTS5 search engine powering MCP Catalog Indexing, Tool Responses Indexing, and Conversation Memory
        </p>
      </div>

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
