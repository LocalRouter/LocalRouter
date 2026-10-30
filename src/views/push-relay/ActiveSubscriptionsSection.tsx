import { Badge } from "@/components/ui/Badge"
import { Card, CardContent, CardDescription, CardHeader, CardTitle } from "@/components/ui/Card"
import type { ActivePushSubscription } from "@/types/tauri-commands"
import { formatRelativeTime } from "./format"

export interface ActiveSubscriptionsSectionProps {
  /** Live subscriptions; `null` until the first load (section hidden). */
  subscriptions: ActivePushSubscription[] | null
  /** Open a client's detail page. */
  onOpenClient?: (clientId: string) => void
}

const STATUS_VARIANT: Record<ActivePushSubscription["status"], "success" | "warning" | "destructive" | "secondary"> = {
  active: "success",
  reconnecting: "warning",
  awaiting_approval: "warning",
  suspended: "secondary",
  denied: "destructive",
}

function pathLabel(s: ActivePushSubscription): string {
  if (s.kind === "webhook") return "Webhook"
  return s.scope === "session" ? "Session connection" : "Shared connection"
}

/** Upstream subscriptions LocalRouter holds open, and clients' webhook deliveries. */
export function ActiveSubscriptionsSection({ subscriptions, onOpenClient }: ActiveSubscriptionsSectionProps) {
  if (subscriptions === null) return null
  return (
    <Card>
      <CardHeader className="pb-3">
        <CardTitle className="text-base">Active subscriptions</CardTitle>
        <CardDescription>
          Upstream subscriptions LocalRouter holds open on behalf of clients, and webhook deliveries.
          One upstream subscription on a shared connection serves every client consuming the same events;
          stdio servers are subscribed on each session's own process.
        </CardDescription>
      </CardHeader>
      <CardContent>
        {subscriptions.length === 0 ? (
          <p className="text-sm text-muted-foreground">No client is subscribed to pushed events.</p>
        ) : (
          <div className="space-y-1.5">
            {subscriptions.map((s) => (
              <div key={s.subscription_id} className="flex items-center justify-between gap-2 py-1 px-2 rounded-md hover:bg-muted/50">
                <div className="min-w-0">
                  <p className="text-sm truncate">
                    <code className="text-xs">{s.event_name}</code>
                    <span className="text-muted-foreground"> from {s.server_name}</span>
                    {s.client_id && s.client_name && (
                      <>
                        <span className="text-muted-foreground"> for </span>
                        {onOpenClient ? (
                          <button className="font-medium hover:underline" onClick={() => onOpenClient(s.client_id!)}>
                            {s.client_name}
                          </button>
                        ) : (
                          <span className="font-medium">{s.client_name}</span>
                        )}
                      </>
                    )}
                  </p>
                  <p className="text-xs text-muted-foreground">
                    {s.kind === "webhook" ? `${s.event_count} delivered` : `${s.event_count} received`}
                    {s.last_event_at ? ` · last ${formatRelativeTime(s.last_event_at)}` : s.kind === "upstream" ? " · no events yet" : ""}
                    {s.reconnects > 0 && ` · ${s.reconnects} reconnects`}
                    {s.webhook_url && ` · ${s.webhook_url}`}
                    {s.lease_expires_at && ` · lease ends ${formatRelativeTime(s.lease_expires_at)}`}
                  </p>
                  {s.last_error && <p className="text-xs text-red-600 truncate" title={s.last_error}>{s.last_error}</p>}
                </div>
                <div className="flex items-center gap-1.5 shrink-0">
                  <Badge variant="outline" className="text-[10px] px-1.5 py-0">{pathLabel(s)}</Badge>
                  {s.ingress && <Badge variant="outline" className="text-[10px] px-1.5 py-0">{s.ingress}</Badge>}
                  <Badge variant={STATUS_VARIANT[s.status]} className="text-[10px] px-1.5 py-0">
                    {s.status.replace("_", " ")}
                  </Badge>
                </div>
              </div>
            ))}
          </div>
        )}
      </CardContent>
    </Card>
  )
}
