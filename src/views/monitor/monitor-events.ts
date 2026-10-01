import type { MonitorEventFilter, MonitorEventSummary } from '../../types/tauri-commands'

/** Mirror the backend predicate for live events, which bypass its query API. */
export function matchesFilter(summary: MonitorEventSummary, filter?: MonitorEventFilter | null): boolean {
  if (!filter) return true
  if (filter.event_types?.length && !filter.event_types.includes(summary.event_type)) return false
  if (filter.client_id && summary.client_id !== filter.client_id) return false
  if (filter.status && summary.status !== filter.status) return false
  if (filter.session_id && summary.session_id !== filter.session_id) return false
  if (filter.search && !summary.summary.toLowerCase().includes(filter.search.toLowerCase())) return false
  return true
}

/** Merge live updates over a snapshot, retaining newest-first order and unique IDs. */
export function mergeMonitorEvents(
  snapshot: MonitorEventSummary[],
  updates: Iterable<MonitorEventSummary>,
  filter?: MonitorEventFilter | null,
  limit = 500,
): MonitorEventSummary[] {
  const byId = new Map(snapshot.map(event => [event.id, event]))
  for (const event of updates) byId.set(event.id, event)
  return [...byId.values()]
    .filter(event => matchesFilter(event, filter))
    .sort((a, b) => b.sequence - a.sequence)
    .slice(0, limit)
}
