import { expect, test } from '@playwright/test'
import { applyInFlight, trafficPoints } from '../../../src/views/dashboard/activity-data'
import { dockTarget } from '../../../src/views/dashboard/dock'
import { apiFromPath, llmApiFlow } from '../../../src/views/monitor/llm-api'
import type { MonitorEventSummary, TrafficTimeline } from '../../../src/types/tauri-commands'

const base = Date.parse('2026-10-09T12:00:00Z')
const timeline = (values: number[], inFlight: number[] = []): TrafficTimeline => ({
  bucket_secs: 10,
  buckets: values.map((value, i) => ({
    timestamp: base / 1000 + i * 10,
    counts: { llm_requests: value, llm_errors: 0, mcp_requests: 0, mcp_errors: 0, cost_usd: 0 },
    in_flight: inFlight[i] ?? 0,
  })),
  totals: { llm_requests: 0, llm_errors: 0, mcp_requests: 0, mcp_errors: 0, cost_usd: 0 },
})
const summary = (changes: Partial<MonitorEventSummary> = {}): MonitorEventSummary => ({
  id: 'a', sequence: 1, timestamp: new Date(base).toISOString(), event_type: 'llm_call',
  session_id: null, client_id: null, client_name: null, status: 'pending', duration_ms: null,
  summary: '', question: '', answer: '', ...changes,
})

test('the live bucket shows requests running now, never less than the sampled peak', () => {
  const now = base + 25_000
  const live = trafficPoints(timeline([1, 2, 3], [0, 4, 1]), now, 2)
  expect(live).toHaveLength(3)
  expect(live[2]).toMatchObject({ timestamp: base + 20_000, completed: 3, inFlight: 2 })
  expect(live[1].inFlight).toBe(4)
  expect(trafficPoints(timeline([1, 2, 3], [0, 0, 5]), now, 2)[2].inFlight).toBe(5)
  expect(trafficPoints(timeline([1, 2, 3]), now, 0)[2].inFlight).toBe(0)
})

test('a bucket rollover since the last read adds the new live bucket and keeps the window length', () => {
  const live = trafficPoints(timeline([4, 5]), base + 21_000, 1)
  expect(live.map((point) => point.timestamp)).toEqual([base + 10_000, base + 20_000])
  expect(live[1]).toMatchObject({ completed: 0, errors: 0, inFlight: 1 })
})

test('stale timelines are not padded and do not place in-flight requests', () => {
  const live = trafficPoints(timeline([4]), base + 7 * 24 * 3_600_000, 3)
  expect(live).toHaveLength(1)
  expect(live[0].inFlight).toBe(0)
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
