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
  cost: number | null
  /** The bucket that contains the current time (still filling). */
  live: boolean
}

/**
 * Chart points for a timeline. The timeline is extended to the bucket that
 * contains `now` (it may have rolled over since the last read), keeping the
 * window length; that bucket is marked `live`.
 */
export function trafficPoints(timeline: TrafficTimeline | null, now: number): TrafficPoint[] {
  if (!timeline || timeline.buckets.length === 0) return []
  const bucketMs = timeline.bucket_secs * 1000
  const points: TrafficPoint[] = timeline.buckets.map(({ timestamp, counts }) => ({
    timestamp: timestamp * 1000,
    completed: counts ? counts.llm_requests + counts.mcp_requests : null,
    llm: counts?.llm_requests ?? null,
    mcp: counts?.mcp_requests ?? null,
    errors: counts ? counts.llm_errors + counts.mcp_errors : null,
    cost: counts?.cost_usd ?? null,
    live: false,
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
        cost: 0,
        live: false,
      })
    }
  }
  const window = points.slice(-length)
  const newest = window[window.length - 1]
  newest.live = newest.timestamp <= now && now - newest.timestamp < bucketMs
  return window
}

/** The bucket still filling, if the timeline reaches the current time. */
export function livePoint(points: TrafficPoint[]): TrafficPoint | null {
  const newest = points[points.length - 1]
  return newest?.live ? newest : null
}

/** Requests still running right now, by kind. */
export interface InFlightCounts {
  llm: number
  mcp: number
}

export const NONE_IN_FLIGHT: InFlightCounts = { llm: 0, mcp: 0 }

export function inFlightCounts(events: MonitorEventSummary[]): InFlightCounts {
  const counts = { llm: 0, mcp: 0 }
  for (const event of events) {
    const kind = activityKind(event)
    if (kind === 'LLM') counts.llm++
    else if (kind === 'MCP') counts.mcp++
  }
  return counts
}

/**
 * Top of the axis: the highest LLM, MCP or errors value of any bucket, the
 * live bucket counting its requests in progress now (its provisional value).
 * Errors cover both kinds, so they can top either requests line.
 */
export function trafficPeak(points: TrafficPoint[], inFlightNow = NONE_IN_FLIGHT): number {
  return Math.max(
    0,
    ...points.map((p) =>
      Math.max(
        (p.llm ?? 0) + (p.live ? inFlightNow.llm : 0),
        (p.mcp ?? 0) + (p.live ? inFlightNow.mcp : 0),
        p.errors ?? 0,
      ),
    ),
  )
}

export interface ChartPoint extends TrafficPoint {
  /** Finished requests near activity; idle baseline buckets are hidden, as for errors. */
  llmLine: number | null
  mcpLine: number | null
  /**
   * The in-progress segments, stacked on the LLM and MCP lines: the line's
   * values up to the live bucket, which counts finished plus in progress now,
   * so the curve matches its line; only the last segment (previous bucket to
   * the live one) is drawn. `null` when none of that kind are running.
   */
  llmInProgress: number | null
  mcpInProgress: number | null
  /**
   * The errors line: the errors where they happened and 0 at the buckets
   * next to them, so each run of errors rises from and falls back to zero;
   * `null` elsewhere, so no line runs along the baseline.
   */
  errorLine: number | null
}

/** Chart rows: the LLM and MCP lines, their in-progress segments, and the errors line. */
export function chartPoints(points: TrafficPoint[], inFlightNow = NONE_IN_FLIGHT): ChartPoint[] {
  // Keep the zero immediately on either side of each active run. Raw counts
  // remain on the row for tooltips, totals, and the in-progress curve.
  const lineValue = (i: number, kind: 'llm' | 'mcp' | 'errors'): number | null => {
    const value = points[i][kind]
    if (value === null) return null
    return value > 0 || Boolean(points[i - 1]?.[kind]) || Boolean(points[i + 1]?.[kind])
      ? value
      : null
  }
  const liveIndex = points.findIndex((p) => p.live)
  const live = points[liveIndex]
  /** The segment's curve at bucket `i`: the line before the live bucket, its top at it. */
  const inProgress = (i: number, kind: 'llm' | 'mcp'): number | null => {
    if (!live || inFlightNow[kind] === 0 || i > liveIndex) return null
    if (i === liveIndex) return (live[kind] ?? 0) + inFlightNow[kind]
    return points[i][kind]
  }
  return points.map((p, i) => ({
    ...p,
    llmLine: lineValue(i, 'llm'),
    mcpLine: lineValue(i, 'mcp'),
    llmInProgress: inProgress(i, 'llm'),
    mcpInProgress: inProgress(i, 'mcp'),
    errorLine: lineValue(i, 'errors'),
  }))
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

// Requests a client made. A pushed event (`mcp_push_event`) is listed in the
// request monitor like every Monitor record, but it is not a request: a held
// one waits for a decision with no timeout, so counting it would keep the
// in-flight number up for as long as nobody answers.
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
