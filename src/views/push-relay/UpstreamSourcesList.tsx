import type { PushSourceInfo } from "@/types/tauri-commands"
import { PushSourceRow } from "./PushSourceRow"

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
        <PushSourceRow key={s.server_id} source={s} />
      ))}
    </div>
  )
}
