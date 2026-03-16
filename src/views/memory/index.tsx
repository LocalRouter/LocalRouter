import { useState, useEffect, useCallback } from "react"
import { invoke } from "@tauri-apps/api/core"
import { listen } from "@tauri-apps/api/event"
import { toast } from "sonner"
import { CheckCircle2, Circle, Download, FolderOpen, Loader2, Play, XCircle } from "lucide-react"
import { FEATURES } from "@/constants/features"
import { TAB_ICONS, TAB_ICON_CLASS } from "@/constants/tab-icons"
import { Badge } from "@/components/ui/Badge"
import { Button } from "@/components/ui/Button"
import { Card, CardContent, CardHeader, CardTitle, CardDescription } from "@/components/ui/Card"
import { Label } from "@/components/ui/label"
import { Input } from "@/components/ui/Input"
import { Tabs, TabsContent, TabsList, TabsTrigger } from "@/components/ui/tabs"
import { Select, SelectContent, SelectGroup, SelectItem, SelectLabel, SelectTrigger, SelectValue } from "@/components/ui/Select"
import { Textarea } from "@/components/ui/textarea"
import { useIncrementalModels } from "@/hooks/useIncrementalModels"
import type { MemoryConfig, MemorySetupProgress, MemoryStatus, UpdateMemoryConfigParams } from "@/types/tauri-commands"

type SetupStepStatus = "idle" | "checking" | "installing" | "ok" | "error"

interface SetupState {
  python: { status: SetupStepStatus; version?: string; error?: string }
  memsearch: { status: SetupStepStatus; version?: string; error?: string }
  model: { status: SetupStepStatus; error?: string }
}

const defaultConfig: MemoryConfig = {
  embedding: { type: "onnx" as const },
  auto_start_daemon: true,
  search_top_k: 5,
  session_inactivity_minutes: 180,
  max_session_minutes: 480,
  recall_tool_name: "MemoryRecall",
  compaction: null,
}

interface MemoryViewProps {
  activeSubTab?: string | null
  onTabChange?: (view: string, subTab?: string | null) => void
}

