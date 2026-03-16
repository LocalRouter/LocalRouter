import { useState, useEffect, useCallback } from "react"
import { invoke } from "@tauri-apps/api/core"
import { toast } from "sonner"
import { FEATURES } from "@/constants/features"
import { Card, CardContent, CardHeader, CardTitle, CardDescription } from "@/components/ui/Card"
import { Switch } from "@/components/ui/switch"
import { AlertTriangle } from "lucide-react"

interface Client {
  id: string
  name: string
  client_id: string
}

interface ClientMemoryTabProps {
  client: Client
  onUpdate: () => void
  onViewChange?: (view: string, subTab?: string | null) => void
}

export function ClientMemoryTab({ client, onUpdate, onViewChange }: ClientMemoryTabProps) {
  const [memoryEnabled, setMemoryEnabled] = useState<boolean | null>(null)
  const [loading, setLoading] = useState(true)

  const loadConfig = useCallback(async () => {
    try {
      const result = await invoke<{ memory_enabled: boolean | null }>("get_client_memory_config", {
        clientId: client.id,
      })
      setMemoryEnabled(result.memory_enabled)
    } catch (err) {
      console.error("Failed to load memory config:", err)
    } finally {
      setLoading(false)
    }
  }, [client.id])

  useEffect(() => {
    loadConfig()
  }, [loadConfig])

  const toggleMemory = async (enabled: boolean) => {
    try {
      await invoke("update_client_memory_config", {
        clientId: client.id,
        enabled,
      })
      setMemoryEnabled(enabled)
      onUpdate()
      toast.success(enabled ? "Memory enabled for this client" : "Memory disabled for this client")
    } catch (err: any) {
      toast.error(`Failed to update: ${err.message || err}`)
      loadConfig()
    }
  }

  if (loading) {
    return (
      <div className="flex items-center justify-center py-8">
        <p className="text-muted-foreground">Loading...</p>
      </div>
    )
  }

  return (
    <Card>
      <CardHeader>
        <div className="flex items-center justify-between">
          <div className="flex items-center gap-2">
            <FEATURES.memory.icon className={`h-5 w-5 ${FEATURES.memory.color}`} />
            <CardTitle>Memory</CardTitle>
          </div>
          <Switch
            checked={memoryEnabled === true}
            onCheckedChange={toggleMemory}
          />
        </div>
        <CardDescription>
          {memoryEnabled
            ? "Conversations with this client are recorded and stored locally for future recall."
            : "Enable to record conversations and make them searchable via the MemoryRecall tool."}
        </CardDescription>
      </CardHeader>
      {memoryEnabled && (
        <CardContent>
          <div className="flex items-start gap-2 text-xs text-amber-600 dark:text-amber-400">
            <AlertTriangle className="h-3.5 w-3.5 mt-0.5 shrink-0" />
            <span>
              Full conversations are stored locally when memory is enabled.{" "}
              <button
                className="underline hover:no-underline"
                onClick={() => onViewChange?.("memory")}
              >
                Review settings
              </button>
            </span>
          </div>
        </CardContent>
      )}
    </Card>
  )
}
