import type { PushPermission, PushSourceInfo } from "../../types/tauri-commands"

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

/** "Current" / "Stale" / "Not known yet" for a server's remembered capabilities. */
export function snapshotStatusLabel(source: PushSourceInfo): string {
  switch (source.snapshot_status) {
    case "current":
      return "Current"
    case "stale":
      return "Stale"
    default:
      return "Not known yet"
  }
}

/**
 * One line on what LocalRouter does with a server's push, for its MCP server page.
 * `defaultPermission` is the global default, so "nobody receives it" can say that
 * push is off rather than only that no client is set up.
 */
export function serverPushSummary(source: PushSourceInfo, defaultPermission: PushPermission): string {
  if (!source.enabled) return "The server is disabled, so nothing is relayed from it."
  if (source.snapshot_status === "none") {
    return "Capabilities not known yet — they appear after the server first connects."
  }
  if (source.snapshot_status === "stale") {
    return "The server's configuration changed since LocalRouter last saw it; its push appears again once it reconnects."
  }
  if (source.has_consumable_push) {
    if (source.receiving_clients > 0) {
      const n = source.receiving_clients
      return `Relayed through Push Relay to ${n} client${n === 1 ? "" : "s"}.`
    }
    return defaultPermission === "off"
      ? "Push is off: the default permission is Off and no client setting turns this server's events on."
      : "Not relayed to any client: none has MCP access to this server with a push permission other than Off."
  }
  if (source.events.length > 0 && source.events.every((e) => !e.supported)) {
    return "Only webhook delivery is advertised. LocalRouter does not receive upstream webhooks, so nothing is relayed."
  }
  return "No push advertised."
}
