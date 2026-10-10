import { useEffect, useId, useState } from 'react'
import {
  CartesianGrid,
  Line,
  LineChart,
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
 * finished requests take categorical slot 1 (blue), requests in progress the
 * yellow slot (amber, like the live indicator; orange sits too close to the
 * error red), and errors the reserved "critical" status red, always next to
 * an icon and a label. Light and dark steps are set as CSS variables on the
 * section.
 */
const DONE = 'var(--traffic-done)'
const LIVE = 'var(--traffic-live)'
const ERROR = 'var(--traffic-error)'
const PALETTE =
  '[--traffic-done:#2a78d6] [--traffic-live:#eda100] [--traffic-error:#d03b3b] dark:[--traffic-done:#3987e5] dark:[--traffic-live:#c98500]'

const AXIS_TICK = { fill: 'hsl(var(--muted-foreground))', fontSize: 10 }
const GRID = 'hsl(var(--border))'

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

function TrafficTooltip({
  active,
  payload,
  range,
  bucketMs,
}: {
  active?: boolean
  payload?: { payload: TrafficPoint }[]
  range: TimeRange
  bucketMs: number
}) {
  const point = payload?.[0]?.payload
  if (!active || !point) return null
  return (
    <div className="min-w-[11rem] rounded-lg border bg-popover px-2.5 py-1.5 text-xs text-popover-foreground shadow-md">
      <div className="mb-1 text-muted-foreground">
        {formatBucket(range, point.timestamp, bucketMs)}
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
          <div className="flex items-center gap-1.5">
            <LineKey color={LIVE} />
            Most in progress
            <strong className="ml-auto pl-3 tabular-nums">{point.inFlight ?? 0}</strong>
          </div>
          <div className="flex items-center gap-1.5">
            <CircleAlert className="h-3 w-3" style={{ color: ERROR }} aria-hidden />
            Errors
            <strong className="ml-auto pl-3 tabular-nums">{point.errors}</strong>
          </div>
        </div>
      )}
    </div>
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
  const id = useId().replace(/:/g, '')
  const [collapsed, toggle] = useCollapsed(COLLAPSED_KEY)
  const now = useNow(2_000)
  const points = trafficPoints(timeline ?? null, now, inFlight)
  const bucketMs = (timeline?.bucket_secs ?? 0) * 1000
  const yTicks = niceTicks(trafficPeak(points))
  // The error panel is short: label only zero and its top.
  const errorScale = niceTicks(Math.max(0, ...points.map((p) => p.errors ?? 0)), 1)
  const errorTicks = [0, errorScale[errorScale.length - 1]]
  const xTicks = roundTicks(range, points.map((p) => p.timestamp), bucketMs)
  const totals = timeline?.totals ?? null
  const total = totals ? totals.llm_requests + totals.mcp_requests : null
  const errors = totals ? totals.llm_errors + totals.mcp_errors : null
  const initialLoading = loading && timeline === undefined
  const unavailable = !initialLoading && timeline === null
  const busy = points.filter((p) => p.completed || p.inFlight || p.errors)

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
      : `${total} finished requests (${totals?.llm_requests} LLM, ${totals?.mcp_requests} MCP), ${errors} ${errors === 1 ? 'error' : 'errors'}.`
  } ${inFlight} in progress.`

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
        <div className="flex items-center gap-4 px-2 pb-1 text-[11px] text-muted-foreground" aria-hidden>
          <span className="flex items-center gap-1.5">
            <LineKey color={DONE} /> Finished per {RANGES[range].bucket}
          </span>
          <span className="flex items-center gap-1.5">
            <LineKey color={LIVE} /> In progress
          </span>
          <span className="flex items-center gap-1.5">
            <LineKey color={ERROR} /> Errors
          </span>
        </div>
        <div className="relative h-[156px]" role="img" aria-label={describe}>
          {initialLoading ? (
            <div className="flex h-full items-center justify-center text-xs text-muted-foreground">
              Loading request traffic…
            </div>
          ) : points.length ? (
            <div className="flex h-full flex-col">
              <div className="min-h-0 flex-[3]">
                <ResponsiveContainer
                  width="100%"
                  height="100%"
                  minWidth={0}
                  initialDimension={{ width: 600, height: 92 }}
                >
                  <LineChart
                    data={points}
                    syncId={`${id}-traffic`}
                    margin={{ top: 6, right: 8, left: 0, bottom: 6 }}
                    accessibilityLayer
                  >
                    <CartesianGrid vertical={false} stroke={GRID} strokeWidth={1} />
                    <XAxis dataKey="timestamp" hide />
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
                      content={<TrafficTooltip range={range} bucketMs={bucketMs} />}
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
                      activeDot={{ r: 4, strokeWidth: 2, stroke: 'hsl(var(--card))', fill: DONE }}
                      isAnimationActive={false}
                    />
                    <Line
                      name="In progress"
                      dataKey="inFlight"
                      type="stepAfter"
                      stroke={LIVE}
                      strokeWidth={2}
                      strokeLinejoin="round"
                      strokeLinecap="round"
                      dot={false}
                      activeDot={{ r: 4, strokeWidth: 2, stroke: 'hsl(var(--card))', fill: LIVE }}
                      isAnimationActive={false}
                    />
                  </LineChart>
                </ResponsiveContainer>
              </div>
              <div className="relative mt-1 min-h-0 flex-[2]">
                <span
                  className="pointer-events-none absolute left-9 top-0 z-10 flex items-center gap-1 text-[10px] text-muted-foreground"
                  aria-hidden
                >
                  <CircleAlert className="h-2.5 w-2.5" style={{ color: ERROR }} /> Errors
                </span>
                <ResponsiveContainer
                  width="100%"
                  height="100%"
                  minWidth={0}
                  initialDimension={{ width: 600, height: 44 }}
                >
                  <LineChart
                    data={points}
                    syncId={`${id}-traffic`}
                    margin={{ top: 16, right: 8, left: 0, bottom: 0 }}
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
                      ticks={errorTicks}
                      interval={0}
                      domain={[0, errorTicks[errorTicks.length - 1]]}
                    />
                    <Tooltip
                      cursor={{ stroke: 'hsl(var(--muted-foreground))', strokeWidth: 1 }}
                      content={() => null}
                      isAnimationActive={false}
                    />
                    <Line
                      name="Errors"
                      dataKey="errors"
                      type="monotone"
                      stroke={ERROR}
                      strokeWidth={2}
                      strokeLinejoin="round"
                      strokeLinecap="round"
                      dot={false}
                      activeDot={{ r: 4, strokeWidth: 2, stroke: 'hsl(var(--card))', fill: ERROR }}
                      isAnimationActive={false}
                    />
                  </LineChart>
                </ResponsiveContainer>
              </div>
            </div>
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
                <th scope="col">Peak running at once</th>
                <th scope="col">Errors</th>
              </tr>
            </thead>
            <tbody>
              {busy.map((p) => (
                <tr key={p.timestamp}>
                  <td>{formatBucket(range, p.timestamp, bucketMs)}</td>
                  <td>{p.completed}</td>
                  <td>{p.inFlight ?? 0}</td>
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
