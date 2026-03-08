import { useState, useEffect } from "react"
import { invoke } from "@tauri-apps/api/core"
import { toast } from "sonner"
import { AlertTriangle } from "lucide-react"
import { Card, CardContent, CardHeader, CardTitle, CardDescription } from "@/components/ui/Card"
import { McpPermissionTree, PermissionStateButton } from "@/components/permissions"
import type { McpPermissions, PermissionState } from "@/components/permissions"

interface Client {
  id: string
  name: string
  client_id: string
  mcp_permissions: McpPermissions
  marketplace_permission: PermissionState
}

interface McpTabProps {
  client: Client
  onUpdate: () => void
}

export function ClientMcpTab({ client, onUpdate }: McpTabProps) {
  const [saving, setSaving] = useState(false)
  const [marketplacePermission, setMarketplacePermission] = useState<PermissionState>(
    client.marketplace_permission
  )

  useEffect(() => {
    setMarketplacePermission(client.marketplace_permission)
  }, [client.marketplace_permission])

  const handleMarketplacePermissionChange = async (state: PermissionState) => {
    try {
      setSaving(true)
      await invoke("set_client_marketplace_permission", {
        clientId: client.client_id,
        state,
      })
      setMarketplacePermission(state)
      toast.success("Marketplace permission updated")
      onUpdate()
    } catch (error) {
      console.error("Failed to update marketplace permission:", error)
      toast.error("Failed to update permission")
    } finally {
      setSaving(false)
    }
  }

  return (
    <div className="space-y-6">
      {/* MCP Server Permissions */}
      <Card>
        <CardHeader>
          <CardTitle>MCP Server Permissions</CardTitle>
          <CardDescription>
            Control which MCP servers and their tools this client can access.
            Use "Ask" to require approval before execution.
          </CardDescription>
        </CardHeader>
        <CardContent>
          <McpPermissionTree
            clientId={client.client_id}
            permissions={client.mcp_permissions}
            onUpdate={onUpdate}
          />
        </CardContent>
      </Card>

      {/* Marketplace Access */}
      <Card>
        <CardHeader>
          <div className="flex items-center justify-between">
            <CardTitle className="text-base">Marketplace Access</CardTitle>
            <PermissionStateButton
              value={marketplacePermission}
              onChange={handleMarketplacePermissionChange}
              disabled={saving}
              size="sm"
            />
          </div>
          <CardDescription>
            Allow this client to search and install MCP servers and skills from the marketplace
          </CardDescription>
        </CardHeader>
        <CardContent className="space-y-4">
          <div className="p-4 rounded-lg bg-muted/50 border">
            <p className="text-sm text-muted-foreground">
              When enabled, this client will have access to 4 marketplace tools:
            </p>
            <ul className="list-disc list-inside mt-2 text-sm text-muted-foreground space-y-1">
              <li><code className="px-1 py-0.5 rounded bg-muted text-xs">marketplace__search_mcp_servers</code> - Search the MCP registry</li>
              <li><code className="px-1 py-0.5 rounded bg-muted text-xs">marketplace__install_mcp_server</code> - Install an MCP server</li>
              <li><code className="px-1 py-0.5 rounded bg-muted text-xs">marketplace__search_skills</code> - Browse skill repositories</li>
              <li><code className="px-1 py-0.5 rounded bg-muted text-xs">marketplace__install_skill</code> - Install a skill</li>
            </ul>
          </div>
          {marketplacePermission === "allow" && (
            <div className="p-3 rounded-lg border border-amber-600/50 bg-amber-500/10">
              <div className="flex gap-2 items-start">
                <AlertTriangle className="h-4 w-4 text-amber-600 dark:text-amber-400 mt-0.5 shrink-0" />
                <p className="text-sm text-amber-900 dark:text-amber-400">
                  Warning: Allowing marketplace grants access to install any item without approval.
                  Only enable if you trust the configured marketplace sources.
                </p>
              </div>
            </div>
          )}
          {marketplacePermission === "ask" && (
            <div className="p-3 rounded-lg border border-blue-600/50 bg-blue-500/10">
              <p className="text-sm text-blue-900 dark:text-blue-400">
                Install requests from AI clients will show a confirmation dialog before proceeding.
              </p>
            </div>
          )}
        </CardContent>
      </Card>

    </div>
  )
}
