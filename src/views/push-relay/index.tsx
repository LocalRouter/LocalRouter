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
import { Switch } from "@/components/ui/Toggle"
import { Tabs, TabsContent, TabsList, TabsTrigger } from "@/components/ui/tabs"
import { InfoTooltip } from "@/components/ui/info-tooltip"
import { listenSafe } from "@/hooks/useTauriListener"
import { PushPermissionButton, pushPermissionLabel } from "@/components/permissions/PushPermissionButton"
import type {
  ActivePushSubscription,
  ClientFeatureStatus,
  GetFeatureClientsStatusParams,
  ListPendingPushApprovalsParams,
  PendingPushApproval,
  PushApprovalAction,
  PushPermission,
  PushRelayConfig,
  PushRelayLimits,
  PushSourceInfo,
  SubmitPushApprovalParams,
  UpdatePushRelayConfigParams,
} from "@/types/tauri-commands"
import { LimitsEditor } from "./LimitsEditor"
import { UpstreamSourcesList } from "./UpstreamSourcesList"
import { PendingApprovalsSection } from "./PendingApprovalsSection"
import { ActiveSubscriptionsSection } from "./ActiveSubscriptionsSection"

interface PushRelayViewProps {
  activeSubTab?: string | null
  onTabChange?: (view: string, subTab?: string | null) => void
}

const PERMISSION_BADGE: Record<PushPermission, string> = {
  allow: "border-emerald-500/50 text-emerald-600",
  ask: "border-amber-500/50 text-amber-600",
  block: "border-red-500/50 text-red-600",
}

