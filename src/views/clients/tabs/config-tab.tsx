
import { useState, useEffect } from "react"
import { invoke } from "@tauri-apps/api/core"
import { listenSafe } from "@/hooks/useTauriListener"
import { HowToConnect } from "@/components/client/HowToConnect"
import type { ClientMode } from "@/types/tauri-commands"

interface Client {
  id: string
  name: string
  client_id: string
  enabled: boolean
  strategy_id: string
  client_mode?: ClientMode
  template_id?: string | null
  sync_config?: boolean
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
    let cancelled = false
    // Reset so the previous client's secret never leaks into the UI while the
    // new fetch is in flight.
    setSecret(null)
    setLoadingSecret(true)

    const fetchSecret = async () => {
      try {
        const value = await invoke<string>("get_client_value", { id: client.id })
        if (cancelled) return
        setSecret(value)
      } catch (error) {
        if (cancelled) return
        console.error("Failed to fetch client secret:", error)
        setSecret(null)
      } finally {
        if (!cancelled) setLoadingSecret(false)
      }
    }
    fetchSecret()

    // Also refetch when clients change (e.g., after credential rotation)
    const l = listenSafe("clients-changed", () => {
      fetchSecret()
    })

    return () => {
      cancelled = true
      l.cleanup()
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
        templateId={client.template_id}
        clientMode={client.client_mode}
        syncConfig={client.sync_config}
      />
    </div>
  )
}
