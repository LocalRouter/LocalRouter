import { useState, useEffect, useCallback } from "react"
import { invoke } from "@tauri-apps/api/core"
import { toast } from "sonner"
import { FolderOpen, Loader2 } from "lucide-react"
import { FEATURES } from "@/constants/features"
import { ExperimentalBadge } from "@/components/shared/ExperimentalBadge"
import { TAB_ICONS, TAB_ICON_CLASS } from "@/constants/tab-icons"
import { Card, CardContent, CardHeader, CardTitle, CardDescription } from "@/components/ui/Card"
import { Label } from "@/components/ui/label"
import { Input } from "@/components/ui/Input"
import { Tabs, TabsContent, TabsList, TabsTrigger } from "@/components/ui/tabs"
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from "@/components/ui/Select"
import { useIncrementalModels } from "@/hooks/useIncrementalModels"
import { McpToolDisplay } from "@/components/shared/McpToolDisplay"
import { FeatureClientsCard } from "@/components/shared/FeatureClientsCard"
import { ContentStorePreview } from "@/components/shared/ContentStorePreview"
import { MemorySessionsTab } from "./sessions-tab"
import type { MemoryConfig, UpdateMemoryConfigParams } from "@/types/tauri-commands"

const defaultConfig: MemoryConfig = {
  compaction_model: null,
  search_top_k: 5,
  session_inactivity_minutes: 180,
  max_session_minutes: 480,
  recall_tool_name: "MemorySearch",
}

interface MemoryViewProps {
  activeSubTab?: string | null
  onTabChange?: (view: string, subTab?: string | null) => void
}

