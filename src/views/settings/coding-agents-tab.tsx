import { useState, useEffect, useCallback } from "react"
import { invoke } from "@tauri-apps/api/core"
import { listen } from "@tauri-apps/api/event"
import { toast } from "sonner"
import { Card, CardContent, CardHeader, CardTitle, CardDescription } from "@/components/ui/Card"
import { Badge } from "@/components/ui/Badge"
import { Label } from "@/components/ui/label"
import { Input } from "@/components/ui/Input"
import type { CodingAgentInfo } from "@/types/tauri-commands"

export function CodingAgentsSettingsTab() {
  const [agents, setAgents] = useState<CodingAgentInfo[]>([])
  const [maxSessions, setMaxSessions] = useState<number>(0)
  const [loading, setLoading] = useState(true)

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
    loadMaxSessions()

    const unsubscribe = listen("coding-agents-changed", () => {
      loadAgents()
    })

    return () => {
      unsubscribe.then((fn) => fn())
    }
  }, [loadAgents, loadMaxSessions])

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

  return (
    <div className="space-y-6">
      <Card>
        <CardHeader>
          <CardTitle>Agents</CardTitle>
          <CardDescription>
            Coding agents that can be assigned to clients and spawned as MCP tools.
          </CardDescription>
        </CardHeader>
        <CardContent>
          {loading ? (
            <p className="text-sm text-muted-foreground">Loading...</p>
          ) : (
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
          )}
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
  )
}
