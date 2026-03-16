import { invoke } from "@tauri-apps/api/core"
import { toast } from "sonner"
import { FlaskConical } from "lucide-react"
import { Card, CardContent, CardHeader, CardTitle, CardDescription } from "@/components/ui/Card"
import { Button } from "@/components/ui/Button"
import { McpPermissionTree, PermissionStateButton } from "@/components/permissions"
import { SamplePopupButton } from "@/components/shared/SamplePopupButton"
import { ClientSkillsTab } from "./skills-tab"
import { ClientCodingAgentsTab } from "./coding-agents-tab"
import { ClientMarketplaceTab } from "./marketplace-tab"
import type { McpPermissions, SkillsPermissions, PermissionState } from "@/components/permissions"
import type { CodingAgentType, ClientMode } from "@/types/tauri-commands"

interface Client {
  id: string
  name: string
  client_id: string
  mcp_permissions: McpPermissions
  skills_permissions: SkillsPermissions
  coding_agent_permission: PermissionState
  coding_agent_type: CodingAgentType | null
  marketplace_permission: PermissionState
  mcp_sampling_permission?: PermissionState
  mcp_elicitation_permission?: PermissionState
  client_mode?: ClientMode
}

interface McpTabProps {
  client: Client
  onUpdate: () => void
}

