import { expect, test } from '@playwright/test'
import { applyInFlight, chartPoints, errorsPeak, inFlightCounts, livePoint, trafficPeak, trafficPoints } from '../../../src/views/dashboard/activity-data'
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
  // The axis fits the live bucket's finished so far plus in progress, per kind.
  expect(trafficPeak(points, { llm: 7, mcp: 0 })).toBe(10)
  expect(trafficPeak(points, { llm: 2, mcp: 0 })).toBe(5)
  expect(trafficPeak(points, { llm: 0, mcp: 4 })).toBe(4)
  expect(trafficPeak(points)).toBe(3)
  expect(trafficPeak(trafficPoints(timeline([9, 2, 3]), now), { llm: 2, mcp: 0 })).toBe(9)
})

test('LLM and MCP each reach the requests axis top on their own; errors have their own axis', () => {
  const points = trafficPoints(
    {
      ...timeline([0]),
      buckets: [
        {
          timestamp: base / 1000,
          counts: { llm_requests: 4, llm_errors: 3, mcp_requests: 5, mcp_errors: 4, cost_usd: 0 },
        },
      ],
    },
    base + 5_000,
  )
  expect(trafficPeak(points)).toBe(5)
  expect(errorsPeak(points)).toBe(7)
})

test('the errors axis follows the busiest error bucket, so more errors draw higher', () => {
  const withErrors = (errors: (number | null)[]): TrafficTimeline => ({
    ...timeline(errors.map(() => 400)),
    buckets: errors.map((value, i) => ({
      timestamp: base / 1000 + i * 10,
      counts:
        value === null
          ? null
          : { llm_requests: 400, llm_errors: value, mcp_requests: 0, mcp_errors: 0, cost_usd: 0 },
    })),
  })
  const points = trafficPoints(withErrors([1, 3, null, 2]), base)
  expect(errorsPeak(points)).toBe(3)
  expect(chartPoints(points).map((row) => row.errorLine)).toEqual([1, 3, null, 2])
  expect(errorsPeak(trafficPoints(withErrors([0, null]), base))).toBe(0)
})

test('requests in progress stack on their line, from the previous bucket to the live one', () => {
  const now = base + 25_000
  const withMcp = (llm: number[], mcp: number[]): TrafficTimeline => ({
    ...timeline(llm),
    buckets: llm.map((value, i) => ({
      timestamp: base / 1000 + i * 10,
      counts: { llm_requests: value, llm_errors: 0, mcp_requests: mcp[i], mcp_errors: 0, cost_usd: 0 },
    })),
  })
  const rows = chartPoints(trafficPoints(withMcp([5, 3, 1], [0, 2, 4]), now), { llm: 1, mcp: 0 })
  // The lines stay at what finished; the live bucket's line is finished so far.
  expect(rows.map((row) => row.llmLine)).toEqual([5, 3, 1])
  expect(rows.map((row) => row.mcpLine)).toEqual([0, 2, 4])
  // 3 before, 1 finished and 1 running now: the segment's curve follows the
  // line and ends at 2 (only its last part, from 3 to 2, is drawn).
  expect(rows.map((row) => row.llmInProgress)).toEqual([5, 3, 2])
  // Nothing of that kind running: no segment.
  expect(rows.map((row) => row.mcpInProgress)).toEqual([null, null, null])
  const both = chartPoints(trafficPoints(withMcp([5, 3, 1], [0, 2, 4]), now), { llm: 0, mcp: 3 })
  expect(both.map((row) => row.mcpInProgress)).toEqual([0, 2, 7])
  expect(both.map((row) => row.llmInProgress)).toEqual([null, null, null])
  // A stale timeline has no live bucket, so nothing is in progress on it.
  const stale = chartPoints(trafficPoints(timeline([4]), base + 7 * 24 * 3_600_000), { llm: 9, mcp: 9 })
  expect(stale.map((row) => [row.llmLine, row.llmInProgress])).toEqual([[4, null]])
})

test('in-flight requests are counted by kind', () => {
  expect(
    inFlightCounts([
      summary({ id: 'a', event_type: 'llm_call' }),
      summary({ id: 'b', event_type: 'mcp_tool_call' }),
      summary({ id: 'c', event_type: 'mcp_resource_read' }),
    ]),
  ).toEqual({ llm: 1, mcp: 2 })
  expect(inFlightCounts([])).toEqual({ llm: 0, mcp: 0 })
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
    chartPoints(trafficPoints(withErrors(errors), base)).map((row) => row.errorLine)
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
  expect(trafficPeak(stale, { llm: 9, mcp: 9 })).toBe(4)
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
