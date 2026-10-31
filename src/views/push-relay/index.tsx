import { useCallback, useEffect, useState } from "react"
import { invoke } from "@tauri-apps/api/core"
import { toast } from "sonner"
import { ExternalLink, RefreshCw } from "lucide-react"
import { cn } from "@/lib/utils"
import { FEATURES } from "@/constants/features"
import { TAB_ICONS, TAB_ICON_CLASS } from "@/constants/tab-icons"
import { Badge } from "@/components/ui/Badge"
import { Button } from "@/components/ui/Button"
import { Card, CardContent, CardDescription, CardHeader, CardTitle } from "@/components/ui/Card"
import { Tabs, TabsContent, TabsList, TabsTrigger } from "@/components/ui/tabs"
import { listenSafe } from "@/hooks/useTauriListener"
import { SamplePopupButton } from "@/components/shared/SamplePopupButton"
import { pushPermissionLabel } from "@/components/permissions/PushPermissionButton"
import { PermissionStateButton } from "@/components/permissions/PermissionStateButton"
import type {
  ClientFeatureStatus,
  GetFeatureClientsStatusParams,
  PushPermission,
  PushRelayConfig,
  PushSourceInfo,
  UpdatePushRelayConfigParams,
} from "@/types/tauri-commands"
import { RelaySettingsCard } from "./RelaySettingsCard"
import { UpstreamSourcesList } from "./UpstreamSourcesList"

interface PushRelayViewProps {
  activeSubTab?: string | null
  onTabChange?: (view: string, subTab?: string | null) => void
}

const PERMISSION_BADGE: Record<PushPermission, string> = {
  allow: "border-emerald-500/50 text-emerald-600",
  ask: "border-amber-500/50 text-amber-600",
  off: "border-zinc-500/50 text-zinc-600",
}

