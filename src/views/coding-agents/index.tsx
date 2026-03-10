import { useState, useEffect, useCallback, useRef } from "react"
import { invoke } from "@tauri-apps/api/core"
import { listen } from "@tauri-apps/api/event"
import { toast } from "sonner"
import { Loader2, FlaskConical, Copy, Check, Terminal, CheckCircle2, XCircle, ExternalLink, Square } from "lucide-react"
import { Card, CardContent, CardHeader, CardTitle, CardDescription } from "@/components/ui/Card"
import { Badge } from "@/components/ui/Badge"
import { Button } from "@/components/ui/Button"
import { Label } from "@/components/ui/label"
import { Input } from "@/components/ui/Input"
import { Switch } from "@/components/ui/switch"
import { ScrollArea } from "@/components/ui/scroll-area"
import { Tabs, TabsContent, TabsList, TabsTrigger } from "@/components/ui/tabs"
import {
  ResizablePanelGroup,
  ResizablePanel,
  ResizableHandle,
} from "@/components/ui/resizable"
import { CodingAgentsIcon } from "@/components/icons/category-icons"
import { McpTab } from "@/views/try-it-out/mcp-tab"
import { ToolList } from "@/components/shared/ToolList"
import type { ToolListItem } from "@/components/shared/ToolList"
import { cn } from "@/lib/utils"
import type {
  CodingAgentInfo,
  CodingAgentType,
  CodingSessionInfo,
  CodingSessionDetail,
  ToolDefinition,
  GetCodingAgentVersionParams,
  GetCodingAgentToolDefinitionsParams,
  GetCodingSessionDetailParams,
  OpenPathParams,
} from "@/types/tauri-commands"

interface CodingAgentsViewProps {
  activeSubTab?: string | null
  onTabChange?: (view: string, subTab?: string | null) => void
}

function statusVariant(status: string): "default" | "secondary" | "destructive" | "outline" | "success" {
  switch (status) {
    case "active":
      return "default"
    case "awaiting_input":
      return "secondary"
    case "done":
      return "success"
    case "error":
      return "destructive"
    case "interrupted":
      return "outline"
    default:
      return "outline"
  }
}

function formatOutputLine(line: string): string {
  const trimmed = line.trim()
  if (
    (trimmed.startsWith("{") && trimmed.endsWith("}")) ||
    (trimmed.startsWith("[") && trimmed.endsWith("]"))
  ) {
    try {
      return JSON.stringify(JSON.parse(trimmed), null, 2)
    } catch {
      // not valid JSON, return as-is
    }
  }
  return line
}

function formatDuration(createdAt: string): string {
  const created = new Date(createdAt)
  const now = new Date()
  const diffMs = now.getTime() - created.getTime()
  const diffSec = Math.floor(diffMs / 1000)
  if (diffSec < 60) return `${diffSec}s`
  const diffMin = Math.floor(diffSec / 60)
  if (diffMin < 60) return `${diffMin}m`
  const diffHr = Math.floor(diffMin / 60)
  const remMin = diffMin % 60
  return `${diffHr}h ${remMin}m`
}

