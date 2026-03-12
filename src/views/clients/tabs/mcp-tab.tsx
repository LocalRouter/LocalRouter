import { useState } from "react"
import { Tabs, TabsContent, TabsList, TabsTrigger } from "@/components/ui/tabs"
import { Card, CardContent, CardHeader, CardTitle, CardDescription } from "@/components/ui/Card"
import { McpPermissionTree } from "@/components/permissions"
import { SamplePopupButton } from "@/components/shared/SamplePopupButton"
import { ClientSkillsTab } from "./skills-tab"
import { ClientCodingAgentsTab } from "./coding-agents-tab"
import { ClientMarketplaceTab } from "./marketplace-tab"
import type { McpPermissions, SkillsPermissions, PermissionState } from "@/components/permissions"
import type { CodingAgentType } from "@/types/tauri-commands"

interface Client {
  id: string
  name: string
  client_id: string
  mcp_permissions: McpPermissions
  skills_permissions: SkillsPermissions
  coding_agent_permission: PermissionState
  coding_agent_type: CodingAgentType | null
  marketplace_permission: PermissionState
}

interface McpTabProps {
  client: Client
  onUpdate: () => void
}

export function ClientMcpTab({ client, onUpdate }: McpTabProps) {
  const [subTab, setSubTab] = useState("servers")

  return (
    <Tabs value={subTab} onValueChange={setSubTab}>
      <TabsList className="bg-muted h-9 p-1 rounded-lg mb-4">
        <TabsTrigger value="servers">MCP Servers</TabsTrigger>
        <TabsTrigger value="skills">Skills</TabsTrigger>
        <TabsTrigger value="coding-agents">Coding Agents</TabsTrigger>
        <TabsTrigger value="marketplace">Marketplace</TabsTrigger>
      </TabsList>

      <TabsContent value="servers">
        <div className="space-y-6">
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
        </div>
      </TabsContent>

      <TabsContent value="skills">
        <ClientSkillsTab client={client} onUpdate={onUpdate} />
      </TabsContent>

      <TabsContent value="coding-agents">
        <ClientCodingAgentsTab client={client} onUpdate={onUpdate} />
      </TabsContent>

      <TabsContent value="marketplace">
        <ClientMarketplaceTab client={client} onUpdate={onUpdate} />
      </TabsContent>
    </Tabs>
  )
}
