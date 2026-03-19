/**
 * StepExtensions - MCP servers, Skills, Coding Agents, and Marketplace in one page.
 *
 * Uses the same shared components as the client detail tabs.
 * All components save directly to the backend since the client
 * is already created before this step.
 */

import { useState, useEffect, useCallback } from "react"
import { invoke } from "@tauri-apps/api/core"
import { listenSafe } from "@/hooks/useTauriListener"
import { toast } from "sonner"
import { Server, Sparkles, Bot, Store, ChevronDown } from "lucide-react"
import { McpPermissionTree, SkillsPermissionTree, PermissionStateButton } from "@/components/permissions"
import type { McpPermissions, SkillsPermissions, PermissionState } from "@/components/permissions"
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/Select"
import { Badge } from "@/components/ui/Badge"
import type {
  CodingAgentInfo,
  CodingAgentType,
  SetClientCodingAgentPermissionParams,
  SetClientCodingAgentTypeParams,
} from "@/types/tauri-commands"
import { cn } from "@/lib/utils"

interface StepExtensionsProps {
  clientId: string
  mcpPermissions: McpPermissions
  skillsPermissions: SkillsPermissions
  codingAgentPermission: PermissionState
  codingAgentType: CodingAgentType | null
  marketplacePermission: PermissionState
  onUpdate: () => void
}

interface SectionProps {
  icon: React.ReactNode
  title: string
  badge?: string
  defaultOpen?: boolean
  children: React.ReactNode
}

function Section({ icon, title, badge, defaultOpen = false, children }: SectionProps) {
  const [open, setOpen] = useState(defaultOpen)

  return (
    <div className="border rounded-lg">
      <button
        onClick={() => setOpen(!open)}
        className="flex items-center gap-3 w-full px-4 py-3 text-left hover:bg-accent/50 transition-colors rounded-lg"
      >
        <div className="text-muted-foreground">{icon}</div>
        <span className="text-sm font-medium flex-1">{title}</span>
        {badge && (
          <Badge variant="secondary" className="text-[10px]">{badge}</Badge>
        )}
        <ChevronDown className={cn("h-4 w-4 text-muted-foreground transition-transform", open && "rotate-180")} />
      </button>
      {open && (
        <div className="px-4 pb-4 pt-1">
          {children}
        </div>
      )}
    </div>
  )
}

export function StepExtensions({
  clientId,
  mcpPermissions,
  skillsPermissions,
  codingAgentPermission,
  codingAgentType,
  marketplacePermission,
  onUpdate,
}: StepExtensionsProps) {
  // Coding agents state
  const [agents, setAgents] = useState<CodingAgentInfo[]>([])
  const [agentsLoading, setAgentsLoading] = useState(true)
  const [saving, setSaving] = useState(false)

  // Local permission state for marketplace
  const [localMarketplace, setLocalMarketplace] = useState<PermissionState>(marketplacePermission)

  useEffect(() => {
    setLocalMarketplace(marketplacePermission)
  }, [marketplacePermission])

  const loadAgents = useCallback(async () => {
    try {
      const agentList = await invoke<CodingAgentInfo[]>("list_coding_agents")
      setAgents(agentList)
    } catch (error) {
      console.error("Failed to load coding agents:", error)
    } finally {
      setAgentsLoading(false)
    }
  }, [])

  useEffect(() => {
    loadAgents()
    const l = listenSafe("coding-agents-changed", () => loadAgents())
    return () => { l.cleanup() }
  }, [loadAgents])

  const handleCodingAgentPermissionChange = async (state: PermissionState) => {
    setSaving(true)
    try {
      await invoke("set_client_coding_agent_permission", {
        clientId,
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

  const handleCodingAgentTypeChange = async (value: string) => {
    setSaving(true)
    try {
      const agentType = value === "none" ? null : (value as CodingAgentType)
      await invoke("set_client_coding_agent_type", {
        clientId,
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

  const handleMarketplacePermissionChange = async (state: PermissionState) => {
    setSaving(true)
    try {
      await invoke("set_client_marketplace_permission", {
        clientId,
        state,
      })
      setLocalMarketplace(state)
      onUpdate()
    } catch (error) {
      console.error("Failed to update marketplace permission:", error)
      toast.error("Failed to update permission")
    } finally {
      setSaving(false)
    }
  }

  const installedAgents = agents.filter((a) => a.installed)

  return (
    <div className="space-y-3">
      <p className="text-sm text-muted-foreground">
        Configure access to MCP servers, skills, and more. You can also set these up later.
      </p>

      {/* MCP Servers */}
      <Section
        icon={<Server className="h-4 w-4" />}
        title="MCP Servers"
        defaultOpen={true}
      >
        <McpPermissionTree
          clientId={clientId}
          permissions={mcpPermissions}
          onUpdate={onUpdate}
        />
      </Section>

      {/* Skills */}
      <Section
        icon={<Sparkles className="h-4 w-4" />}
        title="Skills"
      >
        <SkillsPermissionTree
          clientId={clientId}
          permissions={skillsPermissions}
          onUpdate={onUpdate}
        />
      </Section>

      {/* Coding Agents */}
      <Section
        icon={<Bot className="h-4 w-4" />}
        title="Coding Agents"
      >
        <div className="space-y-4">
          <div className="flex items-center justify-between">
            <div>
              <p className="text-sm font-medium">Access</p>
              <p className="text-xs text-muted-foreground">
                Allow, require approval, or disable
              </p>
            </div>
            <PermissionStateButton
              value={codingAgentPermission}
              onChange={handleCodingAgentPermissionChange}
              disabled={saving}
            />
          </div>

          <div className="flex items-center justify-between">
            <div>
              <p className="text-sm font-medium">Agent</p>
              <p className="text-xs text-muted-foreground">
                Select which coding agent to use
              </p>
            </div>
            <Select
              value={codingAgentType ?? "none"}
              onValueChange={handleCodingAgentTypeChange}
              disabled={saving || agentsLoading}
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

          {!agentsLoading && installedAgents.length === 0 && (
            <p className="text-xs text-muted-foreground">
              No coding agents installed.
            </p>
          )}

          {codingAgentType && !agentsLoading && (() => {
            const selected = agents.find((a) => a.agentType === codingAgentType)
            if (!selected) return null
            return (
              <div className="text-xs text-muted-foreground flex items-center gap-1.5">
                <Badge variant={selected.installed ? "success" : "secondary"} className="text-[10px] px-1 py-0">
                  {selected.installed ? "installed" : "not found"}
                </Badge>
                <code className="bg-muted px-1 py-0.5 rounded">{selected.binaryName}</code>
              </div>
            )
          })()}
        </div>
      </Section>

      {/* Marketplace */}
      <Section
        icon={<Store className="h-4 w-4" />}
        title="Marketplace"
      >
        <div className="space-y-3">
          <div className="flex items-center justify-between">
            <div>
              <p className="text-sm font-medium">Access</p>
              <p className="text-xs text-muted-foreground">
                Search and install from the marketplace
              </p>
            </div>
            <PermissionStateButton
              value={localMarketplace}
              onChange={handleMarketplacePermissionChange}
              disabled={saving}
              allowedStates={["ask", "off"]}
            />
          </div>
        </div>
      </Section>
    </div>
  )
}
