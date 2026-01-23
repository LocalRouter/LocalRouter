import { useEffect, useState, useCallback, useMemo } from 'react'
import {
  BarChart,
  Bar,
  XAxis,
  YAxis,
  CartesianGrid,
  Tooltip,
  Legend,
  ResponsiveContainer,
  ReferenceLine
} from 'recharts'
import { invoke } from '@tauri-apps/api/core'

// Types
export type TimeRange = 'hour' | 'day' | 'week' | 'month'
export type MetricType = 'tokens' | 'cost' | 'requests' | 'latency' | 'successrate'
export type McpMetricType = 'requests' | 'latency' | 'successrate'
export type CompareType = 'api_keys' | 'providers' | 'models'
export type ChartScope = 'global' | 'api_key' | 'provider' | 'model' | 'strategy' | 'client' | 'server'

interface RateLimitInfo {
  limit_type: string
  value: number
  time_window_seconds: number
}

interface GraphData {
  labels: string[]
  datasets: {
    label: string
    data: number[]
    border_color?: string
    background_color?: string
  }[]
  rate_limits?: RateLimitInfo[]
}

interface MetricOption {
  id: string
  label: string
}

interface UnifiedChartProps {
  /** Chart title */
  title: string

  /** Type of chart - single scope metrics, mcp metrics, or comparison */
  chartType: 'llm' | 'mcp' | 'mcp-methods' | 'comparison'

  /** Scope for single metrics charts */
  scope?: ChartScope

  /** Scope ID for non-global scopes */
  scopeId?: string

  /** Compare type for comparison charts */
  compareType?: CompareType

  /** IDs to compare for comparison charts */
  compareIds?: string[]

  /** Available metric options */
  metricOptions: MetricOption[]

  /** Default metric type */
  defaultMetric?: string

  /** Default time range */
  defaultTimeRange?: TimeRange

  /** External refresh trigger */
  refreshTrigger?: number

  /** Whether to show method breakdown for MCP requests */
  showMethodBreakdown?: boolean

  /** Chart height in pixels */
  height?: number
}

const CHART_COLORS = [
  '#3b82f6', // blue
  '#22c55e', // green
  '#f59e0b', // amber
  '#ef4444', // red
  '#8b5cf6', // purple
  '#06b6d4', // cyan
  '#ec4899', // pink
  '#84cc16', // lime
  '#6366f1', // indigo
  '#f97316', // orange
]

