import { test, expect } from '@playwright/test'
import { eventRelationships } from '../../../src/views/monitor/event-relationships'
import type { MonitorEventSummary } from '../../../src/types/tauri-commands'

const event = (id: string, fields: Partial<MonitorEventSummary> = {}): MonitorEventSummary => ({
  id, sequence: 1, timestamp: '2026-10-29T12:00:00Z', event_type: 'llm_call',
  status: 'complete', summary: '', question: '', answer: '', session_id: null,
  client_id: 'client', client_name: 'Client', duration_ms: 1, ...fields,
})

test('links different types through recorded request sessions across interleaved rows', () => {
  const events = [event('scan', { session_id: 'a', event_type: 'secret_scan' }), event('other'), event('llm', { session_id: 'a' })]
  const [group] = eventRelationships(events, null)
  expect([...group.members]).toEqual(['scan', 'llm'])
  expect([group.first, group.last, group.lane]).toEqual([0, 2, 0])
  expect(eventRelationships(events.filter(item => item.id !== 'llm'), null)).toEqual([])
})

test('joins trace-only hops with scans through an LLM event carrying both IDs', () => {
  const events = [event('hop', { trace_id: 't' }), event('scan', { session_id: 's' }), event('llm', { session_id: 's', trace_id: 't' })]
  expect(eventRelationships(events, 'scan')).toHaveLength(1)
  expect([...eventRelationships(events, 'scan')[0].members]).toEqual(['hop', 'scan', 'llm'])
})

test('does not guess relationships or collide session IDs with trace IDs', () => {
  expect(eventRelationships([event('a'), event('b'), event('c', { session_id: 'same' }), event('d', { trace_id: 'same' })], null)).toEqual([])
})

test('keeps overlapping groups apart, reuses free lanes, and reveals selected overflow', () => {
  const events = [...Array.from({ length: 6 }, (_, i) => event(`first-${i}`, { session_id: `s${i}` })),
    ...Array.from({ length: 6 }, (_, i) => event(`last-${i}`, { session_id: `s${i}` }))]
  const groups = eventRelationships(events, null)
  expect(groups.filter(group => group.lane !== null)).toHaveLength(4)
  expect(new Set(groups.filter(group => group.lane !== null).map(group => group.lane)).size).toBe(4)
  expect(eventRelationships(events, 'last-5').find(group => group.members.has('last-5'))?.lane).toBe(0)
  expect(eventRelationships([event('a', { session_id: '1' }), event('b', { session_id: '1' }), event('c', { session_id: '2' }), event('d', { session_id: '2' })], null).map(group => group.lane)).toEqual([0, 0])
})

test('color is stable after new events, filtering, and selection', () => {
  const events = [event('a', { session_id: 's' }), event('b', { session_id: 's' })]
  expect(eventRelationships([event('new'), ...events], 'b')[0].color).toBe(eventRelationships(events, null)[0].color)
})


test('transitive cross-hop groups keep the same identity when display order changes', () => {
  const events = [event('a', { session_id: 'z', trace_id: 't' }), event('b', { session_id: 'a', trace_id: 't' }), event('scan', { session_id: 'z', event_type: 'secret_scan' })]
  const group = eventRelationships(events, null)[0]
  expect(group.members.size).toBe(3)
  expect(eventRelationships([...events].reverse(), null)[0].id).toBe(group.id)
  expect(eventRelationships([...events].reverse(), null)[0].color).toBe(group.color)
})