export function ClientMcpTab({ client, onUpdate }: McpTabProps) {
  const clientMode = client.client_mode || "both"

  const handleSamplingPermissionChange = async (state: PermissionState) => {
    try {
      await invoke("set_client_sampling_permission", {
        clientId: client.client_id,
        state,
      })
      toast.success("Sampling permission updated")
      onUpdate()
    } catch (error) {
      console.error("Failed to update sampling permission:", error)
      toast.error("Failed to update sampling permission")
    }
  }

  const handleElicitationPermissionChange = async (state: PermissionState) => {
    try {
      await invoke("set_client_elicitation_permission", {
        clientId: client.client_id,
        state,
      })
      toast.success("Elicitation permission updated")
      onUpdate()
    } catch (error) {
      console.error("Failed to update elicitation permission:", error)
      toast.error("Failed to update elicitation permission")
    }
  }

  return (
    <div className="space-y-6">
      {/* MCP Servers */}
      <Card>
        <CardHeader>
          <CardTitle>MCP Server Permissions</CardTitle>
          <CardDescription>
            Control which MCP servers and their tools this client can access.
            Use "Ask" to require approval before execution.
          </CardDescription>
        </CardHeader>
        <CardContent>
          <div className="flex items-center justify-between mb-4 pb-4 border-b">
            <div>
              <span className="text-sm font-medium">Approval Popup Preview</span>
              <p className="text-xs text-muted-foreground mt-0.5">
                Preview the popup shown when an MCP tool is set to &ldquo;Ask&rdquo;
              </p>
            </div>
            <SamplePopupButton popupType="mcp_tool" />
          </div>
          <McpPermissionTree
            clientId={client.client_id}
            permissions={client.mcp_permissions}
            onUpdate={onUpdate}
          />
        </CardContent>
      </Card>

      {/* Skills */}
      <ClientSkillsTab client={client} onUpdate={onUpdate} />

      {/* Coding Agents */}
      <ClientCodingAgentsTab client={client} onUpdate={onUpdate} />

      {/* Marketplace */}
      <ClientMarketplaceTab client={client} onUpdate={onUpdate} />

      {/* Sampling */}
      <Card>
        <CardHeader>
          <CardTitle>Sampling</CardTitle>
          <CardDescription>
            Controls how backend MCP servers can request LLM completions through this client
          </CardDescription>
        </CardHeader>
        <CardContent className="space-y-3">
          <div className="flex items-center justify-between">
            <div>
              <p className="text-xs text-muted-foreground">
                {clientMode === "mcp_via_llm"
                  ? ({
                      allow: "Automatically route to LLM",
                      ask: "Show approval popup first",
                      off: "Reject sampling requests",
                    } as Record<string, string>)[client.mcp_sampling_permission || "ask"]
                  : ({
                      allow: "Forward to client",
                      ask: "Show approval popup, then forward",
                      off: "Reject sampling requests",
                    } as Record<string, string>)[client.mcp_sampling_permission || "ask"]
                }
              </p>
            </div>
            <PermissionStateButton
              value={client.mcp_sampling_permission || "ask"}
              onChange={handleSamplingPermissionChange}
              size="sm"
            />
          </div>
          {(client.mcp_sampling_permission || "ask") === "ask" && (
            <div className="flex items-center justify-between pt-3 border-t">
              <div>
                <span className="text-sm font-medium">Approval Popup Preview</span>
                <p className="text-xs text-muted-foreground mt-0.5">
                  Preview the popup shown when sampling permission is set to &ldquo;Ask&rdquo;
                </p>
              </div>
              <Button
                variant="outline"
                size="sm"
                onClick={async () => {
                  try {
                    await invoke("debug_trigger_sampling_approval_popup")
                  } catch (e) {
                    console.error("Failed to trigger sampling popup:", e)
                  }
                }}
              >
                <FlaskConical className="h-3.5 w-3.5 mr-1.5" />
                Sample Popup
              </Button>
            </div>
          )}
        </CardContent>
      </Card>

      {/* Elicitation */}
      <Card>
        <CardHeader>
          <CardTitle>Elicitation</CardTitle>
          <CardDescription>
            Controls how backend MCP servers can request user input through this client
          </CardDescription>
        </CardHeader>
        <CardContent className="space-y-3">
          <div className="flex items-center justify-between">
            <div>
              <p className="text-xs text-muted-foreground">
                {clientMode === "mcp_via_llm"
                  ? ({
                      ask: "Show form popup for user input",
                      off: "Reject elicitation requests",
                    } as Record<string, string>)[client.mcp_elicitation_permission === "allow" ? "ask" : (client.mcp_elicitation_permission || "ask")]
                  : ({
                      allow: "Forward to client",
                      ask: "Show form popup locally",
                      off: "Reject elicitation requests",
                    } as Record<string, string>)[client.mcp_elicitation_permission || "ask"]
                }
              </p>
            </div>
            {clientMode === "mcp_via_llm" ? (
              <div className="inline-flex rounded-md border border-border bg-muted/50">
                {(["ask", "off"] as PermissionState[]).map((state) => (
                  <button
                    key={state}
                    type="button"
                    onClick={() => handleElicitationPermissionChange(state)}
                    className={`px-2 py-0.5 text-xs font-medium transition-colors ${
                      (client.mcp_elicitation_permission === "allow" ? "ask" : (client.mcp_elicitation_permission || "ask")) === state
                        ? state === "ask" ? "bg-amber-500 text-white" : "bg-zinc-500 text-white"
                        : "text-muted-foreground hover:text-foreground hover:bg-muted"
                    } ${state === "ask" ? "rounded-l-md" : "rounded-r-md"}`}
                  >
                    {state === "ask" ? "Ask" : "Off"}
                  </button>
                ))}
              </div>
            ) : (
              <PermissionStateButton
                value={client.mcp_elicitation_permission || "ask"}
                onChange={handleElicitationPermissionChange}
                size="sm"
              />
            )}
          </div>
          {(() => {
            const perm = client.mcp_elicitation_permission || "ask"
            const effective = clientMode === "mcp_via_llm" && perm === "allow" ? "ask" : perm
            return effective === "ask"
          })() && (
            <div className="flex items-center justify-between pt-3 border-t">
              <div>
                <span className="text-sm font-medium">Approval Popup Preview</span>
                <p className="text-xs text-muted-foreground mt-0.5">
                  Preview the popup shown when elicitation permission is set to &ldquo;Ask&rdquo;
                </p>
              </div>
              <Button
                variant="outline"
                size="sm"
                onClick={async () => {
                  try {
                    await invoke("debug_trigger_elicitation_form_popup")
                  } catch (e) {
                    console.error("Failed to trigger elicitation popup:", e)
                  }
                }}
              >
                <FlaskConical className="h-3.5 w-3.5 mr-1.5" />
                Sample Popup
              </Button>
            </div>
          )}
        </CardContent>
      </Card>
    </div>
  )
}
