
import { useState, useEffect } from "react"
import { invoke } from "@tauri-apps/api/core"
import { toast } from "sonner"
import { Card, CardContent, CardHeader, CardTitle, CardDescription } from "@/components/ui/Card"
import { Button } from "@/components/ui/Button"
import { Input } from "@/components/ui/Input"
import { Label } from "@/components/ui/label"

interface Client {
  id: string
  name: string
  client_id: string
  enabled: boolean
  strategy_id: string
  allowed_llm_providers: string[]
  mcp_access_mode: "none" | "all" | "specific"
  mcp_servers: string[]
}

interface ConfigTabProps {
  client: Client
  onUpdate: () => void
}

export function ClientConfigTab({ client, onUpdate }: ConfigTabProps) {
  const [name, setName] = useState(client.name)
  const [saving, setSaving] = useState(false)

  // Sync name state when client prop updates (e.g., after save)
  useEffect(() => {
    setName(client.name)
  }, [client.name])

  const handleSave = async () => {
    try {
      setSaving(true)
      await invoke("update_client_name", {
        clientId: client.client_id,
        name,
      })
      toast.success("Client updated")
      onUpdate()
    } catch (error) {
      console.error("Failed to update client:", error)
      toast.error("Failed to update client")
    } finally {
      setSaving(false)
    }
  }

  return (
    <div className="space-y-6">
      <Card>
        <CardHeader>
          <CardTitle>General Settings</CardTitle>
          <CardDescription>
            Basic client configuration
          </CardDescription>
        </CardHeader>
        <CardContent className="space-y-4">
          <div className="space-y-2">
            <Label htmlFor="name">Client Name</Label>
            <Input
              id="name"
              value={name}
              onChange={(e) => setName(e.target.value)}
              placeholder="Enter client name"
            />
          </div>

          <Button
            onClick={handleSave}
            disabled={saving || name === client.name}
          >
            {saving ? "Saving..." : "Save Changes"}
          </Button>
        </CardContent>
      </Card>
    </div>
  )
}
