import type { PushSourceInfo } from "../../types/tauri-commands"

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
