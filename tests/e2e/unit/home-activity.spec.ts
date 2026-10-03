import { expect, test } from '@playwright/test'
import {
  graphTimestamp,
  graphTotal,
  isRequest,
  requestTimeline,
} from '../../../src/views/dashboard/activity-data'
import type {
  GraphData,
  MonitorEventSummary,
} from '../../../src/types/tauri-commands'

function graph(labels: string[], ...data: number[][]): GraphData {
  return {
    labels,
    datasets: data.map((values) => ({ label: 'Requests', data: values })),
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

test('joins UTC backend labels and ISO labels by timestamp, preserving gaps and totals', () => {
  const llm = graph(['2026-10-02 12:00', '2026-10-02 13:00'], [4, 7], [1, 2])
  const mcp = graph(['2026-10-02T13:00:00Z', '2026-10-02T14:00:00Z'], [3, 0])
  expect(requestTimeline(llm, mcp)).toEqual([
    { timestamp: Date.parse('2026-10-02T12:00:00Z'), llm: 5, mcp: null },
    { timestamp: Date.parse('2026-10-02T13:00:00Z'), llm: 9, mcp: 3 },
    { timestamp: Date.parse('2026-10-02T14:00:00Z'), llm: null, mcp: 0 },
  ])
  expect(graphTotal(llm)).toBe(14)
  expect(graphTimestamp('2026-10-02 12:00')).toBe(
    Date.parse('2026-10-02T12:00:00Z'),
  )
})

test('unavailable metrics remain distinct from measured zero requests', () => {
  expect(graphTotal(null)).toBeNull()
  expect(graphTotal(graph([], []))).toBe(0)
  expect(
    requestTimeline(null, graph(['2026-10-02 12:00'], [0]))[0],
  ).toMatchObject({ llm: null, mcp: 0 })
  expect(requestTimeline(graph(['invalid'], [9]), null)).toEqual([])
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
