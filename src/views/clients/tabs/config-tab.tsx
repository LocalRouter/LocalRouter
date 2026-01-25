
import { useState, useEffect, useRef, useCallback } from "react"
import { invoke } from "@tauri-apps/api/core"
import { toast } from "sonner"
import { Loader2 } from "lucide-react"
import { Card, CardContent, CardHeader, CardTitle, CardDescription } from "@/components/ui/Card"
import { Input } from "@/components/ui/Input"
import { HowToConnect } from "@/components/client/HowToConnect"

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

  // Credentials state
  const [secret, setSecret] = useState<string | null>(null)
  const [loadingSecret, setLoadingSecret] = useState(true)
  const [rotating, setRotating] = useState(false)

  // Debounce ref for name updates
  const nameTimeoutRef = useRef<ReturnType<typeof setTimeout> | null>(null)

  // Sync name state when client prop updates
  useEffect(() => {
    setName(client.name)
  }, [client.name])

  // Fetch the secret from keychain when component mounts or client changes
  useEffect(() => {
    const fetchSecret = async () => {
      setLoadingSecret(true)
      try {
        const value = await invoke<string>("get_client_value", { id: client.id })
        setSecret(value)
      } catch (error) {
        console.error("Failed to fetch client secret:", error)
        setSecret(null)
      } finally {
        setLoadingSecret(false)
      }
    }
    fetchSecret()
  }, [client.id])

  // Cleanup timeout on unmount
  useEffect(() => {
    return () => {
      if (nameTimeoutRef.current) {
        clearTimeout(nameTimeoutRef.current)
      }
    }
  }, [])

  // Debounced name save
  const handleNameChange = useCallback((newName: string) => {
    setName(newName)

    // Clear existing timeout
    if (nameTimeoutRef.current) {
      clearTimeout(nameTimeoutRef.current)
    }

    // Debounce the save
    nameTimeoutRef.current = setTimeout(async () => {
      if (newName === client.name || !newName.trim()) return

      try {
        setSaving(true)
        await invoke("update_client_name", {
          clientId: client.client_id,
          name: newName,
        })
        toast.success("Client name updated")
        onUpdate()
      } catch (error) {
        console.error("Failed to update client:", error)
        toast.error("Failed to update client")
      } finally {
        setSaving(false)
      }
    }, 500) // 500ms debounce
  }, [client.name, client.client_id, onUpdate])

  const handleRotateKey = async () => {
    try {
      setRotating(true)
      await invoke("rotate_client_secret", { clientId: client.id })
      // Refetch the new secret after rotation
      const newSecret = await invoke<string>("get_client_value", { id: client.id })
      setSecret(newSecret)
      toast.success("Credentials rotated successfully")
      onUpdate()
    } catch (error) {
      console.error("Failed to rotate credentials:", error)
      toast.error("Failed to rotate credentials")
    } finally {
      setRotating(false)
    }
  }

  return (
    <div className="space-y-6">
      {/* Client Name */}
      <Card>
        <CardHeader>
          <CardTitle>Client Name</CardTitle>
          <CardDescription>
            Display name for this client
          </CardDescription>
        </CardHeader>
        <CardContent>
          <div className="flex items-center gap-2">
            <Input
              id="name"
              value={name}
              onChange={(e) => handleNameChange(e.target.value)}
              placeholder="Enter client name"
              className="max-w-md"
            />
            {saving && (
              <Loader2 className="h-4 w-4 animate-spin text-muted-foreground" />
            )}
          </div>
        </CardContent>
      </Card>

      {/* Connection Instructions */}
      <HowToConnect
        clientId={client.client_id}
        clientUuid={client.id}
        secret={secret}
        loadingSecret={loadingSecret}
        showRotateCredentials={true}
        onRotate={handleRotateKey}
        rotating={rotating}
      />
    </div>
  )
}