export function MemoryView({ activeSubTab, onTabChange }: MemoryViewProps) {
  const [config, setConfig] = useState<MemoryConfig>(defaultConfig)
  const [isLoading, setIsLoading] = useState(true)
  const [isSettingUp, setIsSettingUp] = useState(false)
  const [status, setStatus] = useState<MemoryStatus | null>(null)
  const [setup, setSetup] = useState<SetupState>({
    python: { status: "idle" },
    memsearch: { status: "idle" },
    model: { status: "idle" },
  })

  // Try It Out state
  const [searchQuery, setSearchQuery] = useState("What database did we choose for auth?")
  const [searchResults, setSearchResults] = useState<string | null>(null)
  const [searchLoading, setSearchLoading] = useState(false)
  const [indexText, setIndexText] = useState("We decided to use PostgreSQL for the auth service. MySQL had connection pooling issues under load, and PostgreSQL's row-level security features will help with multi-tenant isolation. The migration is planned for next sprint.")
  const [indexLoading, setIndexLoading] = useState(false)
  const [hasIndexed, setHasIndexed] = useState(false)
  const [compactLoading, setCompactLoading] = useState(false)
  const [compactResult, setCompactResult] = useState<string | null>(null)

  // Live models for compaction model picker
  const { models: liveModels } = useIncrementalModels({ refreshOnMount: true })
  // Group models by provider
  const modelsByProvider = liveModels.reduce<Record<string, string[]>>((acc, m) => {
    if (!acc[m.provider]) acc[m.provider] = []
    acc[m.provider].push(m.id)
    return acc
  }, {})

  const tab = activeSubTab || "info"

  const handleTabChange = (newTab: string) => {
    onTabChange?.("memory", newTab)
  }

  useEffect(() => {
    loadConfig()
    loadStatus()
  }, [])

  // Listen for setup progress events
  useEffect(() => {
    const unlisteners: (() => void)[] = []
    listen<MemorySetupProgress>("memory-setup-progress", (event) => {
      const { step, status: stepStatus, version, error } = event.payload
      setSetup((prev) => ({
        ...prev,
        [step]: { status: stepStatus, version, error },
      }))
    }).then((unlisten) => unlisteners.push(unlisten))
    return () => { unlisteners.forEach((fn) => fn()) }
  }, [])

  const loadConfig = async () => {
    try {
      const result = await invoke<MemoryConfig>("get_memory_config")
      setConfig(result)
    } catch (error) {
      console.error("Failed to load memory config:", error)
    } finally {
      setIsLoading(false)
    }
  }

  const loadStatus = async () => {
    try {
      const result = await invoke<MemoryStatus>("get_memory_status")
      setStatus(result)
      setSetup({
        python: { status: result.python_ok ? "ok" : "idle" },
        memsearch: {
          status: result.memsearch_installed ? "ok" : "idle",
          version: result.memsearch_version ?? undefined,
        },
        model: { status: result.model_ready ? "ok" : "idle" },
      })
    } catch (error) {
      console.error("Failed to load memory status:", error)
    }
  }

  const saveConfig = useCallback(async (newConfig: MemoryConfig) => {
    try {
      await invoke("update_memory_config", {
        configJson: JSON.stringify(newConfig),
      } satisfies UpdateMemoryConfigParams)
      setConfig(newConfig)
      toast.success("Memory configuration saved")
    } catch (error: any) {
      toast.error(`Failed to save: ${error.message || error}`)
    }
  }, [])

  const runSetup = async () => {
    setIsSettingUp(true)
    try {
      await invoke("memory_setup")
      toast.success("Memory setup complete")
      loadStatus()
    } catch (error: any) {
      toast.error(`Setup failed: ${error.message || error}`)
    } finally {
      setIsSettingUp(false)
    }
  }

  const openMemoryFolder = async () => {
    try {
      await invoke("open_memory_folder")
    } catch (error: any) {
      toast.error(`Failed to open folder: ${error.message || error}`)
    }
  }

  const renderStepIcon = (stepStatus: SetupStepStatus) => {
    switch (stepStatus) {
      case "ok":
        return <CheckCircle2 className="h-4 w-4 text-green-600 dark:text-green-400 shrink-0" />
      case "error":
        return <XCircle className="h-4 w-4 text-destructive shrink-0" />
      case "checking":
      case "installing":
        return <Loader2 className="h-4 w-4 animate-spin text-blue-500 shrink-0" />
      default:
        return <Circle className="h-4 w-4 text-muted-foreground shrink-0" />
    }
  }

  if (isLoading) {
    return (
      <div className="flex items-center justify-center p-8">
        <Loader2 className="h-6 w-6 animate-spin" />
      </div>
    )
  }

  return (
    <div className="flex flex-col h-full min-h-0 gap-4 max-w-5xl">
      <div className="flex-shrink-0">
        <h1 className="text-2xl font-bold tracking-tight flex items-center gap-2">
          <FEATURES.memory.icon className={`h-6 w-6 ${FEATURES.memory.color}`} />
          Memory
        </h1>
        <p className="text-sm text-muted-foreground">
          Persistent conversation memory for LLM sessions powered by Zillis memsearch
        </p>
      </div>

      <Tabs
        value={tab}
        onValueChange={handleTabChange}
        className="flex flex-col flex-1 min-h-0"
      >
        <TabsList className="flex-shrink-0 w-fit">
          <TabsTrigger value="info"><TAB_ICONS.info className={TAB_ICON_CLASS} />Info</TabsTrigger>
          <TabsTrigger value="try-it-out"><TAB_ICONS.tryItOut className={TAB_ICON_CLASS} />Try It Out</TabsTrigger>
          <TabsTrigger value="settings"><TAB_ICONS.settings className={TAB_ICON_CLASS} />Settings</TabsTrigger>
        </TabsList>

        {/* ================================================================ */}
        {/* Info Tab                                                         */}
        {/* ================================================================ */}
        <TabsContent value="info" className="flex-1 min-h-0 mt-4 overflow-y-auto">
          <div className="space-y-4 max-w-2xl">
            {/* Privacy Warning */}
            <Card className="border-orange-600/50 bg-orange-500/5">
              <CardHeader className="pb-3">
                <CardTitle className="text-sm text-orange-900 dark:text-orange-400">
                  Privacy Notice
                </CardTitle>
              </CardHeader>
              <CardContent className="space-y-2 text-sm text-muted-foreground">
                <p>
                  When memory is enabled for a client, <strong>full conversations are recorded</strong> and
                  stored locally as markdown files. This includes all user messages and assistant responses.
                </p>
                <ul className="list-disc list-inside space-y-1 ml-2">
                  <li>Memory is <strong>not enabled by default</strong> &mdash; each client must opt in individually</li>
                  <li>All data stays local &mdash; stored in the LocalRouter config directory</li>
                  <li>Transcripts are plain-text markdown files you can review, edit, or delete at any time</li>
                  <li>The vector index is a derived cache that can be rebuilt from markdown files</li>
                </ul>
                <button
                  onClick={openMemoryFolder}
                  className="text-xs text-orange-600 dark:text-orange-500 hover:underline flex items-center gap-1 mt-1"
                >
                  <FolderOpen className="h-3 w-3" />
                  Open memory folder
                </button>
              </CardContent>
            </Card>

            {/* Setup & Requirements */}
            <Card>
              <CardHeader className="pb-3">
                <CardTitle className="text-sm">Setup</CardTitle>
                <CardDescription>
                  Memory requires Python 3 and the memsearch CLI with its built-in ONNX embedding model
                </CardDescription>
              </CardHeader>
              <CardContent className="space-y-4">
                {/* 3-step checklist */}
                <div className="space-y-2.5">
                  <div className="flex items-center gap-2.5 text-sm">
                    {renderStepIcon(setup.python.status)}
                    <div className="flex-1 min-w-0">
                      <span className="font-medium">Python 3</span>
                      {setup.python.version && (
                        <span className="text-xs text-muted-foreground ml-2">{setup.python.version}</span>
                      )}
                    </div>
                    {setup.python.error && (
                      <span className="text-xs text-destructive truncate max-w-[250px]">{setup.python.error}</span>
                    )}
                  </div>

                  <div className="flex items-center gap-2.5 text-sm">
                    {renderStepIcon(setup.memsearch.status)}
                    <div className="flex-1 min-w-0">
                      <span className="font-medium">memsearch CLI</span>
                      {setup.memsearch.version && (
                        <Badge variant="secondary" className="text-[10px] px-1 py-0 ml-2">
                          {setup.memsearch.version}
                        </Badge>
                      )}
                    </div>
                    {setup.memsearch.error && (
                      <span className="text-xs text-destructive truncate max-w-[250px]">{setup.memsearch.error}</span>
                    )}
                  </div>

                  <div className="flex items-center gap-2.5 text-sm">
                    {renderStepIcon(setup.model.status)}
                    <div className="flex-1 min-w-0">
                      <span className="font-medium">Embedding model</span>
                      <Badge variant="secondary" className="text-[10px] px-1 py-0 ml-2">
                        ONNX bge-m3 int8
                      </Badge>
                    </div>
                    {setup.model.status === "installing" && (
                      <span className="text-xs text-muted-foreground">Downloading...</span>
                    )}
                    {setup.model.error && (
                      <span className="text-xs text-destructive truncate max-w-[250px]">{setup.model.error}</span>
                    )}
                  </div>
                </div>

                <Button
                  size="sm"
                  onClick={runSetup}
                  disabled={isSettingUp}
                >
                  {isSettingUp ? (
                    <>
                      <Loader2 className="h-3.5 w-3.5 mr-1.5 animate-spin" />
                      Setting up...
                    </>
                  ) : (
                    <>
                      <Download className="h-3.5 w-3.5 mr-1.5" />
                      Setup
                    </>
                  )}
                </Button>

                <p className="text-xs text-muted-foreground pt-2 border-t">
                  The ONNX bge-m3 int8 model (~558 MB) is downloaded from HuggingFace on first use.
                  No API key required &mdash; runs locally on CPU.
                </p>
              </CardContent>
            </Card>

            {/* Tool Preview */}
            <Card>
              <CardHeader className="pb-3">
                <CardTitle className="text-sm">Tool Definition</CardTitle>
                <CardDescription>
                  How the {config.recall_tool_name} tool appears to the LLM
                </CardDescription>
              </CardHeader>
              <CardContent>
                <div className="rounded-md border bg-muted/50 p-3 overflow-x-auto">
                  <pre className="text-xs font-mono whitespace-pre">{JSON.stringify({
                    type: "function",
                    function: {
                      name: config.recall_tool_name,
                      description: "Search past conversation memories for relevant context. Use when the current conversation would benefit from information discussed in previous sessions.",
                      parameters: {
                        type: "object",
                        properties: {
                          query: {
                            type: "string",
                            description: "Search query describing what to recall"
                          }
                        },
                        required: ["query"]
                      }
                    }
                  }, null, 2)}</pre>
                </div>
              </CardContent>
            </Card>

            {/* How it works */}
            <Card>
              <CardHeader className="pb-3">
                <CardTitle className="text-sm">How It Works</CardTitle>
              </CardHeader>
              <CardContent className="space-y-2 text-sm text-muted-foreground">
                <p>
                  Memory automatically captures conversation exchanges when enabled for a client.
                  Conversations are grouped into <strong>sessions</strong> (bounded by inactivity timeout or max duration).
                </p>
                <ol className="list-decimal list-inside space-y-1 ml-2">
                  <li>Each user/assistant exchange is appended to a session markdown file</li>
                  <li>A background <code>memsearch watch</code> daemon auto-indexes changes within ~1.5s</li>
                  <li>The LLM can search past conversations using the <strong>{config.recall_tool_name}</strong> tool</li>
                  <li>When a session ends, optional compaction summarizes it using an LLM</li>
                </ol>
                <p className="text-xs mt-2">
                  Enable memory per-client in the client&apos;s Optimize tab. Each client&apos;s memories are isolated.
                </p>
              </CardContent>
            </Card>
          </div>
        </TabsContent>

        {/* ================================================================ */}
        {/* Try It Out Tab                                                   */}
        {/* ================================================================ */}
        <TabsContent value="try-it-out" className="flex-1 min-h-0 mt-4 overflow-y-auto">
          <div className="space-y-4 max-w-2xl">
            {/* Index some content */}
            <Card>
              <CardHeader className="pb-3">
                <CardTitle className="text-sm">1. Index Content</CardTitle>
                <CardDescription>
                  Write a memory note and index it so you can search for it
                </CardDescription>
              </CardHeader>
              <CardContent className="space-y-3">
                <Textarea
                  value={indexText}
                  onChange={(e) => setIndexText(e.target.value)}
                  placeholder="Type a memory note to index, e.g.: 'We decided to use PostgreSQL for the auth service because MySQL had connection pooling issues.'"
                  className="min-h-[80px] text-sm"
                />
                <Button
                  size="sm"
                  disabled={indexLoading || !indexText.trim() || !status?.memsearch_installed}
                  onClick={async () => {
                    setIndexLoading(true)
                    try {
                      await invoke("memory_test_index", { content: indexText })
                      toast.success("Content indexed")
                      setHasIndexed(true)
                    } catch (err: any) {
                      toast.error(`Index failed: ${err.message || err}`)
                    } finally {
                      setIndexLoading(false)
                    }
                  }}
                >
                  {indexLoading ? (
                    <><Loader2 className="h-3.5 w-3.5 mr-1.5 animate-spin" />Indexing...</>
                  ) : (
                    "Index"
                  )}
                </Button>
                {!status?.memsearch_installed && (
                  <p className="text-xs text-muted-foreground">Run Setup first to enable indexing.</p>
                )}
              </CardContent>
            </Card>

            {/* Search */}
            <Card>
              <CardHeader className="pb-3">
                <CardTitle className="text-sm">2. Search Memories</CardTitle>
                <CardDescription>
                  Search for previously indexed memories using semantic search
                </CardDescription>
              </CardHeader>
              <CardContent className="space-y-3">
                <div className="flex gap-2">
                  <Input
                    value={searchQuery}
                    onChange={(e) => setSearchQuery(e.target.value)}
                    placeholder="What database did we choose?"
                    className="h-8 text-sm flex-1"
                    onKeyDown={(e) => {
                      if (e.key === "Enter" && searchQuery.trim()) {
                        runSearch()
                      }
                    }}
                  />
                  <Button
                    size="sm"
                    disabled={searchLoading || !searchQuery.trim() || !hasIndexed}
                    onClick={runSearch}
                  >
                    {searchLoading ? (
                      <Loader2 className="h-3.5 w-3.5 animate-spin" />
                    ) : (
                      <><Play className="h-3.5 w-3.5 mr-1" />Search</>
                    )}
                  </Button>
                </div>
                {searchResults !== null && (
                  <div className="rounded-md border p-3 bg-muted/50">
                    <pre className="text-xs whitespace-pre-wrap font-mono">{searchResults}</pre>
                  </div>
                )}
                {!hasIndexed && (
                  <p className="text-xs text-muted-foreground">Index some content first to enable search.</p>
                )}
              </CardContent>
            </Card>

            {/* Compact */}
            <Card>
              <CardHeader className="pb-3">
                <CardTitle className="text-sm">3. Compact</CardTitle>
                <CardDescription>
                  Summarize indexed content using an LLM, then search the compacted version
                </CardDescription>
              </CardHeader>
              <CardContent className="space-y-3">
                <Button
                  size="sm"
                  disabled={compactLoading || !hasIndexed || !config.compaction?.enabled}
                  onClick={async () => {
                    setCompactLoading(true)
                    try {
                      const result = await invoke<string>("memory_test_compact")
                      setCompactResult(result || "Compaction complete.")
                      toast.success("Content compacted")
                    } catch (err: any) {
                      setCompactResult(`Error: ${err.message || err}`)
                    } finally {
                      setCompactLoading(false)
                    }
                  }}
                >
                  {compactLoading ? (
                    <><Loader2 className="h-3.5 w-3.5 mr-1.5 animate-spin" />Compacting...</>
                  ) : (
                    "Compact"
                  )}
                </Button>
                {!config.compaction?.enabled && (
                  <p className="text-xs text-muted-foreground">
                    Enable compaction in Settings first (select a model).
                  </p>
                )}
                {compactResult && (
                  <div className="rounded-md border p-3 bg-muted/50">
                    <pre className="text-xs whitespace-pre-wrap font-mono">{compactResult}</pre>
                  </div>
                )}
              </CardContent>
            </Card>
          </div>
        </TabsContent>

        {/* ================================================================ */}
        {/* Settings Tab                                                     */}
        {/* ================================================================ */}
        <TabsContent value="settings" className="flex-1 min-h-0 mt-4 overflow-y-auto">
          <div className="space-y-4 max-w-2xl">
            {/* Tool & Search */}
            <Card>
              <CardHeader className="pb-3">
                <CardTitle className="text-sm">Tool Configuration</CardTitle>
              </CardHeader>
              <CardContent className="space-y-4">
                <div className="grid grid-cols-2 gap-4">
                  <div className="space-y-1.5">
                    <Label htmlFor="recall-tool-name" className="text-xs">Tool name</Label>
                    <Input
                      id="recall-tool-name"
                      value={config.recall_tool_name}
                      onChange={(e) => setConfig({ ...config, recall_tool_name: e.target.value })}
                      onBlur={() => saveConfig(config)}
                      className="h-8 text-sm"
                      placeholder="MemoryRecall"
                    />
                    <p className="text-[10px] text-muted-foreground">
                      The MCP tool name exposed to LLMs for searching memories
                    </p>
                  </div>
                  <div className="space-y-1.5">
                    <Label htmlFor="search-top-k" className="text-xs">Search results (top-k)</Label>
                    <Input
                      id="search-top-k"
                      type="number"
                      min={1}
                      max={20}
                      value={config.search_top_k}
                      onChange={(e) => setConfig({ ...config, search_top_k: parseInt(e.target.value) || 5 })}
                      onBlur={() => saveConfig(config)}
                      className="h-8 text-sm"
                    />
                    <p className="text-[10px] text-muted-foreground">
                      Number of memory chunks returned per search
                    </p>
                  </div>
                </div>
              </CardContent>
            </Card>

            {/* Session Grouping */}
            <Card>
              <CardHeader className="pb-3">
                <CardTitle className="text-sm">Session Grouping</CardTitle>
                <CardDescription>
                  Conversations are grouped into sessions based on timing. A session ends when
                  there&apos;s been no activity for the inactivity timeout, or the max duration is reached.
                </CardDescription>
              </CardHeader>
              <CardContent className="space-y-4">
                <div className="grid grid-cols-2 gap-4">
                  <div className="space-y-1.5">
                    <Label htmlFor="inactivity" className="text-xs">Inactivity timeout (minutes)</Label>
                    <Input
                      id="inactivity"
                      type="number"
                      min={10}
                      value={config.session_inactivity_minutes}
                      onChange={(e) => setConfig({ ...config, session_inactivity_minutes: parseInt(e.target.value) || 180 })}
                      onBlur={() => saveConfig(config)}
                      className="h-8 text-sm"
                    />
                    <p className="text-[10px] text-muted-foreground">
                      Close the session after this many minutes of no new messages. Default: 180 (3 hours).
                    </p>
                  </div>
                  <div className="space-y-1.5">
                    <Label htmlFor="max-session" className="text-xs">Max session duration (minutes)</Label>
                    <Input
                      id="max-session"
                      type="number"
                      min={30}
                      value={config.max_session_minutes}
                      onChange={(e) => setConfig({ ...config, max_session_minutes: parseInt(e.target.value) || 480 })}
                      onBlur={() => saveConfig(config)}
                      className="h-8 text-sm"
                    />
                    <p className="text-[10px] text-muted-foreground">
                      Force-close the session after this duration regardless of activity. Default: 480 (8 hours).
                    </p>
                  </div>
                </div>
              </CardContent>
            </Card>

            {/* Compaction */}
            <Card>
              <CardHeader className="pb-3">
                <CardTitle className="text-sm">Compaction</CardTitle>
                <CardDescription>
                  When a session ends, optionally summarize it using an LLM.
                  The summary replaces the raw transcript in the search index while
                  the original is archived for re-compaction.
                </CardDescription>
              </CardHeader>
              <CardContent className="space-y-4">
                <div className="space-y-1.5">
                  <Label className="text-xs">Compaction model</Label>
                  <Select
                    value={config.compaction?.enabled ? `${config.compaction.llm_provider}/${config.compaction.llm_model || ""}` : "disabled"}
                    onValueChange={(value) => {
                      if (value === "disabled") {
                        saveConfig({ ...config, compaction: null })
                      } else {
                        const slashIdx = value.indexOf("/")
                        const provider = value.substring(0, slashIdx)
                        const model = value.substring(slashIdx + 1)
                        saveConfig({
                          ...config,
                          compaction: { enabled: true, llm_provider: provider, llm_model: model || null },
                        })
                      }
                    }}
                  >
                    <SelectTrigger className="h-8 text-sm">
                      <SelectValue placeholder="Select compaction model" />
                    </SelectTrigger>
                    <SelectContent>
                      <SelectItem value="disabled">Disabled (keep raw transcripts)</SelectItem>
                      {Object.entries(modelsByProvider).map(([provider, models]) => (
                        <SelectGroup key={provider}>
                          <SelectLabel className="text-xs text-muted-foreground">{provider}</SelectLabel>
                          {models.map((modelId) => (
                            <SelectItem key={`${provider}/${modelId}`} value={`${provider}/${modelId}`}>
                              {modelId}
                            </SelectItem>
                          ))}
                        </SelectGroup>
                      ))}
                    </SelectContent>
                  </Select>
                  <p className="text-[10px] text-muted-foreground">
                    The LLM used to summarize session transcripts when they expire.
                    Select a model from your configured providers.
                  </p>
                </div>
              </CardContent>
            </Card>

            <p className="text-xs text-muted-foreground">
              Memory is enabled per-client in the client&apos;s Optimize tab.
            </p>
          </div>
        </TabsContent>
      </Tabs>
    </div>
  )

  async function runSearch() {
    setSearchLoading(true)
    try {
      const result = await invoke<string>("memory_test_search", { query: searchQuery, topK: config.search_top_k })
      setSearchResults(result || "No results found.")
    } catch (err: any) {
      setSearchResults(`Error: ${err.message || err}`)
    } finally {
      setSearchLoading(false)
    }
  }
}
