import { useCallback, useEffect, useMemo, useState } from "react"
import { invoke } from "@tauri-apps/api/core"
import { FEATURES } from "@/constants/features"
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/Card"
import { listenSafe } from "@/hooks/useTauriListener"
import type { PushRelayConfig, PushSourceInfo } from "@/types/tauri-commands"
import { PushSourceDetails } from "@/views/push-relay/PushSourceRow"
import { capabilitiesFooter } from "@/views/push-relay/push-relay-utils"
import { usePushStatuses } from "@/views/push-relay/use-push-status"

interface McpServerPushCardProps {
  serverId: string
  onViewChange?: (view: string, subTab?: string | null) => void
}

/**
 * The Push section of an MCP server's Info tab: what the server offers for push
 * (from the capabilities it declared when it last connected) and whether
 * LocalRouter is listening to it right now (from the running relay).
 */
export function McpServerPushCard({ serverId, onViewChange }: McpServerPushCardProps) {
  const [source, setSource] = useState<PushSourceInfo | null>(null)
  const [config, setConfig] = useState<PushRelayConfig | null>(null)
  const [loaded, setLoaded] = useState(false)

  const load = useCallback(async () => {
    try {
      const [sources, relay] = await Promise.all([
        invoke<PushSourceInfo[]>("list_push_sources"),
        invoke<PushRelayConfig>("get_push_relay_config"),
      ])
      setSource(sources.find((s) => s.server_id === serverId) ?? null)
      setConfig(relay)
    } catch (err) {
      console.error("Failed to load push capabilities:", err)
    } finally {
      setLoaded(true)
    }
  }, [serverId])

  useEffect(() => {
    load()
    const listeners = [listenSafe("config-changed", load), listenSafe("clients-changed", load)]
    return () => listeners.forEach((l) => l.cleanup())
  }, [load])

  const ids = useMemo(() => (source?.has_consumable_push ? [serverId] : []), [source?.has_consumable_push, serverId])
  const status = usePushStatuses(ids)[serverId] ?? null
  const footer = source ? capabilitiesFooter(source) : null

  return (
    <Card>
      <CardHeader className="pb-2">
        <CardTitle className="text-sm flex items-center gap-2">
          <FEATURES.pushRelay.icon className={`h-4 w-4 ${FEATURES.pushRelay.color}`} />
          Push
        </CardTitle>
      </CardHeader>
      <CardContent className="space-y-2">
        {!source || !config ? (
          <p className="text-sm text-muted-foreground">
            {loaded ? "Not known yet — appears after the server first connects." : "Loading…"}
          </p>
        ) : (
          <PushSourceDetails
            source={source}
            status={status}
            defaultPermission={config.default_permission}
            showFooter={false}
          />
        )}
        {(footer || onViewChange) && (
          <div className="flex items-center justify-between gap-2 text-xs text-muted-foreground pt-1">
            <span>{footer}</span>
            {onViewChange && (
              <button
                type="button"
                className="shrink-0 underline-offset-2 hover:underline hover:text-foreground"
                onClick={() => onViewChange("push-relay")}
              >
                Push Relay settings
              </button>
            )}
          </div>
        )}
      </CardContent>
    </Card>
  )
}
