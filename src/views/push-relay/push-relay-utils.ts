import type { PushRelayLimits, PushSourceInfo } from "../../types/tauri-commands"

export type NumericLimitKey = { [K in keyof PushRelayLimits]: PushRelayLimits[K] extends number ? K : never }[keyof PushRelayLimits]

export interface LimitRow {
  key: NumericLimitKey
  label: string
  help: string
  /** Display unit, and how many stored units one display unit is */
  unit: string
  scale: number
}

export interface LimitGroup {
  title: string
  rows: LimitRow[]
}

const KIB = 1024
const MIB = 1024 * 1024

/** Every limit from the plan's "Limits" table, grouped. Must cover PushRelayLimits. */
export const LIMIT_GROUPS: LimitGroup[] = [
  {
    title: "Journal",
    rows: [
      { key: "journal_max_events", label: "Events kept per client", help: "Occurrences held for polling and replay", unit: "events", scale: 1 },
      { key: "journal_max_bytes", label: "Bytes kept per client", help: "Same, by size; the first limit reached wins", unit: "MiB", scale: MIB },
      { key: "journal_retention_secs", label: "Retention", help: "Older occurrences are dropped; later cursors see truncated", unit: "hours", scale: 3600 },
      { key: "max_event_bytes", label: "Largest event", help: "One occurrence's data; larger events are dropped with a Monitor error", unit: "KiB", scale: KIB },
    ],
  },
  {
    title: "Approvals (Ask)",
    rows: [
      { key: "ask_timeout_secs", label: "Approval timeout", help: "A held event is denied (fail closed) after this", unit: "minutes", scale: 60 },
      { key: "ask_hold_cap", label: "Held events per client", help: "Further Ask events are denied with a Monitor record", unit: "events", scale: 1 },
    ],
  },
  {
    title: "Upstream",
    rows: [
      { key: "first_discovery_wait_ms", label: "First discovery wait", help: "How long initialize waits for a server LocalRouter has not seen yet", unit: "seconds", scale: 1000 },
      { key: "upstream_poll_min_interval_ms", label: "Poll interval, minimum", help: "Clamp on the upstream's nextPollMs", unit: "seconds", scale: 1000 },
      { key: "upstream_poll_max_interval_ms", label: "Poll interval, maximum", help: "Clamp on the upstream's nextPollMs", unit: "seconds", scale: 1000 },
      { key: "upstream_reconnect_backoff_min_ms", label: "Reconnect backoff, first", help: "Delay before reconnecting a dropped events/stream (jittered)", unit: "seconds", scale: 1000 },
      { key: "upstream_reconnect_backoff_max_ms", label: "Reconnect backoff, longest", help: "Largest reconnect delay", unit: "seconds", scale: 1000 },
      { key: "upstream_heartbeat_timeout_secs", label: "Heartbeat timeout", help: "An upstream stream with no heartbeat for this long is treated as dead", unit: "seconds", scale: 1 },
    ],
  },
  {
    title: "Clients",
    rows: [
      { key: "downstream_heartbeat_secs", label: "Heartbeat interval", help: "notifications/events/heartbeat sent to clients", unit: "seconds", scale: 1 },
      { key: "max_streams_per_session", label: "Streams per session", help: "Concurrent events/stream requests per client session", unit: "streams", scale: 1 },
      { key: "poll_max_events", label: "Events per poll, maximum", help: "Ceiling on one events/poll or EventPoll page", unit: "events", scale: 1 },
      { key: "poll_default_events", label: "Events per poll, default", help: "EventPoll page size when the caller gives no limit", unit: "events", scale: 1 },
      { key: "poll_max_wait_secs", label: "Long-poll wait, maximum", help: "Ceiling on EventPoll's wait_seconds; keep under client tool timeouts", unit: "seconds", scale: 1 },
      { key: "watch_max_lifetime_secs", label: "Watcher lifetime", help: "One /events/watch connection; the agent re-arms after", unit: "hours", scale: 3600 },
    ],
  },
  {
    title: "Claude Channels",
    rows: [
      { key: "channel_max_content_bytes", label: "Message size", help: "Longer content is truncated with a pointer to EventPoll", unit: "KiB", scale: KIB },
      { key: "channel_rate_per_minute", label: "Messages per minute", help: "Beyond this, events are coalesced into one summary message", unit: "per min", scale: 1 },
    ],
  },
  {
    title: "Webhooks",
    rows: [
      { key: "webhook_timeout_secs", label: "Delivery timeout", help: "Per delivery attempt", unit: "seconds", scale: 1 },
      { key: "webhook_max_subscriptions", label: "Subscriptions per client", help: "Active webhook subscriptions", unit: "subs", scale: 1 },
      { key: "webhook_lease_default_secs", label: "Lease, default", help: "Subscription TTL when the client asks for none", unit: "minutes", scale: 60 },
      { key: "webhook_lease_min_secs", label: "Lease, minimum", help: "Shortest TTL accepted", unit: "seconds", scale: 1 },
      { key: "webhook_lease_max_secs", label: "Lease, maximum", help: "Longest TTL accepted", unit: "hours", scale: 3600 },
    ],
  },
]

/** Why a server contributes no push, or null when it does. */
export function noPushReason(source: PushSourceInfo): string | null {
  if (source.has_consumable_push) return null
  if (!source.enabled) return "Disabled"
  if (source.snapshot_status === "none") return "Not observed yet — start a session that uses it"
  if (source.snapshot_status === "stale") return "Configuration changed since last seen — reconnect to refresh"
  if (source.events.length > 0 && source.events.every((e) => !e.supported)) {
    return "Webhook only — not supported"
  }
  return "No push offered"
}
