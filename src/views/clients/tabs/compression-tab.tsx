import { useState, useEffect, useCallback } from "react"
import { invoke } from "@tauri-apps/api/core"
import { toast } from "sonner"
import { Minimize2, Info } from "lucide-react"
import { Card, CardContent, CardHeader, CardTitle, CardDescription } from "@/components/ui/Card"
import { Badge } from "@/components/ui/Badge"
import { Switch } from "@/components/ui/Toggle"
import { Input } from "@/components/ui/Input"
import { Label } from "@/components/ui/label"
import { PresetSlider } from "@/components/ui/PresetSlider"
import { TriStateButton } from "@/components/ui/TriStateButton"
import { COMPRESSION_PRESETS } from "@/components/compression/types"
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
    min_messages: null,
    preserve_recent: null,
    rate: null,
    compress_system_prompt: null,
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

  const effectiveRate = config.rate ?? globalConfig?.default_rate ?? 0.8
  const effectiveMinMessages = config.min_messages ?? globalConfig?.min_messages ?? 6
  const effectivePreserveRecent = config.preserve_recent ?? globalConfig?.preserve_recent ?? 4
  const effectiveCompressSystem = config.compress_system_prompt ?? globalConfig?.compress_system_prompt ?? false

  return (
    <div className="space-y-4">
      {/* Header Card */}
      <Card>
        <CardHeader>
          <div className="flex items-center gap-2">
            <Minimize2 className="h-5 w-5 text-blue-500" />
            <CardTitle>Prompt Compression</CardTitle>
            <Badge variant="outline" className="bg-purple-500/10 text-purple-900 dark:text-purple-400">EXPERIMENTAL</Badge>
          </div>
          <CardDescription>
            Compress prompts before sending to the target LLM using LLMLingua-2 extractive compression.
            Reduces input tokens without hallucination.
            {onViewChange && (
              <button
                className="text-blue-500 hover:underline ml-1"
                onClick={() => onViewChange("compression", "try-it-out")}
              >
                Test in Try It Out
              </button>
            )}
          </CardDescription>
        </CardHeader>
      </Card>

      {/* Enable/Disable */}
      <Card>
        <CardHeader>
          <div className="flex items-center justify-between">
            <CardTitle className="text-base">Enable Compression</CardTitle>
            <TriStateButton
              value={config.enabled}
              onChange={(v) => saveConfig({ ...config, enabled: v })}
              defaultLabel={`Default (${globalConfig?.enabled ? "on" : "off"})`}
              onLabel="On"
              offLabel="Off"
            />
          </div>
          <CardDescription>
            Override the global compression setting for this client.
            When set to "Default", the{" "}
            {onViewChange ? (
              <button
                className="text-blue-500 hover:underline"
                onClick={() => onViewChange("compression", "settings")}
              >
                global setting
              </button>
            ) : (
              "global setting"
            )}{" "}
            applies.
          </CardDescription>
        </CardHeader>
      </Card>

      {/* Compression Settings */}
      <Card>
        <CardHeader>
          <CardTitle className="text-base">Compression Settings</CardTitle>
          <CardDescription>
            Override global defaults for this client. Leave unset to inherit from{" "}
            {onViewChange ? (
              <button
                className="text-blue-500 hover:underline"
                onClick={() => onViewChange("compression", "settings")}
              >
                global settings
              </button>
            ) : (
              "global settings"
            )}.
          </CardDescription>
        </CardHeader>
        <CardContent className="space-y-6">
          {/* Compression Rate */}
          <div className="space-y-2">
            {config.rate !== null && (
              <div className="flex justify-end">
                <button
                  className="text-xs text-muted-foreground hover:text-foreground"
                  onClick={() => saveConfig({ ...config, rate: null })}
                >
                  Reset to global
                </button>
              </div>
            )}
            <PresetSlider
              label={config.rate !== null ? "Compression Rate" : `Compression Rate (global: ${Math.round(effectiveRate * 100)}%)`}
              value={effectiveRate}
              onChange={(v) => saveConfig({ ...config, rate: v })}
              presets={COMPRESSION_PRESETS}
              min={0.1}
              max={1}
              step={0.01}
              minLabel="More compression"
              maxLabel="More tokens preserved"
              formatValue={(v) => `${Math.round(v * 100)}%`}
            />
          </div>

          {/* Min Messages */}
          <div className="space-y-2">
            <div className="flex items-center justify-between">
              <Label className="text-sm">Minimum Messages</Label>
              <div className="flex items-center gap-2">
                {config.min_messages !== null && (
                  <button
                    className="text-xs text-muted-foreground hover:text-foreground"
                    onClick={() => saveConfig({ ...config, min_messages: null })}
                  >
                    Reset to global
                  </button>
                )}
                <Input
                  type="number"
                  min={1}
                  max={100}
                  className="w-20 h-7 text-xs"
                  value={effectiveMinMessages}
                  onChange={(e) => saveConfig({ ...config, min_messages: parseInt(e.target.value) || 1 })}
                />
              </div>
            </div>
            <p className="text-xs text-muted-foreground">
              Only compress when conversation has at least this many messages.
              {config.min_messages === null && ` Global default: ${globalConfig?.min_messages ?? 6}`}
            </p>
          </div>

          {/* Preserve Recent */}
          <div className="space-y-2">
            <div className="flex items-center justify-between">
              <Label className="text-sm">Preserve Recent Messages</Label>
              <div className="flex items-center gap-2">
                {config.preserve_recent !== null && (
                  <button
                    className="text-xs text-muted-foreground hover:text-foreground"
                    onClick={() => saveConfig({ ...config, preserve_recent: null })}
                  >
                    Reset to global
                  </button>
                )}
                <Input
                  type="number"
                  min={0}
                  max={50}
                  className="w-20 h-7 text-xs"
                  value={effectivePreserveRecent}
                  onChange={(e) => saveConfig({ ...config, preserve_recent: parseInt(e.target.value) || 0 })}
                />
              </div>
            </div>
            <p className="text-xs text-muted-foreground">
              Keep the last N messages uncompressed for best context quality.
              {config.preserve_recent === null && ` Global default: ${globalConfig?.preserve_recent ?? 4}`}
            </p>
          </div>

          {/* Compress System Prompt */}
          <div className="flex items-center justify-between">
            <div className="space-y-0.5">
              <Label className="text-sm">Compress System Prompt</Label>
              <p className="text-xs text-muted-foreground">
                Also compress system messages. Usually best left off.
                {config.compress_system_prompt === null && ` Global: ${effectiveCompressSystem ? "on" : "off"}`}
              </p>
            </div>
            <div className="flex items-center gap-2">
              {config.compress_system_prompt !== null && (
                <button
                  className="text-xs text-muted-foreground hover:text-foreground"
                  onClick={() => saveConfig({ ...config, compress_system_prompt: null })}
                >
                  Reset
                </button>
              )}
              <Switch
                checked={effectiveCompressSystem}
                onCheckedChange={(checked) => saveConfig({ ...config, compress_system_prompt: checked })}
              />
            </div>
          </div>
        </CardContent>
      </Card>

      {/* Info */}
      <div className="p-4 rounded-lg bg-blue-500/10 border border-blue-600/50">
        <div className="flex items-start gap-3">
          <Info className="h-5 w-5 text-blue-600 dark:text-blue-400 mt-0.5 shrink-0" />
          <div className="space-y-1">
            <p className="text-sm text-blue-900 dark:text-blue-400">
              Model size and service configuration are managed in{" "}
              {onViewChange ? (
                <button
                  className="text-blue-500 hover:underline"
                  onClick={() => onViewChange("compression", "settings")}
                >
                  Compression &rarr; Settings
                </button>
              ) : (
                "Compression settings"
              )}.
              Currently using <strong>{globalConfig?.model_size || "mobile"}</strong> model.
            </p>
          </div>
        </div>
      </div>
    </div>
  )
}
