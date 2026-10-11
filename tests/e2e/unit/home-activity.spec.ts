import { expect, test } from '@playwright/test'
import {
  isRequest,
  niceTicks,
  roundTicks,
  trafficPeak,
  trafficPoints,
} from '../../../src/views/dashboard/activity-data'
import type {
  MonitorEventSummary,
  TrafficBucket,
  TrafficTimeline,
} from '../../../src/types/tauri-commands'

const base = Date.parse('2026-10-02T12:00:00Z') / 1000
const counts = (llm: number, mcp: number, errors = 0) => ({
  llm_requests: llm,
  llm_errors: errors,
  mcp_requests: mcp,
  mcp_errors: 0,
  cost_usd: llm * 0.01,
})
function timeline(buckets: TrafficBucket[], bucketSecs = 10): TrafficTimeline {
  return {
    bucket_secs: bucketSecs,
    buckets,
    totals: counts(0, 0),
  }
}
const event = (
  changes: Partial<MonitorEventSummary> = {},
): MonitorEventSummary => ({
  id: 'a',
  sequence: 1,
  timestamp: '2026-10-02T12:00:00Z',
  event_type: 'llm_call',
  session_id: null,
  client_id: 'client-a',
  client_name: 'Client A',
  status: 'complete',
  duration_ms: 100,
  summary: 'A request',
  question: 'Hello',
  answer: 'Hi',
  ...changes,
})

test('buckets become points with finished requests and errors; unknown stays unknown', () => {
  const points = trafficPoints(
    timeline([
      { timestamp: base, counts: null },
      { timestamp: base + 10, counts: counts(4, 1, 2) },
    ]),
    (base + 15) * 1000,
  )
  expect(points).toEqual([
    { timestamp: base * 1000, completed: null, llm: null, mcp: null, errors: null, cost: null, live: false },
    { timestamp: (base + 10) * 1000, completed: 5, llm: 4, mcp: 1, errors: 2, cost: 0.04, live: true },
  ])
  expect(trafficPeak(points)).toBe(5)
  expect(trafficPoints(null, base * 1000)).toEqual([])
  expect(trafficPoints(timeline([]), base * 1000)).toEqual([])
})

test('y-axis ticks are whole numbers from zero to a round top', () => {
  expect(niceTicks(0)).toEqual([0, 1, 2])
  expect(niceTicks(0, 1)).toEqual([0, 1])
  expect(niceTicks(3)).toEqual([0, 2, 4])
  expect(niceTicks(7)).toEqual([0, 4, 8])
  expect(niceTicks(11)).toEqual([0, 10, 20])
  expect(niceTicks(150)).toEqual([0, 100, 200])
  expect(niceTicks(1000)).toEqual([0, 500, 1000])
})

test('in-flight request classification excludes internal steps and duplicate hops', () => {
  const items = [
    event(),
    event({ id: 'duplicate', duplicate_hop: 2 }),
    event({ id: 'step', event_type: 'routing_decision' }),
    event({ id: 'error', event_type: 'auth_error', status: 'error' }),
    event({ id: 'mcp', event_type: 'mcp_tool_call' }),
  ]
  expect(items.filter(isRequest).map((item) => item.id)).toEqual(['a', 'mcp'])
  expect(isRequest(event({ event_type: 'mcp_sampling' }))).toBe(true)
})

test('x-axis ticks land on round local times at any bucket alignment', () => {
  const start = Date.parse('2026-10-02T00:00:00Z')
  const tenSeconds = Array.from({ length: 60 }, (_, i) => start + i * 10_000)
  expect(roundTicks('ten_minutes', tenSeconds, 10_000)).toHaveLength(5)
  const eightHours = Array.from({ length: 90 }, (_, i) => start + i * 8 * 3_600_000)
  const ticks = roundTicks('month', eightHours, 8 * 3_600_000)
  expect(ticks.length).toBeGreaterThanOrEqual(5)
  expect(ticks.length).toBeLessThanOrEqual(7)
})
