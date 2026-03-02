import { useState, useEffect } from "react"
import { invoke } from "@tauri-apps/api/core"
import { listen } from "@tauri-apps/api/event"
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/Card"
import { Badge } from "@/components/ui/Badge"
import { ScrollArea } from "@/components/ui/scroll-area"
import { CodingAgentsIcon } from "@/components/icons/category-icons"
import type {
  CodingAgentInfo,
  CodingAgentType,
  CodingSessionInfo,
} from "@/types/tauri-commands"

interface CodingAgentsViewProps {
  activeSubTab?: string | null
  onTabChange?: (view: string, subTab?: string | null) => void
}

/** @deprecated This view is no longer reachable from the sidebar. Coding agent settings are now in Settings > Coding Agents. */
export function CodingAgentsView({ activeSubTab }: CodingAgentsViewProps) {
  const [agents, setAgents] = useState<CodingAgentInfo[]>([])
  const [sessions, setSessions] = useState<CodingSessionInfo[]>([])
  const [selectedAgent, setSelectedAgent] = useState<CodingAgentType | null>(null)
  const [loading, setLoading] = useState(true)

  const loadAgents = async () => {
    try {
      const agentList = await invoke<CodingAgentInfo[]>("list_coding_agents")
      setAgents(agentList)
    } catch (error) {
      console.error("Failed to load coding agents:", error)
    } finally {
      setLoading(false)
    }
  }

  const loadSessions = async () => {
    try {
      const sessionList = await invoke<CodingSessionInfo[]>("list_coding_sessions")
      setSessions(sessionList)
    } catch (error) {
      console.error("Failed to load sessions:", error)
    }
  }

  useEffect(() => {
    loadAgents()
    loadSessions()

    const unsubscribe = listen("coding-agents-changed", () => {
      loadAgents()
    })

    return () => {
      unsubscribe.then((fn) => fn())
    }
  }, [])

  useEffect(() => {
    if (activeSubTab) {
      setSelectedAgent(activeSubTab as CodingAgentType)
    }
  }, [activeSubTab])

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

      <div className="flex flex-1 min-h-0 rounded-lg border">
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
                <div>
                  <h2 className="text-xl font-bold">{selected.displayName}</h2>
                </div>
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
                            <Badge variant="outline">{session.status}</Badge>
                          </div>
                        ))}
                    </div>
                  </CardContent>
                </Card>
              )}
            </div>
          ) : (
            <div className="flex items-center justify-center h-full">
              <div className="text-center">
                <p className="text-muted-foreground">Select a coding agent to view details</p>
              </div>
            </div>
          )}
        </ScrollArea>
      </div>
      </div>
    </div>
  )
}
