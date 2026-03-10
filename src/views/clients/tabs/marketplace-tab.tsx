import { useState, useEffect } from "react"
import { invoke } from "@tauri-apps/api/core"
import { toast } from "sonner"
import { AlertTriangle } from "lucide-react"
import { Card, CardContent, CardHeader, CardTitle, CardDescription } from "@/components/ui/Card"
import { PermissionStateButton } from "@/components/permissions"
import { SamplePopupButton } from "@/components/shared/SamplePopupButton"
import { ToolList } from "@/components/shared/ToolList"
import type { ToolListItem } from "@/components/shared/ToolList"
import type { PermissionState } from "@/components/permissions"
import type { ToolDefinition } from "@/types/tauri-commands"

interface Client {
  id: string
  name: string
  client_id: string
  marketplace_permission: PermissionState
}

interface MarketplaceTabProps {
  client: Client
  onUpdate: () => void
}

export function ClientMarketplaceTab({ client, onUpdate }: MarketplaceTabProps) {
  const [saving, setSaving] = useState(false)
  const [marketplacePermission, setMarketplacePermission] = useState<PermissionState>(
    client.marketplace_permission
  )
  const [marketplaceTools, setMarketplaceTools] = useState<ToolListItem[]>([])

  useEffect(() => {
    setMarketplacePermission(client.marketplace_permission)
  }, [client.marketplace_permission])

  // Fetch tool definitions from backend
  useEffect(() => {
    invoke<ToolDefinition[]>("get_marketplace_tool_definitions")
      .then((defs) =>
        setMarketplaceTools(
          defs.map((d): ToolListItem => ({
            name: d.name,
            description: d.description,
            inputSchema: d.input_schema,
          }))
        )
      )
      .catch(() => setMarketplaceTools([]))
  }, [])

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
            Allow this client to search and install MCP servers and skills from the marketplace.
            Approval is only required for installation &mdash; searching is always allowed.
          </CardDescription>
        </CardHeader>
        <CardContent className="space-y-4">
          <div className="p-4 rounded-lg bg-muted/50 border space-y-2">
            <p className="text-sm text-muted-foreground">
              When enabled, this client will have access to {marketplaceTools.length} marketplace tools:
            </p>
            <ToolList
              tools={marketplaceTools}
              compact
            />
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
          <div className="border-t pt-3 mt-3 flex items-center justify-between">
            <div>
              <span className="text-sm font-medium">Approval Popup Preview</span>
              <p className="text-xs text-muted-foreground mt-0.5">
                Preview the popup shown when a client attempts to install from the marketplace
              </p>
            </div>
            <SamplePopupButton popupType="marketplace" />
          </div>
        </CardContent>
      </Card>
    </div>
  )
}
