import type { PushEventDecision } from '../../types/tauri-commands'

/** What happened to a pushed event, for the Monitor (McpPushEvent `decision`). */
export const PUSH_DECISION_LABELS: Record<PushEventDecision, string> = {
  pending: 'Held for approval',
  allowed: 'Allowed',
  asked_allowed: 'Approved',
  asked_edited: 'Approved with edits',
  denied: 'Denied',
  timeout: 'Denied (approval timed out)',
  blocked: 'Blocked by permission',
  hold_cap: 'Dropped (too many held events)',
  too_large: 'Dropped (too large)',
}

/** How the event reached LocalRouter (`ingress_mode`). */
export const PUSH_INGRESS_LABELS: Record<string, string> = {
  push: 'MCP Events stream',
  poll: 'MCP Events poll',
  channel: 'Claude channel',
}

/** How the client receives events (`egress_modes`). */
export const PUSH_EGRESS_LABELS: Record<string, string> = {
  stream: 'events/stream',
  poll: 'events/poll',
  channel: 'Claude Channels',
  webhook: 'Webhook',
  tools: 'EventPoll / EventWait tools',
}

export function pushDecisionLabel(decision: string): string {
  return PUSH_DECISION_LABELS[decision as PushEventDecision] ?? decision
}

/** The event reached the client. */
export function pushDecisionDelivered(decision: string): boolean {
  return decision === 'allowed' || decision === 'asked_allowed' || decision === 'asked_edited'
}

/**
 * The delivered payload differs from the one received (edited before
 * approval). Compares structurally, so key order does not count.
 */
export function pushPayloadEdited(original: unknown, delivered: unknown): boolean {
  if (delivered === undefined || delivered === null || original === undefined) return false
  return canonical(original) !== canonical(delivered)
}

function canonical(value: unknown): string {
  return JSON.stringify(value, (_key, v) =>
    v && typeof v === 'object' && !Array.isArray(v)
      ? Object.fromEntries(Object.entries(v as Record<string, unknown>).sort(([a], [b]) => a.localeCompare(b)))
      : v,
  )
}
