import { expect, test } from '@playwright/test'
import {
  applyInFlight,
  liveTimeline,
  RANGES,
  requestTimeline,
} from '../../../src/views/dashboard/activity-data'
import { dockTarget } from '../../../src/views/dashboard/dock'
import { apiFromPath, llmApiFlow } from '../../../src/views/monitor/llm-api'
import type { GraphData, MonitorEventSummary } from '../../../src/types/tauri-commands'

const MINUTE = 60_000
const base = Date.parse('2026-10-09T12:00:00Z')
const graph = (start: number, values: number[]): GraphData => ({
  labels: values.map((_, i) => new Date(start + i * MINUTE).toISOString()),
  datasets: [{ label: 'Requests', data: values }],
})
const summary = (changes: Partial<MonitorEventSummary> = {}): MonitorEventSummary => ({
  id: 'a', sequence: 1, timestamp: new Date(base).toISOString(), event_type: 'llm_call',
  session_id: null, client_id: null, client_name: null, status: 'pending', duration_ms: null,
  summary: '', question: '', answer: '', ...changes,
})

test('the live bucket holds in-flight requests and the current minute is the last bar', () => {
  const points = requestTimeline(graph(base, [1, 2, 3]), graph(base, [0, 0, 1]))
  const now = base + 2 * MINUTE + 30_000
  const live = liveTimeline(points, RANGES.ten_minutes.bucketMs, now, 2)
  expect(live).toHaveLength(3)
  expect(live[2]).toMatchObject({ timestamp: base + 2 * MINUTE, llm: 3, mcp: 1, inFlight: 2 })
  expect(live.slice(0, 2).every(point => point.inFlight === null)).toBe(true)
  expect(liveTimeline(points, MINUTE, now, 0)[2].inFlight).toBeNull()
})

test('a minute rollover since the last read adds the new, empty live bucket', () => {
  const points = requestTimeline(graph(base, [4]), graph(base, [0]))
  const live = liveTimeline(points, MINUTE, base + MINUTE + 5_000, 1)
  expect(live.map(point => point.timestamp)).toEqual([base, base + MINUTE])
  expect(live[1]).toMatchObject({ llm: 0, mcp: 0, inFlight: 1 })
})

test('stale series are not padded and do not place in-flight requests', () => {
  const points = requestTimeline(graph(base, [4]), graph(base, [0]))
  const live = liveTimeline(points, MINUTE, base + 7 * 24 * 60 * MINUTE, 3)
  expect(live).toHaveLength(1)
  expect(live[0].inFlight).toBeNull()
  expect(liveTimeline([], MINUTE, base, 3)).toEqual([])
})

test('in-flight tracking adds pending requests and reports when one settles', () => {
  const inFlight = new Map<string, MonitorEventSummary>()
  expect(applyInFlight(inFlight, summary())).toBe(false)
  expect(applyInFlight(inFlight, summary({ id: 'step', event_type: 'routing_decision' }))).toBe(false)
  expect(applyInFlight(inFlight, summary({ id: 'dup', duplicate_hop: 2 }))).toBe(false)
  expect([...inFlight.keys()]).toEqual(['a'])
  expect(applyInFlight(inFlight, summary({ status: 'complete' }))).toBe(true)
  expect(inFlight.size).toBe(0)
})

test('client and upstream APIs mark translation and fall back to the endpoint', () => {
  expect(llmApiFlow({ client_api: 'responses', upstream_api: 'chat_completions' })).toEqual({
    client: 'responses', upstream: 'chat_completions', translated: true,
  })
  expect(llmApiFlow({ client_api: 'chat_completions', upstream_api: 'chat_completions' }).translated).toBe(false)
  expect(llmApiFlow({ endpoint: '/v1/responses' })).toEqual({ client: 'responses', upstream: null, translated: false })
  expect(llmApiFlow({ endpoint: '/unknown', protocol: 'anthropic' }).client).toBe('anthropic_messages')
  expect(apiFromPath('/v1/chat/completions')).toBe('chat_completions')
  expect(apiFromPath('/v1/completions')).toBe('completions')
  expect(apiFromPath('/v1/messages?beta=true')).toBe('anthropic_messages')
  expect(apiFromPath('/v1beta/models/x:streamGenerateContent')).toBe('gemini_generate_content')
  expect(apiFromPath('/api/chat')).toBe('ollama_chat')
  expect(apiFromPath('/v1/models')).toBeNull()
})

test('dragging docks the detail at the nearest edge', () => {
  const rect = { left: 0, top: 0, width: 1000, height: 500 }
  expect(dockTarget(rect, 950, 100)).toBe('right')
  expect(dockTarget(rect, 400, 480)).toBe('bottom')
  expect(dockTarget(rect, 900, 480)).toBe('bottom')
})
