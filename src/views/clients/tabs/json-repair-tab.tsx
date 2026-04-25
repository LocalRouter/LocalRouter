import { useState, useEffect, useCallback, useRef } from "react"
import { invoke } from "@tauri-apps/api/core"
import { toast } from "sonner"
import { FEATURES } from "@/constants/features"
import { Card, CardHeader, CardTitle, CardDescription } from "@/components/ui/Card"
import { TriStateButton } from "@/components/ui/TriStateButton"
import { InfoTooltip } from "@/components/ui/info-tooltip"
import type {
  ClientJsonRepairConfig,
  JsonRepairConfig,
  GetClientJsonRepairConfigParams,
  UpdateClientJsonRepairConfigParams,
} from "@/types/tauri-commands"

interface Client {
  id: string
  name: string
  client_id: string
}

interface JsonRepairTabProps {
  client: Client
  onUpdate: () => void
  onViewChange?: (view: string, subTab?: string | null) => void
}

export function ClientJsonRepairTab({ client, onUpdate, onViewChange }: JsonRepairTabProps) {
  const [config, setConfig] = useState<ClientJsonRepairConfig>({
    enabled: null,
    syntax_repair: null,
    schema_coercion: null,
  })
  const [globalConfig, setGlobalConfig] = useState<JsonRepairConfig | null>(null)
  const [loading, setLoading] = useState(true)

  const loadReqIdRef = useRef(0)

  const loadConfig = useCallback(async () => {
    const reqId = ++loadReqIdRef.current
    try {
      const [clientConfig, global] = await Promise.all([
        invoke<ClientJsonRepairConfig>("get_client_json_repair_config", {
          clientId: client.id,
        } satisfies GetClientJsonRepairConfigParams as Record<string, unknown>),
        invoke<JsonRepairConfig>("get_json_repair_config"),
      ])
      if (loadReqIdRef.current !== reqId) return
      setConfig({
        enabled: clientConfig.enabled ?? null,
        syntax_repair: clientConfig.syntax_repair ?? null,
        schema_coercion: clientConfig.schema_coercion ?? null,
      })
      setGlobalConfig(global)
    } catch (err) {
      if (loadReqIdRef.current !== reqId) return
      console.error("Failed to load JSON repair config:", err)
      toast.error("Failed to load JSON repair configuration")
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

  const saveConfig = async (newConfig: ClientJsonRepairConfig) => {
    setConfig(newConfig)
    try {
      await invoke("update_client_json_repair_config", {
        clientId: client.id,
        configJson: JSON.stringify(newConfig),
      } satisfies UpdateClientJsonRepairConfigParams as Record<string, unknown>)
      onUpdate()
    } catch (err) {
      toast.error("Failed to save JSON repair configuration")
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
              <FEATURES.jsonRepair.icon className={`h-5 w-5 ${FEATURES.jsonRepair.color}`} />
              <CardTitle>JSON Repair</CardTitle>
            </div>
            <div className="flex items-center gap-1">
              <InfoTooltip content="Automatically repairs malformed JSON in LLM responses and coerces values to match the expected schema. Default inherits the global setting." />
              <TriStateButton
                value={config.enabled}
                onChange={(v) => saveConfig({ ...config, enabled: v })}
                defaultLabel={`Default (${globalConfig?.enabled ? "On" : "Off"})`}
                onLabel="On"
                offLabel="Off"
              />
            </div>
          </div>
          <CardDescription>
            Automatically repair malformed JSON responses and coerce valid JSON to match requested schemas.
            Configure repair settings in{" "}
            {onViewChange ? (
              <button
                className="text-blue-500 hover:underline"
                onClick={() => onViewChange("json-repair", "settings")}
              >
                JSON Repair settings
              </button>
            ) : (
              "JSON Repair settings"
            )}.
          </CardDescription>
        </CardHeader>
      </Card>
    </div>
  )
}
