import { useState } from "react"
import { ChevronDown, ChevronRight } from "lucide-react"
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
  const [open, setOpen] = useState(source.events.length > 0 && source.events.length <= 6)
  const reason = noPushReason(source)
  return (
    <div className="rounded-md border">
      <button
        type="button"
        className="w-full flex items-center gap-2 p-3 text-left hover:bg-muted/30"
        onClick={() => setOpen((o) => !o)}
        disabled={source.events.length === 0}
      >
        {source.events.length > 0 ? (
          open ? <ChevronDown className="h-4 w-4 text-muted-foreground shrink-0" /> : <ChevronRight className="h-4 w-4 text-muted-foreground shrink-0" />
        ) : (
          <span className="w-4 shrink-0" />
        )}
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
      </button>
      {open && source.events.length > 0 && (
        <div className="border-t divide-y divide-border/50">
          {source.events.map((event) => (
            <div key={event.name} className={cn("flex items-center justify-between gap-2 px-3 py-1.5 pl-9", !event.supported && "opacity-70")}>
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

/** Upstream MCP servers with their remembered push capabilities. */
export function UpstreamSourcesList({ sources }: UpstreamSourcesListProps) {
  const [showOthers, setShowOthers] = useState(false)
  const withPush = sources.filter((s) => s.has_consumable_push)
  const others = sources.filter((s) => !s.has_consumable_push)
  // Servers that list event types but none LocalRouter can consume are worth
  // seeing even when collapsed: they look like push sources and are not.
  const webhookOnly = others.filter((s) => s.events.length > 0)
  const rest = others.filter((s) => s.events.length === 0)

  return (
    <div className="space-y-2">
      {withPush.length === 0 && webhookOnly.length === 0 && (
        <p className="text-sm text-muted-foreground">
          No upstream server offers push yet. LocalRouter remembers what each server offers the first
          time it connects; servers with MCP Events or Claude Channels appear here.
        </p>
      )}
      {withPush.map((s) => <SourceRow key={s.server_id} source={s} />)}
      {webhookOnly.map((s) => <SourceRow key={s.server_id} source={s} />)}
      {rest.length > 0 && (
        <div>
          <button
            type="button"
            className="flex items-center gap-1 text-xs text-muted-foreground hover:text-foreground"
            onClick={() => setShowOthers((v) => !v)}
          >
            {showOthers ? <ChevronDown className="h-3.5 w-3.5" /> : <ChevronRight className="h-3.5 w-3.5" />}
            {rest.length} other server{rest.length === 1 ? "" : "s"} without push
          </button>
          {showOthers && (
            <div className="mt-1.5 space-y-1">
              {rest.map((s) => (
                <div key={s.server_id} className="flex items-center justify-between px-3 py-1.5 rounded-md bg-muted/30">
                  <span className="text-sm">{s.server_name}</span>
                  <span className="text-xs text-muted-foreground">{noPushReason(s)}</span>
                </div>
              ))}
            </div>
          )}
        </div>
      )}
    </div>
  )
}
