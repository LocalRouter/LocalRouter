import { test, expect } from '@playwright/test'
import { eventRelationships } from '../../../src/views/monitor/event-relationships'
import type { MonitorEventSummary } from '../../../src/types/tauri-commands'

const event = (id: string, fields: Partial<MonitorEventSummary> = {}): MonitorEventSummary => ({
  id, sequence: 1, timestamp: '2026-10-29T12:00:00Z', event_type: 'llm_call',
  status: 'complete', summary: '', question: '', answer: '', session_id: null,
  client_id: 'client', client_name: 'Client', duration_ms: 1, ...fields,
})
const conversation = { id: 'c', title: 'First user message', source: 'explicit' as const }

test('links requests across a conversation and scans through their parent request', () => {
  const events = [event('scan', { session_id: 'b', event_type: 'secret_scan' }), event('other'),
    event('next', { session_id: 'b', conversation }), event('first', { session_id: 'a', conversation })]
  const group = eventRelationships(events).find(g => g.members.has('scan'))!
  expect([...group.members]).toEqual(['scan', 'next', 'first'])
  expect([group.first, group.last]).toEqual([0, 3])
  expect(group.title).toBe('First user message')
})

test('joins trace-only hops with scans and keeps singleton unknown requests separate', () => {
  const events = [event('hop', { trace_id: 't' }), event('scan', { session_id: 's' }),
    event('llm', { session_id: 's', trace_id: 't' }), event('unknown')]
  expect(eventRelationships(events).map(g => g.members.size)).toEqual([3, 1])
})

test('does not conflate clients, repeated prompt text, request IDs and trace IDs', () => {
  const events = [event('a', { question: 'Same' }), event('b', { question: 'Same' }),
    event('c', { session_id: 'same' }), event('d', { trace_id: 'same' }),
    event('e', { conversation }), event('f', { conversation, client_id: 'other' })]
  expect(eventRelationships(events)).toHaveLength(6)
})

test('moves the whole conversation to the top on new activity and sorts its rows newest first', () => {
  const a = event('a', { sequence: 1, conversation })
  const b = event('b', { sequence: 2 })
  const next = event('next', { sequence: 3, conversation })
  expect(eventRelationships([b, a])[0].members.has('b')).toBe(true)
  const groups = eventRelationships([next, b, a])
  expect(groups[0].events.map(e => e.id)).toEqual(['next', 'a'])
  expect(groups[1].events.map(e => e.id)).toEqual(['b'])
})

test('fallback headers are honest and redacted events cannot regain links', () => {
  const groups = eventRelationships([event('one', { session_id: 's' }), event('two', { session_id: 's', payload_redacted: true })])
  expect(groups).toHaveLength(2)
  expect(groups[0].title).toContain('Request')
  expect(groups[0].description).toContain('No conversation identity')
})

test('push events join the request of the session that read them and otherwise stand alone', () => {
  const events = [
    event('poll', { session_id: 'r', event_type: 'mcp_tool_call' }),
    event('push', { session_id: 'r', event_type: 'mcp_push_event' }),
    event('llm', { session_id: 'r', conversation }),
    event('webhook', { event_type: 'mcp_push_event' }),
  ]
  const groups = eventRelationships(events)
  expect(groups.find(g => g.members.has('push'))!.members).toEqual(new Set(['poll', 'push', 'llm']))
  expect(groups.find(g => g.members.has('webhook'))!.members.size).toBe(1)
})
