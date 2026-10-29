import type { MonitorEventSummary } from '@/types/tauri-commands'

export interface EventRelationship {
  id: string
  members: Set<string>
  events: MonitorEventSummary[]
  keys: string[]
  first: number
  last: number
  title: string
  description: string
}

/** Conversation IDs span requests; session and trace IDs only link one request.
 * Scope all identities to a client. Never use timing or client alone as a link.
 */
export function eventRelationships(events: MonitorEventSummary[]): EventRelationship[] {
  const parents = new Map<string, string>()
  const root = (key: string): string => {
    let current = key
    while (parents.has(current)) current = parents.get(current)!
    return current
  }
  const keys = events.map(event => {
    if (event.payload_redacted) return [`event:${event.id}`]
    const scope = JSON.stringify(event.client_id)
    return [
      event.conversation?.id ? `conversation:${scope}:${event.conversation.id}` : null,
      event.session_id ? `session:${scope}:${event.session_id}` : null,
      event.trace_id ? `trace:${scope}:${event.trace_id}` : null,
    ].filter((key): key is string => key !== null)
  }).map((keys, i) => keys.length ? keys : [`event:${events[i].id}`])
  for (const list of keys) {
    for (const key of list.slice(1)) {
      const a = root(list[0]), b = root(key)
      if (a !== b) parents.set(a < b ? b : a, a < b ? a : b)
    }
  }
  const groups = new Map<string, EventRelationship>()
  events.forEach((event, index) => {
    const id = root(keys[index][0])
    let group = groups.get(id)
    if (!group) {
      group = { id, members: new Set(), events: [], keys: [], first: index, last: index, title: '', description: '' }
      groups.set(id, group)
    }
    group.members.add(event.id)
    group.events.push(event)
    group.keys.push(...keys[index])
    group.last = index
  })
  return [...groups.values()].map(group => {
    group.events.sort((a, b) => b.sequence - a.sequence)
    const oldest = [...group.events].reverse()
    const captured = oldest.find(e => e.conversation?.title)
    const conversation = (oldest.find(e => e.conversation?.source === 'explicit')
      || oldest.find(e => e.conversation?.source === 'response_chain')
      || oldest.find(e => e.conversation))?.conversation
    const first = oldest[0]
    group.keys = [...new Set(group.keys)]
    group.title = captured?.conversation?.title || `${first.client_name || first.client_id?.slice(0, 8) || 'Unknown client'} · ${conversation ? 'Conversation' : 'Request'} ${(conversation?.id || first.session_id || first.trace_id || first.id).slice(-8)}`
    group.description = conversation
      ? `${captured ? 'First captured user message.' : 'No user-message excerpt was captured.'} ${conversation.source === 'explicit' ? 'Linked by caller conversation ID.' : conversation.source === 'response_chain' ? 'Linked by a previous response ID.' : 'Inferred from matching message history; identical prompts alone are not linked.'}`
      : 'No conversation identity was captured. This group contains only related request events.'
    return group
  }).sort((a, b) => b.events[0].sequence - a.events[0].sequence)
}
