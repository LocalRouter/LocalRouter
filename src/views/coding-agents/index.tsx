import { useState, useEffect, useCallback } from "react"
import { invoke } from "@tauri-apps/api/core"
import { listen } from "@tauri-apps/api/event"
import { toast } from "sonner"
import { Terminal, Loader2 } from "lucide-react"
import { Card, CardContent, CardHeader, CardTitle, CardDescription } from "@/components/ui/Card"
import { Badge } from "@/components/ui/Badge"
import { Button } from "@/components/ui/Button"
import { Label } from "@/components/ui/label"
import { Input } from "@/components/ui/Input"
import { ScrollArea } from "@/components/ui/scroll-area"
import { Tabs, TabsContent, TabsList, TabsTrigger } from "@/components/ui/tabs"
import {
  ResizablePanelGroup,
  ResizablePanel,
  ResizableHandle,
} from "@/components/ui/resizable"
import { CodingAgentsIcon } from "@/components/icons/category-icons"
import { cn } from "@/lib/utils"
import type {
  CodingAgentInfo,
  CodingAgentType,
  CodingSessionInfo,
  GetCodingAgentVersionParams,
} from "@/types/tauri-commands"

interface CodingAgentsViewProps {
  activeSubTab?: string | null
  onTabChange?: (view: string, subTab?: string | null) => void
}

const PERMISSION_MODE_LABELS: Record<string, string> = {
  auto: "Auto",
  supervised: "Supervised",
  plan: "Plan",
}

export function CodingAgentsView({ activeSubTab, onTabChange }: CodingAgentsViewProps) {
  const [agents, setAgents] = useState<CodingAgentInfo[]>([])
  const [sessions, setSessions] = useState<CodingSessionInfo[]>([])
  const [selectedAgent, setSelectedAgent] = useState<CodingAgentType | null>(null)
  const [detailTab, setDetailTab] = useState("info")
  const [loading, setLoading] = useState(true)
  const [maxSessions, setMaxSessions] = useState<number>(0)
  const [search, setSearch] = useState("")
  const [agentVersion, setAgentVersion] = useState<string | null>(null)
  const [versionLoading, setVersionLoading] = useState(false)

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
    } catch (error) {
      console.error("Failed to load max sessions:", error)
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
    } catch (error) {
      toast.error(`Failed to end session: ${error}`)
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
            <ResizablePanel defaultSize={35} minSize={25}>
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
            <ResizablePanel defaultSize={65}>
              {selected ? (
                <ScrollArea className="h-full">
                  <div className="p-6 space-y-6">
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

                    <Tabs value={detailTab} onValueChange={setDetailTab}>
                      <TabsList>
                        <TabsTrigger value="info">Info</TabsTrigger>
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
                              <div className="grid grid-cols-2 gap-3 text-sm">
                                <div>
                                  <span className="text-muted-foreground">Model Selection:</span>{" "}
                                  <span className="font-medium">
                                    {selected.supportsModelSelection ? "Yes" : "No"}
                                  </span>
                                </div>
                                <div>
                                  <span className="text-muted-foreground">Permission Modes:</span>{" "}
                                  <span className="font-medium">
                                    {selected.supportedPermissionModes
                                      .map((m) => PERMISSION_MODE_LABELS[m] || m)
                                      .join(", ")}
                                  </span>
                                </div>
                              </div>
                            </CardContent>
                          </Card>

                          {/* MCP Tools */}
                          <Card>
                            <CardHeader className="pb-3">
                              <CardTitle className="text-sm">MCP Tools</CardTitle>
                              <CardDescription>
                                These tools are exposed to clients through the MCP gateway when this agent is assigned.
                              </CardDescription>
                            </CardHeader>
                            <CardContent>
                              <div className="space-y-1.5">
                                {[
                                  { suffix: "start", desc: "Start a new coding session" },
                                  { suffix: "say", desc: "Send a message to an active session" },
                                  { suffix: "status", desc: "Get session status and recent output" },
                                  { suffix: "respond", desc: "Answer a pending question or approval" },
                                  { suffix: "interrupt", desc: "Interrupt the running session" },
                                  { suffix: "list", desc: "List all active sessions" },
                                ].map((tool) => (
                                  <div
                                    key={tool.suffix}
                                    className="flex items-center gap-3 py-1.5 px-2 rounded text-sm"
                                  >
                                    <Terminal className="h-3.5 w-3.5 text-muted-foreground shrink-0" />
                                    <code className="text-xs bg-muted px-1.5 py-0.5 rounded shrink-0">
                                      {selected.mcpToolPrefix}_{tool.suffix}
                                    </code>
                                    <span className="text-xs text-muted-foreground">
                                      {tool.desc}
                                    </span>
                                  </div>
                                ))}
                              </div>
                            </CardContent>
                          </Card>
                        </div>
                      </TabsContent>

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
          <div className="space-y-4">
            {sessions.length === 0 ? (
              <Card>
                <CardContent className="py-8">
                  <div className="text-center text-muted-foreground">
                    <p className="font-medium">No active sessions</p>
                    <p className="text-sm mt-1">
                      Sessions will appear here when coding agents are running.
                    </p>
                  </div>
                </CardContent>
              </Card>
            ) : (
              sessions.map((session) => (
                <Card key={session.sessionId}>
                  <CardContent className="py-4">
                    <div className="flex items-center justify-between">
                      <div className="space-y-1">
                        <div className="flex items-center gap-2">
                          <span className="text-sm font-medium">{session.displayText}</span>
                          <Badge variant="outline">{session.status}</Badge>
                          <Badge variant="secondary" className="text-[10px]">
                            {agents.find((a) => a.agentType === session.agentType)?.displayName ||
                              session.agentType}
                          </Badge>
                        </div>
                        <div className="text-xs text-muted-foreground">
                          {session.workingDirectory}
                        </div>
                      </div>
                      <Button
                        variant="outline"
                        size="sm"
                        onClick={() => handleEndSession(session.sessionId)}
                      >
                        End Session
                      </Button>
                    </div>
                  </CardContent>
                </Card>
              ))
            )}
          </div>
        </TabsContent>

        {/* Settings Tab */}
        <TabsContent value="settings" className="flex-1 min-h-0 mt-4">
          <div className="space-y-6 max-w-2xl">
            <Card>
              <CardHeader>
                <CardTitle>Concurrency</CardTitle>
                <CardDescription>
                  Limit the total number of coding agent sessions that can run at the same time across all clients.
                  Set to 0 for unlimited.
                </CardDescription>
              </CardHeader>
              <CardContent>
                <div className="space-y-2">
                  <Label>Max Concurrent Sessions</Label>
                  <Input
                    type="number"
                    min={0}
                    max={50}
                    value={maxSessions}
                    onChange={(e) => handleMaxSessionsChange(e.target.value)}
                    className="w-32"
                  />
                  <p className="text-xs text-muted-foreground">
                    {maxSessions === 0
                      ? "Unlimited"
                      : `${maxSessions} session${maxSessions !== 1 ? "s" : ""} max`}
                  </p>
                </div>
              </CardContent>
            </Card>
          </div>
        </TabsContent>
      </Tabs>
    </div>
  )
}
