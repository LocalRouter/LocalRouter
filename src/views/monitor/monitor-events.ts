import type { MonitorEventFilter, MonitorEventSummary, MonitorEventType } from '../../types/tauri-commands'

/** Mirror the backend predicate for live events, which bypass its query API. */
export function matchesFilter(summary: MonitorEventSummary, filter?: MonitorEventFilter | null): boolean {
  if (!filter) return true
  if (filter.event_types != null && !filter.event_types.includes(summary.event_type)) return false
  if (filter.client_id && summary.client_id !== filter.client_id) return false
  if (filter.status && summary.status !== filter.status) return false
  if (filter.session_id && summary.session_id !== filter.session_id) return false
  const search = filter.search?.toLowerCase()
  if (search && ![summary.summary, summary.question, summary.answer, summary.trace_id].some(text => text?.toLowerCase().includes(search))) return false
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

/** Column visibility follows the filter, including types with no captured events. */
export function showEventTypeColumn(types: MonitorEventType[] | null): boolean {
  return types == null || new Set(types).size > 1
}

export function singleLinePreview(text: string | null | undefined): string {
  return text?.replace(/\s+/g, ' ').trim() || '—'
}
