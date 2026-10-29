import type { MonitorEventSummary } from '@/types/tauri-commands'

export const RELATIONSHIP_COLORS = ['#2563eb', '#c026d3', '#059669', '#d97706', '#7c3aed', '#4f46e5', '#0891b2']
const MAX_LANES = 4

export interface EventRelationship {
  id: string
  members: Set<string>
  first: number
  last: number
  color: string
  lane: number | null
}

/** Only recorded request/session and cross-hop trace IDs establish a relationship.
 * Union both IDs so a hop without a session can still join its request's scans.
 * Never infer a conversation from matching clients, timestamps, or message text.
 */
export function eventRelationships(events: MonitorEventSummary[], selectedId: string | null): EventRelationship[] {
  const parents = new Map<string, string>()
  const root = (key: string): string => {
    let current = key
    while (parents.has(current)) current = parents.get(current)!
    return current
  }
  const keys = events.map(event => [
    event.session_id ? `session:${event.session_id}` : null,
    event.trace_id ? `trace:${event.trace_id}` : null,
  ].filter((key): key is string => key !== null))
  for (const [session, trace] of keys) {
    if (session && trace) {
      const a = root(session)
      const b = root(trace)
      if (a !== b) parents.set(a < b ? b : a, a < b ? a : b)
    }
  }
  const groups = new Map<string, EventRelationship>()
  events.forEach((event, index) => {
    if (!keys[index].length) return
    const id = root(keys[index][0])
    let group = groups.get(id)
    if (!group) {
      let hash = 0
      for (const char of id) hash = (hash * 31 + char.charCodeAt(0)) >>> 0
      group = { id, members: new Set(), first: index, last: index, color: RELATIONSHIP_COLORS[hash % RELATIONSHIP_COLORS.length], lane: null }
      groups.set(id, group)
    }
    group.members.add(event.id)
    group.last = index
  })
  const relationships = [...groups.values()].filter(group => group.members.size > 1)
  // Reserve a visible lane for the selected group, even in very busy traffic.
  const allocated: EventRelationship[] = []
  const prioritized = [...relationships].sort((a, b) => Number(b.members.has(selectedId ?? '')) - Number(a.members.has(selectedId ?? '')) || a.first - b.first)
  for (const group of prioritized) {
    const occupied = new Set(allocated.filter(other => other.first <= group.last && other.last >= group.first).map(other => other.lane))
    group.lane = Array.from({ length: MAX_LANES }, (_, lane) => lane).find(lane => !occupied.has(lane)) ?? null
    if (group.lane !== null) allocated.push(group)
  }
  return relationships
}
