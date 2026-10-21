import { type ReactNode, useEffect, useId, useState } from 'react'
import {
  CartesianGrid,
  ComposedChart,
  Curve,
  type CurveProps,
  Line,
  ResponsiveContainer,
  Tooltip,
  XAxis,
  YAxis,
} from 'recharts'
import { Activity, CircleAlert } from 'lucide-react'
import { cn } from '@/lib/utils'
import type { TimeRange, TrafficTimeline } from '@/types/tauri-commands'
import {
  chartPoints,
  type ChartPoint,
  type InFlightCounts,
  niceTicks,
  RANGES,
  roundTicks,
  trafficPeak,
  trafficPoints,
} from './activity-data'
import { CollapsibleSection, useCollapsed } from './collapsible-section'

const COLLAPSED_KEY = 'localrouter.dashboard.traffic.collapsed'

const compact = new Intl.NumberFormat(undefined, {
  notation: 'compact',
  maximumFractionDigits: 1,
})
const money = new Intl.NumberFormat(undefined, {
  style: 'currency',
  currency: 'USD',
  minimumFractionDigits: 2,
  maximumFractionDigits: 2,
})

/*
 * Series colors (validated with the dataviz palette checker, light and dark,
 * all pairs): LLM requests take categorical slot 1 (blue), MCP requests slot 3
 * (aqua; slot 2's orange is too close to the error red); errors the reserved
 * "critical" status red, labelled in the legend and tooltip.
 * Light and dark steps are set as CSS variables on the section.
 */
const LLM = 'var(--traffic-llm)'
const MCP = 'var(--traffic-mcp)'
const ERROR = 'var(--traffic-error)'
/**
 * Requests in progress: the same color as their line, lighter and dashed, so
 * they read as that line still growing. Dark mode keeps more of the stroke,
 * since a faded line sinks into the dark card.
 */
const IN_PROGRESS_OPACITY = 'var(--traffic-in-progress)'
const IN_PROGRESS_DASH = '3 3'
const PALETTE =
  '[--traffic-llm:#2a78d6] [--traffic-mcp:#1baf7a] [--traffic-error:#d03b3b] [--traffic-in-progress:0.4] dark:[--traffic-llm:#3987e5] dark:[--traffic-mcp:#199e70] dark:[--traffic-in-progress:0.55]'

const AXIS_TICK = { fill: 'hsl(var(--muted-foreground))', fontSize: 10 }
const GRID = 'hsl(var(--border))'
const SURFACE = 'hsl(var(--card))'
/** Chart height, its top margin and the time axis under the plot. */
const CHART_HEIGHT = 156
const CHART_TOP = 14
const AXIS_HEIGHT = 18

interface RequestTrafficProps {
  range: TimeRange
  onRangeChange: (range: TimeRange) => void
  /** `undefined` while the first read of this range is pending; `null` if it failed. */
  timeline: TrafficTimeline | null | undefined
  loading: boolean
  /** Requests still running right now, by kind. */
  inFlight: InFlightCounts
  className?: string
}

/** Clock for the live edge of the chart; only the chart re-renders on it. */
function useNow(intervalMs: number) {
  const [now, setNow] = useState(() => Date.now())
  useEffect(() => {
    const timer = window.setInterval(() => setNow(Date.now()), intervalMs)
    return () => window.clearInterval(timer)
  }, [intervalMs])
  return now
}

function formatTick(range: TimeRange, value: number): string {
  const date = new Date(value)
  if (range === 'week') return date.toLocaleString(undefined, { weekday: 'short', day: 'numeric' })
  if (range === 'month') return date.toLocaleString(undefined, { month: 'short', day: 'numeric' })
  return date.toLocaleString(undefined, { hour: '2-digit', minute: '2-digit' })
}

function formatBucket(range: TimeRange, start: number, bucketMs: number): string {
  const seconds = range === 'ten_minutes'
  const options: Intl.DateTimeFormatOptions = {
    hour: '2-digit',
    minute: '2-digit',
    ...(seconds ? { second: '2-digit' } : {}),
  }
  const from = new Date(start)
  const to = new Date(start + bucketMs)
  const day =
    range === 'week' || range === 'month' || range === 'day'
      ? `${from.toLocaleDateString(undefined, { weekday: 'short', month: 'short', day: 'numeric' })} `
      : ''
  return `${day}${from.toLocaleTimeString(undefined, options)}–${to.toLocaleTimeString(undefined, options)}`
}

