import { Card, CardContent, CardHeader, CardTitle, CardDescription } from "@/components/ui/Card"
import { SkillsPermissionTree } from "@/components/permissions"
import type { SkillsPermissions } from "@/components/permissions"

interface Client {
  id: string
  name: string
  client_id: string
  skills_permissions: SkillsPermissions
}

interface SkillsTabProps {
  client: Client
  onUpdate: () => void
}

export function ClientSkillsTab({ client, onUpdate }: SkillsTabProps) {
  return (
    <div className="space-y-6">
      <Card>
        <CardHeader>
          <CardTitle>Skills Permissions</CardTitle>
          <CardDescription>
            Control which skills and their tools this client can access.
            Use "Ask" to require approval before execution.
          </CardDescription>
        </CardHeader>
        <CardContent>
          <SkillsPermissionTree
            clientId={client.client_id}
            permissions={client.skills_permissions}
            onUpdate={onUpdate}
          />
        </CardContent>
      </Card>
    </div>
  )
}
