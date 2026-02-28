import { useState, useEffect } from "react"
import { invoke } from "@tauri-apps/api/core"
import { listen } from "@tauri-apps/api/event"
import { toast } from "sonner"
import { Card, CardContent, CardHeader, CardTitle, CardDescription } from "@/components/ui/Card"
import { Switch } from "@/components/ui/switch"
import { Badge } from "@/components/ui/Badge"
import { Label } from "@/components/ui/label"
import { Input } from "@/components/ui/Input"
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from "@/components/ui/Select"
import { ScrollArea } from "@/components/ui/scroll-area"
import type {
  CodingAgentInfo,
  CodingAgentType,
  CodingPermissionMode,
  CodingSessionInfo,
} from "@/types/tauri-commands"

interface CodingAgentsViewProps {
  activeSubTab?: string | null
  onTabChange?: (view: string, subTab?: string | null) => void
}

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

  const handleToggleEnabled = async (agentType: CodingAgentType, enabled: boolean) => {
    try {
      await invoke("set_coding_agent_enabled", { agentType, enabled })
      toast.success(`${agents.find(a => a.agentType === agentType)?.displayName} ${enabled ? "enabled" : "disabled"}`)
    } catch (error) {
      console.error("Failed to toggle agent:", error)
      toast.error("Failed to update agent")
    }
  }

  const handleUpdateConfig = async (
    agentType: CodingAgentType,
    field: string,
    value: string | null,
  ) => {
    try {
      await invoke("update_coding_agent_config", {
        agentType,
        [field]: value,
      })
    } catch (error) {
      console.error("Failed to update config:", error)
      toast.error("Failed to update configuration")
    }
  }

  const selected = selectedAgent ? agents.find((a) => a.agentType === selectedAgent) : null

  if (loading) {
    return (
      <div className="flex items-center justify-center h-full">
        <p className="text-muted-foreground">Loading coding agents...</p>
      </div>
    )
  }

  return (
    <div className="flex h-full">
      {/* Agent list */}
      <div className="w-64 border-r">
        <div className="p-4 border-b">
          <h2 className="text-sm font-semibold">Coding Agents</h2>
          <p className="text-xs text-muted-foreground mt-1">
            AI coding agents available as MCP tools
          </p>
        </div>
        <ScrollArea className="h-[calc(100vh-8rem)]">
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
                  {agent.installed && (
                    <Badge variant="outline" className="text-[10px] px-1 py-0">
                      installed
                    </Badge>
                  )}
                  {agent.enabled && (
                    <div className="h-1.5 w-1.5 rounded-full bg-green-500" />
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
                  <p className="text-sm text-muted-foreground mt-1">
                    Tool prefix: <code className="text-xs bg-muted px-1 py-0.5 rounded">{selected.toolPrefix}</code>
                  </p>
                </div>
                <div className="flex items-center gap-3">
                  {selected.installed ? (
                    <Badge variant="outline">Installed</Badge>
                  ) : (
                    <Badge variant="secondary">Not Found</Badge>
                  )}
                  <Switch
                    checked={selected.enabled}
                    onCheckedChange={(checked) => handleToggleEnabled(selected.agentType, checked)}
                  />
                </div>
              </div>

              {selected.enabled && (
                <>
                  <Card>
                    <CardHeader>
                      <CardTitle>Configuration</CardTitle>
                      <CardDescription>
                        Configure how this coding agent operates when spawned via MCP tools.
                      </CardDescription>
                    </CardHeader>
                    <CardContent className="space-y-4">
                      <div className="space-y-2">
                        <Label>Working Directory</Label>
                        <Input
                          placeholder="Default (client's working directory)"
                          defaultValue={selected.workingDirectory || ""}
                          onBlur={(e) =>
                            handleUpdateConfig(selected.agentType, "workingDirectory", e.target.value || null)
                          }
                        />
                        <p className="text-xs text-muted-foreground">
                          Override the default working directory for this agent.
                        </p>
                      </div>

                      <div className="space-y-2">
                        <Label>Model Override</Label>
                        <Input
                          placeholder="Default (agent's default model)"
                          defaultValue={selected.modelId || ""}
                          onBlur={(e) =>
                            handleUpdateConfig(selected.agentType, "modelId", e.target.value || null)
                          }
                        />
                        <p className="text-xs text-muted-foreground">
                          Override the model used by this agent (e.g., claude-sonnet-4-6).
                        </p>
                      </div>

                      <div className="space-y-2">
                        <Label>Permission Mode</Label>
                        <Select
                          value={selected.permissionMode}
                          onValueChange={(value) =>
                            handleUpdateConfig(selected.agentType, "permissionMode", value as CodingPermissionMode)
                          }
                        >
                          <SelectTrigger>
                            <SelectValue />
                          </SelectTrigger>
                          <SelectContent>
                            <SelectItem value="auto">Auto</SelectItem>
                            <SelectItem value="supervised">Supervised</SelectItem>
                            <SelectItem value="plan">Plan</SelectItem>
                          </SelectContent>
                        </Select>
                        <p className="text-xs text-muted-foreground">
                          Controls tool approval behavior. Auto allows all, Supervised requires approval, Plan requires plan approval.
                        </p>
                      </div>
                    </CardContent>
                  </Card>

                  <Card>
                    <CardHeader>
                      <CardTitle>MCP Tools</CardTitle>
                      <CardDescription>
                        When enabled, these tools are exposed to MCP clients through the gateway.
                      </CardDescription>
                    </CardHeader>
                    <CardContent>
                      <div className="space-y-2">
                        {[
                          { suffix: "_start", desc: "Start a new coding session" },
                          { suffix: "_say", desc: "Send a message to a session" },
                          { suffix: "_status", desc: "Check session status and output" },
                          { suffix: "_respond", desc: "Respond to pending questions" },
                          { suffix: "_interrupt", desc: "Interrupt a running session" },
                          { suffix: "_list", desc: "List sessions for this agent" },
                        ].map(({ suffix, desc }) => (
                          <div
                            key={suffix}
                            className="flex items-center justify-between py-1.5 px-3 rounded bg-muted/50"
                          >
                            <code className="text-xs font-mono">
                              {selected.toolPrefix}{suffix}
                            </code>
                            <span className="text-xs text-muted-foreground">{desc}</span>
                          </div>
                        ))}
                      </div>
                    </CardContent>
                  </Card>
                </>
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
                <p className="text-muted-foreground">Select a coding agent to configure</p>
                <p className="text-xs text-muted-foreground mt-1">
                  Enable agents to expose them as MCP tools in the gateway
                </p>
              </div>
            </div>
          )}
        </ScrollArea>
      </div>
    </div>
  )
}
