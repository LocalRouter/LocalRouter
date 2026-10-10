import type {
  MonitorEventSummary,
  TimeRange,
  TrafficTimeline,
} from '../../types/tauri-commands'

/** Ranges of the traffic chart; buckets mirror `TimeRange::traffic_bucket_secs`. */
export const RANGES: Record<
  TimeRange,
  {
    label: string
    description: string
    /** Bucket length, for screen-reader text. */
    bucket: string
    /** How often the chart re-reads traffic while nothing else triggers it. */
    pollMs: number
  }
> = {
  ten_minutes: {
    label: '10m',
    description: 'last 10 minutes',
    bucket: '10 seconds',
    pollMs: 5_000,
  },
  hour: {
    label: '1h',
    description: 'last hour',
    bucket: 'minute',
    pollMs: 10_000,
  },
  day: {
    label: '24h',
    description: 'last 24 hours',
    bucket: '15 minutes',
    pollMs: 30_000,
  },
  week: {
    label: '7d',
    description: 'last 7 days',
    bucket: '2 hours',
    pollMs: 60_000,
  },
  month: {
    label: '30d',
    description: 'last 30 days',
    bucket: '8 hours',
    pollMs: 60_000,
  },
}

export interface TrafficPoint {
  /** Bucket start, ms. */
  timestamp: number
  /** Finished requests (LLM + MCP), failed ones included; `null` when unknown. */
  completed: number | null
  llm: number | null
  mcp: number | null
  /** Failed requests (LLM + MCP). */
  errors: number | null
  /** Most requests in progress at once during the bucket. */
  inFlight: number | null
  cost: number | null
}

/**
 * Chart points for a timeline. The timeline is extended to the bucket that
 * contains `now` (it may have rolled over since the last read), keeping the
 * window length, and the requests running right now are placed on that live
 * bucket so the chart reacts before the next read.
 */
export function trafficPoints(
  timeline: TrafficTimeline | null,
  now: number,
  inFlight: number,
): TrafficPoint[] {
  if (!timeline || timeline.buckets.length === 0) return []
  const bucketMs = timeline.bucket_secs * 1000
  const points: TrafficPoint[] = timeline.buckets.map(({ timestamp, counts, in_flight }) => ({
    timestamp: timestamp * 1000,
    completed: counts ? counts.llm_requests + counts.mcp_requests : null,
    llm: counts?.llm_requests ?? null,
    mcp: counts?.mcp_requests ?? null,
    errors: counts ? counts.llm_errors + counts.mcp_errors : null,
    inFlight: in_flight,
    cost: counts?.cost_usd ?? null,
  }))
  const length = points.length
  const last = points[length - 1]
  // Only bridge rollovers since the last read; an older timeline is stale
  // data, not a run of empty buckets.
  if (now - last.timestamp < 2 * bucketMs) {
    for (let next = last.timestamp + bucketMs; next <= now; next += bucketMs) {
      points.push({
        timestamp: next,
        completed: 0,
        llm: 0,
        mcp: 0,
        errors: 0,
        inFlight: 0,
        cost: 0,
      })
    }
  }
  const window = points.slice(-length)
  const live = window[window.length - 1]
  if (live.completed !== null && now - live.timestamp < bucketMs) {
    live.inFlight = Math.max(live.inFlight ?? 0, inFlight)
  }
  return window
}

/** Highest finished-request or in-progress value, for the shared y-axis. */
export function trafficPeak(points: TrafficPoint[]): number {
  return Math.max(0, ...points.map((p) => Math.max(p.completed ?? 0, p.inFlight ?? 0)))
}

/** Whole-number y-axis ticks from 0 to a round top of at least `peak`. */
export function niceTicks(peak: number, min = 2): number[] {
  const target = Math.max(min, Math.ceil(peak))
  const magnitude = 10 ** Math.floor(Math.log10(target))
  const top =
    [1, 2, 4, 6, 8, 10].map((m) => m * magnitude).find((m) => m >= target) ?? 10 * magnitude
  return top >= 2 ? [0, top / 2, top] : [0, top]
}

/** X-axis tick spacing per range: round local times. */
const TICK_EVERY: Record<TimeRange, number> = {
  ten_minutes: 2 * 60_000,
  hour: 10 * 60_000,
  day: 3 * 3_600_000,
  week: 24 * 3_600_000,
  month: 5 * 24 * 3_600_000,
}

/**
 * Bucket starts to label: the first bucket at or after each round local time
 * (every 2 minutes, 10 minutes, 3 hours, midnight, every 5th midnight), so
 * ticks stay evenly spaced in any time zone.
 */
export function roundTicks(range: TimeRange, timestamps: number[], bucketMs: number): number[] {
  const every = TICK_EVERY[range]
  return timestamps.filter((t) => {
    const local = t - new Date(t).getTimezoneOffset() * 60_000
    return Math.floor(local / every) !== Math.floor((local - bucketMs) / every)
  })
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
