import { Card, CardContent, CardHeader, CardTitle, CardDescription } from "@/components/ui/Card"
import { SkillsPermissionTree } from "@/components/permissions"
import { SamplePopupButton } from "@/components/shared/SamplePopupButton"
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
          <div className="flex items-center justify-between mb-4 pb-4 border-b">
            <div>
              <span className="text-sm font-medium">Approval Popup Preview</span>
              <p className="text-xs text-muted-foreground mt-0.5">
                Preview the popup shown when a skill tool is set to &ldquo;Ask&rdquo;
              </p>
            </div>
            <SamplePopupButton popupType="skill" />
          </div>
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
