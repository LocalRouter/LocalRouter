import { useState, useEffect, useCallback, useRef } from "react"
import { invoke } from "@tauri-apps/api/core"
import { toast } from "sonner"
import { Search, Loader2, BookOpen } from "lucide-react"
import { Button } from "@/components/ui/Button"
import { Card, CardContent, CardHeader, CardTitle, CardDescription } from "@/components/ui/Card"
import { Input } from "@/components/ui/Input"
import type {
  RagPreviewIndexResult,
  RagSearchResult,
  RagReadResult,
  PreviewRagIndexParams,
  PreviewRagSearchParams,
  PreviewRagReadParams,
} from "@/types/tauri-commands"

interface ContentStorePreviewProps {
  loadSample: () => Promise<string>
  sourceLabel: string
  responseThresholdBytes: number
  searchPlaceholder?: string
  defaultMode?: "index" | "compress"
}

export function ContentStorePreview({
  loadSample,
  sourceLabel,
  responseThresholdBytes,
  searchPlaceholder = 'Search query... (e.g. "rate limiting", "login endpoint")',
  defaultMode = "index",
}: ContentStorePreviewProps) {
  const [content, setContent] = useState("")
  const [indexResult, setIndexResult] = useState<RagPreviewIndexResult | null>(null)
  const [indexing, setIndexing] = useState(false)
  const [mode, setMode] = useState<"index" | "compress">(defaultMode)

  // Search state
  const [searchQuery, setSearchQuery] = useState("")
  const [searchResults, setSearchResults] = useState<RagSearchResult[] | null>(null)
  const [searching, setSearching] = useState(false)

  // Read state
  const [readOffset, setReadOffset] = useState("")
  const [readLimit, setReadLimit] = useState("")
  const [readResult, setReadResult] = useState<RagReadResult | null>(null)
  const [reading, setReading] = useState(false)

  // Track whether sample has been loaded
  const sampleLoaded = useRef(false)

  // Load sample on mount
  useEffect(() => {
    if (sampleLoaded.current) return
    sampleLoaded.current = true

    loadSample()
      .then((sample) => {
        if (sample) setContent(sample)
      })
      .catch((err) => {
        console.error("Failed to load sample:", err)
      })
  }, [loadSample])

  const doIndex = useCallback(
    async (text: string) => {
      if (!text.trim()) return
      setIndexing(true)
      try {
        const result = await invoke<RagPreviewIndexResult>("preview_rag_index", {
          content: text,
          label: sourceLabel,
          responseThresholdBytes,
        } satisfies PreviewRagIndexParams)
        setIndexResult(result)
      } catch (e) {
        toast.error(`Failed to index: ${e}`)
      } finally {
        setIndexing(false)
      }
    },
    [sourceLabel, responseThresholdBytes]
  )

  // Auto-index with 500ms debounce on content change
  useEffect(() => {
    if (!content.trim()) return
    setSearchResults(null)
    setReadResult(null)
    const timer = setTimeout(() => {
      doIndex(content)
    }, 500)
    return () => clearTimeout(timer)
  }, [content, doIndex])

  const doSearch = useCallback(async () => {
    if (!searchQuery.trim()) return
    setSearching(true)
    try {
      const results = await invoke<RagSearchResult[]>("preview_rag_search", {
        query: searchQuery,
        limit: 5,
      } satisfies PreviewRagSearchParams)
      setSearchResults(results)
    } catch (e) {
      toast.error(`Search failed: ${e}`)
    } finally {
      setSearching(false)
    }
  }, [searchQuery])

  const doRead = useCallback(async () => {
    if (!indexResult) return
    setReading(true)
    try {
      const result = await invoke<RagReadResult>("preview_rag_read", {
        label: indexResult.sources[0]?.label ?? sourceLabel,
        offset: readOffset || null,
        limit: readLimit ? parseInt(readLimit) : null,
      } satisfies PreviewRagReadParams)
      setReadResult(result)
    } catch (e) {
      toast.error(`Read failed: ${e}`)
    } finally {
      setReading(false)
    }
  }, [indexResult, sourceLabel, readOffset, readLimit])

  return (
    <div className="space-y-4">
      {/* Card 1: Document Input */}
      <Card>
        <CardHeader>
          <CardTitle className="text-base">Document Input</CardTitle>
          <CardDescription>
            Paste or edit a document to see how it is indexed and compressed. Changes are
            automatically re-indexed.
          </CardDescription>
        </CardHeader>
        <CardContent className="space-y-3">
          <textarea
            value={content}
            onChange={(e) => setContent(e.target.value)}
            className="w-full h-48 rounded-md border border-input bg-background px-3 py-2 text-xs font-mono ring-offset-background focus:outline-none focus:ring-2 focus:ring-ring focus:ring-offset-2 resize-y"
            placeholder="Paste document content here..."
          />
          <div className="flex items-center gap-3 flex-wrap text-xs text-muted-foreground">
            <span>{content.length.toLocaleString()} bytes</span>
            {content.trim() && (
              <span>{content.split("\n").length.toLocaleString()} lines</span>
            )}
            {indexing && (
              <span className="flex items-center gap-1">
                <Loader2 className="h-3 w-3 animate-spin" />
                Indexing...
              </span>
            )}
            {indexResult && !indexing && (
              <>
                <span>
                  {indexResult.index_result.total_chunks} chunks (
                  {indexResult.index_result.code_chunks} code)
                </span>
                <span>{formatBytes(indexResult.index_result.content_bytes)}</span>
              </>
            )}
          </div>
          <div className="flex items-center gap-1 rounded-md border p-0.5 w-fit">
            <button
              type="button"
              onClick={() => setMode("index")}
              className={`px-3 py-1 text-xs rounded transition-colors ${
                mode === "index"
                  ? "bg-primary text-primary-foreground"
                  : "text-muted-foreground hover:text-foreground"
              }`}
            >
              Index
            </button>
            <button
              type="button"
              onClick={() => setMode("compress")}
              className={`px-3 py-1 text-xs rounded transition-colors ${
                mode === "compress"
                  ? "bg-primary text-primary-foreground"
                  : "text-muted-foreground hover:text-foreground"
              }`}
            >
              Compress
            </button>
          </div>
        </CardContent>
      </Card>

      {/* Card 2: Compressed Preview (compress mode only) */}
      {mode === "compress" && indexResult && (
        <Card>
          <CardHeader className="pb-3">
            <CardTitle className="text-base flex items-center justify-between">
              <span>Compressed (what LLM sees)</span>
              <span className="text-xs font-mono font-normal text-muted-foreground">
                {formatBytes(indexResult.compressed_preview.length)}
              </span>
            </CardTitle>
            <CardDescription>
              This is the compressed version that replaces the original{" "}
              {formatBytes(content.length)} response in the LLM&apos;s context window.
            </CardDescription>
          </CardHeader>
          <CardContent>
            <div className="bg-muted/50 rounded-md p-3 max-h-[400px] overflow-y-auto">
              <pre className="text-xs whitespace-pre-wrap font-mono leading-relaxed">
                {indexResult.compressed_preview}
              </pre>
            </div>
          </CardContent>
        </Card>
      )}

      {/* Card 3: Search */}
      {indexResult && (
        <Card>
          <CardHeader className="pb-3">
            <CardTitle className="text-base flex items-center gap-2">
              <Search className="h-4 w-4" />
              IndexSearch
            </CardTitle>
            <CardDescription>
              Search the indexed content using FTS5 full-text search. This is what the LLM calls
              to find relevant sections.
            </CardDescription>
          </CardHeader>
          <CardContent className="space-y-3">
            <div className="flex gap-2">
              <Input
                placeholder={searchPlaceholder}
                value={searchQuery}
                onChange={(e) => setSearchQuery(e.target.value)}
                onKeyDown={(e) => {
                  if (e.key === "Enter") doSearch()
                }}
                className="flex-1"
              />
              <Button
                size="sm"
                onClick={doSearch}
                disabled={searching || !searchQuery.trim()}
              >
                {searching ? (
                  <Loader2 className="h-3.5 w-3.5 animate-spin mr-1" />
                ) : (
                  <Search className="h-3.5 w-3.5 mr-1" />
                )}
                Search
              </Button>
            </div>

            {searchResults !== null && (
              <div className="bg-muted/50 rounded-md p-3 max-h-[400px] overflow-y-auto">
                {searchResults.length === 0 ||
                searchResults.every((r) => r.hits.length === 0) ? (
                  <p className="text-sm text-muted-foreground">No results found.</p>
                ) : (
                  <div className="space-y-3">
                    {searchResults.map((r, ri) => (
                      <div key={ri}>
                        {r.corrected_query && (
                          <p className="text-xs text-muted-foreground italic mb-2">
                            Corrected to: &quot;{r.corrected_query}&quot;
                          </p>
                        )}
                        {r.hits.map((hit, hi) => (
                          <div key={hi} className="mb-3">
                            <p className="text-xs font-semibold mb-1">
                              [{hi + 1}] {hit.source} — {hit.title} (lines {hit.line_start}-
                              {hit.line_end})
                            </p>
                            <pre className="text-xs whitespace-pre-wrap font-mono leading-relaxed pl-2 border-l-2 border-border">
                              {hit.content}
                            </pre>
                          </div>
                        ))}
                        <p className="text-[10px] text-muted-foreground mt-2">
                          Use IndexRead(source=&quot;
                          {r.hits[0]?.source ?? sourceLabel}&quot;, offset, limit) for full
                          context.
                        </p>
                      </div>
                    ))}
                  </div>
                )}
              </div>
            )}
          </CardContent>
        </Card>
      )}

      {/* Card 4: Read */}
      {indexResult && (
        <Card>
          <CardHeader className="pb-3">
            <CardTitle className="text-base flex items-center gap-2">
              <BookOpen className="h-4 w-4" />
              IndexRead
            </CardTitle>
            <CardDescription>
              Read the full indexed content with pagination. The LLM uses this to retrieve
              specific sections after searching.
            </CardDescription>
          </CardHeader>
          <CardContent className="space-y-3">
            <div className="flex gap-2 items-center flex-wrap">
              <div className="flex items-center gap-1.5">
                <label className="text-sm text-muted-foreground whitespace-nowrap">
                  Source:
                </label>
                <code className="text-xs bg-muted px-1.5 py-0.5 rounded">
                  {indexResult.sources[0]?.label ?? sourceLabel}
                </code>
              </div>
              <div className="flex items-center gap-1.5">
                <label className="text-sm text-muted-foreground">Offset:</label>
                <Input
                  placeholder="e.g. 10"
                  value={readOffset}
                  onChange={(e) => setReadOffset(e.target.value)}
                  className="w-20 h-8 text-sm"
                />
              </div>
              <div className="flex items-center gap-1.5">
                <label className="text-sm text-muted-foreground">Limit:</label>
                <Input
                  placeholder="e.g. 50"
                  value={readLimit}
                  onChange={(e) => setReadLimit(e.target.value)}
                  className="w-20 h-8 text-sm"
                />
              </div>
              <Button size="sm" onClick={doRead} disabled={reading}>
                {reading ? (
                  <Loader2 className="h-3.5 w-3.5 animate-spin mr-1" />
                ) : (
                  <BookOpen className="h-3.5 w-3.5 mr-1" />
                )}
                Read
              </Button>
            </div>

            {readResult && (
              <div className="space-y-2">
                <div className="flex gap-3 text-xs text-muted-foreground">
                  <span>
                    Lines {readResult.showing_start}-{readResult.showing_end} of{" "}
                    {readResult.total_lines}
                  </span>
                </div>
                <div className="bg-muted/50 rounded-md p-3 max-h-[400px] overflow-y-auto">
                  <pre className="text-xs whitespace-pre-wrap font-mono leading-relaxed">
                    {readResult.content}
                  </pre>
                </div>
              </div>
            )}
          </CardContent>
        </Card>
      )}
    </div>
  )
}

function formatBytes(bytes: number): string {
  if (bytes === 0) return "0 B"
  if (bytes < 1024) return `${bytes} B`
  const kb = bytes / 1024
  if (kb < 1024) return `${kb.toFixed(kb < 10 ? 1 : 0)} KB`
  const mb = kb / 1024
  return `${mb.toFixed(1)} MB`
}
