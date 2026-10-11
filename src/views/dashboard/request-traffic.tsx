import { useEffect, useState } from 'react'
import {
  Bar,
  CartesianGrid,
  ComposedChart,
  LabelList,
  type LabelProps,
  Line,
  ReferenceDot,
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
  livePoint,
  niceTicks,
  RANGES,
  roundTicks,
  trafficPeak,
  trafficPoints,
  type TrafficPoint,
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
 * finished requests take categorical slot 1 (blue); the requests in progress
 * right now, drawn once at the live edge, the yellow slot (amber, like the
 * header's live indicator; orange sits too close to the error red); errors the
 * reserved "critical" status red, always next to an icon and a label. Light
 * and dark steps are set as CSS variables on the section.
 */
const DONE = 'var(--traffic-done)'
const LIVE = 'var(--traffic-live)'
const ERROR = 'var(--traffic-error)'
const PALETTE =
  '[--traffic-done:#2a78d6] [--traffic-live:#eda100] [--traffic-error:#d03b3b] dark:[--traffic-done:#3987e5] dark:[--traffic-live:#c98500]'

const AXIS_TICK = { fill: 'hsl(var(--muted-foreground))', fontSize: 10 }
const GRID = 'hsl(var(--border))'
const SURFACE = 'hsl(var(--card))'
/** Labels on the plot keep a ring of the card color so lines never cut them. */
const HALO = { paintOrder: 'stroke', stroke: SURFACE, strokeWidth: 3, strokeLinejoin: 'round' } as const
/** Error buckets get a count label while they are few enough to read. */
const MAX_LABELLED_ERROR_BUCKETS = 8
/**
 * Space right of the plot for the "now" label, kept whether or not anything
 * is running so the chart does not shift as requests start and finish.
 */
const NOW_GUTTER = 112

/** Errors of a bucket, or nothing: buckets without errors draw no mark. */
const errorMark = (point: TrafficPoint) => (point.errors ? point.errors : null)

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

function DotKey({ color }: { color: string }) {
  return <span className="h-2 w-2 shrink-0 rounded-full" style={{ background: color }} aria-hidden />
}

function BarKey({ color }: { color: string }) {
  return (
    <span className="flex h-3 w-3 shrink-0 items-end justify-center" aria-hidden>
      <span className="h-2 w-1 rounded-t-sm" style={{ background: color }} />
    </span>
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
  payload?: { payload: TrafficPoint }[]
  range: TimeRange
  bucketMs: number
  inFlight: number
}) {
  const point = payload?.[0]?.payload
  if (!active || !point) return null
  return (
    <div className="min-w-[11rem] rounded-lg border bg-popover px-2.5 py-1.5 text-xs text-popover-foreground shadow-md">
      <div className="mb-1 text-muted-foreground">
        {formatBucket(range, point.timestamp, bucketMs)}
        {point.live ? ' · now' : ''}
      </div>
      {point.completed === null ? (
        <div className="text-muted-foreground">Not recorded at this resolution</div>
      ) : (
        <div className="space-y-0.5">
          <div className="flex items-center gap-1.5">
            <LineKey color={DONE} />
            Finished
            <strong className="ml-auto pl-3 tabular-nums">{point.completed}</strong>
          </div>
          <div className="pl-[1.125rem] text-muted-foreground">
            LLM {point.llm} · MCP {point.mcp}
            {point.cost ? ` · ${money.format(point.cost)}` : ''}
          </div>
          <div className={cn('flex items-center gap-1.5', !point.errors && 'text-muted-foreground')}>
            <CircleAlert
              className="h-3 w-3"
              style={point.errors ? { color: ERROR } : undefined}
              aria-hidden
            />
            Errors
            <strong className="ml-auto pl-3 tabular-nums">{point.errors}</strong>
          </div>
          {point.live && (
            <div className="flex items-center gap-1.5">
              <DotKey color={LIVE} />
              In progress now
              <strong className="ml-auto pl-3 tabular-nums">{inFlight}</strong>
            </div>
          )}
        </div>
      )}
    </div>
  )
}

/** Count above an error bar. */
function ErrorCount({ x, y, width, value }: LabelProps) {
  if (!value || x === undefined || y === undefined) return null
  return (
    <text
      x={Number(x) + Number(width ?? 0) / 2}
      y={Number(y) - 4}
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
 * The requests in progress right now, at the live bucket: an amber dot at
 * their count on the shared axis, labelled "N in progress now" in the gutter
 * right of the plot, where no line can cross the label. With nothing running
 * there is no dot, only a muted "Idle now" at the baseline.
 */
function NowMarker({ cx, cy, count }: { cx?: number; cy?: number; count: number }) {
  if (cx === undefined || cy === undefined) return <g />
  const label = (
    <text
      x={cx + 10}
      y={cy}
      dy="0.35em"
      fontSize={11}
      fontWeight={count ? 600 : 400}
      fill={count ? 'hsl(var(--foreground))' : 'hsl(var(--muted-foreground))'}
    >
      {count ? `${count} in progress now` : 'Idle now'}
    </text>
  )
  if (!count) return <g data-testid="traffic-now">{label}</g>
  return (
    <g data-testid="traffic-now">
      <circle
        cx={cx}
        cy={cy}
        r={5}
        fill={LIVE}
        className="animate-ping opacity-60 [transform-box:fill-box] [transform-origin:center] motion-reduce:hidden"
      />
      <circle cx={cx} cy={cy} r={5} fill={LIVE} stroke={SURFACE} strokeWidth={2} />
      {label}
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
  const showNow = live !== null && inFlight > 0
  const bucketMs = (timeline?.bucket_secs ?? 0) * 1000
  const yTicks = niceTicks(trafficPeak(points, inFlight))
  const xTicks = roundTicks(range, points.map((p) => p.timestamp), bucketMs)
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
        <div
          className="flex items-center gap-4 px-2 pb-1 text-[11px] text-muted-foreground"
          data-testid="traffic-legend"
          aria-hidden
        >
          <span className="flex items-center gap-1.5">
            <LineKey color={DONE} /> Finished per {RANGES[range].bucket}
          </span>
          {errorBuckets > 0 && (
            <span className="flex items-center gap-1.5">
              <BarKey color={ERROR} /> Errors
            </span>
          )}
          {showNow && (
            <span className="flex items-center gap-1.5">
              <DotKey color={LIVE} /> In progress now
            </span>
          )}
        </div>
        <div className="relative h-[156px]" role="img" aria-label={describe}>
          {initialLoading ? (
            <div className="flex h-full items-center justify-center text-xs text-muted-foreground">
              Loading request traffic…
            </div>
          ) : points.length ? (
            <ResponsiveContainer
              width="100%"
              height="100%"
              minWidth={0}
              initialDimension={{ width: 600, height: 156 }}
            >
              <ComposedChart
                data={points}
                margin={{ top: 14, right: NOW_GUTTER, left: 0, bottom: 0 }}
                barCategoryGap={0}
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
                  height={18}
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
                  name="Finished"
                  dataKey="completed"
                  type="monotone"
                  stroke={DONE}
                  strokeWidth={2}
                  strokeLinejoin="round"
                  strokeLinecap="round"
                  dot={false}
                  activeDot={{ r: 4, strokeWidth: 2, stroke: SURFACE, fill: DONE }}
                  isAnimationActive={false}
                />
                {/*
                  Errors over the line, so a line never hides one. Always
                  mounted (buckets without errors draw nothing) so the time
                  axis keeps the same layout whether or not any errors exist.
                */}
                <Bar
                  name="Errors"
                  dataKey={errorMark}
                  fill={ERROR}
                  barSize={8}
                  radius={[3, 3, 0, 0]}
                  minPointSize={(value) => (value ? 8 : 0)}
                  isAnimationActive={false}
                >
                  {errorBuckets > 0 && errorBuckets <= MAX_LABELLED_ERROR_BUCKETS && (
                    <LabelList dataKey={errorMark} content={ErrorCount} />
                  )}
                </Bar>
                {live && (
                  <ReferenceDot
                    x={live.timestamp}
                    y={inFlight}
                    r={5}
                    shape={(props) => (
                      <NowMarker cx={props.cx} cy={props.cy} count={inFlight} />
                    )}
                  />
                )}
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
