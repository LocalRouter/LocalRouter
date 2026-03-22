import { useState, useEffect, useCallback } from "react"
import { invoke } from "@tauri-apps/api/core"
import { listenSafe } from "@/hooks/useTauriListener"
import { toast } from "sonner"
import { Search, BookOpen, Loader2, FolderOpen, RefreshCw, Trash2, Archive, RotateCcw, Sparkles } from "lucide-react"
import { Button } from "@/components/ui/Button"
import { Card, CardContent, CardHeader, CardTitle, CardDescription } from "@/components/ui/Card"
import { Input } from "@/components/ui/Input"
import { Progress } from "@/components/ui/progress"
import { ScrollArea } from "@/components/ui/scroll-area"
import {
  ResizablePanelGroup,
  ResizablePanel,
  ResizableHandle,
} from "@/components/ui/resizable"
import {
  AlertDialog,
  AlertDialogAction,
  AlertDialogCancel,
  AlertDialogContent,
  AlertDialogDescription,
  AlertDialogFooter,
  AlertDialogHeader,
  AlertDialogTitle,
  AlertDialogTrigger,
} from "@/components/ui/alert-dialog"
import { cn } from "@/lib/utils"
import { InfoTooltip } from "@/components/ui/info-tooltip"
import type {
  MemoryClientInfo,
  RagSearchResult,
  RagReadResult,
  CompactionStatsResult,
  MemoryCompactProgress,
  MemoryCompactComplete,
  MemoryRecompactProgress,
  MemoryRecompactComplete,
  MemoryReindexProgress,
  MemoryReindexComplete,
} from "@/types/tauri-commands"

interface MemorySessionsTabProps {}

