import { useState, useEffect, useCallback } from "react"
import { invoke } from "@tauri-apps/api/core"
import { toast } from "sonner"
import { Search, BookOpen, Loader2, FolderOpen, RefreshCw, Trash2 } from "lucide-react"
import { Button } from "@/components/ui/Button"
import { Card, CardContent, CardHeader, CardTitle, CardDescription } from "@/components/ui/Card"
import { Input } from "@/components/ui/Input"
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
import type { MemoryClientInfo, RagSearchResult, RagReadResult } from "@/types/tauri-commands"

interface MemorySessionsTabProps {}

export function MemorySessionsTab({}: MemorySessionsTabProps) {
  const [clients, setClients] = useState<MemoryClientInfo[]>([])
  const [loading, setLoading] = useState(true)
  const [selectedClientId, setSelectedClientId] = useState<string | null>(null)

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

  const handleOpenFolder = async () => {
    if (!selectedClientId) return
    try {
      await invoke("open_client_memory_folder", { clientId: selectedClientId })
    } catch (error: any) {
      toast.error(`Failed to open folder: ${error.message || error}`)
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
                      onClick={() => {
                        setLoading(true)
                        loadClients()
                      }}
                    >
                      <RefreshCw className={cn("h-3.5 w-3.5 mr-1", loading && "animate-spin")} />
                      Refresh
                    </Button>
                  </div>
                </CardContent>
              </Card>

              {/* Card 2: Search */}
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

              {/* Card 3: Read */}
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

              {/* Card 4: Danger Zone */}
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
