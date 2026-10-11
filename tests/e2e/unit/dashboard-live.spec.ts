import { expect, test } from '@playwright/test'
import { applyInFlight, chartPoints, livePoint, trafficPeak, trafficPoints } from '../../../src/views/dashboard/activity-data'
import { dockTarget } from '../../../src/views/dashboard/dock'
import { apiFromPath, llmApiFlow } from '../../../src/views/monitor/llm-api'
import type { MonitorEventSummary, TrafficTimeline } from '../../../src/types/tauri-commands'

const base = Date.parse('2026-10-09T12:00:00Z')
const timeline = (values: number[]): TrafficTimeline => ({
  bucket_secs: 10,
  buckets: values.map((value, i) => ({
    timestamp: base / 1000 + i * 10,
    counts: { llm_requests: value, llm_errors: 0, mcp_requests: 0, mcp_errors: 0, cost_usd: 0 },
  })),
  totals: { llm_requests: 0, llm_errors: 0, mcp_requests: 0, mcp_errors: 0, cost_usd: 0 },
})
const summary = (changes: Partial<MonitorEventSummary> = {}): MonitorEventSummary => ({
  id: 'a', sequence: 1, timestamp: new Date(base).toISOString(), event_type: 'llm_call',
  session_id: null, client_id: null, client_name: null, status: 'pending', duration_ms: null,
  summary: '', question: '', answer: '', ...changes,
})

test('only the bucket containing now is live, and the requests in progress count only there', () => {
  const now = base + 25_000
  const points = trafficPoints(timeline([1, 2, 3]), now)
  expect(points).toHaveLength(3)
  expect(points.map((point) => point.live)).toEqual([false, false, true])
  expect(livePoint(points)).toMatchObject({ timestamp: base + 20_000, completed: 3 })
  // The axis fits the live bucket's provisional value: finished so far plus in progress.
  expect(trafficPeak(points, 7)).toBe(10)
  expect(trafficPeak(points, 2)).toBe(5)
  expect(trafficPeak(points)).toBe(3)
  expect(trafficPeak(trafficPoints(timeline([9, 2, 3]), now), 2)).toBe(9)
})

test('the requests line is provisional only at the live bucket', () => {
  const rows = chartPoints(trafficPoints(timeline([1, 2, 3]), base + 25_000), 4)
  expect(rows.map((row) => row.requests)).toEqual([1, 2, 7])
  expect(rows.map((row) => row.completed)).toEqual([1, 2, 3])
  // Nothing finished and nothing running: the open bucket is at zero.
  const idle = chartPoints(trafficPoints(timeline([5, 0]), base + 15_000), 0)
  expect(idle.map((row) => row.requests)).toEqual([5, 0])
  // A stale timeline has no live bucket, so nothing is provisional.
  const stale = chartPoints(trafficPoints(timeline([4]), base + 7 * 24 * 3_600_000), 9)
  expect(stale.map((row) => row.requests)).toEqual([4])
})

test('the errors line rises from and falls back to zero around each run of errors only', () => {
  const withErrors = (errors: (number | null)[]): TrafficTimeline => ({
    ...timeline(errors.map(() => 5)),
    buckets: errors.map((value, i) => ({
      timestamp: base / 1000 + i * 10,
      counts:
        value === null
          ? null
          : { llm_requests: 5, llm_errors: value, mcp_requests: 0, mcp_errors: 0, cost_usd: 0 },
    })),
  })
  const line = (errors: (number | null)[]) =>
    chartPoints(trafficPoints(withErrors(errors), base), 0).map((row) => row.errorLine)
  expect(line([0, 0, 2, 0, 0, 0, 1, 3, 0])).toEqual([null, 0, 2, 0, null, 0, 1, 3, 0])
  // Runs one bucket apart share their zero.
  expect(line([1, 0, 1])).toEqual([1, 0, 1])
  expect(line([0, 0, 0])).toEqual([null, null, null])
  // Unknown buckets stay unknown, even next to errors.
  expect(line([null, 2, 0])).toEqual([null, 2, 0])
})

test('a bucket rollover since the last read adds the new live bucket and keeps the window length', () => {
  const live = trafficPoints(timeline([4, 5]), base + 21_000)
  expect(live.map((point) => point.timestamp)).toEqual([base + 10_000, base + 20_000])
  expect(live[1]).toMatchObject({ completed: 0, errors: 0, live: true })
  expect(live[0].live).toBe(false)
})

test('stale timelines are not padded and have no live bucket', () => {
  const stale = trafficPoints(timeline([4]), base + 7 * 24 * 3_600_000)
  expect(stale).toHaveLength(1)
  expect(livePoint(stale)).toBeNull()
  expect(trafficPeak(stale, 9)).toBe(4)
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
