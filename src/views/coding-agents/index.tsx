import { useState, useEffect, useCallback } from "react"
import { invoke } from "@tauri-apps/api/core"
import { listen } from "@tauri-apps/api/event"
import { toast } from "sonner"
import { Card, CardContent, CardHeader, CardTitle, CardDescription } from "@/components/ui/Card"
import { Badge } from "@/components/ui/Badge"
import { Button } from "@/components/ui/Button"
import { Label } from "@/components/ui/label"
import { Input } from "@/components/ui/Input"
import { ScrollArea } from "@/components/ui/scroll-area"
import { Tabs, TabsContent, TabsList, TabsTrigger } from "@/components/ui/tabs"
import { CodingAgentsIcon } from "@/components/icons/category-icons"
import { McpTab } from "@/views/try-it-out/mcp-tab"
import type {
  CodingAgentInfo,
  CodingAgentType,
  CodingSessionInfo,
} from "@/types/tauri-commands"

interface CodingAgentsViewProps {
  activeSubTab?: string | null
  onTabChange?: (view: string, subTab?: string | null) => void
}

export function CodingAgentsView({ activeSubTab, onTabChange }: CodingAgentsViewProps) {
  const [agents, setAgents] = useState<CodingAgentInfo[]>([])
  const [sessions, setSessions] = useState<CodingSessionInfo[]>([])
  const [selectedAgent, setSelectedAgent] = useState<CodingAgentType | null>(null)
  const [loading, setLoading] = useState(true)
  const [maxSessions, setMaxSessions] = useState<number>(0)

  // Parse activeSubTab: "agents", "sessions", "try-it-out", "settings", or "agents/<agentType>"
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

  if (loading) {
    return (
      <div className="flex flex-col h-full min-h-0">
        <div className="flex-shrink-0 pb-4">
          <h1 className="text-2xl font-bold tracking-tight flex items-center gap-2">
            <CodingAgentsIcon className="h-6 w-6" />
            Coding Agents
          </h1>
          <p className="text-sm text-muted-foreground">
            AI coding agents available as MCP tools through the gateway
          </p>
        </div>
        <div className="flex items-center justify-center flex-1">
          <p className="text-muted-foreground">Loading coding agents...</p>
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
          AI coding agents available as MCP tools through the gateway
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
          <TabsTrigger value="try-it-out">Try It Out</TabsTrigger>
          <TabsTrigger value="settings">Settings</TabsTrigger>
        </TabsList>

        {/* Agents Tab */}
        <TabsContent value="agents" className="flex-1 min-h-0 mt-4">
          <div className="flex flex-1 min-h-0 rounded-lg border h-full">
            {/* Agent list */}
            <div className="w-64 border-r">
              <ScrollArea className="h-full">
                <div className="p-2 space-y-1">
                  {agents.map((agent) => (
                    <button
                      key={agent.agentType}
                      onClick={() => setSelectedAgent(agent.agentType)}
                      className={`w-full flex items-center gap-2 px-3 py-2 rounded-md text-sm transition-colors ${
                        selectedAgent === agent.agentType
                          ? "bg-accent text-accent-foreground"
                          : "hover:bg-accent/50"
                      }`}
                    >
                      <div className="flex-1 text-left">
                        <div className="font-medium">{agent.displayName}</div>
                        <div className="text-xs text-muted-foreground">{agent.binaryName}</div>
                      </div>
                      <div className="flex items-center gap-1.5">
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
                    </button>
                  ))}
                </div>
              </ScrollArea>
            </div>

            {/* Agent detail */}
            <div className="flex-1">
              <ScrollArea className="h-full">
                {selected ? (
                  <div className="p-6 space-y-6">
                    <div className="flex items-center justify-between">
                      <h2 className="text-xl font-bold">{selected.displayName}</h2>
                      <div className="flex items-center gap-3">
                        {selected.installed ? (
                          <Badge variant="success">Installed</Badge>
                        ) : (
                          <Badge variant="secondary">Not Found</Badge>
                        )}
                      </div>
                    </div>

                    {!selected.installed && (
                      <Card>
                        <CardContent className="py-8">
                          <div className="text-center text-muted-foreground">
                            <p className="font-medium">Agent not installed</p>
                            <p className="text-sm mt-1">
                              Install <code className="bg-muted px-1 py-0.5 rounded">{selected.binaryName}</code> to make it available as an MCP tool.
                            </p>
                          </div>
                        </CardContent>
                      </Card>
                    )}

                    {/* Active sessions for this agent */}
                    {sessions.filter((s) => s.agentType === selected.agentType).length > 0 && (
                      <Card>
                        <CardHeader>
                          <CardTitle>Active Sessions</CardTitle>
                        </CardHeader>
                        <CardContent>
                          <div className="space-y-2">
                            {sessions
                              .filter((s) => s.agentType === selected.agentType)
                              .map((session) => (
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
                        </CardContent>
                      </Card>
                    )}
                  </div>
                ) : (
                  <div className="flex items-center justify-center h-full">
                    <p className="text-muted-foreground">Select a coding agent to view details</p>
                  </div>
                )}
              </ScrollArea>
            </div>
          </div>
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
                            {session.agentType}
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

        {/* Try It Out Tab */}
        <TabsContent value="try-it-out" className="flex-1 min-h-0 mt-4">
          <McpTab
            innerPath={null}
            onPathChange={() => {}}
          />
        </TabsContent>

        {/* Settings Tab */}
        <TabsContent value="settings" className="flex-1 min-h-0 mt-4">
          <div className="space-y-6">
            <Card>
              <CardHeader>
                <CardTitle>Agents</CardTitle>
                <CardDescription>
                  Coding agents that can be assigned to clients and spawned as MCP tools.
                </CardDescription>
              </CardHeader>
              <CardContent>
                <div className="space-y-2">
                  {agents.map((agent) => (
                    <div
                      key={agent.agentType}
                      className="flex items-center justify-between py-2 px-3 rounded bg-muted/50"
                    >
                      <div>
                        <p className="text-sm font-medium">{agent.displayName}</p>
                        <p className="text-xs text-muted-foreground">
                          <code>{agent.binaryName}</code>
                        </p>
                      </div>
                      {agent.installed ? (
                        <Badge variant="success">Installed</Badge>
                      ) : (
                        <Badge variant="secondary">Not Found</Badge>
                      )}
                    </div>
                  ))}
                </div>
              </CardContent>
            </Card>

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
                    {maxSessions === 0 ? "Unlimited" : `${maxSessions} session${maxSessions !== 1 ? "s" : ""} max`}
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