export function MemorySessionsTab({}: MemorySessionsTabProps) {
  const [clients, setClients] = useState<MemoryClientInfo[]>([])
  const [loading, setLoading] = useState(true)
  const [selectedClientId, setSelectedClientId] = useState<string | null>(null)

  // Compaction state
  const [compactionStats, setCompactionStats] = useState<CompactionStatsResult | null>(null)
  const [loadingStats, setLoadingStats] = useState(false)
  const [compacting, setCompacting] = useState(false)
  const [compactProgress, setCompactProgress] = useState<{ current: number; total: number } | null>(null)
  const [recompacting, setRecompacting] = useState(false)
  const [recompactProgress, setRecompactProgress] = useState<{ current: number; total: number } | null>(null)
  const [reindexing, setReindexing] = useState(false)
  const [reindexProgress, setReindexProgress] = useState<{ current: number; total: number } | null>(null)

  // Search state
  const [searchQuery, setSearchQuery] = useState("")
  const [searchResults, setSearchResults] = useState<RagSearchResult[] | null>(null)
  const [searching, setSearching] = useState(false)

  // Read state
  const [readLabel, setReadLabel] = useState("")
  const [readOffset, setReadOffset] = useState("")
  const [readLimit, setReadLimit] = useState("")
  const [readResult, setReadResult] = useState<RagReadResult | null>(null)
  const [reading, setReading] = useState(false)

  // Clear state
  const [clearing, setClearing] = useState(false)

  const selectedClient = clients.find((c) => c.client_id === selectedClientId) ?? null

  useEffect(() => {
    loadClients()
  }, [])

  // Reset search/read results when client changes
  useEffect(() => {
    setSearchQuery("")
    setSearchResults(null)
    setReadLabel("")
    setReadOffset("")
    setReadLimit("")
    setReadResult(null)
    setCompactionStats(null)
    if (selectedClientId) {
      loadCompactionStats(selectedClientId)
    }
  }, [selectedClientId])

  // Listen for compact progress events
  useEffect(() => {
    const compactProgressListener = listenSafe<MemoryCompactProgress>("memory-compact-progress", (e) => {
      if (e.payload.client_id === selectedClientId) {
        setCompactProgress({ current: e.payload.current, total: e.payload.total })
      }
    })
    const compactCompleteListener = listenSafe<MemoryCompactComplete>("memory-compact-complete", (e) => {
      if (e.payload.client_id === selectedClientId) {
        setCompacting(false)
        setCompactProgress(null)
        const { archived_count, summarized_count } = e.payload
        if (summarized_count > 0) {
          toast.success(`Compacted ${archived_count} session${archived_count !== 1 ? "s" : ""} (${summarized_count} summarized)`)
        } else {
          toast.success(`Archived ${archived_count} session${archived_count !== 1 ? "s" : ""}`)
        }
        loadCompactionStats(e.payload.client_id)
        loadClients()
      }
    })
    const compactFailedListener = listenSafe<{ client_id: string; error: string }>("memory-compact-failed", (e) => {
      if (e.payload.client_id === selectedClientId) {
        setCompacting(false)
        setCompactProgress(null)
        toast.error(`Compaction failed: ${e.payload.error}`)
      }
    })

    // Listen for recompact progress events
    const recompactProgressListener = listenSafe<MemoryRecompactProgress>("memory-recompact-progress", (e) => {
      if (e.payload.client_id === selectedClientId) {
        setRecompactProgress({ current: e.payload.current, total: e.payload.total })
      }
    })
    const recompactCompleteListener = listenSafe<MemoryRecompactComplete>("memory-recompact-complete", (e) => {
      if (e.payload.client_id === selectedClientId) {
        setRecompacting(false)
        setRecompactProgress(null)
        toast.success(`Re-compacted ${e.payload.recompacted_count} session${e.payload.recompacted_count !== 1 ? "s" : ""}`)
        loadCompactionStats(e.payload.client_id)
        loadClients()
      }
    })
    const recompactFailedListener = listenSafe<{ client_id: string; error: string }>("memory-recompact-failed", (e) => {
      if (e.payload.client_id === selectedClientId) {
        setRecompacting(false)
        setRecompactProgress(null)
        toast.error(`Re-compaction failed: ${e.payload.error}`)
      }
    })

    // Listen for reindex progress events
    const reindexProgressListener = listenSafe<MemoryReindexProgress>("memory-reindex-progress", (e) => {
      if (e.payload.client_id === selectedClientId) {
        setReindexProgress({ current: e.payload.current, total: e.payload.total })
      }
    })
    const reindexCompleteListener = listenSafe<MemoryReindexComplete>("memory-reindex-complete", (e) => {
      if (e.payload.client_id === selectedClientId) {
        setReindexing(false)
        setReindexProgress(null)
        toast.success(`Index rebuilt: ${e.payload.indexed_count} file${e.payload.indexed_count !== 1 ? "s" : ""} indexed`)
        loadCompactionStats(e.payload.client_id)
        loadClients()
      }
    })
    const reindexFailedListener = listenSafe<{ client_id: string; error: string }>("memory-reindex-failed", (e) => {
      if (e.payload.client_id === selectedClientId) {
        setReindexing(false)
        setReindexProgress(null)
        toast.error(`Reindex failed: ${e.payload.error}`)
      }
    })

    return () => {
      compactProgressListener.cleanup()
      compactCompleteListener.cleanup()
      compactFailedListener.cleanup()
      recompactProgressListener.cleanup()
      recompactCompleteListener.cleanup()
      recompactFailedListener.cleanup()
      reindexProgressListener.cleanup()
      reindexCompleteListener.cleanup()
      reindexFailedListener.cleanup()
    }
  }, [selectedClientId])

  const loadClients = useCallback(async () => {
    try {
      const result = await invoke<MemoryClientInfo[]>("list_memory_clients")
      setClients(result)
    } catch (error) {
      console.error("Failed to load memory clients:", error)
      toast.error("Failed to load memory clients")
    } finally {
      setLoading(false)
    }
  }, [])

  const loadCompactionStats = useCallback(async (clientId: string) => {
    setLoadingStats(true)
    try {
      const stats = await invoke<CompactionStatsResult>("get_memory_compaction_stats", { clientId })
      setCompactionStats(stats)
    } catch (error) {
      console.error("Failed to load compaction stats:", error)
    } finally {
      setLoadingStats(false)
    }
  }, [])

  const handleOpenFolder = async () => {
    if (!selectedClientId) return
    try {
      await invoke("open_client_memory_folder", { clientId: selectedClientId })
    } catch (error: any) {
      toast.error(`Failed to open folder: ${error.message || error}`)
    }
  }

  const handleForceCompact = async () => {
    if (!selectedClientId) return
    setCompacting(true)
    setCompactProgress(null)
    try {
      await invoke("force_compact_memory", { clientId: selectedClientId })
    } catch (error: any) {
      setCompacting(false)
      setCompactProgress(null)
      toast.error(`Compaction failed: ${error.message || error}`)
    }
  }

  const handleRecompact = async () => {
    if (!selectedClientId) return
    setRecompacting(true)
    setRecompactProgress(null)
    try {
      await invoke("recompact_memory", { clientId: selectedClientId })
    } catch (error: any) {
      setRecompacting(false)
      setRecompactProgress(null)
      toast.error(`Re-compaction failed: ${error.message || error}`)
    }
  }

  const handleReindex = async () => {
    if (!selectedClientId) return
    setReindexing(true)
    setReindexProgress(null)
    try {
      await invoke("reindex_client_memory", { clientId: selectedClientId })
    } catch (error: any) {
      setReindexing(false)
      toast.error(`Reindex failed: ${error.message || error}`)
    }
  }

  const handleSearch = useCallback(async () => {
    if (!selectedClientId || !searchQuery.trim()) return
    setSearching(true)
    try {
      const results = await invoke<RagSearchResult[]>("search_client_memory", {
        clientId: selectedClientId,
        query: searchQuery,
        limit: 5,
      })
      setSearchResults(results)
    } catch (error: any) {
      toast.error(`Search failed: ${error.message || error}`)
    } finally {
      setSearching(false)
    }
  }, [selectedClientId, searchQuery])

  const handleRead = useCallback(async () => {
    if (!selectedClientId || !readLabel.trim()) return
    setReading(true)
    try {
      const result = await invoke<RagReadResult>("read_client_memory", {
        clientId: selectedClientId,
        label: readLabel,
        offset: readOffset || null,
        limit: readLimit ? parseInt(readLimit) : null,
      })
      setReadResult(result)
    } catch (error: any) {
      toast.error(`Read failed: ${error.message || error}`)
    } finally {
      setReading(false)
    }
  }, [selectedClientId, readLabel, readOffset, readLimit])

  const handleClear = async () => {
    if (!selectedClientId) return
    setClearing(true)
    try {
      await invoke("clear_client_memory", { clientId: selectedClientId })
      toast.success("Memory cleared")
      setSelectedClientId(null)
      await loadClients()
    } catch (error: any) {
      toast.error(`Failed to clear memory: ${error.message || error}`)
    } finally {
      setClearing(false)
    }
  }

  const handleRefresh = async () => {
    setLoading(true)
    await loadClients()
    if (selectedClientId) {
      await loadCompactionStats(selectedClientId)
    }
  }

  return (
    <ResizablePanelGroup direction="horizontal" className="flex-1 min-h-0 rounded-lg border">
      {/* Left sidebar: client list */}
      <ResizablePanel defaultSize={25} minSize={18}>
        <div className="flex flex-col h-full">
          <div className="p-4 border-b">
            <p className="text-sm font-medium">Memory Clients</p>
          </div>
          <ScrollArea className="flex-1">
            <div className="p-2 space-y-1">
              {loading ? (
                <p className="text-sm text-muted-foreground p-4">Loading...</p>
              ) : clients.length === 0 ? (
                <div className="p-4 text-center">
                  <p className="text-sm text-muted-foreground">
                    No clients have memory enabled.
                  </p>
                  <p className="text-xs text-muted-foreground mt-1">
                    Enable memory in a client&apos;s Optimize tab.
                  </p>
                </div>
              ) : (
                clients.map((client) => (
                  <div
                    key={client.client_id}
                    onClick={() => setSelectedClientId(client.client_id)}
                    className={cn(
                      "flex items-center gap-3 p-3 rounded-md cursor-pointer",
                      selectedClientId === client.client_id
                        ? "bg-accent"
                        : "hover:bg-muted"
                    )}
                  >
                    <div className="flex-1 min-w-0">
                      <p className="font-medium truncate">{client.client_name}</p>
                      <p className="text-xs text-muted-foreground">
                        {client.source_count} source{client.source_count !== 1 ? "s" : ""} &middot; {client.total_lines.toLocaleString()} line{client.total_lines !== 1 ? "s" : ""}
                      </p>
                    </div>
                  </div>
                ))
              )}
            </div>
          </ScrollArea>
        </div>
      </ResizablePanel>

      <ResizableHandle withHandle />

      {/* Right detail panel */}
      <ResizablePanel defaultSize={75}>
        {selectedClient ? (
          <ScrollArea className="h-full">
            <div className="p-6 space-y-6">
              {/* Card 1: Client Info */}
              <Card>
                <CardHeader className="pb-3">
                  <CardTitle className="text-base">{selectedClient.client_name}</CardTitle>
                  <CardDescription>
                    {selectedClient.source_count} session{selectedClient.source_count !== 1 ? "s" : ""} &middot; {selectedClient.total_lines.toLocaleString()} total line{selectedClient.total_lines !== 1 ? "s" : ""}
                  </CardDescription>
                </CardHeader>
                <CardContent>
                  <div className="flex items-center gap-2">
                    <Button variant="outline" size="sm" onClick={handleOpenFolder}>
                      <FolderOpen className="h-3.5 w-3.5 mr-1" />
                      Open Folder
                    </Button>
                    <Button
                      variant="outline"
                      size="sm"
                      onClick={handleRefresh}
                    >
                      <RefreshCw className={cn("h-3.5 w-3.5 mr-1", loading && "animate-spin")} />
                      Refresh
                    </Button>
                  </div>
                </CardContent>
              </Card>

              {/* Card 2: Compaction Status */}
              <Card>
                <CardHeader className="pb-3">
                  <CardTitle className="text-base flex items-center gap-2">
                    <Archive className="h-4 w-4" />
                    Compaction Status
                    <InfoTooltip content="Session lifecycle management. Active sessions are recording, pending sessions await compaction, archived sessions have been processed, and summarized sessions have LLM-generated summaries." />
                  </CardTitle>
                  <CardDescription>
                    Session lifecycle and index statistics
                  </CardDescription>
                </CardHeader>
                <CardContent className="space-y-4">
                  {loadingStats ? (
                    <div className="flex items-center gap-2 text-sm text-muted-foreground">
                      <Loader2 className="h-3.5 w-3.5 animate-spin" />
                      Loading stats...
                    </div>
                  ) : compactionStats ? (
                    <>
                      <div className="grid grid-cols-2 gap-3 sm:grid-cols-5">
                        <div className="rounded-md border p-3">
                          <p className="text-xs text-muted-foreground">Active</p>
                          <p className="text-lg font-semibold text-green-600 dark:text-green-400">
                            {compactionStats.active_sessions}
                          </p>
                        </div>
                        <div className="rounded-md border p-3">
                          <p className="text-xs text-muted-foreground">Pending</p>
                          <p className={cn(
                            "text-lg font-semibold",
                            compactionStats.pending_compaction > 0
                              ? "text-amber-600 dark:text-amber-400"
                              : "text-muted-foreground"
                          )}>
                            {compactionStats.pending_compaction}
                          </p>
                        </div>
                        <div className="rounded-md border p-3">
                          <p className="text-xs text-muted-foreground">Archived</p>
                          <p className="text-lg font-semibold text-blue-600 dark:text-blue-400">
                            {compactionStats.archived_sessions}
                          </p>
                        </div>
                        <div className="rounded-md border p-3">
                          <p className="text-xs text-muted-foreground">Summarized</p>
                          <p className={cn(
                            "text-lg font-semibold",
                            compactionStats.summarized_sessions > 0
                              ? "text-purple-600 dark:text-purple-400"
                              : "text-muted-foreground"
                          )}>
                            {compactionStats.summarized_sessions}
                          </p>
                        </div>
                        <div className="rounded-md border p-3">
                          <p className="text-xs text-muted-foreground">Indexed</p>
                          <p className="text-lg font-semibold">
                            {compactionStats.indexed_sources}
                          </p>
                          <p className="text-[10px] text-muted-foreground">
                            {compactionStats.total_lines.toLocaleString()} lines
                          </p>
                        </div>
                      </div>

                      <div className="flex items-center gap-2 flex-wrap">
                        <AlertDialog>
                          <AlertDialogTrigger asChild>
                            <Button
                              variant="outline"
                              size="sm"
                              disabled={compacting || compactionStats.pending_compaction === 0}
                            >
                              {compacting ? (
                                <Loader2 className="h-3.5 w-3.5 animate-spin mr-1" />
                              ) : (
                                <Archive className="h-3.5 w-3.5 mr-1" />
                              )}
                              Compact Now
                            </Button>
                          </AlertDialogTrigger>
                          <AlertDialogContent>
                            <AlertDialogHeader>
                              <AlertDialogTitle>Compact sessions?</AlertDialogTitle>
                              <AlertDialogDescription>
                                This will archive and summarize {compactionStats.pending_compaction} expired session{compactionStats.pending_compaction !== 1 ? "s" : ""} using
                                the configured compaction model. Raw transcripts are preserved for re-compaction.
                              </AlertDialogDescription>
                            </AlertDialogHeader>
                            <AlertDialogFooter>
                              <AlertDialogCancel>Cancel</AlertDialogCancel>
                              <AlertDialogAction onClick={handleForceCompact}>
                                Compact
                              </AlertDialogAction>
                            </AlertDialogFooter>
                          </AlertDialogContent>
                        </AlertDialog>

                        <AlertDialog>
                          <AlertDialogTrigger asChild>
                            <Button
                              variant="outline"
                              size="sm"
                              disabled={recompacting || compactionStats.archived_sessions === 0}
                            >
                              {recompacting ? (
                                <Loader2 className="h-3.5 w-3.5 animate-spin mr-1" />
                              ) : (
                                <Sparkles className="h-3.5 w-3.5 mr-1" />
                              )}
                              Re-compact
                            </Button>
                          </AlertDialogTrigger>
                          <AlertDialogContent>
                            <AlertDialogHeader>
                              <AlertDialogTitle>Re-compact archived sessions?</AlertDialogTitle>
                              <AlertDialogDescription>
                                This will re-run LLM summarization on {compactionStats.archived_sessions} archived session{compactionStats.archived_sessions !== 1 ? "s" : ""} using
                                the current compaction model. Existing summaries will be overwritten.
                              </AlertDialogDescription>
                            </AlertDialogHeader>
                            <AlertDialogFooter>
                              <AlertDialogCancel>Cancel</AlertDialogCancel>
                              <AlertDialogAction onClick={handleRecompact}>
                                Re-compact
                              </AlertDialogAction>
                            </AlertDialogFooter>
                          </AlertDialogContent>
                        </AlertDialog>

                        <AlertDialog>
                          <AlertDialogTrigger asChild>
                            <Button
                              variant="outline"
                              size="sm"
                              disabled={reindexing}
                            >
                              {reindexing ? (
                                <Loader2 className="h-3.5 w-3.5 animate-spin mr-1" />
                              ) : (
                                <RotateCcw className="h-3.5 w-3.5 mr-1" />
                              )}
                              Rebuild Index
                            </Button>
                          </AlertDialogTrigger>
                          <AlertDialogContent>
                            <AlertDialogHeader>
                              <AlertDialogTitle>Rebuild FTS5 index?</AlertDialogTitle>
                              <AlertDialogDescription>
                                This will delete and rebuild the search index from all session and
                                archive files on disk. Summaries will be indexed instead of raw transcripts
                                where available.
                              </AlertDialogDescription>
                            </AlertDialogHeader>
                            <AlertDialogFooter>
                              <AlertDialogCancel>Cancel</AlertDialogCancel>
                              <AlertDialogAction onClick={handleReindex}>
                                Rebuild
                              </AlertDialogAction>
                            </AlertDialogFooter>
                          </AlertDialogContent>
                        </AlertDialog>
                      </div>

                      {compacting && compactProgress && (
                        <div className="space-y-1.5">
                          <div className="flex justify-between text-xs text-muted-foreground">
                            <span>Compacting...</span>
                            <span>{compactProgress.current}/{compactProgress.total} sessions</span>
                          </div>
                          <Progress
                            value={compactProgress.total > 0 ? (compactProgress.current / compactProgress.total) * 100 : 0}
                          />
                        </div>
                      )}

                      {recompacting && recompactProgress && (
                        <div className="space-y-1.5">
                          <div className="flex justify-between text-xs text-muted-foreground">
                            <span>Re-compacting...</span>
                            <span>{recompactProgress.current}/{recompactProgress.total} sessions</span>
                          </div>
                          <Progress
                            value={recompactProgress.total > 0 ? (recompactProgress.current / recompactProgress.total) * 100 : 0}
                          />
                        </div>
                      )}

                      {reindexing && reindexProgress && (
                        <div className="space-y-1.5">
                          <div className="flex justify-between text-xs text-muted-foreground">
                            <span>Reindexing...</span>
                            <span>{reindexProgress.current}/{reindexProgress.total} files</span>
                          </div>
                          <Progress
                            value={reindexProgress.total > 0 ? (reindexProgress.current / reindexProgress.total) * 100 : 0}
                          />
                        </div>
                      )}
                    </>
                  ) : null}
                </CardContent>
              </Card>

              {/* Card 3: Search */}
              <Card>
                <CardHeader className="pb-3">
                  <CardTitle className="text-base flex items-center gap-2">
                    <Search className="h-4 w-4" />
                    Search
                  </CardTitle>
                  <CardDescription>
                    Search this client&apos;s memory using full-text search.
                  </CardDescription>
                </CardHeader>
                <CardContent className="space-y-3">
                  <div className="flex gap-2">
                    <Input
                      placeholder='Search query... (e.g. "rate limiting", "login endpoint")'
                      value={searchQuery}
                      onChange={(e) => setSearchQuery(e.target.value)}
                      onKeyDown={(e) => {
                        if (e.key === "Enter") handleSearch()
                      }}
                      className="flex-1"
                    />
                    <Button
                      size="sm"
                      onClick={handleSearch}
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
                                    [{hi + 1}] {hit.source} &mdash; {hit.title} (lines {hit.line_start}-
                                    {hit.line_end})
                                  </p>
                                  <pre className="text-xs whitespace-pre-wrap font-mono leading-relaxed pl-2 border-l-2 border-border">
                                    {hit.content}
                                  </pre>
                                </div>
                              ))}
                            </div>
                          ))}
                        </div>
                      )}
                    </div>
                  )}
                </CardContent>
              </Card>

              {/* Card 4: Read */}
              <Card>
                <CardHeader className="pb-3">
                  <CardTitle className="text-base flex items-center gap-2">
                    <BookOpen className="h-4 w-4" />
                    Read
                  </CardTitle>
                  <CardDescription>
                    Read the full content of a memory source with pagination.
                  </CardDescription>
                </CardHeader>
                <CardContent className="space-y-3">
                  <div className="flex gap-2 items-center flex-wrap">
                    <div className="flex items-center gap-1.5">
                      <label className="text-sm text-muted-foreground whitespace-nowrap">
                        Source:
                      </label>
                      <Input
                        placeholder='e.g. "session/abc123"'
                        value={readLabel}
                        onChange={(e) => setReadLabel(e.target.value)}
                        className="w-48 h-8 text-sm"
                      />
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
                    <Button
                      size="sm"
                      onClick={handleRead}
                      disabled={reading || !readLabel.trim()}
                    >
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

              {/* Card 5: Danger Zone */}
              <Card className="border-red-200 dark:border-red-900">
                <CardHeader>
                  <CardTitle className="text-red-600 dark:text-red-400">Danger Zone</CardTitle>
                  <CardDescription>
                    Irreversible and destructive actions for this client&apos;s memory
                  </CardDescription>
                </CardHeader>
                <CardContent>
                  <div className="flex items-center justify-between">
                    <div>
                      <p className="text-sm font-medium">Clear memory</p>
                      <p className="text-sm text-muted-foreground">
                        Permanently delete all memory data for &quot;{selectedClient.client_name}&quot;
                      </p>
                    </div>
                    <AlertDialog>
                      <AlertDialogTrigger asChild>
                        <Button variant="destructive" disabled={clearing}>
                          {clearing ? (
                            <>
                              <Loader2 className="h-4 w-4 mr-2 animate-spin" />
                              Clearing...
                            </>
                          ) : (
                            <>
                              <Trash2 className="h-4 w-4 mr-2" />
                              Clear Memory
                            </>
                          )}
                        </Button>
                      </AlertDialogTrigger>
                      <AlertDialogContent>
                        <AlertDialogHeader>
                          <AlertDialogTitle>
                            Clear memory for &quot;{selectedClient.client_name}&quot;?
                          </AlertDialogTitle>
                          <AlertDialogDescription>
                            This will permanently delete all memory sessions and indexed data
                            for this client. This action cannot be undone.
                          </AlertDialogDescription>
                        </AlertDialogHeader>
                        <AlertDialogFooter>
                          <AlertDialogCancel>Cancel</AlertDialogCancel>
                          <AlertDialogAction
                            onClick={handleClear}
                            className="bg-destructive text-destructive-foreground hover:bg-destructive/90"
                          >
                            Clear Memory
                          </AlertDialogAction>
                        </AlertDialogFooter>
                      </AlertDialogContent>
                    </AlertDialog>
                  </div>
                </CardContent>
              </Card>
            </div>
          </ScrollArea>
        ) : (
          <div className="flex flex-col items-center justify-center h-full text-muted-foreground gap-4">
            <BookOpen className="h-12 w-12 opacity-30" />
            <div className="text-center">
              <p className="font-medium">Select a client to view their memories</p>
              <p className="text-sm">
                Choose a memory-enabled client from the list on the left
              </p>
            </div>
          </div>
        )}
      </ResizablePanel>
    </ResizablePanelGroup>
  )
}
