import { useState, useEffect, useCallback } from "react"
import { invoke } from "@tauri-apps/api/core"
import { listen } from "@tauri-apps/api/event"
import { Card, CardContent, CardHeader, CardTitle, CardDescription } from "@/components/ui/Card"
import { SkillsPermissionTree } from "@/components/permissions"
import { SamplePopupButton } from "@/components/shared/SamplePopupButton"
import { ToolList } from "@/components/shared/ToolList"
import type { ToolListItem } from "@/components/shared/ToolList"
import type { SkillsPermissions } from "@/components/permissions"
import type { SkillInfo, SkillToolInfo } from "@/types/tauri-commands"

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
  const [skillTools, setSkillTools] = useState<ToolListItem[]>([])

  const loadSkillTools = useCallback(async () => {
    try {
      const skills = await invoke<SkillInfo[]>("list_skills")
      const enabledSkills = skills.filter((s) => s.enabled)

      const allTools: ToolListItem[] = []
      for (const skill of enabledSkills) {
        try {
          const tools = await invoke<SkillToolInfo[]>("get_skill_tools", {
            skillName: skill.name,
          })
          for (const t of tools) {
            allTools.push({
              name: `${skill.name}/${t.name}`,
              description: t.description,
            })
          }
        } catch {
          // skip skill if tools fail to load
        }
      }
      setSkillTools(allTools)
    } catch {
      setSkillTools([])
    }
  }, [])

  useEffect(() => {
    loadSkillTools()

    const unsubscribe = listen("skills-changed", () => {
      loadSkillTools()
    })

    return () => {
      unsubscribe.then((fn) => fn())
    }
  }, [loadSkillTools])

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
          {skillTools.length > 0 && (
            <div className="p-4 rounded-lg bg-muted/50 border space-y-2 mb-4">
              <p className="text-sm text-muted-foreground">
                When enabled, this client will have access to {skillTools.length} skill tools:
              </p>
              <ToolList
                tools={skillTools}
                compact
              />
            </div>
          )}
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
