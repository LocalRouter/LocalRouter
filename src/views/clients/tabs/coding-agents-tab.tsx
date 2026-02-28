import { Card, CardContent, CardHeader, CardTitle, CardDescription } from "@/components/ui/Card"
import { CodingAgentsPermissionTree } from "@/components/permissions"
import type { CodingAgentsPermissions } from "@/components/permissions"

interface Client {
  id: string
  name: string
  client_id: string
  coding_agents_permissions: CodingAgentsPermissions
}

interface CodingAgentsTabProps {
  client: Client
  onUpdate: () => void
}

export function ClientCodingAgentsTab({ client, onUpdate }: CodingAgentsTabProps) {
  return (
    <div className="space-y-6">
      <Card>
        <CardHeader>
          <CardTitle>Coding Agents Permissions</CardTitle>
          <CardDescription>
            Control which AI coding agents this client can spawn and interact with.
            Use "Ask" to require approval before starting a session.
          </CardDescription>
        </CardHeader>
        <CardContent>
          <CodingAgentsPermissionTree
            clientId={client.client_id}
            permissions={client.coding_agents_permissions}
            onUpdate={onUpdate}
          />
        </CardContent>
      </Card>
    </div>
  )
}