function LineKey({ color, opacity, dashed }: { color: string; opacity?: string; dashed?: boolean }) {
  if (dashed) {
    return (
      <span
        className="h-0 w-3 shrink-0 border-t-2 border-dashed"
        style={{ borderColor: color, opacity }}
        aria-hidden
      />
    )
  }
  return (
    <span
      className="h-0.5 w-3 shrink-0 rounded-full"
      style={{ background: color, opacity }}
      aria-hidden
    />
  )
}

function TooltipRow({
  marker,
  label,
  value,
  muted,
}: {
  marker: ReactNode
  label: string
  value: ReactNode
  muted?: boolean
}) {
  return (
    <div className={cn('flex items-center gap-1.5', muted && 'text-muted-foreground')}>
      {marker}
      {label}
      <strong className="ml-auto pl-3 tabular-nums">{value}</strong>
    </div>
  )
}

function TrafficTooltip({
  active,
  payload,
  range,
  bucketMs,
  inFlight,
}: {
  active?: boolean
  payload?: { payload: ChartPoint }[]
  range: TimeRange
  bucketMs: number
  inFlight: InFlightCounts
}) {
  const point = payload?.[0]?.payload
  if (!active || !point) return null
  return (
    <div className="min-w-[11rem] rounded-lg border bg-popover px-2.5 py-1.5 text-xs text-popover-foreground shadow-md">
      {point.live && <div className="font-medium">Now (in progress)</div>}
      <div className="mb-1 text-muted-foreground">
        {formatBucket(range, point.timestamp, bucketMs)}
      </div>
      {point.completed === null ? (
        <div className="text-muted-foreground">Not recorded at this resolution</div>
      ) : (
        <div className="space-y-0.5">
          <TooltipRow marker={<LineKey color={LLM} />} label="LLM requests" value={point.llm} />
          <TooltipRow marker={<LineKey color={MCP} />} label="MCP requests" value={point.mcp} />
          <TooltipRow
            marker={<LineKey color={ERROR} />}
            label="Errors"
            value={point.errors}
            muted={!point.errors}
          />
          {point.live && inFlight.llm > 0 && (
            <TooltipRow
              marker={<LineKey color={LLM} opacity={IN_PROGRESS_OPACITY} dashed />}
              label="LLM in progress"
              value={inFlight.llm}
            />
          )}
          {point.live && inFlight.mcp > 0 && (
            <TooltipRow
              marker={<LineKey color={MCP} opacity={IN_PROGRESS_OPACITY} dashed />}
              label="MCP in progress"
              value={inFlight.mcp}
            />
          )}
          {point.cost ? (
            <div className="pl-[1.125rem] text-muted-foreground">
              Est. cost {money.format(point.cost)}
            </div>
          ) : null}
        </div>
      )}
    </div>
  )
}

/** Static key above the plot, one entry per line. */
function TrafficLegend() {
  return (
    <div
      data-testid="traffic-legend"
      className="flex flex-wrap items-center gap-x-3 gap-y-0.5 px-1 pb-1 text-[11px] text-muted-foreground"
    >
      <span className="flex items-center gap-1.5">
        <LineKey color={LLM} /> LLM requests
      </span>
      <span className="flex items-center gap-1.5">
        <LineKey color={MCP} /> MCP requests
      </span>
      <span className="flex items-center gap-1.5">
        <LineKey color={ERROR} /> Errors
      </span>
    </div>
  )
}

/**
 * Each traffic line, one piece per active run: a piece rises from the zero
 * before the run and falls to the zero after it, and no line joins two zeros,
 * so nothing runs along the baseline between runs.
 */
function TrafficCurve(props: CurveProps) {
  const runs: NonNullable<CurveProps['points']>[number][][] = []
  let run: NonNullable<CurveProps['points']>[number][] = []
  for (const point of props.points ?? []) {
    const value = (point as { value?: number | null }).value
    const previous = run[run.length - 1] as { value?: number | null } | undefined
    if (value === null || value === undefined || (value === 0 && previous?.value === 0)) {
      if (run.length > 1) runs.push(run)
      run = value === 0 ? [point] : []
    } else {
      run.push(point)
    }
  }
  if (run.length > 1) runs.push(run)
  return (
    <g>
      {runs.map((points) => (
        <Curve key={points[0].x} {...props} pathRef={undefined} points={points} />
      ))}
    </g>
  )
}

