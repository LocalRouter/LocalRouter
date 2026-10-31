import type {
  EventDeliveryMode,
  McpServerPushStatus,
  PushEventIngressStatus,
  PushEventTypeInfo,
  PushPermission,
  PushSourceInfo,
} from "../../types/tauri-commands"
import { formatRelativeTime } from "./format"

/** Why a server contributes no push, or null when it does. */
export function noPushReason(source: PushSourceInfo): string | null {
  if (source.has_consumable_push) return null
  if (!source.enabled) return "Disabled"
  if (source.snapshot_status === "none") return "Not known yet — appears after the server first connects"
  if (source.snapshot_status === "stale") return "Configuration changed — appears again after it reconnects"
  if (source.events.length > 0 && source.events.every((e) => !e.supported)) {
    return "Webhook only — not supported"
  }
  return "Doesn't offer push"
}

/**
 * One line on what a server offers when there is no event list to show, or null
 * when it lists event types or channels (those are shown instead).
 */
export function offersLine(source: PushSourceInfo): string | null {
  if (source.snapshot_status === "none") return "Not known yet — appears after the server first connects."
  if (source.snapshot_status === "stale") {
    return "The server's configuration changed — what it offers appears again after it reconnects."
  }
  if (source.events.length > 0 || source.channels || source.resource_subscribe) return null
  return "This server doesn't offer push."
}

/**
 * How one event type can be received: `push, poll`, never `webhook` (LocalRouter
 * does not receive upstream webhooks), or "webhook only — not supported".
 */
export function eventModesLabel(event: Pick<PushEventTypeInfo, "modes">): string {
  const usable = event.modes.filter((m): m is Exclude<EventDeliveryMode, "webhook"> => m !== "webhook")
  if (usable.length === 0) return "webhook only — not supported"
  return usable.join(", ")
}

/** Other push the server offers besides its event types, as one short line. */
export function extrasLine(source: PushSourceInfo): string | null {
  const extras = [
    source.channels && "Claude Code channel messages",
    source.resource_subscribe && "resource update notifications",
  ].filter(Boolean) as string[]
  if (extras.length === 0) return null
  const text = extras.join(" and ")
  return text.charAt(0).toUpperCase() + text.slice(1)
}

/** Muted footer: when capabilities were last checked and the protocol version. */
export function capabilitiesFooter(source: PushSourceInfo, now: number = Date.now()): string | null {
  const parts: string[] = []
  if (source.observed_at) parts.push(`Capabilities checked ${formatRelativeTime(source.observed_at, now)}`)
  if (source.protocol_version) parts.push(`MCP ${source.protocol_version}`)
  return parts.length > 0 ? parts.join(" · ") : null
}

/** listening = green dot; idle = gray; warning = amber; error = red */
export type PushLiveTone = "listening" | "idle" | "warning" | "error"

export interface PushLiveLine {
  tone: PushLiveTone
  text: string
}

const INGRESS_LABELS: Record<NonNullable<PushEventIngressStatus["ingress_mode"]>, string> = {
  push: "push stream",
  poll: "polling",
  channel: "Claude Code channel",
}

function plural(n: number, word: string): string {
  return `${n} ${word}${n === 1 ? "" : "s"}`
}

function receivedPart(count: number, last: string | null, now: number): string {
  if (count === 0) return "no events yet"
  const lastText = last ? ` · last ${formatRelativeTime(last, now)}` : ""
  return `${plural(count, "event")} received${lastText}`
}

/**
 * The live-status line for a server's push, from the running relay. Green only
 * when LocalRouter is actually connected to the server's push (a stream open,
 * polling, or a channel session live). Null when there is nothing to listen to
 * (what the server offers already says why) or while the status loads.
 */
export function pushLiveLine(
  source: PushSourceInfo,
  status: McpServerPushStatus | null,
  defaultPermission: PushPermission,
  now: number = Date.now(),
): PushLiveLine | null {
  if (!source.enabled) {
    return { tone: "idle", text: "Server is disabled — nothing is received from it" }
  }
  if (!source.has_consumable_push || !status) return null
  if (!status.relay_running) return { tone: "error", text: "Push Relay is not running" }

  const listening = status.event_types.filter((t) => t.state === "listening")
  const failing = status.event_types.filter((t) => t.state === "reconnecting")
  const connecting = status.event_types.filter((t) => t.state === "connecting")
  const received = receivedPart(status.events_received, status.last_event_at, now)

  if (listening.length > 0) {
    const modes = [...new Set(listening.map((t) => t.ingress_mode).filter((m) => m !== null))]
      .map((m) => INGRESS_LABELS[m])
    const via = modes.length > 0 ? ` via ${modes.join(" and ")}` : ""
    const clients = status.consuming_clients > 0 ? ` · ${plural(status.consuming_clients, "client")}` : ""
    return { tone: "listening", text: `Listening${via} · ${received}${clients}` }
  }
  if (failing.length > 0) {
    const error = failing.find((t) => t.last_error)?.last_error
    return { tone: "warning", text: `Reconnecting${error ? ` — ${error}` : ""}` }
  }
  if (connecting.length > 0) return { tone: "idle", text: "Connecting to the server's push…" }

  const history = status.events_received > 0 ? ` · ${received}` : ""
  if (source.receiving_clients === 0) {
    return defaultPermission === "off"
      ? { tone: "idle", text: `Push is off for all clients${history}` }
      : { tone: "idle", text: `Not listening — no client may receive these events${history}` }
  }
  return { tone: "idle", text: `Not listening — no client is receiving these events right now${history}` }
}

/** A failing event type's state, shown under that event; null when it is fine. */
export function eventIssue(status: McpServerPushStatus | null, event: string): string | null {
  const t = status?.event_types.find((e) => e.event === event)
  if (!t || t.state !== "reconnecting") return null
  return t.last_error ? `Reconnecting — ${t.last_error}` : "Reconnecting"
}
