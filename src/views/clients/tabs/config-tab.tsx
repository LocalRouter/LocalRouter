
import { useState, useEffect } from "react"
import { invoke } from "@tauri-apps/api/core"
import { listen } from "@tauri-apps/api/event"
import { HowToConnect } from "@/components/client/HowToConnect"

interface Client {
  id: string
  name: string
  client_id: string
  enabled: boolean
  strategy_id: string
}

interface ConfigTabProps {
  client: Client
  onUpdate: () => void
}

export function ClientConfigTab({ client }: ConfigTabProps) {
  // Credentials state
  const [secret, setSecret] = useState<string | null>(null)
  const [loadingSecret, setLoadingSecret] = useState(true)

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

    // Also refetch when clients change (e.g., after credential rotation)
    const unsubscribe = listen("clients-changed", () => {
      fetchSecret()
    })

    return () => {
      unsubscribe.then((fn) => fn())
    }
  }, [client.id])

  return (
    <div className="space-y-6">
      {/* Connection Instructions */}
      <HowToConnect
        clientId={client.client_id}
        clientUuid={client.id}
        secret={secret}
        loadingSecret={loadingSecret}
      />
    </div>
  )
}
