import { useEffect, useState } from 'react'
import { BarChart, Bar, XAxis, YAxis, CartesianGrid, Tooltip, Legend, ResponsiveContainer } from 'recharts'
import { invoke } from '@tauri-apps/api/core'

interface McpMetricsChartProps {
  scope: 'global' | 'client' | 'server'
  scopeId?: string
  timeRange: 'hour' | 'day' | 'week' | 'month'
  metricType: 'requests' | 'latency' | 'successrate'
  title?: string
  refreshTrigger?: number
}

interface GraphData {
  labels: string[]
  datasets: {
    label: string
    data: number[]
    border_color?: string
    background_color?: string
  }[]
}

export function McpMetricsChart({
  scope,
  scopeId,
  timeRange,
  metricType,
  title,
  refreshTrigger = 0
}: McpMetricsChartProps) {
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

  const loadMetrics = async () => {
    try {
      setLoading(true)
      setError(null)

      let command = ''
      const args: any = { timeRange, metricType }

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

      const result = await invoke<GraphData>(command, args)
      setData(result)
    } catch (err) {
      console.error('Failed to load MCP metrics:', err)
      setError(err instanceof Error ? err.message : 'Failed to load MCP metrics')
    } finally {
      setLoading(false)
    }
  }

  useEffect(() => {
    loadMetrics()
  }, [scope, scopeId, timeRange, metricType, refreshTrigger])

  if (loading) {
    return (
      <div className="flex items-center justify-center h-64 bg-gray-50 dark:bg-gray-700 rounded-lg">
        <p className="text-gray-500 dark:text-gray-400">Loading MCP metrics...</p>
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

  if (!data || data.datasets.length === 0) {
    return (
      <div className="flex items-center justify-center h-64 bg-gray-50 dark:bg-gray-700 rounded-lg">
        <p className="text-gray-500 dark:text-gray-400">No MCP requests yet</p>
      </div>
    )
  }

  // Transform GraphData to Recharts format with parsed timestamps
  const chartData = data.labels.map((label, index) => {
    // Parse timestamp string (format: "YYYY-MM-DD HH:MM")
    const timestamp = new Date(label).getTime()
    const point: any = {
      timestamp: timestamp,
      timeLabel: label
    }
    data.datasets.forEach(dataset => {
      point[dataset.label] = dataset.data[index] || 0
    })
    return point
  })

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

  return (
    <div className="bg-white dark:bg-gray-800 p-4 rounded-lg shadow">
      <div className="flex justify-between items-center mb-4">
        {title && <h3 className="text-lg font-semibold text-gray-900 dark:text-gray-100">{title}</h3>}
        <button
          onClick={loadMetrics}
          disabled={loading}
          className="px-3 py-1 text-sm bg-blue-500 hover:bg-blue-600 disabled:bg-gray-400 dark:disabled:bg-gray-600 text-white rounded transition-colors"
          title="Refresh metrics"
        >
          {loading ? 'Refreshing...' : 'Refresh'}
        </button>
      </div>

      <ResponsiveContainer width="100%" height={300}>
        <BarChart data={chartData}>
          <CartesianGrid strokeDasharray="3 3" stroke={isDark ? '#4b5563' : '#e5e7eb'} />
          <XAxis
            dataKey="timestamp"
            type="number"
            domain={['dataMin', 'dataMax']}
            tickFormatter={formatXAxis}
            tick={{ fontSize: 12, fill: isDark ? '#9ca3af' : '#6b7280' }}
            angle={-45}
            textAnchor="end"
            height={80}
            scale="time"
          />
          <YAxis tick={{ fontSize: 12, fill: isDark ? '#9ca3af' : '#6b7280' }} />
          <Tooltip
            labelFormatter={formatTooltipLabel}
            contentStyle={{
              backgroundColor: isDark ? '#1f2937' : 'rgba(255, 255, 255, 0.95)',
              border: `1px solid ${isDark ? '#374151' : '#e5e7eb'}`,
              borderRadius: '0.5rem',
              color: isDark ? '#f3f4f6' : '#111827'
            }}
          />
          <Legend wrapperStyle={{ fontSize: '12px', color: isDark ? '#f3f4f6' : '#111827' }} />
          {data.datasets.map((dataset, i) => (
            <Bar
              key={i}
              dataKey={dataset.label}
              fill={dataset.border_color || `hsl(${i * 60}, 70%, 50%)`}
              animationDuration={100}
            />
          ))}
        </BarChart>
      </ResponsiveContainer>
    </div>
  )
}
