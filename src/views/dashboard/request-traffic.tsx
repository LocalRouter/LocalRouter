import { useEffect, useId, useState } from 'react'
import {
  CartesianGrid,
  ComposedChart,
  Curve,
  type CurveProps,
  LabelList,
  type LabelProps,
  Line,
  ResponsiveContainer,
  Tooltip,
  XAxis,
  YAxis,
} from 'recharts'
import { Activity, CircleAlert, RefreshCw } from 'lucide-react'
import { Button } from '@/components/ui/Button'
import { cn } from '@/lib/utils'
import type { TimeRange, TrafficTimeline } from '@/types/tauri-commands'
import {
  chartPoints,
  type ChartPoint,
  livePoint,
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
 * Series colors (validated with the dataviz palette checker, light and dark):
 * requests take categorical slot 1 (blue); errors the reserved "critical"
 * status red, always next to an icon and a label. Light and dark steps are
 * set as CSS variables on the section.
 */
const DONE = 'var(--traffic-done)'
const ERROR = 'var(--traffic-error)'
/**
 * The open bucket's segment: the same blue, lighter and dashed, so it reads
 * as the line still growing rather than another series. Dark mode keeps more
 * of the stroke, since a faded blue sinks into the dark card.
 */
const PROVISIONAL_DASH = '4 4'
const PROVISIONAL_OPACITY = 'var(--traffic-provisional)'
const PALETTE =
  '[--traffic-done:#2a78d6] [--traffic-error:#d03b3b] [--traffic-provisional:0.55] dark:[--traffic-done:#3987e5] dark:[--traffic-provisional:0.75]'

const AXIS_TICK = { fill: 'hsl(var(--muted-foreground))', fontSize: 10 }
const GRID = 'hsl(var(--border))'
const SURFACE = 'hsl(var(--card))'
/** Labels on the plot keep a ring of the card color so lines never cut them. */
const HALO = { paintOrder: 'stroke', stroke: SURFACE, strokeWidth: 3, strokeLinejoin: 'round' } as const
/** Error buckets get a count label while they are few enough to read. */
const MAX_LABELLED_ERROR_BUCKETS = 8
/** Chart height, and the plot inside it (less the top margin and time axis). */
const CHART_HEIGHT = 156
const CHART_TOP = 14
const AXIS_HEIGHT = 18
const PLOT_HEIGHT = CHART_HEIGHT - CHART_TOP - AXIS_HEIGHT
/**
 * Lowest drawn error peak, in px: a single error next to busy traffic would
 * otherwise be a flat smear on the baseline. Its count label and the tooltip
 * carry the exact number.
 */
const MIN_ERROR_PEAK = 8

interface RequestTrafficProps {
  range: TimeRange
  onRangeChange: (range: TimeRange) => void
  /** `undefined` while the first read of this range is pending; `null` if it failed. */
  timeline: TrafficTimeline | null | undefined
  loading: boolean
  /** Requests still running right now (LLM and MCP). */
  inFlight: number
  onRefresh: () => void
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

function LineKey({ color }: { color: string }) {
  return <span className="h-0.5 w-3 shrink-0 rounded-full" style={{ background: color }} aria-hidden />
}

function DashedKey({ color }: { color: string }) {
  return (
    <span
      className="h-0.5 w-3 shrink-0"
      style={{
        backgroundImage: `linear-gradient(to right, ${color} 50%, transparent 50%)`,
        backgroundSize: '4px 100%',
        opacity: PROVISIONAL_OPACITY,
      }}
      aria-hidden
    />
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
  inFlight: number
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
          <div className="flex items-center gap-1.5">
            <LineKey color={DONE} />
            {point.live ? 'Finished so far' : 'Finished'}
            <strong className="ml-auto pl-3 tabular-nums">{point.completed}</strong>
          </div>
          <div className="pl-[1.125rem] text-muted-foreground">
            LLM {point.llm} · MCP {point.mcp}
            {point.cost ? ` · ${money.format(point.cost)}` : ''}
          </div>
          {point.live && (
            <div className="flex items-center gap-1.5">
              <DashedKey color={DONE} />
              In progress
              <strong className="ml-auto pl-3 tabular-nums">{inFlight}</strong>
            </div>
          )}
          <div className={cn('flex items-center gap-1.5', !point.errors && 'text-muted-foreground')}>
            <CircleAlert
              className="h-3 w-3"
              style={point.errors ? { color: ERROR } : undefined}
              aria-hidden
            />
            Errors
            <strong className="ml-auto pl-3 tabular-nums">{point.errors}</strong>
          </div>
        </div>
      )}
    </div>
  )
}

/** Count above an error peak. */
function ErrorCount({ x, y, value }: LabelProps) {
  if (!value || x === undefined || y === undefined) return null
  return (
    <text
      x={Number(x)}
      y={Number(y) - 6}
      textAnchor="middle"
      fontSize={10}
      fontWeight={600}
      fill="hsl(var(--foreground))"
      style={HALO}
    >
      {value}
    </text>
  )
}

/**
 * The requests line, drawn as one curve so its smoothing runs through the
 * open bucket: solid up to the last finished bucket (`splitIndex`), then the
 * same curve lighter and dashed to the open bucket's provisional value. When
 * the bucket closes, the next render draws that stretch solid.
 */