/**
 * A requests-in-progress segment: the curve through its line's values and
 * the live bucket's finished plus in progress, so it bends like the line it
 * continues, clipped to the last segment (previous bucket to the live one).
 */
function InProgressCurve(props: CurveProps) {
  const id = useId()
  const drawn = (props.points ?? []).filter(
    (point) => (point as { value?: number | null }).value != null,
  )
  const from = drawn[drawn.length - 2]?.x
  if (from == null) return null
  return (
    <g>
      <defs>
        <clipPath id={id}>
          <rect x={from} y={-CHART_HEIGHT} width={10_000} height={3 * CHART_HEIGHT} />
        </clipPath>
      </defs>
      <g clipPath={`url(#${id})`}>
        <Curve {...props} pathRef={undefined} />
      </g>
    </g>
  )
}

function InFlightPill({ inFlight }: { inFlight: number }) {
  return (
    <span
      aria-live="polite"
      className={cn(
        'flex items-center gap-1.5 rounded-full px-2 py-0.5 font-medium',
        inFlight > 0
          ? 'bg-amber-500/10 text-amber-700 dark:text-amber-300'
          : 'text-muted-foreground',
      )}
    >
      <span className="relative flex h-2 w-2">
        {inFlight > 0 && (
          <span className="absolute inline-flex h-full w-full animate-ping rounded-full bg-amber-400 opacity-75 motion-reduce:animate-none" />
        )}
        <span
          className={cn(
            'relative inline-flex h-2 w-2 rounded-full',
            inFlight > 0 ? 'bg-amber-500' : 'bg-muted-foreground/40',
          )}
        />
      </span>
      {inFlight > 0 ? `${inFlight} in progress` : 'Idle'}
    </span>
  )
}