export function PushRelayView({ activeSubTab, onTabChange }: PushRelayViewProps) {
  const [config, setConfig] = useState<PushRelayConfig | null>(null)
  const [sources, setSources] = useState<PushSourceInfo[]>([])
  const [clientStatuses, setClientStatuses] = useState<ClientFeatureStatus[]>([])
  const [saving, setSaving] = useState(false)
  const [refreshing, setRefreshing] = useState(false)
  // Null until loaded; a failed load (relay not running) keeps the section hidden.
  const [pendingApprovals, setPendingApprovals] = useState<PendingPushApproval[] | null>(null)
  const [subscriptions, setSubscriptions] = useState<ActivePushSubscription[] | null>(null)

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

  const loadApprovals = useCallback(async () => {
    try {
      setPendingApprovals(
        await invoke<PendingPushApproval[]>("list_pending_push_approvals", {} satisfies ListPendingPushApprovalsParams),
      )
    } catch (err) {
      console.error("Failed to load pending push approvals:", err)
    }
  }, [])

  const loadSubscriptions = useCallback(async () => {
    try {
      setSubscriptions(await invoke<ActivePushSubscription[]>("list_active_push_subscriptions"))
    } catch (err) {
      console.error("Failed to load push subscriptions:", err)
    }
  }, [])

  useEffect(() => {
    loadConfig()
    loadSources()
    loadClientStatuses()
    loadApprovals()
    loadSubscriptions()
  }, [loadConfig, loadSources, loadClientStatuses, loadApprovals, loadSubscriptions])

  // Subscriptions change with client demand, which raises no event: refresh
  // them while the page is open.
  useEffect(() => {
    const timer = window.setInterval(loadSubscriptions, 5000)
    return () => window.clearInterval(timer)
  }, [loadSubscriptions])

  useEffect(() => {
    const listeners = [
      listenSafe("clients-changed", loadClientStatuses),
      listenSafe("push-approvals-changed", () => {
        loadApprovals()
        loadSubscriptions()
      }),
      listenSafe("config-changed", () => {
        loadConfig()
        loadSources()
        loadClientStatuses()
      }),
    ]
    return () => listeners.forEach((l) => l.cleanup())
  }, [loadConfig, loadSources, loadClientStatuses, loadApprovals, loadSubscriptions])

  const decide = async (requestId: string, action: PushApprovalAction, editedData?: unknown) => {
    try {
      await invoke<number>("submit_push_approval", {
        requestId,
        action,
        editedData: editedData ?? null,
      } satisfies SubmitPushApprovalParams)
    } catch (err) {
      toast.error(`Failed to answer the held event: ${err}`)
    } finally {
      loadApprovals()
    }
  }

  const update = async (params: UpdatePushRelayConfigParams) => {
    setSaving(true)
    try {
      const next = await invoke<PushRelayConfig>("update_push_relay_config", { ...params } satisfies UpdatePushRelayConfigParams)
      setConfig(next)
      loadClientStatuses()
    } catch (err) {
      toast.error(`Failed to update Push Relay: ${err}`)
      throw err
    } finally {
      setSaving(false)
    }
  }

  const saveLimits = async (limits: PushRelayLimits) => {
    await update({ limits }).then(() => toast.success("Limits saved")).catch(() => {})
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
          in a form it can consume, with Allow / Ask / Block control over each event.
        </p>
      </div>

      <Tabs value={tab} onValueChange={handleTabChange} className="flex flex-col flex-1 min-h-0">
        <TabsList className="flex-shrink-0 w-fit">
          <TabsTrigger value="info"><TAB_ICONS.info className={TAB_ICON_CLASS} />Info</TabsTrigger>
          <TabsTrigger value="sources">
            <FEATURES.pushRelay.icon className={TAB_ICON_CLASS} />Upstream sources
            {pushSourceCount > 0 && <Badge variant="secondary" className="ml-1.5 text-[10px] px-1.5 py-0">{pushSourceCount}</Badge>}
          </TabsTrigger>
          <TabsTrigger value="settings"><TAB_ICONS.settings className={TAB_ICON_CLASS} />Settings</TabsTrigger>
        </TabsList>

        <TabsContent value="info" className="flex-1 min-h-0 mt-4">
          <div className="space-y-4 max-w-2xl overflow-y-auto">
            {config && (
              <Card>
                <CardHeader className="pb-3">
                  <div className="flex items-center justify-between gap-4">
                    <div>
                      <CardTitle className="text-base">Push Relay</CardTitle>
                      <CardDescription>
                        Every pushed event passes through LocalRouter, where it is recorded in Monitor and can
                        be held for approval. Push is offered to a client only when one of its permitted MCP
                        servers actually offers it, and never widens the client's MCP access.
                      </CardDescription>
                    </div>
                    <InfoTooltip content="When off, no push is advertised or relayed to any client.">
                      <Switch
                        checked={config.enabled}
                        onCheckedChange={(enabled) => update({ enabled }).catch(() => {})}
                        disabled={saving}
                      />
                    </InfoTooltip>
                  </div>
                </CardHeader>
                <CardContent className="pt-0 space-y-3">
                  <div className="flex items-center justify-between gap-4 border-t pt-3">
                    <div>
                      <span className="text-sm font-medium">Default permission</span>
                      <p className="text-xs text-muted-foreground mt-0.5">
                        Applies to every client, server and event type without its own setting. Ask holds each
                        event until you approve it.
                      </p>
                    </div>
                    <PushPermissionButton
                      value={config.default_permission}
                      onChange={(p) => p !== "inherit" && update({ defaultPermission: p }).catch(() => {})}
                      showInherit={false}
                      disabled={saving || !config.enabled}
                      size="sm"
                    />
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

            <PendingApprovalsSection approvals={pendingApprovals} onDecide={decide} onOpenClient={openClient} />
            <ActiveSubscriptionsSection subscriptions={subscriptions} onOpenClient={openClient} />

            <Card>
              <CardHeader className="pb-3">
                <CardTitle className="text-base">How clients receive events</CardTitle>
              </CardHeader>
              <CardContent className="space-y-2 text-sm text-muted-foreground">
                <p>
                  <span className="text-foreground font-medium">MCP Events clients</span> subscribe with{" "}
                  <code className="text-xs bg-muted px-1 rounded">events/stream</code>,{" "}
                  <code className="text-xs bg-muted px-1 rounded">events/poll</code> or a webhook LocalRouter
                  signs and sends.
                </p>
                <p>
                  <span className="text-foreground font-medium">Claude Code</span> receives events as Claude
                  Channels messages through the STDIO bridge.
                </p>
                <p>
                  <span className="text-foreground font-medium">Every other client</span> gets the{" "}
                  <code className="text-xs bg-muted px-1 rounded">EventList</code>,{" "}
                  <code className="text-xs bg-muted px-1 rounded">EventPoll</code> and{" "}
                  <code className="text-xs bg-muted px-1 rounded">EventWait</code> tools.
                </p>
                <p>
                  Upstream webhooks are never received: a server that offers only webhooks is shown as not
                  supported.
                </p>
              </CardContent>
            </Card>
          </div>
        </TabsContent>

        <TabsContent value="sources" className="flex-1 min-h-0 mt-4">
          <div className="space-y-4 max-w-3xl overflow-y-auto">
            <Card>
              <CardHeader className="pb-3">
                <div className="flex items-center justify-between gap-4">
                  <div>
                    <CardTitle className="text-base">Upstream sources</CardTitle>
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

        <TabsContent value="settings" className="flex-1 min-h-0 mt-4">
          <div className="space-y-4 max-w-2xl overflow-y-auto">
            {config && (
              <>
                <Card>
                  <CardHeader className="pb-3">
                    <CardTitle className="text-base">Webhook delivery</CardTitle>
                    <CardDescription>
                      Clients may subscribe with a callback URL; LocalRouter signs each event (Standard
                      Webhooks) and POSTs it after permissions and approval. The first delivery to a new URL
                      always asks.
                    </CardDescription>
                  </CardHeader>
                  <CardContent className="space-y-4">
                    <div className="flex items-center justify-between gap-4">
                      <div>
                        <span className="text-sm font-medium">Allow webhook subscriptions</span>
                        <p className="text-xs text-muted-foreground mt-0.5">Accept <code className="text-xs bg-muted px-0.5 rounded">events/subscribe</code> from clients</p>
                      </div>
                      <Switch
                        checked={config.webhook_egress_enabled}
                        onCheckedChange={(v) => update({ webhookEgressEnabled: v }).catch(() => {})}
                        disabled={saving || !config.enabled}
                      />
                    </div>
                    <div className="flex items-center justify-between gap-4">
                      <div>
                        <span className="text-sm font-medium">Allow http:// to loopback</span>
                        <p className="text-xs text-muted-foreground mt-0.5">
                          Callback URLs must be HTTPS, except <code className="text-xs bg-muted px-0.5 rounded">http://127.0.0.1</code> /{" "}
                          <code className="text-xs bg-muted px-0.5 rounded">localhost</code> for local clients
                        </p>
                      </div>
                      <Switch
                        checked={config.allow_loopback_http}
                        onCheckedChange={(v) => update({ allowLoopbackHttp: v }).catch(() => {})}
                        disabled={saving || !config.enabled || !config.webhook_egress_enabled}
                      />
                    </div>
                  </CardContent>
                </Card>

                <Card>
                  <CardHeader className="pb-3">
                    <CardTitle className="text-base">Advanced: Limits</CardTitle>
                    <CardDescription>
                      Bounds on memory, waiting and retries. The defaults are generous: they exist to stop
                      runaway loops, not to ration normal use.
                    </CardDescription>
                  </CardHeader>
                  <CardContent>
                    <LimitsEditor
                      limits={config.limits}
                      defaults={config.default_limits}
                      onSave={saveLimits}
                      disabled={saving}
                    />
                  </CardContent>
                </Card>
              </>
            )}
          </div>
        </TabsContent>
      </Tabs>
    </div>
  )
}
