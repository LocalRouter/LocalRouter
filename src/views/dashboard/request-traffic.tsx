import { useEffect, useId, useState } from 'react'
import {
  Bar,
  BarChart,
  ResponsiveContainer,
  Tooltip,
  XAxis,
  YAxis,
} from 'recharts'
import { Activity, RefreshCw } from 'lucide-react'
import { Button } from '@/components/ui/Button'
import { cn } from '@/lib/utils'
import type { GraphData, TimeRange } from '@/types/tauri-commands'
import {
  graphTotal,
  liveTimeline,
  RANGES,
  requestTimeline,
  type TrafficPoint,
} from './activity-data'

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

const LLM_COLOR = '#14b8a6'
const MCP_COLOR = '#a78bfa'
const LIVE_COLOR = '#f59e0b'

interface RequestTrafficProps {
  range: TimeRange
  onRangeChange: (range: TimeRange) => void
  metrics: {
    llm: GraphData | null
    mcp: GraphData | null
    cost: GraphData | null
  } | null
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

function TrafficTooltip({
  active,
  payload,
  label,
}: {
  active?: boolean
  payload?: { payload: TrafficPoint }[]
  label?: number
}) {
  const point = payload?.[0]?.payload
  if (!active || !point) return null
  return (
    <div className="rounded-lg border bg-popover px-2.5 py-1.5 text-xs text-popover-foreground shadow-md">
      <div className="mb-1 text-muted-foreground">
        {new Date(Number(label)).toLocaleString()}
      </div>
      <div className="flex items-center gap-1.5">
        <span className="h-2 w-2 rounded-sm" style={{ background: LLM_COLOR }} />
        LLM <strong className="ml-auto pl-3 tabular-nums">{point.llm ?? '—'}</strong>
      </div>
      <div className="flex items-center gap-1.5">
        <span className="h-2 w-2 rounded-sm" style={{ background: MCP_COLOR }} />
        MCP <strong className="ml-auto pl-3 tabular-nums">{point.mcp ?? '—'}</strong>
      </div>
      {point.inFlight != null && (
        <div className="flex items-center gap-1.5 text-amber-600 dark:text-amber-400">
          <span className="h-2 w-2 rounded-sm" style={{ background: LIVE_COLOR }} />
          In progress <strong className="ml-auto pl-3 tabular-nums">{point.inFlight}</strong>
        </div>
      )}
    </div>
  )
}

export function RequestTraffic({
  range,
  onRangeChange,
  metrics,
  loading,
  inFlight,
  onRefresh,
  className,
}: RequestTrafficProps) {
  const id = useId().replace(/:/g, '')
  const now = useNow(2_000)
  const points = liveTimeline(
    requestTimeline(metrics?.llm ?? null, metrics?.mcp ?? null),
    RANGES[range].bucketMs,
    now,
    inFlight,
  )
  const peak = Math.max(
    2,
    ...points.map((p) => (p.llm ?? 0) + (p.mcp ?? 0) + (p.inFlight ?? 0)),
  )
  const yTicks = [...new Set([0, Math.ceil(peak / 2), peak])]
  const llm = graphTotal(metrics?.llm ?? null)
  const mcp = graphTotal(metrics?.mcp ?? null)
  const cost = graphTotal(metrics?.cost ?? null)
  const total = llm !== null && mcp !== null ? llm + mcp : null
  const initialLoading = loading && !metrics
  const unavailable = !initialLoading && (llm === null || mcp === null)
  const formatTime = (value: number) =>
    new Date(value).toLocaleString(
      undefined,
      range === 'ten_minutes' || range === 'hour' || range === 'day'
        ? { hour: '2-digit', minute: '2-digit' }
        : {
            month: 'short',
            day: 'numeric',
            ...(range === 'week' ? { hour: 'numeric' as const } : {}),
          },
    )

  return (
    <section
      className={cn('overflow-hidden rounded-xl border bg-card', className)}
      aria-label="Request traffic"
    >
      <div className="flex flex-wrap items-center gap-x-5 gap-y-2 px-4 pt-3">
        <h2 className="text-sm font-semibold">Traffic</h2>
        <div className="flex flex-wrap items-center gap-x-4 gap-y-1 text-xs text-muted-foreground">
          <span>
            <strong className="text-sm font-semibold text-foreground tabular-nums">
              {initialLoading ? '…' : (total?.toLocaleString() ?? '—')}
            </strong>{' '}
            requests
          </span>
          <span className="flex items-center gap-1.5">
            <span className="h-2 w-2 rounded-sm" style={{ background: LLM_COLOR }} />
            LLM{' '}
            <strong className="font-medium text-foreground tabular-nums">
              {llm?.toLocaleString() ?? '—'}
            </strong>
          </span>
          <span className="flex items-center gap-1.5">
            <span className="h-2 w-2 rounded-sm" style={{ background: MCP_COLOR }} />
            MCP{' '}
            <strong className="font-medium text-foreground tabular-nums">
              {mcp?.toLocaleString() ?? '—'}
            </strong>
          </span>
          <span>
            <strong className="font-medium text-foreground tabular-nums">
              {cost === null ? '—' : money.format(cost)}
            </strong>{' '}
            est. cost
          </span>
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
        </div>
        <div className="ml-auto flex items-center gap-1.5">
          <div
            className="flex gap-0.5 rounded-md bg-muted/80 p-0.5"
            aria-label="Traffic time range"
          >
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
            size="icon"
            className="h-6 w-6 text-muted-foreground"
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
        </div>
      </div>
      <div
        className="relative h-[112px] px-2 pb-2 pt-2"
        role="img"
        aria-label={`Request timeline, ${RANGES[range].description}. ${llm ?? 'Unavailable'} LLM requests and ${mcp ?? 'unavailable'} MCP requests, per ${RANGES[range].bucket}. ${inFlight} in progress.`}
      >
        {initialLoading ? (
          <div className="flex h-full items-center justify-center text-xs text-muted-foreground">
            Loading request traffic…
          </div>
        ) : points.length ? (
          <ResponsiveContainer
            width="100%"
            height="100%"
            minWidth={0}
            initialDimension={{ width: 600, height: 102 }}
          >
            <BarChart
              data={points}
              margin={{ top: 4, right: 6, left: 0, bottom: 0 }}
              barCategoryGap="18%"
              accessibilityLayer
            >
              <defs>
                <pattern
                  id={`${id}-live`}
                  width="6"
                  height="6"
                  patternUnits="userSpaceOnUse"
                  patternTransform="rotate(45)"
                >
                  <rect width="6" height="6" fill={LIVE_COLOR} fillOpacity={0.25} />
                  <rect width="3" height="6" fill={LIVE_COLOR} />
                </pattern>
              </defs>
              <XAxis
                dataKey="timestamp"
                tickFormatter={formatTime}
                minTickGap={40}
                tick={{ fill: 'hsl(var(--muted-foreground))', fontSize: 10 }}
                axisLine={{ stroke: 'hsl(var(--border))' }}
                tickLine={false}
                tickMargin={3}
                height={20}
              />
              <YAxis
                allowDecimals={false}
                tickFormatter={(value) => compact.format(value)}
                tick={{ fill: 'hsl(var(--muted-foreground))', fontSize: 10 }}
                axisLine={false}
                tickLine={false}
                width={28}
                ticks={yTicks}
                domain={[0, peak]}
              />
              <Tooltip
                cursor={{ fill: 'hsl(var(--muted))', opacity: 0.5 }}
                content={<TrafficTooltip />}
              />
              <Bar
                name="LLM"
                dataKey="llm"
                stackId="requests"
                fill={LLM_COLOR}
                isAnimationActive={false}
              />
              <Bar
                name="MCP"
                dataKey="mcp"
                stackId="requests"
                fill={MCP_COLOR}
                isAnimationActive={false}
              />
              <Bar
                name="In progress"
                dataKey="inFlight"
                stackId="requests"
                fill={`url(#${id}-live)`}
                stroke={LIVE_COLOR}
                strokeWidth={1}
                radius={[2, 2, 0, 0]}
                className="motion-safe:animate-pulse"
                isAnimationActive={false}
              />
            </BarChart>
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
          <div className="pointer-events-none absolute inset-0 flex items-center justify-center pb-4 text-xs text-muted-foreground">
            No requests in this period. Your next request will appear here live.
          </div>
        )}
      </div>
    </section>
  )
}