export function RequestTraffic({
  range,
  onRangeChange,
  timeline,
  loading,
  inFlight,
  className,
}: RequestTrafficProps) {
  const [collapsed, toggle] = useCollapsed(COLLAPSED_KEY)
  const now = useNow(2_000)
  const points = trafficPoints(timeline ?? null, now)
  const rows = chartPoints(points, inFlight)
  const inFlightTotal = inFlight.llm + inFlight.mcp
  const bucketMs = (timeline?.bucket_secs ?? 0) * 1000
  const yTicks = niceTicks(trafficPeak(points, inFlight))
  // The newest point sits on the plot's right edge: a time label on the last
  // two buckets would run past it.
  const lastTick = points[points.length - 3]?.timestamp ?? -Infinity
  const xTicks = roundTicks(range, points.map((p) => p.timestamp), bucketMs).filter(
    (t) => t <= lastTick,
  )
  const errorBuckets = points.filter((p) => p.errors).length
  const totals = timeline?.totals ?? null
  const total = totals ? totals.llm_requests + totals.mcp_requests : null
  const errors = totals ? totals.llm_errors + totals.mcp_errors : null
  const initialLoading = loading && timeline === undefined
  const unavailable = !initialLoading && timeline === null
  const busy = points.filter((p) => p.completed || p.errors)

  const strong = (value: string) => (
    <strong className="font-medium text-foreground tabular-nums">{value}</strong>
  )
  const summary = (
    <div className="flex flex-wrap items-center gap-x-4 gap-y-1 text-xs text-muted-foreground">
      <span>
        <strong className="text-sm font-semibold text-foreground tabular-nums">
          {initialLoading ? '…' : (total?.toLocaleString() ?? '—')}
        </strong>{' '}
        requests
      </span>
      <span>
        LLM {strong(totals?.llm_requests.toLocaleString() ?? '—')} · MCP{' '}
        {strong(totals?.mcp_requests.toLocaleString() ?? '—')}
      </span>
      <span className="flex items-center gap-1">
        <CircleAlert
          className="h-3.5 w-3.5"
          style={errors ? { color: ERROR } : undefined}
          aria-hidden
        />
        {strong(errors?.toLocaleString() ?? '—')} {errors === 1 ? 'error' : 'errors'}
      </span>
      <span>
        {strong(totals ? money.format(totals.cost_usd) : '—')} est. cost
      </span>
      <InFlightPill inFlight={inFlightTotal} />
    </div>
  )

  const actions = (
    <>
      <div className="flex gap-0.5 rounded-md bg-muted/80 p-0.5" aria-label="Traffic time range">
        {(Object.keys(RANGES) as TimeRange[]).map((value) => (
          <button
            key={value}
            aria-pressed={range === value}
            aria-label={RANGES[value].description}
            onClick={() => onRangeChange(value)}
            className={cn(
              'rounded px-2 py-0.5 text-[11px] font-medium transition-colors focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring',
              range === value
                ? 'bg-card text-foreground shadow-sm'
                : 'text-muted-foreground hover:text-foreground',
            )}
          >
            {RANGES[value].label}
          </button>
        ))}
      </div>
    </>
  )

  const describe = `Request timeline, ${RANGES[range].description}, per ${RANGES[range].bucket}. ${
    total === null
      ? 'Request counts unavailable.'
      : `${total} finished requests (${totals?.llm_requests} LLM, ${totals?.mcp_requests} MCP), ${errors} ${errors === 1 ? 'error' : 'errors'}${
          errorBuckets ? ` in ${errorBuckets} ${errorBuckets === 1 ? 'interval' : 'intervals'}` : ''
        }.`
  } ${inFlightTotal} in progress now.`

  return (
    <CollapsibleSection
      ariaLabel="Request traffic"
      title="Traffic"
      icon={Activity}
      collapsed={collapsed}
      onToggle={toggle}
      // Expanded, the chart and its tooltip carry these numbers.
      summary={collapsed ? summary : undefined}
      actions={actions}
      className={cn(PALETTE, className)}
    >
      <div className="px-2 pb-2">
        {!initialLoading && points.length > 0 && <TrafficLegend />}
        <div className="relative" style={{ height: CHART_HEIGHT }} role="img" aria-label={describe}>
          {initialLoading ? (
            <div className="flex h-full items-center justify-center text-xs text-muted-foreground">
              Loading request traffic…
            </div>
          ) : points.length ? (
            <ResponsiveContainer
              width="100%"
              height="100%"
              minWidth={0}
              initialDimension={{ width: 600, height: CHART_HEIGHT }}
            >
              <ComposedChart
                data={rows}
                margin={{ top: CHART_TOP, right: 4, left: 0, bottom: 0 }}
                accessibilityLayer
              >
                <CartesianGrid vertical={false} stroke={GRID} strokeWidth={1} />
                <XAxis
                  dataKey="timestamp"
                  ticks={xTicks}
                  interval={0}
                  tickFormatter={(value) => formatTick(range, value)}
                  tick={AXIS_TICK}
                  axisLine={{ stroke: GRID }}
                  tickLine={false}
                  tickMargin={3}
                  height={AXIS_HEIGHT}
                />
                <YAxis
                  allowDecimals={false}
                  tickFormatter={(value) => compact.format(value)}
                  tick={AXIS_TICK}
                  axisLine={false}
                  tickLine={false}
                  width={32}
                  ticks={yTicks}
                  interval={0}
                  domain={[0, yTicks[yTicks.length - 1]]}
                />
                <Tooltip
                  cursor={{ stroke: 'hsl(var(--muted-foreground))', strokeWidth: 1 }}
                  content={
                    <TrafficTooltip range={range} bucketMs={bucketMs} inFlight={inFlight} />
                  }
                  filterNull={false}
                  isAnimationActive={false}
                />
                {/*
                  Each requests line, and under it the requests of its kind
                  in progress now stacked on top: a lighter, dashed segment from the
                  previous bucket to finished plus in progress at the live one,
                  bending with the line it continues.
                */}
                <Line
                  name="LLM requests in progress"
                  dataKey="llmInProgress"
                  type="monotone"
                  stroke={LLM}
                  strokeOpacity={IN_PROGRESS_OPACITY}
                  strokeDasharray={IN_PROGRESS_DASH}
                  strokeWidth={2}
                  strokeLinejoin="round"
                  strokeLinecap="round"
                  dot={false}
                  activeDot={false}
                  className="traffic-in-progress"
                  shape={<InProgressCurve />}
                  isAnimationActive={false}
                />
                <Line
                  name="LLM requests"
                  dataKey="llmLine"
                  type="monotone"
                  stroke={LLM}
                  strokeWidth={2}
                  strokeLinejoin="round"
                  strokeLinecap="round"
                  dot={false}
                  activeDot={(props: { cx?: number; cy?: number; payload?: ChartPoint }) =>
                    props.payload?.llm ? (
                      <circle
                        key="traffic-llm-active"
                        cx={props.cx}
                        cy={props.cy}
                        r={4}
                        strokeWidth={2}
                        stroke={SURFACE}
                        fill={LLM}
                      />
                    ) : (
                      <g key="traffic-llm-active" />
                    )
                  }
                  className="traffic-llm"
                  shape={<TrafficCurve />}
                  isAnimationActive={false}
                />
                <Line
                  name="MCP requests in progress"
                  dataKey="mcpInProgress"
                  type="monotone"
                  stroke={MCP}
                  strokeOpacity={IN_PROGRESS_OPACITY}
                  strokeDasharray={IN_PROGRESS_DASH}
                  strokeWidth={2}
                  strokeLinejoin="round"
                  strokeLinecap="round"
                  dot={false}
                  activeDot={false}
                  className="traffic-in-progress"
                  shape={<InProgressCurve />}
                  isAnimationActive={false}
                />
                <Line
                  name="MCP requests"
                  dataKey="mcpLine"
                  type="monotone"
                  stroke={MCP}
                  strokeWidth={2}
                  strokeLinejoin="round"
                  strokeLinecap="round"
                  dot={false}
                  activeDot={(props: { cx?: number; cy?: number; payload?: ChartPoint }) =>
                    props.payload?.mcp ? (
                      <circle
                        key="traffic-mcp-active"
                        cx={props.cx}
                        cy={props.cy}
                        r={4}
                        strokeWidth={2}
                        stroke={SURFACE}
                        fill={MCP}
                      />
                    ) : (
                      <g key="traffic-mcp-active" />
                    )
                  }
                  className="traffic-mcp"
                  shape={<TrafficCurve />}
                  isAnimationActive={false}
                />
                {/*
                  Errors over the requests lines: each run of errors rises from
                  zero and falls back to it; no line where there are none.
                */}
                <Line
                  name="Errors"
                  dataKey="errorLine"
                  type="monotone"
                  stroke={ERROR}
                  strokeWidth={2}
                  strokeLinejoin="round"
                  strokeLinecap="round"
                  dot={false}
                  activeDot={(props: { cx?: number; cy?: number; payload?: ChartPoint }) =>
                    props.payload?.errors ? (
                      <circle
                        key="errors-active"
                        cx={props.cx}
                        cy={props.cy}
                        r={4}
                        strokeWidth={2}
                        stroke={SURFACE}
                        fill={ERROR}
                      />
                    ) : (
                      <g key="errors-active" />
                    )
                  }
                  className="traffic-errors"
                  shape={<TrafficCurve />}
                  isAnimationActive={false}
                />
              </ComposedChart>
            </ResponsiveContainer>
          ) : (
            <div className="flex h-full items-center justify-center gap-2 text-xs text-muted-foreground">
              <Activity className="h-4 w-4 opacity-50" />
              {unavailable
                ? 'Request traffic is unavailable'
                : 'No traffic in this period'}
            </div>
          )}
          {total === 0 && inFlightTotal === 0 && points.length > 0 && (
            <div className="pointer-events-none absolute inset-x-0 top-0 flex h-[60%] items-center justify-center text-xs text-muted-foreground">
              No requests in this period. Your next request will appear here live.
            </div>
          )}
        </div>
        {busy.length > 0 && (
          <table className="sr-only">
            <caption>Request traffic per {RANGES[range].bucket}</caption>
            <thead>
              <tr>
                <th scope="col">Time</th>
                <th scope="col">Finished</th>
                <th scope="col">Errors</th>
              </tr>
            </thead>
            <tbody>
              {busy.map((p) => (
                <tr key={p.timestamp}>
                  <td>{formatBucket(range, p.timestamp, bucketMs)}</td>
                  <td>{p.completed}</td>
                  <td>{p.errors}</td>
                </tr>
              ))}
            </tbody>
          </table>
        )}
      </div>
    </CollapsibleSection>
  )
}
