import type { PushPermission, PushSourceInfo } from "@/types/tauri-commands"
import { PushSourceRow } from "./PushSourceRow"
import { usePushStatuses } from "./use-push-status"

interface UpstreamSourcesListProps {
  sources: PushSourceInfo[]
  defaultPermission: PushPermission
}

/** Every upstream MCP server with what it offers for push and its live status, servers with push first. */
export function UpstreamSourcesList({ sources, defaultPermission }: UpstreamSourcesListProps) {
  const withPush = sources.filter((s) => s.has_consumable_push)
  // Servers that list event types but none LocalRouter can consume come next:
  // they look like push sources and are not.
  const webhookOnly = sources.filter((s) => !s.has_consumable_push && s.events.length > 0)
  const rest = sources.filter((s) => !s.has_consumable_push && s.events.length === 0)
  const statuses = usePushStatuses(withPush.map((s) => s.server_id))

  return (
    <div className="space-y-2">
      {sources.length === 0 && (
        <p className="text-sm text-muted-foreground">No MCP servers are configured.</p>
      )}
      {sources.length > 0 && withPush.length === 0 && (
        <p className="text-sm text-muted-foreground">
          No upstream server offers push yet. LocalRouter learns what each server offers the first
          time it connects; servers with MCP Events or Claude Channels are listed first.
        </p>
      )}
      {[...withPush, ...webhookOnly, ...rest].map((s) => (
        <PushSourceRow
          key={s.server_id}
          source={s}
          status={statuses[s.server_id] ?? null}
          defaultPermission={defaultPermission}
        />
      ))}
    </div>
  )
}
