import { useState, useEffect } from "react"
import { invoke } from "@tauri-apps/api/core"
import { toast } from "sonner"
import { Card, CardContent, CardHeader, CardTitle, CardDescription } from "@/components/ui/Card"
import { PermissionStateButton } from "@/components/permissions"
import { SamplePopupButton } from "@/components/shared/SamplePopupButton"
import type { PermissionState } from "@/components/permissions"

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
              allowedStates={["ask", "off"]}
            />
          </div>
          <CardDescription>
            Allow this client to search and install MCP servers and skills from the marketplace.
            Approval is only required for installation &mdash; searching is always allowed.
          </CardDescription>
        </CardHeader>
        <CardContent className="space-y-4">
          <div className="border-t pt-3 flex items-center justify-between">
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