export function PushRelayView({ activeSubTab, onTabChange }: PushRelayViewProps) {
  const [config, setConfig] = useState<PushRelayConfig | null>(null)
  const [sources, setSources] = useState<PushSourceInfo[]>([])
  const [clientStatuses, setClientStatuses] = useState<ClientFeatureStatus[]>([])
  const [saving, setSaving] = useState(false)
  const [refreshing, setRefreshing] = useState(false)

  const tab = activeSubTab || "info"
  const handleTabChange = (newTab: string) => onTabChange?.("push-relay", newTab)
  const openClient = onTabChange ? (clientId: string) => onTabChange("clients", `${clientId}|optimize`) : undefined

  const loadConfig = useCallback(async () => {
    try {
      setConfig(await invoke<PushRelayConfig>("get_push_relay_config"))
    } catch (err) {
      console.error("Failed to load Push Relay config:", err)
    }
  }, [])

  const loadSources = useCallback(async () => {
    try {
      setSources(await invoke<PushSourceInfo[]>("list_push_sources"))
    } catch (err) {
      console.error("Failed to load push sources:", err)
    }
  }, [])

  const loadClientStatuses = useCallback(async () => {
    try {
      setClientStatuses(
        await invoke<ClientFeatureStatus[]>("get_feature_clients_status", {
          feature: "push_relay",
        } satisfies GetFeatureClientsStatusParams),
      )
    } catch (err) {
      console.error("Failed to load client statuses:", err)
    }
  }, [])

  useEffect(() => {
    loadConfig()
    loadSources()
    loadClientStatuses()
  }, [loadConfig, loadSources, loadClientStatuses])

  useEffect(() => {
    const listeners = [
      listenSafe("clients-changed", loadClientStatuses),
      listenSafe("config-changed", () => {
        loadConfig()
        loadSources()
        loadClientStatuses()
      }),
    ]
    return () => listeners.forEach((l) => l.cleanup())
  }, [loadConfig, loadSources, loadClientStatuses])

  const update = async (params: UpdatePushRelayConfigParams) => {
    setSaving(true)
    try {
      const next = await invoke<PushRelayConfig>("update_push_relay_config", { ...params } satisfies UpdatePushRelayConfigParams)
      setConfig(next)
      loadClientStatuses()
    } catch (err) {
      toast.error(`Failed to update Push Relay: ${err}`)
    } finally {
      setSaving(false)
    }
  }

  const refreshSources = async () => {
    setRefreshing(true)
    await loadSources()
    setRefreshing(false)
  }

  const pushSourceCount = sources.filter((s) => s.has_consumable_push).length

  return (
    <div className="flex flex-col h-full min-h-0 gap-4 max-w-5xl">
      <div className="flex-shrink-0">
        <h1 className="text-2xl font-bold tracking-tight flex items-center gap-2 mb-1">
          <FEATURES.pushRelay.icon className={`h-6 w-6 ${FEATURES.pushRelay.color}`} />
          Push Relay
        </h1>
        <p className="text-sm text-muted-foreground">
          Relay events that upstream MCP servers push — MCP Events and Claude Channels — to every client
          in a form it can consume, with Allow / Ask / Off control over each event.
        </p>
      </div>

      <Tabs value={tab} onValueChange={handleTabChange} className="flex flex-col flex-1 min-h-0">
        <TabsList className="flex-shrink-0 w-fit">
          <TabsTrigger value="info"><TAB_ICONS.info className={TAB_ICON_CLASS} />Info</TabsTrigger>
        </TabsList>

        <TabsContent value="info" className="flex-1 min-h-0 mt-4">
          <div className="space-y-4 max-w-3xl overflow-y-auto">
            {config && (
              <Card>
                <CardHeader className="pb-3">
                  <CardTitle className="text-base">Push Relay</CardTitle>
                  <CardDescription>
                    Every pushed event passes through LocalRouter, where it is recorded in Monitor and can
                    be held for approval. Push is offered to a client only when one of its permitted MCP
                    servers actually offers it, and never widens the client's MCP access.
                  </CardDescription>
                </CardHeader>
                <CardContent className="pt-0 space-y-3">
                  <div className="flex items-center justify-between gap-4 border-t pt-3">
                    <div>
                      <span className="text-sm font-medium">Default permission</span>
                      <p className="text-xs text-muted-foreground mt-0.5">
                        Applies to every client, server and event type without its own setting. Ask holds each
                        event until you approve, edit or deny it. Off with no client setting that turns
                        something on means push is off: nothing is advertised, subscribed or waited for.
                      </p>
                    </div>
                    <div className="flex items-center gap-2 shrink-0">
                      <SamplePopupButton popupType="push_event" />
                      <PermissionStateButton
                        value={config.default_permission}
                        onChange={(p) => update({ defaultPermission: p })}
                        disabled={saving}
                        size="sm"
                      />
                    </div>
                  </div>
                  {clientStatuses.length > 0 && (
                    <div className="border-t pt-3 space-y-1.5">
                      {clientStatuses.map((s) => (
                        <div key={s.client_id} className="flex items-center justify-between py-1 px-2 rounded-md hover:bg-muted/50 group">
                          <div className="flex items-center gap-2 min-w-0">
                            {openClient ? (
                              <button onClick={() => openClient(s.client_id)} className="text-sm font-medium truncate hover:underline text-left">
                                {s.client_name}
                              </button>
                            ) : (
                              <span className="text-sm font-medium truncate">{s.client_name}</span>
                            )}
                            {s.source === "override" && (
                              <Badge variant="outline" className="text-[10px] px-1 py-0 shrink-0">Override</Badge>
                            )}
                          </div>
                          <div className="flex items-center gap-2 shrink-0">
                            {s.effective_value && (
                              <Badge
                                variant="outline"
                                className={cn("text-[10px] px-1.5 py-0", PERMISSION_BADGE[s.effective_value as PushPermission])}
                              >
                                {pushPermissionLabel(s.effective_value as PushPermission)}
                                {s.effective_value === "off" && s.active && " · some events on"}
                              </Badge>
                            )}
                            {openClient && (
                              <button
                                onClick={() => openClient(s.client_id)}
                                className="opacity-0 group-hover:opacity-100 text-muted-foreground hover:text-foreground transition-opacity"
                                title="Go to client settings"
                              >
                                <ExternalLink className="h-3.5 w-3.5" />
                              </button>
                            )}
                          </div>
                        </div>
                      ))}
                    </div>
                  )}
                </CardContent>
              </Card>
            )}

            {config && (
              <RelaySettingsCard config={config} disabled={saving} onUpdate={update} />
            )}

            <Card>
              <CardHeader className="pb-3">
                <div className="flex items-center justify-between gap-4">
                  <div>
                    <CardTitle className="text-base flex items-center gap-2">
                      Upstream sources
                      {pushSourceCount > 0 && (
                        <Badge variant="secondary" className="text-[10px] px-1.5 py-0">{pushSourceCount}</Badge>
                      )}
                    </CardTitle>
                    <CardDescription>
                      What each MCP server offered the last time it connected. Event types offering only
                      webhooks are not supported.
                    </CardDescription>
                  </div>
                  <Button variant="ghost" size="icon" onClick={refreshSources} disabled={refreshing} title="Refresh">
                    <RefreshCw className={cn("h-4 w-4", refreshing && "animate-spin")} />
                  </Button>
                </div>
              </CardHeader>
              <CardContent>
                <UpstreamSourcesList sources={sources} />
              </CardContent>
            </Card>
          </div>
        </TabsContent>
      </Tabs>
    </div>
  )
}