export function MemoryView({ activeSubTab, onTabChange }: MemoryViewProps) {
  const [config, setConfig] = useState<MemoryConfig>(defaultConfig)
  const [isLoading, setIsLoading] = useState(true)

  // Live models for compaction model picker
  const { models: liveModels } = useIncrementalModels({ refreshOnMount: true })
  const modelsByProvider = liveModels.reduce<Record<string, string[]>>((acc, m) => {
    if (!acc[m.provider]) acc[m.provider] = []
    if (!acc[m.provider].includes(m.id)) acc[m.provider].push(m.id)
    return acc
  }, {})

  const tab = activeSubTab || "info"
  const handleTabChange = (newTab: string) => onTabChange?.("memory", newTab)

  // Derive read tool name from search tool name
  const readToolName = config.recall_tool_name.endsWith("Search")
    ? config.recall_tool_name.replace(/Search$/, "Read")
    : config.recall_tool_name.endsWith("Recall")
      ? config.recall_tool_name.replace(/Recall$/, "Read")
      : `${config.recall_tool_name}Read`

  useEffect(() => {
    loadConfig()
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

  const openMemoryFolder = async () => {
    try {
      await invoke("open_memory_folder")
    } catch (error: any) {
      toast.error(`Failed to open folder: ${error.message || error}`)
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
          {FEATURES.memory.name}
          {FEATURES.memory.experimental && <ExperimentalBadge />}
        </h1>
        <p className="text-sm text-muted-foreground">
          Persistent conversation memory with native FTS5 search and optional semantic vector search
        </p>
      </div>

      <Tabs
        value={tab}
        onValueChange={handleTabChange}
        className="flex flex-col flex-1 min-h-0"
      >
        <TabsList className="flex-shrink-0 w-fit">
          <TabsTrigger value="info"><TAB_ICONS.info className={TAB_ICON_CLASS} />Info</TabsTrigger>
          <TabsTrigger value="sessions"><TAB_ICONS.tryItOut className={TAB_ICON_CLASS} />Sessions</TabsTrigger>
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

            {/* Semantic Search — configured on Indexing page */}
            <Card>
              <CardContent className="py-3">
                <p className="text-sm text-muted-foreground">
                  Semantic search (hybrid FTS5 + embeddings) is configured on the{' '}
                  <button onClick={() => onTabChange?.('indexing', null)} className="text-primary hover:underline">
                    Indexing page
                  </button>.
                </p>
              </CardContent>
            </Card>

            {/* Tool Preview */}
            <Card>
              <CardHeader className="pb-3">
                <CardTitle className="text-sm">Tool Definitions</CardTitle>
                <CardDescription>
                  How the memory tools appear to the LLM &mdash; two tools for search and read
                </CardDescription>
              </CardHeader>
              <CardContent>
                <McpToolDisplay
                  tools={[
                    {
                      name: config.recall_tool_name,
                      description: `Search past conversation memories. Returns results with source labels and line numbers. Use ${readToolName}(label, offset) to read full context around hits. Pass ALL search questions as queries array in ONE call.`,
                      inputSchema: {
                        type: "object",
                        properties: {
                          query: { type: "string", description: "Single search query" },
                          queries: { type: "array", items: { type: "string" }, description: "Multiple search queries to batch" },
                          source: { type: "string", description: "Filter to a specific source" },
                          limit: { type: "number", description: "Max results per query (default: 3)" },
                        },
                      },
                      itemType: "tool",
                    },
                    {
                      name: readToolName,
                      description: `Read the full content of a memory source. Use after ${config.recall_tool_name} to get complete context around a search hit. Supports offset and limit for pagination.`,
                      inputSchema: {
                        type: "object",
                        properties: {
                          label: { type: "string", description: 'Source label from search results (e.g., "session/abc123")' },
                          offset: { type: "string", description: 'Line offset (e.g., "5" or "5-2")' },
                          limit: { type: "number", description: "Number of lines to return (default: 15)" },
                        },
                        required: ["label"],
                      },
                      itemType: "tool",
                    },
                  ]}
                />
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
                  <li>Content is indexed into a native FTS5 database (no external dependencies)</li>
                  <li>The LLM searches memories using <strong>{config.recall_tool_name}</strong> and reads full context with <strong>{readToolName}</strong></li>
                  <li>When a session ends, optional compaction summarizes it using an LLM</li>
                  <li>If the embedding model is downloaded, search automatically upgrades to hybrid mode (FTS5 + semantic)</li>
                </ol>
                <p className="text-xs mt-2">
                  Enable memory per-client in the client&apos;s Optimize tab. Each client&apos;s memories are isolated.
                </p>
              </CardContent>
            </Card>

            <FeatureClientsCard feature="memory" clientTab="optimize" onNavigateToClient={onTabChange} />
          </div>
        </TabsContent>

        {/* ================================================================ */}
        {/* Sessions Tab                                                     */}
        {/* ================================================================ */}
        <TabsContent value="sessions" className="flex-1 min-h-0 mt-4">
          <MemorySessionsTab />
        </TabsContent>

        {/* ================================================================ */}
        {/* Try It Out Tab                                                   */}
        {/* ================================================================ */}
        <TabsContent value="try-it-out" className="flex-1 min-h-0 mt-4 overflow-y-auto">
          <div className="max-w-2xl">
            <ContentStorePreview
              loadSample={() => invoke<string>("memory_test_sample")}
              sourceLabel="session/sample-session"
              responseThresholdBytes={200}
              searchPlaceholder='e.g. "session token storage", "database choice"'
              defaultMode="index"
            />
          </div>
        </TabsContent>

        {/* ================================================================ */}
        {/* Settings Tab                                                     */}
        {/* ================================================================ */}
        <TabsContent value="settings" className="flex-1 min-h-0 mt-4 overflow-y-auto">
          <div className="space-y-4 max-w-2xl">
            {/* Compaction Model */}
            <Card>
              <CardHeader className="pb-3">
                <CardTitle className="text-sm">Compaction Model</CardTitle>
                <CardDescription>
                  LLM used to summarize session transcripts when they expire.
                  Leave disabled to keep raw transcripts.
                </CardDescription>
              </CardHeader>
              <CardContent className="space-y-4">
                <div className="grid grid-cols-2 gap-3">
                  <div className="space-y-1.5">
                    <Label className="text-xs">Provider</Label>
                    <Select
                      value={config.compaction_model?.split("/")[0] || "none"}
                      onValueChange={(value) => {
                        if (value === "none") {
                          saveConfig({ ...config, compaction_model: null })
                        } else {
                          // Auto-select first model from this provider
                          const models = modelsByProvider[value]
                          if (models?.length) {
                            saveConfig({ ...config, compaction_model: `${value}/${models[0]}` })
                          }
                        }
                      }}
                    >
                      <SelectTrigger className="h-8 text-sm">
                        <SelectValue placeholder="Select provider" />
                      </SelectTrigger>
                      <SelectContent>
                        <SelectItem value="none">Disabled</SelectItem>
                        {Object.keys(modelsByProvider).map((provider) => (
                          <SelectItem key={provider} value={provider}>{provider}</SelectItem>
                        ))}
                      </SelectContent>
                    </Select>
                  </div>
                  <div className="space-y-1.5">
                    <Label className="text-xs">Model</Label>
                    <Select
                      value={config.compaction_model?.split("/").slice(1).join("/") || ""}
                      disabled={!config.compaction_model}
                      onValueChange={(modelId) => {
                        const provider = config.compaction_model?.split("/")[0]
                        if (provider) {
                          saveConfig({ ...config, compaction_model: `${provider}/${modelId}` })
                        }
                      }}
                    >
                      <SelectTrigger className="h-8 text-sm">
                        <SelectValue placeholder={config.compaction_model ? "Select model" : "Select provider first"} />
                      </SelectTrigger>
                      <SelectContent>
                        {(modelsByProvider[config.compaction_model?.split("/")[0] || ""] || []).map((modelId) => (
                          <SelectItem key={modelId} value={modelId}>{modelId}</SelectItem>
                        ))}
                      </SelectContent>
                    </Select>
                  </div>
                </div>
              </CardContent>
            </Card>

            {/* Tool & Search */}
            <Card>
              <CardHeader className="pb-3">
                <CardTitle className="text-sm">Tool Configuration</CardTitle>
              </CardHeader>
              <CardContent className="space-y-4">
                <div className="grid grid-cols-2 gap-4">
                  <div className="space-y-1.5">
                    <Label htmlFor="recall-tool-name" className="text-xs">Search tool name</Label>
                    <Input
                      id="recall-tool-name"
                      value={config.recall_tool_name}
                      onChange={(e) => setConfig({ ...config, recall_tool_name: e.target.value })}
                      onBlur={() => saveConfig(config)}
                      className="h-8 text-sm"
                      placeholder="MemorySearch"
                    />
                    <p className="text-[10px] text-muted-foreground">
                      Read tool is derived automatically: <code>{readToolName}</code>
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

            <p className="text-xs text-muted-foreground">
              Memory is enabled per-client in the client&apos;s Optimize tab.
            </p>
          </div>
        </TabsContent>
      </Tabs>
    </div>
  )
}
