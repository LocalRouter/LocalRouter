import { useEffect, useState } from 'react'
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

interface StackedAreaChartProps {
  compareType: 'api_keys' | 'providers' | 'models'
  ids: string[]
  timeRange: 'hour' | 'day' | 'week' | 'month'
  metricType: 'cost' | 'tokens' | 'requests'
  title?: string
  refreshTrigger?: number
}

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

export function StackedAreaChart({
  compareType,
  ids,
  timeRange,
  metricType,
  title,
  refreshTrigger = 0
}: StackedAreaChartProps) {
  const [data, setData] = useState<GraphData | null>(null)
  const [loading, setLoading] = useState(true)
  const [error, setError] = useState<string | null>(null)
  const [isDark, setIsDark] = useState(false)

  // Detect dark mode
  useEffect(() => {
    const checkDarkMode = () => {
      setIsDark(document.documentElement.classList.contains('dark'))
    }
    checkDarkMode()

    // Watch for theme changes
    const observer = new MutationObserver(checkDarkMode)
    observer.observe(document.documentElement, {
      attributes: true,
      attributeFilter: ['class']
    })

    return () => observer.disconnect()
  }, [])

  useEffect(() => {
    const loadData = async () => {
      try {
        setLoading(true)
        setError(null)

        // Determine command and arg name based on compareType
        let command: string
        let argName: string

        if (compareType === 'api_keys') {
          command = 'compare_api_keys'
          argName = 'apiKeyIds'
        } else if (compareType === 'providers') {
          command = 'compare_providers'
          argName = 'providers'
        } else {
          command = 'compare_models'
          argName = 'models'
        }

        const args = {
          [argName]: ids,
          timeRange: timeRange,
          metricType: metricType,
        }

        const result = await invoke<GraphData>(command, args)
        setData(result)
      } catch (err) {
        console.error('Failed to load comparison data:', err)
        setError(err instanceof Error ? err.message : 'Failed to load comparison data')
      } finally {
        setLoading(false)
      }
    }

    if (ids.length > 0) {
      loadData()
    }
  }, [compareType, ids, timeRange, metricType, refreshTrigger])

  if (loading) {
    return (
      <div className="h-64 bg-gray-50 dark:bg-gray-700 rounded-lg flex items-center justify-center">
        <p className="text-gray-500 dark:text-gray-400">Loading...</p>
      </div>
    )
  }

  if (error) {
    return (
      <div className="h-64 bg-red-50 dark:bg-red-900/30 rounded-lg flex items-center justify-center">
        <p className="text-red-600 dark:text-red-400">{error}</p>
      </div>
    )
  }

  if (!data || data.datasets.length === 0) {
    return (
      <div className="h-64 bg-gray-50 dark:bg-gray-700 rounded-lg flex items-center justify-center">
        <p className="text-gray-500 dark:text-gray-400">No data available</p>
      </div>
    )
  }

  // Format tick values for X-axis
  const formatXAxis = (tickItem: number) => {
    const date = new Date(tickItem)
    switch (timeRange) {
      case 'hour':
        return date.toLocaleTimeString('en-US', { hour: 'numeric', minute: '2-digit' })
      case 'day':
        return date.toLocaleTimeString('en-US', { hour: 'numeric', minute: '2-digit' })
      case 'week':
        return date.toLocaleDateString('en-US', { weekday: 'short', hour: 'numeric' })
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

  // Transform to Recharts format with parsed timestamps
  const chartData = data.labels.map((label, index) => {
    // Parse timestamp string (format: "YYYY-MM-DD HH:MM")
    const timestamp = new Date(label).getTime()
    const point: any = {
      timestamp: timestamp,
      timeLabel: label
    }
    data.datasets.forEach((dataset) => {
      point[dataset.label] = dataset.data[index] || 0
    })
    return point
  })

  const colors = [
    '#8884d8', '#82ca9d', '#ffc658', '#ff7c7c', '#a28bd9',
    '#f59e42', '#6dd5ed', '#ff6b9d', '#c471ed', '#12c2e9'
  ]

  return (
    <div className="bg-white dark:bg-gray-800 p-4 rounded-lg shadow">
      <div className="flex justify-between items-center mb-4">
        {title && <h3 className="text-lg font-semibold text-gray-900 dark:text-gray-100">{title}</h3>}
        <button
          onClick={() => {
            if (ids.length > 0) {
              const loadData = async () => {
                try {
                  setLoading(true)
                  setError(null)

                  let command: string
                  let argName: string

                  if (compareType === 'api_keys') {
                    command = 'compare_api_keys'
                    argName = 'apiKeyIds'
                  } else if (compareType === 'providers') {
                    command = 'compare_providers'
                    argName = 'providers'
                  } else {
                    command = 'compare_models'
                    argName = 'models'
                  }

                  const args = {
                    [argName]: ids,
                    timeRange: timeRange,
                    metricType: metricType,
                  }

                  const result = await invoke<GraphData>(command, args)
                  setData(result)
                } catch (err) {
                  console.error('Failed to load comparison data:', err)
                  setError(err instanceof Error ? err.message : 'Failed to load comparison data')
                } finally {
                  setLoading(false)
                }
              }
              loadData()
            }
          }}
          disabled={loading || ids.length === 0}
          className="px-3 py-1 text-sm bg-blue-500 hover:bg-blue-600 disabled:bg-gray-400 dark:disabled:bg-gray-600 text-white rounded transition-colors"
          title="Refresh metrics"
        >
          {loading ? 'Refreshing...' : 'Refresh'}
        </button>
      </div>

      <ResponsiveContainer width="100%" height={400}>
        <BarChart data={chartData}>
          <CartesianGrid strokeDasharray="3 3" stroke={isDark ? '#4b5563' : '#e5e7eb'} />
          <XAxis
            dataKey="timestamp"
            type="number"
            domain={['dataMin', 'dataMax']}
            tickFormatter={formatXAxis}
            tick={{ fontSize: 11, fill: isDark ? '#9ca3af' : '#6b7280' }}
            angle={-45}
            textAnchor="end"
            height={70}
            scale="time"
          />
          <YAxis tick={{ fontSize: 11, fill: isDark ? '#9ca3af' : '#6b7280' }} />
          <Tooltip
            labelFormatter={formatTooltipLabel}
            contentStyle={{
              backgroundColor: isDark ? '#1f2937' : '#fff',
              border: `1px solid ${isDark ? '#374151' : '#e5e7eb'}`,
              borderRadius: '0.5rem',
              fontSize: '12px',
              color: isDark ? '#f3f4f6' : '#111827'
            }}
          />
          <Legend wrapperStyle={{ fontSize: '12px', color: isDark ? '#f3f4f6' : '#111827' }} />
          {data.datasets.map((dataset, i) => (
            <Bar
              key={i}
              dataKey={dataset.label}
              stackId="1"
              fill={colors[i % colors.length]}
              animationDuration={100}
            />
          ))}
          {/* Render rate limit reference lines if available */}
          {data.rate_limits?.map((limit, i) => (
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
                fontSize: 12
              }}
            />
          ))}
        </BarChart>
      </ResponsiveContainer>
    </div>
  )
}
