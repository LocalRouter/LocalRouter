import { Badge } from "@/components/ui/Badge"
import { Card, CardContent, CardDescription, CardHeader, CardTitle } from "@/components/ui/Card"
import type { ActivePushSubscription, PushEgressMode } from "@/types/tauri-commands"
import { formatRelativeTime } from "./format"

export interface ActiveSubscriptionsSectionProps {
  /** Active delivery paths; `null` while the relay core provides no source (section hidden). */
  subscriptions: ActivePushSubscription[] | null
  /** Open a client's detail page. */
  onOpenClient?: (clientId: string) => void
}

export const EGRESS_LABELS: Record<PushEgressMode, string> = {
  events_stream: "events/stream",
  events_poll: "events/poll",
  channel: "Claude Channels",
  webhook: "Webhook",
  fallback_tools: "EventPoll tools",
  watch: "EventWait watcher",
}

/** Who is receiving what, and how. */
export function ActiveSubscriptionsSection({ subscriptions, onOpenClient }: ActiveSubscriptionsSectionProps) {
  if (subscriptions === null) return null
  return (
    <Card>
      <CardHeader className="pb-3">
        <CardTitle className="text-base">Active subscriptions</CardTitle>
        <CardDescription>
          Delivery paths currently open to clients. One upstream subscription is shared by every
          client consuming the same events.
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
                    {onOpenClient ? (
                      <button className="font-medium hover:underline" onClick={() => onOpenClient(s.client_id)}>
                        {s.client_name}
                      </button>
                    ) : (
                      <span className="font-medium">{s.client_name}</span>
                    )}
                    <span className="text-muted-foreground"> ← {s.server_name}</span>
                    {s.event_names.length > 0 && (
                      <span className="text-muted-foreground"> · {s.event_names.join(", ")}</span>
                    )}
                  </p>
                  <p className="text-xs text-muted-foreground">
                    {s.delivered_count} delivered
                    {s.held_count > 0 && ` · ${s.held_count} held`}
                    {s.last_event_at ? ` · last ${formatRelativeTime(s.last_event_at)}` : " · no events yet"}
                    {s.webhook_url && ` · ${s.webhook_url}`}
                    {s.lease_expires_at && ` · lease ends ${formatRelativeTime(s.lease_expires_at)}`}
                  </p>
                </div>
                <div className="flex items-center gap-1.5 shrink-0">
                  <Badge variant="outline" className="text-[10px] px-1.5 py-0">{EGRESS_LABELS[s.egress]}</Badge>
                  <Badge
                    variant={s.status === "active" ? "success" : "warning"}
                    className="text-[10px] px-1.5 py-0"
                  >
                    {s.status}
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
