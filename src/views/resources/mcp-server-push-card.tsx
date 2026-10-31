import { useCallback, useEffect, useState } from "react"
import { invoke } from "@tauri-apps/api/core"
import { ArrowRight } from "lucide-react"
import { FEATURES } from "@/constants/features"
import { Badge } from "@/components/ui/Badge"
import { Button } from "@/components/ui/Button"
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/Card"
import { listenSafe } from "@/hooks/useTauriListener"
import type { PushRelayConfig, PushSourceInfo } from "@/types/tauri-commands"
import { formatRelativeTime } from "@/views/push-relay/format"
import { PushSourceRow } from "@/views/push-relay/PushSourceRow"
import { serverPushSummary, snapshotStatusLabel } from "@/views/push-relay/push-relay-utils"

interface McpServerPushCardProps {
  serverId: string
  onViewChange?: (view: string, subTab?: string | null) => void
}

const STATUS_VARIANT = {
  current: "success",
  stale: "warning",
  none: "secondary",
} as const

/**
 * The Push section of an MCP server's Info tab: what push the server advertised
 * the last time it connected (from the remembered capability snapshot), and what
 * LocalRouter does with it.
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

  return (
    <Card>
      <CardHeader className="pb-3">
        <div className="flex items-center justify-between gap-2">
          <CardTitle className="text-sm flex items-center gap-2">
            <FEATURES.pushRelay.icon className={`h-4 w-4 ${FEATURES.pushRelay.color}`} />
            Push
          </CardTitle>
          {onViewChange && (
            <Button variant="ghost" size="sm" className="gap-1.5 h-7" onClick={() => onViewChange("push-relay")}>
              Push Relay
              <ArrowRight className="h-3 w-3" />
            </Button>
          )}
        </div>
      </CardHeader>
      <CardContent className="space-y-3">
        {!source || !config ? (
          <p className="text-sm text-muted-foreground">
            {loaded ? "No push information is available for this server." : "Loading…"}
          </p>
        ) : (
          <>
            <div className="flex items-center gap-2 flex-wrap text-xs text-muted-foreground">
              <span>Remembered capabilities</span>
              <Badge variant={STATUS_VARIANT[source.snapshot_status]} className="text-[10px] px-1.5 py-0">
                {snapshotStatusLabel(source)}
              </Badge>
              {source.protocol_version && <span>· MCP {source.protocol_version}</span>}
              {source.observed_at && <span>· seen {formatRelativeTime(source.observed_at)}</span>}
            </div>
            <p className="text-sm">{serverPushSummary(source, config.default_permission)}</p>
            {source.snapshot_status !== "none" && <PushSourceRow source={source} showName={false} />}
          </>
        )}
      </CardContent>
    </Card>
  )
}
