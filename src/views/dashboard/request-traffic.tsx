import { useId } from 'react'
import {
  Area,
  AreaChart,
  CartesianGrid,
  ResponsiveContainer,
  Tooltip,
  XAxis,
  YAxis,
} from 'recharts'
import { Activity, ArrowUpRight, RefreshCw } from 'lucide-react'
import { Button } from '@/components/ui/Button'
import { cn } from '@/lib/utils'
import type { GraphData, TimeRange } from '@/types/tauri-commands'
import { graphTotal, RANGES, requestTimeline } from './activity-data'

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

interface RequestTrafficProps {
  range: TimeRange
  onRangeChange: (range: TimeRange) => void
  metrics: {
    llm: GraphData | null
    mcp: GraphData | null
    cost: GraphData | null
  } | null
  loading: boolean
  onRefresh: () => void
}

export function RequestTraffic({
  range,
  onRangeChange,
  metrics,
  loading,
  onRefresh,
}: RequestTrafficProps) {
  const id = useId().replace(/:/g, '')
  const points = requestTimeline(metrics?.llm ?? null, metrics?.mcp ?? null)
  const llm = graphTotal(metrics?.llm ?? null)
  const mcp = graphTotal(metrics?.mcp ?? null)
  const cost = graphTotal(metrics?.cost ?? null)
  const total = llm !== null && mcp !== null ? llm + mcp : null
  const initialLoading = loading && !metrics
  const unavailable = !initialLoading && (llm === null || mcp === null)
  const formatTime = (value: number) =>
    new Date(value).toLocaleString(
      undefined,
      range === 'hour' || range === 'day'
        ? { hour: '2-digit', minute: '2-digit' }
        : {
            month: 'short',
            day: 'numeric',
            ...(range === 'week' ? { hour: 'numeric' as const } : {}),
          },
    )

  return (
    <section
      className="overflow-hidden rounded-2xl border bg-card"
      aria-label="Request traffic"
    >
      <div className="flex flex-wrap items-center justify-between gap-3 px-6 pt-5">
        <h2 className="text-sm font-semibold">Request traffic</h2>
        <div className="flex items-center gap-2">
          <div
            className="flex gap-1 rounded-lg bg-muted/80 p-1"
            aria-label="Traffic time range"
          >
            {(Object.keys(RANGES) as TimeRange[]).map((value) => (
              <button
                key={value}
                aria-pressed={range === value}
                aria-label={RANGES[value].description}
                onClick={() => onRangeChange(value)}
                className={cn(
                  'rounded-md px-3 py-1 text-xs font-medium transition-colors focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring',
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
            className="h-7 w-7 text-muted-foreground"
            onClick={onRefresh}
            aria-label="Refresh activity"
            disabled={loading}
          >
            <RefreshCw
              className={cn(
                'h-3.5 w-3.5',
                loading && 'animate-spin motion-reduce:animate-none',
              )}
            />
          </Button>
        </div>
      </div>
      <div className="flex flex-wrap items-end justify-between gap-4 px-6 pb-5 pt-4">
        <div>
          <div className="flex items-baseline gap-2.5">
            <span className="text-[38px] font-semibold leading-none tracking-tight tabular-nums">
              {initialLoading ? '…' : (total?.toLocaleString() ?? '—')}
            </span>
            <span className="text-sm text-muted-foreground">requests</span>
          </div>
          <p className="mt-2 text-xs text-muted-foreground">
            {RANGES[range].description}{' '}
            <span className="mx-1.5 opacity-50">/</span> per{' '}
            {RANGES[range].bucket}
          </p>
        </div>
        <div className="flex gap-5 pb-1 text-xs">
          <span className="flex items-center gap-2">
            <span className="h-2 w-2 rounded-full bg-teal-500" />
            LLM{' '}
            <strong className="font-medium tabular-nums">
              {llm?.toLocaleString() ?? '—'}
            </strong>
          </span>
          <span className="flex items-center gap-2">
            <span className="h-2 w-2 rounded-full bg-violet-400" />
            MCP{' '}
            <strong className="font-medium tabular-nums">
              {mcp?.toLocaleString() ?? '—'}
            </strong>
          </span>
        </div>
      </div>
      <div
        className="relative h-[190px] px-3"
        role="img"
        aria-label={`Request timeline, ${RANGES[range].description}. ${llm ?? 'Unavailable'} LLM requests and ${mcp ?? 'unavailable'} MCP requests, per ${RANGES[range].bucket}.`}
      >
        {initialLoading ? (
          <div className="flex h-full items-center justify-center text-sm text-muted-foreground">
            Loading request traffic…
          </div>
        ) : points.length ? (
          <ResponsiveContainer
            width="100%"
            height="100%"
            minWidth={0}
            initialDimension={{ width: 600, height: 190 }}
          >
            <AreaChart
              data={points}
              margin={{ top: 12, right: 12, left: -15, bottom: 0 }}
              accessibilityLayer
            >
              <defs>
                <linearGradient id={`${id}-llm`} x1="0" y1="0" x2="0" y2="1">
                  <stop offset="0%" stopColor="#14b8a6" stopOpacity={0.2} />
                  <stop offset="100%" stopColor="#14b8a6" stopOpacity={0.01} />
                </linearGradient>
                <linearGradient id={`${id}-mcp`} x1="0" y1="0" x2="0" y2="1">
                  <stop offset="0%" stopColor="#a78bfa" stopOpacity={0.12} />
                  <stop offset="100%" stopColor="#a78bfa" stopOpacity={0} />
                </linearGradient>
              </defs>
              <CartesianGrid
                vertical={false}
                stroke="hsl(var(--border))"
                strokeDasharray="3 5"
              />
              <XAxis
                dataKey="timestamp"
                type="number"
                domain={['dataMin', 'dataMax']}
                scale="time"
                tickFormatter={formatTime}
                minTickGap={45}
                tick={{ fill: 'hsl(var(--muted-foreground))', fontSize: 10 }}
                axisLine={false}
                tickLine={false}
                tickMargin={12}
              />
              <YAxis
                allowDecimals={false}
                tickFormatter={(value) => compact.format(value)}
                tick={{ fill: 'hsl(var(--muted-foreground))', fontSize: 10 }}
                axisLine={false}
                tickLine={false}
                domain={[0, (max: number) => Math.max(4, max)]}
              />
              <Tooltip
                labelFormatter={(value) =>
                  new Date(Number(value)).toLocaleString()
                }
                formatter={(value, name) => [
                  Number(value).toLocaleString(),
                  `${name} requests`,
                ]}
                contentStyle={{
                  backgroundColor: 'hsl(var(--popover))',
                  border: '1px solid hsl(var(--border))',
                  borderRadius: 10,
                  fontSize: 12,
                  color: 'hsl(var(--foreground))',
                }}
              />
              <Area
                name="LLM"
                type="linear"
                dataKey="llm"
                stroke="#14b8a6"
                strokeWidth={2.5}
                fill={`url(#${id}-llm)`}
                isAnimationActive={false}
                dot={false}
                activeDot={{ r: 4, strokeWidth: 2, stroke: 'hsl(var(--card))' }}
              />
              <Area
                name="MCP"
                type="linear"
                dataKey="mcp"
                stroke="#a78bfa"
                strokeWidth={2}
                fill={`url(#${id}-mcp)`}
                isAnimationActive={false}
                dot={false}
                activeDot={{ r: 4, strokeWidth: 2, stroke: 'hsl(var(--card))' }}
              />
            </AreaChart>
          </ResponsiveContainer>
        ) : (
          <div className="flex h-full flex-col items-center justify-center gap-2 text-sm text-muted-foreground">
            <Activity className="h-6 w-6 opacity-50" />
            {unavailable
              ? 'Request traffic is unavailable'
              : 'No traffic in this period'}
          </div>
        )}
        {total === 0 && points.length > 0 && (
          <div className="pointer-events-none absolute inset-0 flex items-center justify-center pb-5 text-sm text-muted-foreground">
            No requests in this period. Your next request will appear here.
          </div>
        )}
      </div>
      <div className="mt-3 flex flex-wrap items-center justify-between gap-2 border-t bg-muted/20 px-6 py-3 text-xs text-muted-foreground">
        <span className="flex items-center gap-1.5">
          <ArrowUpRight className="h-3.5 w-3.5" />
          <span className="font-medium text-foreground">
            {cost === null ? '—' : money.format(cost)}
          </span>{' '}
          estimated LLM cost
        </span>
        <span>
          {unavailable
            ? 'Some metrics unavailable · retry with refresh'
            : 'Local time · latest bucket still collecting'}
        </span>
      </div>
    </section>
  )
}
