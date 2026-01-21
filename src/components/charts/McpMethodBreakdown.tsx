import { useEffect, useState } from 'react'
import { AreaChart, Area, XAxis, YAxis, CartesianGrid, Tooltip, Legend, ResponsiveContainer } from 'recharts'
import { invoke } from '@tauri-apps/api/core'

interface McpMethodBreakdownProps {
  scope: string // "global", "client:{id}", or "server:{id}"
  timeRange: 'hour' | 'day' | 'week' | 'month'
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

export function McpMethodBreakdown({
  scope,
  timeRange,
  title,
  refreshTrigger = 0
}: McpMethodBreakdownProps) {
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

    const observer = new MutationObserver(checkDarkMode)
    observer.observe(document.documentElement, {
      attributes: true,
      attributeFilter: ['class']
    })

    return () => observer.disconnect()
  }, [])

  const loadMethodBreakdown = async () => {
    try {
      setLoading(true)
      setError(null)

      const result = await invoke<GraphData>('get_mcp_method_breakdown', {
        scope,
        timeRange
      })
      setData(result)
    } catch (err) {
      console.error('Failed to load MCP method breakdown:', err)
      setError(err instanceof Error ? err.message : 'Failed to load method breakdown')
    } finally {
      setLoading(false)
    }
  }

  useEffect(() => {
    loadMethodBreakdown()
  }, [scope, timeRange, refreshTrigger])

  if (loading) {
    return (
      <div className="flex items-center justify-center h-64 bg-gray-50 dark:bg-gray-700 rounded-lg">
        <p className="text-gray-500 dark:text-gray-400">Loading method breakdown...</p>
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
        <p className="text-gray-500 dark:text-gray-400">No MCP method data available</p>
      </div>
    )
  }

  // Transform GraphData to Recharts format with parsed timestamps
  const chartData = data.labels.map((label, index) => {
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
          onClick={loadMethodBreakdown}
          disabled={loading}
          className="px-3 py-1 text-sm bg-blue-500 hover:bg-blue-600 disabled:bg-gray-400 dark:disabled:bg-gray-600 text-white rounded transition-colors"
          title="Refresh method breakdown"
        >
          {loading ? 'Refreshing...' : 'Refresh'}
        </button>
      </div>

      <ResponsiveContainer width="100%" height={300}>
        <AreaChart data={chartData}>
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
            <Area
              key={i}
              type="monotone"
              dataKey={dataset.label}
              stackId="1"
              stroke={dataset.border_color || `hsl(${i * 60}, 70%, 50%)`}
              fill={dataset.background_color || `hsl(${i * 60}, 70%, 80%)`}
              fillOpacity={0.6}
              animationDuration={100}
            />
          ))}
        </AreaChart>
      </ResponsiveContainer>
    </div>
  )
}