function RequestsCurve({ splitIndex, ...props }: CurveProps & { splitIndex: number | null }) {
  const id = `traffic-split-${useId().replace(/[^\w-]/g, '')}`
  const splitX = splitIndex === null ? null : props.points?.[splitIndex]?.x
  if (splitX === null || splitX === undefined) return <Curve {...props} />
  const reach = 100_000
  return (
    <g>
      <defs>
        <clipPath id={`${id}-done`}>
          <rect x={splitX - reach} y={-reach} width={reach} height={2 * reach} />
        </clipPath>
        <clipPath id={`${id}-open`}>
          <rect x={splitX} y={-reach} width={reach} height={2 * reach} />
        </clipPath>
      </defs>
      <Curve {...props} clipPath={`url(#${id}-done)`} />
      <Curve
        {...props}
        pathRef={undefined}
        className="traffic-provisional"
        clipPath={`url(#${id}-open)`}
        strokeDasharray={PROVISIONAL_DASH}
        strokeOpacity={PROVISIONAL_OPACITY}
        strokeLinecap="butt"
      />
    </g>
  )
}

/**
 * The errors line, one piece per run of errors: a piece rises from the zero
 * before the run and falls to the zero after it, and no line joins two zeros,
 * so nothing runs along the baseline between runs.
 */
function ErrorsCurve(props: CurveProps) {
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
  onRefresh,
  className,
}: RequestTrafficProps) {
  const [collapsed, toggle] = useCollapsed(COLLAPSED_KEY)
  const now = useNow(2_000)
  const points = trafficPoints(timeline ?? null, now)
  const live = livePoint(points)
  const rows = chartPoints(points, inFlight)
  // The requests line turns provisional after the last finished bucket.
  const splitIndex = live && points.length > 1 ? points.length - 2 : null
  const bucketMs = (timeline?.bucket_secs ?? 0) * 1000
  const yTicks = niceTicks(trafficPeak(points, inFlight))
  const errorFloor = (yTicks[yTicks.length - 1] * MIN_ERROR_PEAK) / PLOT_HEIGHT
  const errorPeak = (p: ChartPoint) =>
    p.errorLine ? Math.max(p.errorLine, errorFloor) : p.errorLine
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
      <InFlightPill inFlight={inFlight} />
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
      <Button
        variant="ghost"
        size="sm"
        className="h-7 w-7 p-0"
        onClick={onRefresh}
        aria-label="Refresh activity"
        disabled={initialLoading}
      >
        <RefreshCw
          className={cn(
            'h-3.5 w-3.5',
            // Live polling stays quiet; only the first load spins.
            initialLoading && 'animate-spin motion-reduce:animate-none',
          )}
        />
      </Button>
    </>
  )

  const describe = `Request timeline, ${RANGES[range].description}, per ${RANGES[range].bucket}. ${
    total === null
      ? 'Request counts unavailable.'
      : `${total} finished requests (${totals?.llm_requests} LLM, ${totals?.mcp_requests} MCP), ${errors} ${errors === 1 ? 'error' : 'errors'}${
          errorBuckets ? ` in ${errorBuckets} ${errorBuckets === 1 ? 'interval' : 'intervals'}` : ''
        }.`
  } ${inFlight} in progress now.`

  return (
    <CollapsibleSection
      ariaLabel="Request traffic"
      title="Traffic"
      icon={Activity}
      collapsed={collapsed}
      onToggle={toggle}
      summary={summary}
      actions={actions}
      className={cn(PALETTE, className)}
    >
      <div className="px-2 pb-2">
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
                margin={{ top: CHART_TOP, right: 16, left: 0, bottom: 0 }}
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
                  isAnimationActive={false}
                />
                <Line
                  name="Requests"
                  dataKey="requests"
                  type="monotone"
                  stroke={DONE}
                  strokeWidth={2}
                  strokeLinejoin="round"
                  strokeLinecap="round"
                  dot={false}
                  activeDot={(props: { cx?: number; cy?: number; payload?: ChartPoint }) => (
                    <circle
                      key="requests-active"
                      cx={props.cx}
                      cy={props.cy}
                      r={4}
                      strokeWidth={2}
                      stroke={props.payload?.live ? DONE : SURFACE}
                      fill={props.payload?.live ? SURFACE : DONE}
                    />
                  )}
                  shape={<RequestsCurve splitIndex={splitIndex} />}
                  isAnimationActive={false}
                />
                {/*
                  Errors over the requests line, drawn the same way: each run
                  of errors rises from zero and falls back to it; no line where
                  there are none.
                */}
                <Line
                  name="Errors"
                  dataKey={errorPeak}
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
                  shape={<ErrorsCurve />}
                  isAnimationActive={false}
                >
                  {errorBuckets > 0 && errorBuckets <= MAX_LABELLED_ERROR_BUCKETS && (
                    <LabelList dataKey="errors" content={ErrorCount} />
                  )}
                </Line>
              </ComposedChart>
            </ResponsiveContainer>
          ) : (
            <div className="flex h-full items-center justify-center gap-2 text-xs text-muted-foreground">
              <Activity className="h-4 w-4 opacity-50" />
              {unavailable
                ? 'Request traffic is unavailable · retry with refresh'
                : 'No traffic in this period'}
            </div>
          )}
          {total === 0 && inFlight === 0 && points.length > 0 && (
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
