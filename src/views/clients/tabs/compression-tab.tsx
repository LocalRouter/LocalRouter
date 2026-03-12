import { useState, useEffect, useCallback } from "react"
import { invoke } from "@tauri-apps/api/core"
import { toast } from "sonner"
import { Minimize2 } from "lucide-react"
import { Card, CardHeader, CardTitle, CardDescription } from "@/components/ui/Card"
import { Badge } from "@/components/ui/Badge"
import { TriStateButton } from "@/components/ui/TriStateButton"
import type {
  ClientPromptCompressionConfig,
  PromptCompressionConfig,
  GetClientCompressionConfigParams,
  UpdateClientCompressionConfigParams,
} from "@/types/tauri-commands"

interface Client {
  id: string
  name: string
  client_id: string
}

interface CompressionTabProps {
  client: Client
  onUpdate: () => void
  onViewChange?: (view: string, subTab?: string | null) => void
}

export function ClientCompressionTab({ client, onUpdate, onViewChange }: CompressionTabProps) {
  const [config, setConfig] = useState<ClientPromptCompressionConfig>({
    enabled: null,
  })
  const [globalConfig, setGlobalConfig] = useState<PromptCompressionConfig | null>(null)
  const [loading, setLoading] = useState(true)

  const loadConfig = useCallback(async () => {
    try {
      const [clientConfig, global] = await Promise.all([
        invoke<ClientPromptCompressionConfig>("get_client_compression_config", {
          clientId: client.id,
        } satisfies GetClientCompressionConfigParams as Record<string, unknown>),
        invoke<PromptCompressionConfig>("get_compression_config"),
      ])
      setConfig(clientConfig)
      setGlobalConfig(global)
    } catch (err) {
      console.error("Failed to load compression config:", err)
      toast.error("Failed to load compression configuration")
    } finally {
      setLoading(false)
    }
  }, [client.id])

  useEffect(() => {
    loadConfig()
  }, [loadConfig])

  const saveConfig = async (newConfig: ClientPromptCompressionConfig) => {
    setConfig(newConfig)
    try {
      await invoke("update_client_compression_config", {
        clientId: client.id,
        configJson: JSON.stringify(newConfig),
      } satisfies UpdateClientCompressionConfigParams as Record<string, unknown>)
      onUpdate()
    } catch (err) {
      toast.error("Failed to save compression configuration")
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
    <div className="space-y-4">
      <Card>
        <CardHeader>
          <div className="flex items-center justify-between">
            <div className="flex items-center gap-2">
              <Minimize2 className="h-5 w-5 text-blue-500" />
              <CardTitle>Prompt Compression</CardTitle>
              <Badge variant="outline" className="bg-purple-500/10 text-purple-900 dark:text-purple-400">EXPERIMENTAL</Badge>
            </div>
            <TriStateButton
              value={config.enabled}
              onChange={(v) => saveConfig({ ...config, enabled: v })}
              defaultLabel={`Default (${globalConfig?.enabled ? "On" : "Off"})`}
              onLabel="On"
              offLabel="Off"
            />
          </div>
          <CardDescription>
            Compress prompts before sending to the target LLM using LLMLingua-2 extractive compression.
            Reduces input tokens without hallucination. Configure compression settings in{" "}
            {onViewChange ? (
              <button
                className="text-blue-500 hover:underline"
                onClick={() => onViewChange("compression", "settings")}
              >
                Prompt Compression settings
              </button>
            ) : (
              "Prompt Compression settings"
            )}.
          </CardDescription>
        </CardHeader>
      </Card>
    </div>
  )
}
