
import { useState, useEffect } from "react"
import { invoke } from "@tauri-apps/api/core"
import { toast } from "sonner"
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
  // Credentials state
  const [secret, setSecret] = useState<string | null>(null)
  const [loadingSecret, setLoadingSecret] = useState(true)
  const [rotating, setRotating] = useState(false)

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
