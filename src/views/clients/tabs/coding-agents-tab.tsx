import { useState, useEffect, useCallback } from "react"
import { invoke } from "@tauri-apps/api/core"
import { listen } from "@tauri-apps/api/event"
import { toast } from "sonner"
import { Card, CardContent, CardHeader, CardTitle, CardDescription } from "@/components/ui/Card"
import { PermissionStateButton } from "@/components/permissions"
import { SamplePopupButton } from "@/components/shared/SamplePopupButton"
import { ToolList } from "@/components/shared/ToolList"
import type { ToolListItem } from "@/components/shared/ToolList"
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/Select"
import { Badge } from "@/components/ui/Badge"
import type { PermissionState } from "@/components/permissions"
import type {
  CodingAgentInfo,
  CodingAgentType,
  ToolDefinition,
  SetClientCodingAgentPermissionParams,
  SetClientCodingAgentTypeParams,
  GetCodingAgentToolDefinitionsParams,
} from "@/types/tauri-commands"

interface Client {
  id: string
  name: string
  client_id: string
  coding_agent_permission: PermissionState
  coding_agent_type: CodingAgentType | null
}

interface CodingAgentsTabProps {
  client: Client
  onUpdate: () => void
}

export function ClientCodingAgentsTab({ client, onUpdate }: CodingAgentsTabProps) {
  const [agents, setAgents] = useState<CodingAgentInfo[]>([])
  const [loading, setLoading] = useState(true)
  const [saving, setSaving] = useState(false)
  const [agentTools, setAgentTools] = useState<ToolListItem[]>([])

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

  useEffect(() => {
    loadAgents()

    const unsubscribe = listen("coding-agents-changed", () => {
      loadAgents()
    })

    return () => {
      unsubscribe.then((fn) => fn())
    }
  }, [loadAgents])

  // Fetch tool definitions when agent type changes
  useEffect(() => {
    if (!client.coding_agent_type) {
      setAgentTools([])
      return
    }
    invoke<ToolDefinition[]>("get_coding_agent_tool_definitions", {
      agentType: client.coding_agent_type,
    } satisfies GetCodingAgentToolDefinitionsParams)
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
  }, [client.coding_agent_type])

  const handlePermissionChange = async (state: PermissionState) => {
    setSaving(true)
    try {
      await invoke("set_client_coding_agent_permission", {
        clientId: client.client_id,
        permission: state,
      } satisfies SetClientCodingAgentPermissionParams)
      onUpdate()
    } catch (error) {
      console.error("Failed to set permission:", error)
      toast.error("Failed to update permission")
    } finally {
      setSaving(false)
    }
  }

  const handleAgentTypeChange = async (value: string) => {
    setSaving(true)
    try {
      const agentType = value === "none" ? null : (value as CodingAgentType)
      await invoke("set_client_coding_agent_type", {
        clientId: client.client_id,
        agentType,
      } satisfies SetClientCodingAgentTypeParams)
      onUpdate()
    } catch (error) {
      console.error("Failed to set agent type:", error)
      toast.error("Failed to update agent type")
    } finally {
      setSaving(false)
    }
  }

  const installedAgents = agents.filter((a) => a.installed)

  return (
    <div className="space-y-6">
      <Card>
        <CardHeader>
          <CardTitle>Coding Agent Permission</CardTitle>
          <CardDescription>
            Control whether this client can spawn and interact with a coding agent via MCP tools.
            Use "Ask" to require approval before starting a session.
          </CardDescription>
        </CardHeader>
        <CardContent className="space-y-6">
          <div className="flex items-center justify-between">
            <div>
              <p className="text-sm font-medium">Access</p>
              <p className="text-xs text-muted-foreground">
                Allow, require approval, or disable coding agent access
              </p>
            </div>
            <PermissionStateButton
              value={client.coding_agent_permission}
              onChange={handlePermissionChange}
              disabled={saving}
            />
          </div>

          <div className="flex items-center justify-between">
            <div>
              <p className="text-sm font-medium">Agent</p>
              <p className="text-xs text-muted-foreground">
                Select which coding agent this client uses
              </p>
            </div>
            <Select
              value={client.coding_agent_type ?? "none"}
              onValueChange={handleAgentTypeChange}
              disabled={saving || loading}
            >
              <SelectTrigger className="w-48">
                <SelectValue placeholder="Select agent..." />
              </SelectTrigger>
              <SelectContent>
                <SelectItem value="none">None</SelectItem>
                {installedAgents.map((agent) => (
                  <SelectItem key={agent.agentType} value={agent.agentType}>
                    {agent.displayName}
                  </SelectItem>
                ))}
              </SelectContent>
            </Select>
          </div>

          {client.coding_agent_type && agentTools.length > 0 && (
            <div className="p-4 rounded-lg bg-muted/50 border space-y-2">
              <p className="text-sm text-muted-foreground">
                When enabled, this client will have access to {agentTools.length} coding agent tools:
              </p>
              <ToolList
                tools={agentTools}
                compact
              />
            </div>
          )}

          {!loading && installedAgents.length === 0 && (
            <p className="text-xs text-muted-foreground">
              No coding agents installed. Install a supported coding agent to enable this feature.
            </p>
          )}

          <div className="border-t pt-3 flex items-center justify-between">
            <div>
              <span className="text-sm font-medium">Approval Popup Preview</span>
              <p className="text-xs text-muted-foreground mt-0.5">
                Preview the popup shown when a client starts a coding agent session.
                Only session creation requires approval &mdash; subsequent interactions proceed freely.
              </p>
            </div>
            <SamplePopupButton popupType="coding_agent" />
          </div>

          {client.coding_agent_type && !loading && (
            <div className="text-xs text-muted-foreground">
              {(() => {
                const selected = agents.find((a) => a.agentType === client.coding_agent_type)
                if (!selected) return null
                return selected.installed ? (
                  <span className="flex items-center gap-1.5">
                    <Badge variant="success" className="text-[10px] px-1 py-0">installed</Badge>
                    <code className="bg-muted px-1 py-0.5 rounded">{selected.binaryName}</code>
                  </span>
                ) : (
                  <span className="flex items-center gap-1.5">
                    <Badge variant="secondary" className="text-[10px] px-1 py-0">not found</Badge>
                    Install <code className="bg-muted px-1 py-0.5 rounded">{selected.binaryName}</code> to use this agent
                  </span>
                )
              })()}
            </div>
          )}
        </CardContent>
      </Card>
    </div>
  )
}
