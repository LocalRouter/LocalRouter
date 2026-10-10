import type {
  GraphData,
  MonitorEventSummary,
  TimeRange,
} from '../../types/tauri-commands'

const MINUTE = 60_000

export const RANGES: Record<
  TimeRange,
  {
    label: string
    description: string
    bucket: string
    bucketMs: number
    /** How often the chart re-reads metrics while nothing else triggers it. */
    pollMs: number
  }
> = {
  ten_minutes: {
    label: '10m',
    description: 'last 10 minutes',
    bucket: 'minute',
    bucketMs: MINUTE,
    pollMs: 5_000,
  },
  hour: {
    label: '1h',
    description: 'last hour',
    bucket: '5 minutes',
    bucketMs: 5 * MINUTE,
    pollMs: 10_000,
  },
  day: {
    label: '24h',
    description: 'last 24 hours',
    bucket: 'hour',
    bucketMs: 60 * MINUTE,
    pollMs: 30_000,
  },
  week: {
    label: '7d',
    description: 'last 7 days',
    bucket: '6 hours',
    bucketMs: 360 * MINUTE,
    pollMs: 60_000,
  },
  month: {
    label: '30d',
    description: 'last 30 days',
    bucket: 'day',
    bucketMs: 1440 * MINUTE,
    pollMs: 60_000,
  },
}

// Backend graph labels are UTC, without an explicit timezone. ISO labels are
// also accepted for demo data. Normalize before joining independently loaded series.
export function graphTimestamp(label: string): number {
  return Date.parse(
    /^\d{4}-\d\d-\d\d \d\d:\d\d$/.test(label)
      ? `${label.replace(' ', 'T')}:00Z`
      : label,
  )
}

export function graphValues(graph: GraphData | null): Map<number, number> {
  const values = new Map<number, number>()
  graph?.labels.forEach((label, index) => {
    const timestamp = graphTimestamp(label)
    if (!Number.isFinite(timestamp)) return
    values.set(
      timestamp,
      graph.datasets.reduce(
        (sum, dataset) => sum + (dataset.data[index] ?? 0),
        0,
      ),
    )
  })
  return values
}

export function requestTimeline(llm: GraphData | null, mcp: GraphData | null) {
  const llmValues = graphValues(llm)
  const mcpValues = graphValues(mcp)
  return [...new Set([...llmValues.keys(), ...mcpValues.keys()])]
    .sort((a, b) => a - b)
    .map((timestamp) => ({
      timestamp,
      // A missing bucket is unknown, not evidence of zero requests.
      llm: llmValues.get(timestamp) ?? null,
      mcp: mcpValues.get(timestamp) ?? null,
    }))
}

export interface TrafficPoint {
  timestamp: number
  llm: number | null
  mcp: number | null
  /** Requests still running; only set on the bucket that contains "now". */
  inFlight: number | null
}

/**
 * Extend the timeline to the bucket containing `now` (the backend may not have
 * emitted it yet) and place running requests on it, so the chart's right edge
 * is always the live bucket.
 */
export function liveTimeline(
  points: ReturnType<typeof requestTimeline>,
  bucketMs: number,
  now: number,
  inFlight: number,
): TrafficPoint[] {
  const result: TrafficPoint[] = points.map((point) => ({ ...point, inFlight: null }))
  const last = result[result.length - 1]
  // Only bridge a minute rollover since the last read; an older series is
  // stale data, not a run of empty buckets.
  if (last && now - last.timestamp < 2 * bucketMs) {
    // Align new buckets to the backend's own boundaries.
    for (let next = last.timestamp + bucketMs; next <= now; next += bucketMs) {
      result.push({ timestamp: next, llm: 0, mcp: 0, inFlight: null })
    }
  }
  const live = result[result.length - 1]
  if (live && inFlight > 0 && now - live.timestamp < bucketMs) live.inFlight = inFlight
  return result
}

export function graphTotal(graph: GraphData | null): number | null {
  return graph
    ? [...graphValues(graph).values()].reduce((sum, value) => sum + value, 0)
    : null
}

const MCP_REQUESTS = new Set([
  'mcp_tool_call',
  'mcp_resource_read',
  'mcp_prompt_get',
  'mcp_sampling',
  'mcp_elicitation',
])

export function activityKind(
  event: MonitorEventSummary,
): 'LLM' | 'MCP' | 'System' {
  if (event.event_type === 'llm_call') return 'LLM'
  if (MCP_REQUESTS.has(event.event_type)) return 'MCP'
  return 'System'
}

export function isRequest(event: MonitorEventSummary): boolean {
  return (
    activityKind(event) !== 'System' &&
    !(event.duplicate_hop && event.duplicate_hop >= 2)
  )
}

/** Apply one live summary to the in-flight set; returns whether a request settled. */
export function applyInFlight(
  inFlight: Map<string, MonitorEventSummary>,
  event: MonitorEventSummary,
): boolean {
  if (!isRequest(event)) return false
  if (event.status === 'pending') {
    inFlight.set(event.id, event)
    return false
  }
  inFlight.delete(event.id)
  return true
}