export function CodingAgentsView({ activeSubTab, onTabChange }: CodingAgentsViewProps) {
  const [agents, setAgents] = useState<CodingAgentInfo[]>([])
  const [sessions, setSessions] = useState<CodingSessionInfo[]>([])
  const [selectedAgent, setSelectedAgent] = useState<CodingAgentType | null>(null)
  const [detailTab, setDetailTab] = useState("info")
  const [loading, setLoading] = useState(true)
  const [maxSessions, setMaxSessions] = useState<number>(0)
  const [search, setSearch] = useState("")
  const [sessionSearch, setSessionSearch] = useState("")
  const [agentVersion, setAgentVersion] = useState<string | null>(null)
  const [versionLoading, setVersionLoading] = useState(false)
  const lastLimitRef = useRef(5)

  // Tool definitions state
  const [agentTools, setAgentTools] = useState<ToolListItem[]>([])

  // Session detail state
  const [selectedSessionId, setSelectedSessionId] = useState<string | null>(null)
  const [sessionDetail, setSessionDetail] = useState<CodingSessionDetail | null>(null)
  const [sessionDetailLoading, setSessionDetailLoading] = useState(false)
  const [copiedId, setCopiedId] = useState(false)

  // Parse activeSubTab
  const parseSubTab = (subTab: string | null) => {
    if (!subTab) return { mainTab: "agents", agentId: null }
    const parts = subTab.split("/")
    const mainTab = parts[0] || "agents"
    const agentId = parts[1] || null
    return { mainTab, agentId }
  }

  const { mainTab, agentId } = parseSubTab(activeSubTab ?? null)

  const loadAgents = useCallback(async () => {
    try {
      const agentList = await invoke<CodingAgentInfo[]>("list_coding_agents")
      setAgents(agentList)
    } catch (error) {
      console.error("Failed to load coding agents:", error)
    } finally {
      setLoading(false)
    }
  }, [])

  const loadSessions = useCallback(async () => {
    try {
      const sessionList = await invoke<CodingSessionInfo[]>("list_coding_sessions")
      setSessions(sessionList)
    } catch (error) {
      console.error("Failed to load sessions:", error)
    }
  }, [])

  const loadMaxSessions = useCallback(async () => {
    try {
      const max = await invoke<number>("get_max_coding_sessions")
      setMaxSessions(max)
      if (max > 0) lastLimitRef.current = max
    } catch (error) {
      console.error("Failed to load max sessions:", error)
    }
  }, [])

  const loadSessionDetail = useCallback(async (sessionId: string) => {
    setSessionDetailLoading(true)
    try {
      const detail = await invoke<CodingSessionDetail>("get_coding_session_detail", {
        sessionId,
      } satisfies GetCodingSessionDetailParams as Record<string, unknown>)
      setSessionDetail(detail)
    } catch (error) {
      console.error("Failed to load session detail:", error)
      setSessionDetail(null)
    } finally {
      setSessionDetailLoading(false)
    }
  }, [])

  useEffect(() => {
    loadAgents()
    loadSessions()
    loadMaxSessions()

    const unsubscribe = listen("coding-agents-changed", () => {
      loadAgents()
      loadSessions()
    })

    return () => {
      unsubscribe.then((fn) => fn())
    }
  }, [loadAgents, loadSessions, loadMaxSessions])

  useEffect(() => {
    if (agentId) {
      setSelectedAgent(agentId as CodingAgentType)
    }
  }, [agentId])

  // Load version when selecting an installed agent
  useEffect(() => {
    if (!selectedAgent) {
      setAgentVersion(null)
      return
    }
    const agent = agents.find((a) => a.agentType === selectedAgent)
    if (!agent?.installed) {
      setAgentVersion(null)
      return
    }
    setVersionLoading(true)
    setAgentVersion(null)
    invoke<string | null>("get_coding_agent_version", {
      agentType: selectedAgent,
    } satisfies GetCodingAgentVersionParams as Record<string, unknown>)
      .then((v) => setAgentVersion(v))
      .catch(() => setAgentVersion(null))
      .finally(() => setVersionLoading(false))
  }, [selectedAgent, agents])

  // Load tool definitions when selecting an agent
  useEffect(() => {
    if (!selectedAgent) {
      setAgentTools([])
      return
    }
    invoke<ToolDefinition[]>("get_coding_agent_tool_definitions", {
      agentType: selectedAgent,
    } satisfies GetCodingAgentToolDefinitionsParams as Record<string, unknown>)
      .then((defs) =>
        setAgentTools(
          defs.map((d): ToolListItem => ({
            name: d.name,
            description: d.description,
            inputSchema: d.input_schema,
          }))
        )
      )
      .catch(() => setAgentTools([]))
  }, [selectedAgent])

  // Load session detail when selected, and poll for live sessions
  useEffect(() => {
    if (!selectedSessionId) {
      setSessionDetail(null)
      return
    }
    loadSessionDetail(selectedSessionId)

    // Poll every 2s for active sessions
    const interval = setInterval(() => {
      if (selectedSessionId) {
        loadSessionDetail(selectedSessionId)
      }
    }, 2000)

    return () => clearInterval(interval)
  }, [selectedSessionId, loadSessionDetail])

  // Refresh detail when sessions change (e.g. session ended)
  useEffect(() => {
    if (selectedSessionId) {
      // Check if selected session still exists
      const stillExists = sessions.some((s) => s.sessionId === selectedSessionId)
      if (!stillExists) {
        setSelectedSessionId(null)
        setSessionDetail(null)
      }
    }
  }, [sessions, selectedSessionId])

  const handleMaxSessionsChange = async (value: string) => {
    const num = parseInt(value, 10)
    if (isNaN(num) || num < 0) return
    setMaxSessions(num)
    try {
      await invoke("set_max_coding_sessions", { maxSessions: num })
    } catch (error) {
      console.error("Failed to set max sessions:", error)
      toast.error("Failed to update max sessions")
    }
  }

  const handleEndSession = async (sessionId: string) => {
    try {
      await invoke("end_coding_session", { sessionId })
      toast.success("Session ended")
      loadSessions()
      if (selectedSessionId === sessionId) {
        setSelectedSessionId(null)
        setSessionDetail(null)
      }
    } catch (error) {
      toast.error(`Failed to end session: ${error}`)
    }
  }

  const handleCopySessionId = (sessionId: string) => {
    navigator.clipboard.writeText(sessionId)
    setCopiedId(true)
    setTimeout(() => setCopiedId(false), 2000)
  }

  const handleOpenPath = async (path: string) => {
    try {
      await invoke("open_path", { path } satisfies OpenPathParams as Record<string, unknown>)
    } catch (error) {
      console.error("Failed to open path:", error)
      toast.error("Failed to open folder")
    }
  }

  const handleTabChange = (tab: string) => {
    onTabChange?.("coding-agents", tab)
  }

  const selected = selectedAgent ? agents.find((a) => a.agentType === selectedAgent) : null
  const agentSessions = selected
    ? sessions.filter((s) => s.agentType === selected.agentType)
    : []

  const filteredAgents = agents.filter(
    (a) =>
      a.displayName.toLowerCase().includes(search.toLowerCase()) ||
      a.binaryName.toLowerCase().includes(search.toLowerCase())
  )

  const filteredSessions = sessions.filter(
    (s) =>
      s.displayText.toLowerCase().includes(sessionSearch.toLowerCase()) ||
      s.workingDirectory.toLowerCase().includes(sessionSearch.toLowerCase()) ||
      s.sessionId.toLowerCase().includes(sessionSearch.toLowerCase())
  )

  if (loading) {
    return (
      <div className="flex flex-col h-full min-h-0">
        <div className="flex-shrink-0 pb-4">
          <h1 className="text-2xl font-bold tracking-tight flex items-center gap-2">
            <CodingAgentsIcon className="h-6 w-6" />
            Coding Agents
          </h1>
          <p className="text-sm text-muted-foreground">Loading...</p>
        </div>
      </div>
    )
  }

  return (
    <div className="flex flex-col h-full min-h-0">
      <div className="flex-shrink-0 pb-4">
        <h1 className="text-2xl font-bold tracking-tight flex items-center gap-2">
          <CodingAgentsIcon className="h-6 w-6" />
          Coding Agents
        </h1>
        <p className="text-sm text-muted-foreground">
          Locally installed coding agents exposed as MCP tools through the unified gateway.
        </p>
      </div>

      <Tabs
        value={mainTab}
        onValueChange={handleTabChange}
        className="flex flex-col flex-1 min-h-0"
      >
        <TabsList className="flex-shrink-0 w-fit">
          <TabsTrigger value="agents">Agents</TabsTrigger>
          <TabsTrigger value="sessions">
            Sessions
            {sessions.length > 0 && (
              <Badge variant="secondary" className="ml-1.5 text-[10px] px-1 py-0">
                {sessions.length}
              </Badge>
            )}
          </TabsTrigger>
          <TabsTrigger value="settings">Settings</TabsTrigger>
        </TabsList>

        {/* Agents Tab */}
        <TabsContent value="agents" className="flex-1 min-h-0 mt-4">
          <ResizablePanelGroup direction="horizontal" className="flex-1 min-h-0 rounded-lg border">
            {/* List Panel */}
            <ResizablePanel defaultSize={21} minSize={15}>
              <div className="flex flex-col h-full">
                <div className="p-4 border-b">
                  <Input
                    placeholder="Search agents..."
                    value={search}
                    onChange={(e) => setSearch(e.target.value)}
                  />
                </div>
                <ScrollArea className="flex-1">
                  <div className="p-2 space-y-1">
                    {filteredAgents.map((agent) => {
                      const sessionCount = sessions.filter(
                        (s) => s.agentType === agent.agentType
                      ).length
                      return (
                        <div
                          key={agent.agentType}
                          onClick={() => {
                            setSelectedAgent(agent.agentType)
                            setDetailTab("info")
                          }}
                          className={cn(
                            "flex items-center gap-3 p-3 rounded-md cursor-pointer",
                            selectedAgent === agent.agentType
                              ? "bg-accent"
                              : "hover:bg-muted"
                          )}
                        >
                          <div className="flex-1 min-w-0">
                            <p className="font-medium truncate">{agent.displayName}</p>
                            <p className="text-xs text-muted-foreground truncate">
                              {agent.binaryName}
                            </p>
                          </div>
                          <div className="flex items-center gap-1.5 shrink-0">
                            {sessionCount > 0 && (
                              <Badge variant="default" className="text-[10px] px-1 py-0">
                                {sessionCount}
                              </Badge>
                            )}
                            {agent.installed ? (
                              <Badge variant="success" className="text-[10px] px-1 py-0">
                                installed
                              </Badge>
                            ) : (
                              <Badge variant="secondary" className="text-[10px] px-1 py-0">
                                not found
                              </Badge>
                            )}
                          </div>
                        </div>
                      )
                    })}
                  </div>
                </ScrollArea>
              </div>
            </ResizablePanel>

            <ResizableHandle withHandle />

            {/* Detail Panel */}
            <ResizablePanel defaultSize={79}>
              {selected ? (
                <ScrollArea className="h-full">
                  <div className="p-6 space-y-6">
                    <div className="flex items-start justify-between">
                      <div>
                        <div className="flex items-center gap-2">
                          <h2 className="text-xl font-bold">{selected.displayName}</h2>
                          {selected.installed ? (
                            <Badge variant="success">Installed</Badge>
                          ) : (
                            <Badge variant="secondary">Not Found</Badge>
                          )}
                        </div>
                        <p className="text-sm text-muted-foreground mt-1">
                          {selected.description}
                        </p>
                      </div>
                      {selected.installed && (
                        <div className="flex items-center gap-2">
                          <Button
                            variant="outline"
                            size="sm"
                            onClick={() => setDetailTab("try-it-out")}
                          >
                            <FlaskConical className="h-4 w-4 mr-1" />
                            Try It Out
                          </Button>
                        </div>
                      )}
                    </div>

                    <Tabs value={detailTab} onValueChange={setDetailTab}>
                      <TabsList>
                        <TabsTrigger value="info">Info</TabsTrigger>
                        {selected.installed && <TabsTrigger value="try-it-out">Try It Out</TabsTrigger>}
                        {agentSessions.length > 0 && (
                          <TabsTrigger value="sessions">
                            Sessions
                            <Badge variant="secondary" className="ml-1.5 text-[10px] px-1 py-0">
                              {agentSessions.length}
                            </Badge>
                          </TabsTrigger>
                        )}
                      </TabsList>

                      <TabsContent value="info">
                        <div className="space-y-4">
                          {/* Installation */}
                          <Card>
                            <CardHeader className="pb-3">
                              <CardTitle className="text-sm">Installation</CardTitle>
                            </CardHeader>
                            <CardContent className="space-y-3">
                              <div className="grid grid-cols-2 gap-3 text-sm">
                                <div>
                                  <span className="text-muted-foreground">Binary:</span>{" "}
                                  <code className="bg-muted px-1 py-0.5 rounded text-xs">
                                    {selected.binaryName}
                                  </code>
                                </div>
                                <div>
                                  <span className="text-muted-foreground">Status:</span>{" "}
                                  <span className="font-medium">
                                    {selected.installed ? "Installed" : "Not found"}
                                  </span>
                                </div>
                                {selected.binaryPath && (
                                  <div className="col-span-2">
                                    <span className="text-muted-foreground">Path:</span>{" "}
                                    <code className="bg-muted px-1 py-0.5 rounded text-xs break-all">
                                      {selected.binaryPath}
                                    </code>
                                  </div>
                                )}
                                {selected.installed && (
                                  <div className="col-span-2">
                                    <span className="text-muted-foreground">Version:</span>{" "}
                                    {versionLoading ? (
                                      <Loader2 className="inline h-3 w-3 animate-spin ml-1" />
                                    ) : agentVersion ? (
                                      <code className="bg-muted px-1 py-0.5 rounded text-xs">
                                        {agentVersion}
                                      </code>
                                    ) : (
                                      <span className="text-xs text-muted-foreground">Unknown</span>
                                    )}
                                  </div>
                                )}
                              </div>

                              {!selected.installed && (
                                <p className="text-sm text-muted-foreground">
                                  Install{" "}
                                  <code className="bg-muted px-1 py-0.5 rounded">
                                    {selected.binaryName}
                                  </code>{" "}
                                  to make it available as an MCP tool.
                                </p>
                              )}
                            </CardContent>
                          </Card>

                          {/* Capabilities */}
                          <Card>
                            <CardHeader className="pb-3">
                              <CardTitle className="text-sm">Capabilities</CardTitle>
                            </CardHeader>
                            <CardContent className="space-y-3">
                              <div className={cn("flex items-center gap-2.5", !selected.supportsModelSelection && "opacity-45")}>
                                {selected.supportsModelSelection ? (
                                  <CheckCircle2 className="h-4 w-4 text-green-600 dark:text-green-400 shrink-0" />
                                ) : (
                                  <XCircle className="h-4 w-4 text-muted-foreground shrink-0" />
                                )}
                                <div>
                                  <p className="text-sm">Model Selection</p>
                                  <p className="text-xs text-muted-foreground">
                                    {selected.supportsModelSelection
                                      ? "A specific model can be passed when starting a session."
                                      : "Uses its own default model, cannot be overridden."}
                                  </p>
                                </div>
                              </div>

                              <div className="border-t pt-3">
                                <p className="text-xs text-muted-foreground mb-2">Permission Modes</p>
                                <div className="space-y-1">
                                  {(
                                    [
                                      {
                                        key: "auto",
                                        label: "Auto",
                                        desc: "Tools auto-approved without prompting",
                                      },
                                      {
                                        key: "supervised",
                                        label: "Supervised",
                                        desc: "Tools require explicit approval",
                                      },
                                      {
                                        key: "plan",
                                        label: "Plan",
                                        desc: "Plans only, no code execution",
                                      },
                                    ] as const
                                  ).map((mode) => {
                                    const supported = selected.supportedPermissionModes.includes(mode.key)
                                    return (
                                      <div
                                        key={mode.key}
                                        className={cn(
                                          "flex items-center gap-2.5",
                                          !supported && "opacity-45"
                                        )}
                                      >
                                        {supported ? (
                                          <CheckCircle2 className="h-4 w-4 text-green-600 dark:text-green-400 shrink-0" />
                                        ) : (
                                          <XCircle className="h-4 w-4 text-muted-foreground shrink-0" />
                                        )}
                                        <span className="text-sm">{mode.label}</span>
                                        <span className="text-xs text-muted-foreground">{mode.desc}</span>
                                      </div>
                                    )
                                  })}
                                </div>
                              </div>
                            </CardContent>
                          </Card>

                          {/* MCP Tools */}
                          <Card>
                            <CardHeader className="pb-3">
                              <CardTitle className="text-sm">MCP Tools</CardTitle>
                              <CardDescription>
                                Tools exposed through the unified MCP gateway when this agent is enabled.
                              </CardDescription>
                            </CardHeader>
                            <CardContent>
                              <ToolList tools={agentTools} />
                            </CardContent>
                          </Card>
                        </div>
                      </TabsContent>

                      {selected.installed && (
                      <TabsContent value="try-it-out">
                        <McpTab
                          initialMode="direct"
                          initialDirectTarget={`coding_agent:${selected.agentType}`}
                          hideModeSwitcher
                          hideDirectTargetSelector
                          innerPath={null}
                          onPathChange={() => {}}
                        />
                      </TabsContent>
                      )}

                      <TabsContent value="sessions">
                        <div className="space-y-2">
                          {agentSessions.map((session) => (
                            <div
                              key={session.sessionId}
                              className="flex items-center justify-between py-2 px-3 rounded bg-muted/50"
                            >
                              <div>
                                <div className="text-sm font-medium">{session.displayText}</div>
                                <div className="text-xs text-muted-foreground">
                                  {session.workingDirectory}
                                </div>
                              </div>
                              <div className="flex items-center gap-2">
                                <Badge variant="outline">{session.status}</Badge>
                                <Button
                                  variant="ghost"
                                  size="sm"
                                  className="h-6 text-xs"
                                  onClick={() => handleEndSession(session.sessionId)}
                                >
                                  End
                                </Button>
                              </div>
                            </div>
                          ))}
                        </div>
                      </TabsContent>
                    </Tabs>
                  </div>
                </ScrollArea>
              ) : (
                <div className="flex flex-col items-center justify-center h-full text-muted-foreground gap-4">
                  <CodingAgentsIcon className="h-12 w-12 opacity-30" />
                  <div className="text-center">
                    <p className="font-medium">Select an agent to view details</p>
                  </div>
                </div>
              )}
            </ResizablePanel>
          </ResizablePanelGroup>
        </TabsContent>

        {/* Sessions Tab */}
        <TabsContent value="sessions" className="flex-1 min-h-0 mt-4">
          {sessions.length === 0 ? (
            <div className="flex flex-col items-center justify-center h-full text-muted-foreground gap-4 border rounded-lg">
              <Terminal className="h-12 w-12 opacity-30" />
              <div className="text-center">
                <p className="font-medium">No active sessions</p>
                <p className="text-sm mt-1">
                  Sessions will appear here when coding agents are running.
                </p>
              </div>
            </div>
          ) : (
            <ResizablePanelGroup direction="horizontal" className="flex-1 min-h-0 rounded-lg border">
              {/* Session List Panel */}
              <ResizablePanel defaultSize={21} minSize={15}>
                <div className="flex flex-col h-full">
                  <div className="p-4 border-b">
                    <Input
                      placeholder="Search sessions..."
                      value={sessionSearch}
                      onChange={(e) => setSessionSearch(e.target.value)}
                    />
                  </div>
                  <ScrollArea className="flex-1">
                    <div className="p-2 space-y-1">
                      {filteredSessions.map((session) => (
                        <div
                          key={session.sessionId}
                          onClick={() => setSelectedSessionId(session.sessionId)}
                          className={cn(
                            "flex flex-col gap-1 p-3 rounded-md cursor-pointer",
                            selectedSessionId === session.sessionId
                              ? "bg-accent"
                              : "hover:bg-muted"
                          )}
                        >
                          <div className="flex items-center justify-between gap-2">
                            <p className="text-sm font-medium truncate flex-1">{session.displayText}</p>
                            <Badge variant={statusVariant(session.status)} className="text-[10px] px-1.5 py-0 shrink-0">
                              {session.status}
                            </Badge>
                          </div>
                          <div className="flex items-center justify-between gap-2">
                            <p className="text-xs text-muted-foreground truncate">
                              {session.workingDirectory}
                            </p>
                            <span className="text-[10px] text-muted-foreground shrink-0">
                              {formatDuration(session.createdAt)}
                            </span>
                          </div>
                          <div className="flex items-center justify-between gap-1.5">
                            <div className="flex items-center gap-1.5 min-w-0">
                              <Badge variant="secondary" className="text-[10px] px-1 py-0">
                                {agents.find((a) => a.agentType === session.agentType)?.displayName ||
                                  session.agentType}
                              </Badge>
                              <span className="text-[10px] text-muted-foreground font-mono truncate">
                                {session.sessionId.slice(0, 12)}...
                              </span>
                            </div>
                            {(session.status === "active" || session.status === "awaiting_input") && (
                              <Button
                                variant="ghost"
                                size="sm"
                                className="h-5 w-5 p-0 shrink-0 text-muted-foreground hover:text-destructive"
                                title="End session"
                                onClick={(e) => {
                                  e.stopPropagation()
                                  handleEndSession(session.sessionId)
                                }}
                              >
                                <Square className="h-3 w-3" />
                              </Button>
                            )}
                          </div>
                        </div>
                      ))}
                    </div>
                  </ScrollArea>
                </div>
              </ResizablePanel>

              <ResizableHandle withHandle />

              {/* Session Detail Panel */}
              <ResizablePanel defaultSize={79}>
                {selectedSessionId && sessionDetail ? (
                  <ScrollArea className="h-full">
                    <div className="p-6 space-y-4">
                      {/* Header */}
                      <div className="flex items-start justify-between">
                        <div>
                          <div className="flex items-center gap-2">
                            <h2 className="text-lg font-bold">{sessionDetail.displayText}</h2>
                            <Badge variant={statusVariant(sessionDetail.status)}>
                              {sessionDetail.status}
                            </Badge>
                          </div>
                          <div className="flex items-center gap-1.5 mt-1">
                            <p className="text-sm text-muted-foreground">
                              {sessionDetail.workingDirectory}
                            </p>
                            <Button
                              variant="ghost"
                              size="sm"
                              className="h-5 px-1.5 text-xs text-muted-foreground"
                              onClick={() => handleOpenPath(sessionDetail.workingDirectory)}
                            >
                              <ExternalLink className="h-3 w-3" />
                            </Button>
                          </div>
                        </div>
                        {(sessionDetail.status === "active" || sessionDetail.status === "awaiting_input") && (
                          <Button
                            variant="outline"
                            size="sm"
                            onClick={() => handleEndSession(sessionDetail.sessionId)}
                          >
                            End Session
                          </Button>
                        )}
                      </div>

                      {/* Session Info */}
                      <Card>
                        <CardHeader className="pb-3">
                          <CardTitle className="text-sm">Session Info</CardTitle>
                        </CardHeader>
                        <CardContent>
                          <div className="grid grid-cols-2 gap-3 text-sm">
                            <div className="col-span-2">
                              <span className="text-muted-foreground">Session ID:</span>{" "}
                              <code className="bg-muted px-1 py-0.5 rounded text-xs">
                                {sessionDetail.sessionId}
                              </code>
                              <Button
                                variant="ghost"
                                size="sm"
                                className="h-5 w-5 p-0 ml-1 inline-flex"
                                onClick={() => handleCopySessionId(sessionDetail.sessionId)}
                              >
                                {copiedId ? (
                                  <Check className="h-3 w-3 text-green-500" />
                                ) : (
                                  <Copy className="h-3 w-3" />
                                )}
                              </Button>
                            </div>
                            <div>
                              <span className="text-muted-foreground">Agent:</span>{" "}
                              <span className="font-medium">
                                {agents.find((a) => a.agentType === sessionDetail.agentType)?.displayName ||
                                  sessionDetail.agentType}
                              </span>
                            </div>
                            <div>
                              <span className="text-muted-foreground">Status:</span>{" "}
                              <Badge variant={statusVariant(sessionDetail.status)} className="text-[10px]">
                                {sessionDetail.status}
                              </Badge>
                            </div>
                            {sessionDetail.clientId && (
                              <div>
                                <span className="text-muted-foreground">Client:</span>{" "}
                                <code className="bg-muted px-1 py-0.5 rounded text-xs">
                                  {sessionDetail.clientId}
                                </code>
                              </div>
                            )}
                            <div>
                              <span className="text-muted-foreground">Created:</span>{" "}
                              <span>{new Date(sessionDetail.createdAt).toLocaleString()}</span>
                            </div>
                            {sessionDetail.turnCount != null && (
                              <div>
                                <span className="text-muted-foreground">Turns:</span>{" "}
                                <span className="font-medium">{sessionDetail.turnCount}</span>
                              </div>
                            )}
                            {sessionDetail.costUsd != null && (
                              <div>
                                <span className="text-muted-foreground">Cost:</span>{" "}
                                <span className="font-medium">${sessionDetail.costUsd.toFixed(4)}</span>
                              </div>
                            )}
                            {sessionDetail.exitCode != null && (
                              <div>
                                <span className="text-muted-foreground">Exit Code:</span>{" "}
                                <code className="bg-muted px-1 py-0.5 rounded text-xs">
                                  {sessionDetail.exitCode}
                                </code>
                              </div>
                            )}
                          </div>
                        </CardContent>
                      </Card>

                      {/* Result / Error */}
                      {sessionDetail.result && (
                        <Card>
                          <CardHeader className="pb-3">
                            <CardTitle className="text-sm">Result</CardTitle>
                          </CardHeader>
                          <CardContent>
                            <pre className="text-xs bg-muted p-3 rounded-md whitespace-pre-wrap break-all max-h-48 overflow-y-auto">
                              {sessionDetail.result}
                            </pre>
                          </CardContent>
                        </Card>
                      )}

                      {sessionDetail.error && (
                        <Card className="border-destructive/50">
                          <CardHeader className="pb-3">
                            <CardTitle className="text-sm text-destructive">Error</CardTitle>
                          </CardHeader>
                          <CardContent>
                            <pre className="text-xs bg-destructive/10 text-destructive p-3 rounded-md whitespace-pre-wrap break-all max-h-48 overflow-y-auto">
                              {sessionDetail.error}
                            </pre>
                          </CardContent>
                        </Card>
                      )}

                      {/* Recent Output */}
                      <Card>
                        <CardHeader className="pb-3">
                          <div className="flex items-center justify-between">
                            <CardTitle className="text-sm">Recent Output</CardTitle>
                            {(sessionDetail.status === "active" || sessionDetail.status === "awaiting_input") && (
                              <Badge variant="outline" className="text-[10px]">
                                Live
                              </Badge>
                            )}
                          </div>
                        </CardHeader>
                        <CardContent>
                          {sessionDetail.recentOutput.length > 0 ? (
                            <pre className="text-xs bg-muted p-3 rounded-md whitespace-pre-wrap break-all max-h-96 overflow-y-auto font-mono leading-relaxed">
                              {sessionDetail.recentOutput.map(formatOutputLine).join("\n")}
                            </pre>
                          ) : (
                            <p className="text-sm text-muted-foreground">No output yet.</p>
                          )}
                        </CardContent>
                      </Card>
                    </div>
                  </ScrollArea>
                ) : selectedSessionId && sessionDetailLoading ? (
                  <div className="flex items-center justify-center h-full">
                    <Loader2 className="h-6 w-6 animate-spin text-muted-foreground" />
                  </div>
                ) : (
                  <div className="flex flex-col items-center justify-center h-full text-muted-foreground gap-4">
                    <Terminal className="h-12 w-12 opacity-30" />
                    <div className="text-center">
                      <p className="font-medium">Select a session to view details</p>
                    </div>
                  </div>
                )}
              </ResizablePanel>
            </ResizablePanelGroup>
          )}
        </TabsContent>

        {/* Settings Tab */}
        <TabsContent value="settings" className="flex-1 min-h-0 mt-4">
          <div className="space-y-6 max-w-2xl">
            <Card>
              <CardHeader>
                <CardTitle>Concurrency</CardTitle>
                <CardDescription>
                  Limit the total number of coding agent sessions that can run at the same time across all clients.
                </CardDescription>
              </CardHeader>
              <CardContent>
                <div className="space-y-4">
                  <div className="flex items-center justify-between">
                    <div>
                      <Label>Limit Concurrent Sessions</Label>
                      <p className="text-xs text-muted-foreground">
                        Restrict how many sessions can run simultaneously.
                      </p>
                    </div>
                    <Switch
                      checked={maxSessions > 0}
                      onCheckedChange={(checked) => {
                        if (checked) {
                          handleMaxSessionsChange(String(lastLimitRef.current))
                        } else {
                          handleMaxSessionsChange("0")
                        }
                      }}
                    />
                  </div>
                  {maxSessions > 0 && (
                    <div className="space-y-2">
                      <Label>Max Concurrent Sessions</Label>
                      <Input
                        type="number"
                        min={1}
                        max={50}
                        value={maxSessions}
                        onChange={(e) => {
                          const val = e.target.value
                          const num = parseInt(val, 10)
                          if (!isNaN(num) && num > 0) lastLimitRef.current = num
                          handleMaxSessionsChange(val)
                        }}
                        className="w-32"
                      />
                    </div>
                  )}
                </div>
              </CardContent>
            </Card>
          </div>
        </TabsContent>
      </Tabs>
    </div>
  )
}
