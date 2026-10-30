import { Badge } from "@/components/ui/Badge"
import { cn } from "@/lib/utils"
import type { EventDeliveryMode, PushSourceInfo } from "@/types/tauri-commands"
import { formatRelativeTime } from "./format"
import { noPushReason } from "./push-relay-utils"

const MODE_LABELS: Record<EventDeliveryMode, string> = {
  push: "push",
  poll: "poll",
  webhook: "webhook",
}

function SourceRow({ source }: { source: PushSourceInfo }) {
  const reason = noPushReason(source)
  return (
    <div className="rounded-md border">
      <div className="flex items-center gap-2 p-3">
        <div className="flex-1 min-w-0">
          <div className="flex items-center gap-2 flex-wrap">
            <span className="text-sm font-medium">{source.server_name}</span>
            {source.channels && <Badge variant="info" className="text-[10px] px-1.5 py-0">Claude Channels</Badge>}
            {source.events.some((e) => e.supported) && (
              <Badge variant="info" className="text-[10px] px-1.5 py-0">MCP Events</Badge>
            )}
            {source.resource_subscribe && (
              <Badge variant="outline" className="text-[10px] px-1.5 py-0">Resource updates</Badge>
            )}
            {reason && <Badge variant="warning" className="text-[10px] px-1.5 py-0">{reason}</Badge>}
          </div>
          <p className="text-xs text-muted-foreground mt-0.5">
            {source.events.length > 0 &&
              `${source.events.length} event type${source.events.length === 1 ? "" : "s"} · `}
            {source.protocol_version ? `MCP ${source.protocol_version}` : "Capabilities unknown"}
            {source.observed_at && ` · seen ${formatRelativeTime(source.observed_at)}`}
          </p>
        </div>
      </div>
      {source.events.length > 0 && (
        <div className="border-t divide-y divide-border/50">
          {source.events.map((event) => (
            <div key={event.name} className={cn("flex items-center justify-between gap-2 px-3 py-1.5 pl-6", !event.supported && "opacity-70")}>
              <div className="min-w-0">
                <code className="text-xs">{event.name}</code>
                {event.description && <p className="text-xs text-muted-foreground truncate">{event.description}</p>}
              </div>
              <div className="flex items-center gap-1 shrink-0">
                {event.modes.map((m) => (
                  <Badge
                    key={m}
                    variant="outline"
                    className={cn("text-[10px] px-1.5 py-0", m === "webhook" && "line-through text-muted-foreground")}
                    title={m === "webhook" ? "LocalRouter does not receive upstream webhooks" : undefined}
                  >
                    {MODE_LABELS[m]}
                  </Badge>
                ))}
                {!event.supported && (
                  <Badge variant="warning" className="text-[10px] px-1.5 py-0">Not supported</Badge>
                )}
              </div>
            </div>
          ))}
        </div>
      )}
    </div>
  )
}

interface UpstreamSourcesListProps {
  sources: PushSourceInfo[]
}

/** Every upstream MCP server with its remembered push capabilities, servers with push first. */
export function UpstreamSourcesList({ sources }: UpstreamSourcesListProps) {
  const withPush = sources.filter((s) => s.has_consumable_push)
  // Servers that list event types but none LocalRouter can consume come next:
  // they look like push sources and are not.
  const webhookOnly = sources.filter((s) => !s.has_consumable_push && s.events.length > 0)
  const rest = sources.filter((s) => !s.has_consumable_push && s.events.length === 0)

  return (
    <div className="space-y-2">
      {sources.length === 0 && (
        <p className="text-sm text-muted-foreground">No MCP servers are configured.</p>
      )}
      {sources.length > 0 && withPush.length === 0 && (
        <p className="text-sm text-muted-foreground">
          No upstream server offers push yet. LocalRouter remembers what each server offers the first
          time it connects; servers with MCP Events or Claude Channels are listed first.
        </p>
      )}
      {[...withPush, ...webhookOnly, ...rest].map((s) => (
        <SourceRow key={s.server_id} source={s} />
      ))}
    </div>
  )
}