export function UnifiedChart({
  title,
  chartType,
  scope = 'global',
  scopeId,
  compareType,
  compareIds = [],
  metricOptions,
  defaultMetric,
  defaultTimeRange = 'day',
  refreshTrigger = 0,
  showMethodBreakdown = false,
  height = 300,
}: UnifiedChartProps) {
  const [data, setData] = useState<GraphData | null>(null)
  const [loading, setLoading] = useState(true)
  const [error, setError] = useState<string | null>(null)
  const [isDark, setIsDark] = useState(false)
  const [timeRange, setTimeRange] = useState<TimeRange>(defaultTimeRange)
  const [selectedMetric, setSelectedMetric] = useState<string>(
    defaultMetric || metricOptions[0]?.id || 'requests'
  )

  // Detect dark mode
  useEffect(() => {
    const checkDarkMode = () => {
      setIsDark(document.documentElement.classList.contains('dark'))
    }
    checkDarkMode()

    const observer = new MutationObserver(checkDarkMode)
    observer.observe(document.documentElement, {
      attributes: true,
      attributeFilter: ['class']
    })

    return () => observer.disconnect()
  }, [])

  const loadData = useCallback(async () => {
    try {
      setLoading(true)
      setError(null)

      let command = ''
      const args: Record<string, unknown> = { timeRange, metricType: selectedMetric }

      if (chartType === 'comparison') {
        if (!compareType || compareIds.length === 0) {
          setData(null)
          setLoading(false)
          return
        }

        switch (compareType) {
          case 'api_keys':
            command = 'compare_api_keys'
            args.apiKeyIds = compareIds
            break
          case 'providers':
            command = 'compare_providers'
            args.providers = compareIds
            break
          case 'models':
            command = 'compare_models'
            args.models = compareIds
            break
        }
      } else if (chartType === 'mcp' || chartType === 'mcp-methods') {
        if (chartType === 'mcp-methods' && showMethodBreakdown && selectedMetric === 'requests') {
          command = 'get_mcp_method_breakdown'
          args.scope = scopeId ? `${scope}:${scopeId}` : scope
        } else {
          switch (scope) {
            case 'global':
              command = 'get_global_mcp_metrics'
              break
            case 'client':
              command = 'get_client_mcp_metrics'
              args.clientId = scopeId
              break
            case 'server':
              command = 'get_mcp_server_metrics'
              args.serverId = scopeId
              break
          }
        }
      } else {
        // LLM metrics
        switch (scope) {
          case 'global':
            command = 'get_global_metrics'
            break
          case 'api_key':
            command = 'get_api_key_metrics'
            args.apiKeyId = scopeId
            break
          case 'provider':
            command = 'get_provider_metrics'
            args.provider = scopeId
            break
          case 'model':
            command = 'get_model_metrics'
            args.model = scopeId
            break
          case 'strategy':
            command = 'get_strategy_metrics'
            args.strategyId = scopeId
            break
        }
      }

      if (!command) {
        console.error(`[UnifiedChart] No command set for chartType=${chartType}, scope=${scope}`)
        setError(`No command for chartType=${chartType}, scope=${scope}`)
        setLoading(false)
        return
      }
      console.log(`[UnifiedChart] Invoking command: ${command}`, { chartType, scope, scopeId, args })
      const result = await invoke<GraphData>(command, args)
      console.log(`[UnifiedChart] Result for ${command}:`, result)
      setData(result)
    } catch (err) {
      console.error('Failed to load chart data:', err)
      setError(err instanceof Error ? err.message : 'Failed to load data')
    } finally {
      setLoading(false)
    }
  }, [chartType, scope, scopeId, compareType, compareIds, timeRange, selectedMetric, showMethodBreakdown])

  useEffect(() => {
    loadData()
  }, [loadData, refreshTrigger])

  // Generate timeline with gaps filled
  const chartData = useMemo(() => {
    if (!data || data.labels.length === 0) return []

    // Parse all timestamps
    const parsedPoints = data.labels.map((label, index) => {
      const timestamp = new Date(label).getTime()
      const point: Record<string, unknown> = {
        timestamp,
        timeLabel: label,
      }
      data.datasets.forEach(dataset => {
        point[dataset.label] = dataset.data[index] || 0
      })
      return point
    })

    // Sort by timestamp
    parsedPoints.sort((a, b) => (a.timestamp as number) - (b.timestamp as number))

    // If we have sparse data, we should fill gaps
    // But only for ranges where it makes sense (hour/day should have more granularity)
    if (parsedPoints.length < 2) {
      return parsedPoints
    }

    return parsedPoints
  }, [data])

  // Calculate proper time domain for X-axis
  const timeDomain = useMemo(() => {
    if (chartData.length === 0) return ['auto', 'auto']

    const now = Date.now()
    let rangeMs: number

    switch (timeRange) {
      case 'hour':
        rangeMs = 60 * 60 * 1000
        break
      case 'day':
        rangeMs = 24 * 60 * 60 * 1000
        break
      case 'week':
        rangeMs = 7 * 24 * 60 * 60 * 1000
        break
      case 'month':
        rangeMs = 30 * 24 * 60 * 60 * 1000
        break
      default:
        rangeMs = 24 * 60 * 60 * 1000
    }

    return [now - rangeMs, now]
  }, [timeRange, chartData])

  // Format tick values for X-axis
  const formatXAxis = (tickItem: number) => {
    const date = new Date(tickItem)
    switch (timeRange) {
      case 'hour':
        return date.toLocaleTimeString('en-US', { hour: 'numeric', minute: '2-digit' })
      case 'day':
        return date.toLocaleTimeString('en-US', { hour: 'numeric', minute: '2-digit' })
      case 'week':
        return date.toLocaleDateString('en-US', { weekday: 'short', day: 'numeric' })
      case 'month':
        return date.toLocaleDateString('en-US', { month: 'short', day: 'numeric' })
      default:
        return date.toLocaleString()
    }
  }

  // Format tooltip timestamp
  const formatTooltipLabel = (timestamp: number) => {
    const date = new Date(timestamp)
    return date.toLocaleString('en-US', {
      month: 'short',
      day: 'numeric',
      hour: 'numeric',
      minute: '2-digit',
      hour12: true
    })
  }

  const renderContent = () => {
    if (loading) {
      return (
        <div className="flex items-center justify-center h-64 bg-gray-50 dark:bg-gray-700 rounded-lg">
          <p className="text-gray-500 dark:text-gray-400">Loading...</p>
        </div>
      )
    }

    if (error) {
      return (
        <div className="flex items-center justify-center h-64 bg-red-50 dark:bg-red-900/30 rounded-lg">
          <p className="text-red-600 dark:text-red-400">{error}</p>
        </div>
      )
    }

    const hasData = data && data.datasets.length > 0 && chartData.length > 0
    const isStacked = chartType === 'comparison' || (chartType === 'mcp-methods' && showMethodBreakdown)

    return (
      <ResponsiveContainer width="100%" height={height}>
        <BarChart data={hasData ? chartData : []} margin={{ top: 10, right: 10, left: 0, bottom: 60 }}>
          <CartesianGrid strokeDasharray="3 3" stroke={isDark ? '#4b5563' : '#e5e7eb'} />
          <XAxis
            dataKey="timestamp"
            type="number"
            domain={timeDomain as [number, number]}
            tickFormatter={formatXAxis}
            tick={{ fontSize: 11, fill: isDark ? '#9ca3af' : '#6b7280' }}
            angle={-45}
            textAnchor="end"
            height={60}
            scale="time"
          />
          <YAxis
            tick={{ fontSize: 11, fill: isDark ? '#9ca3af' : '#6b7280' }}
            width={50}
            domain={hasData ? ['auto', 'auto'] : [0, 10]}
          />
          <Tooltip
            labelFormatter={formatTooltipLabel}
            contentStyle={{
              backgroundColor: isDark ? '#1f2937' : 'rgba(255, 255, 255, 0.95)',
              border: `1px solid ${isDark ? '#374151' : '#e5e7eb'}`,
              borderRadius: '0.5rem',
              fontSize: '12px',
              color: isDark ? '#f3f4f6' : '#111827'
            }}
          />
          {hasData && (
            <Legend
              wrapperStyle={{ fontSize: '12px', color: isDark ? '#f3f4f6' : '#111827' }}
              verticalAlign="top"
              height={36}
            />
          )}
          {hasData && data?.datasets.map((dataset, i) => (
            <Bar
              key={i}
              dataKey={dataset.label}
              stackId={isStacked ? '1' : undefined}
              fill={dataset.border_color || CHART_COLORS[i % CHART_COLORS.length]}
              animationDuration={200}
            />
          ))}
          {hasData && data?.rate_limits?.map((limit, i) => (
            <ReferenceLine
              key={`limit-${i}`}
              y={limit.value}
              stroke="#ef4444"
              strokeDasharray="5 5"
              strokeWidth={2}
              label={{
                value: `${limit.limit_type} Limit: ${limit.value}`,
                position: 'insideTopRight',
                fill: '#ef4444',
                fontSize: 10
              }}
            />
          ))}
        </BarChart>
      </ResponsiveContainer>
    )
  }

  return (
    <div className="bg-white dark:bg-gray-800 rounded-lg shadow p-6">
      {/* Header */}
      <div className="flex items-center justify-between mb-4">
        <h2 className="text-xl font-semibold text-gray-900 dark:text-gray-100">{title}</h2>
        <div className="flex items-center gap-3">
          <select
            value={timeRange}
            onChange={(e) => setTimeRange(e.target.value as TimeRange)}
            className="px-3 py-1.5 border border-gray-300 dark:border-gray-600 rounded-lg bg-white dark:bg-gray-700 text-gray-900 dark:text-gray-100 hover:bg-gray-50 dark:hover:bg-gray-600 focus:outline-none focus:ring-2 focus:ring-blue-500 text-sm"
          >
            <option value="hour">Last Hour</option>
            <option value="day">Last 24 Hours</option>
            <option value="week">Last 7 Days</option>
            <option value="month">Last 30 Days</option>
          </select>
          <button
            onClick={loadData}
            disabled={loading}
            className="px-3 py-1.5 text-sm bg-blue-500 hover:bg-blue-600 disabled:bg-gray-400 dark:disabled:bg-gray-600 text-white rounded-lg transition-colors"
            title="Refresh"
          >
            {loading ? 'Loading...' : 'Refresh'}
          </button>
        </div>
      </div>

      {/* Metric Type Tabs */}
      <div className="flex gap-2 mb-4 flex-wrap">
        {metricOptions.map((metric) => (
          <button
            key={metric.id}
            onClick={() => setSelectedMetric(metric.id)}
            className={`px-3 py-1.5 text-sm rounded-md transition-colors ${
              selectedMetric === metric.id
                ? 'bg-blue-600 text-white'
                : 'bg-gray-100 dark:bg-gray-700 text-gray-700 dark:text-gray-300 hover:bg-gray-200 dark:hover:bg-gray-600'
            }`}
          >
            {metric.label}
          </button>
        ))}
      </div>

      {/* Chart */}
      {renderContent()}
    </div>
  )
}

export default UnifiedChart
