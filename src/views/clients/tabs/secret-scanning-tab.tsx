import { useState, useEffect, useCallback, useRef } from "react"
import { invoke } from "@tauri-apps/api/core"
import { toast } from "sonner"
import { FEATURES } from "@/constants/features"
import { Card, CardHeader, CardTitle, CardDescription } from "@/components/ui/Card"
import { cn } from "@/lib/utils"
import type {
  ClientSecretScanningConfig,
  SecretScanningConfig,
  SecretScanAction,
} from "@/types/tauri-commands"

interface Client {
  id: string
  name: string
  client_id: string
}

interface SecretScanningTabProps {
  client: Client
  onUpdate: () => void
  onViewChange?: (view: string, subTab?: string | null) => void
}

const ACTION_LABELS: Record<SecretScanAction, string> = {
  ask: "Ask",
  notify: "Notify",
  off: "Off",
}

type ButtonValue = "default" | SecretScanAction

const BUTTON_STYLES: Record<ButtonValue, string> = {
  default: "bg-zinc-500 text-white",
  ask: "bg-amber-500 text-white",
  notify: "bg-blue-500 text-white",
  off: "bg-red-500 text-white",
}

export function ClientSecretScanningTab({ client, onUpdate, onViewChange }: SecretScanningTabProps) {
  const [config, setConfig] = useState<ClientSecretScanningConfig>({ action: null })
  const [globalConfig, setGlobalConfig] = useState<SecretScanningConfig | null>(null)
  const [loading, setLoading] = useState(true)

  const loadReqIdRef = useRef(0)

  const loadConfig = useCallback(async () => {
    const reqId = ++loadReqIdRef.current
    try {
      const [clientConfig, global] = await Promise.all([
        invoke<ClientSecretScanningConfig>("get_client_secret_scanning_config", {
          clientId: client.id,
        } as Record<string, unknown>),
        invoke<SecretScanningConfig>("get_secret_scanning_config"),
      ])
      if (loadReqIdRef.current !== reqId) return
      setConfig(clientConfig)
      setGlobalConfig(global)
    } catch (err) {
      if (loadReqIdRef.current !== reqId) return
      console.error("Failed to load secret scanning config:", err)
      toast.error("Failed to load secret scanning configuration")
    } finally {
      if (loadReqIdRef.current === reqId) setLoading(false)
    }
  }, [client.id])

  useEffect(() => {
    setLoading(true)
    loadConfig()
    return () => {
      loadReqIdRef.current++
    }
  }, [loadConfig])

  const saveConfig = async (newConfig: ClientSecretScanningConfig) => {
    setConfig(newConfig)
    try {
      await invoke("update_client_secret_scanning_config", {
        clientId: client.id,
        configJson: JSON.stringify(newConfig),
      } as Record<string, unknown>)
      onUpdate()
    } catch (err) {
      toast.error("Failed to save secret scanning configuration")
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

  const globalActionLabel = globalConfig ? ACTION_LABELS[globalConfig.action] : "Off"
  const current: ButtonValue = config.action ?? "default"

  const buttons: { key: ButtonValue; label: string }[] = [
    { key: "default", label: `Default (${globalActionLabel})` },
    { key: "ask", label: "Ask" },
    { key: "notify", label: "Notify" },
    { key: "off", label: "Off" },
  ]

  return (
    <div className="space-y-4">
      <Card>
        <CardHeader>
          <div className="flex items-center justify-between">
            <div className="flex items-center gap-2">
              <FEATURES.secretScanning.icon className={`h-5 w-5 ${FEATURES.secretScanning.color}`} />
              <CardTitle>Secret Scanning</CardTitle>
            </div>
            <div className="inline-flex rounded-md border border-border bg-muted/50">
              {buttons.map(({ key, label }, i) => {
                const isActive = current === key
                return (
                  <button
                    key={key}
                    type="button"
                    onClick={() => saveConfig({ action: key === "default" ? null : key })}
                    className={cn(
                      "px-2 py-0.5 text-xs transition-colors font-medium",
                      isActive
                        ? BUTTON_STYLES[key]
                        : "text-muted-foreground hover:text-foreground hover:bg-muted",
                      i === 0 && "rounded-l-md",
                      i === buttons.length - 1 && "rounded-r-md"
                    )}
                  >
                    {label}
                  </button>
                )
              })}
            </div>
          </div>
          <CardDescription>
            Detect potential secrets (API keys, tokens, passwords) in outbound requests
            before they reach the provider. Configure global scanning rules in{" "}
            {onViewChange ? (
              <button
                className="text-blue-500 hover:underline"
                onClick={() => onViewChange("secret-scanning")}
              >
                Secret Scanning settings
              </button>
            ) : (
              "Secret Scanning settings"
            )}.
          </CardDescription>
        </CardHeader>
      </Card>
    </div>
  )
}
