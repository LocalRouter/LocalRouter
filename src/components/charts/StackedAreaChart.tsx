import { useEffect, useState } from 'react'
import { AreaChart, Area, XAxis, YAxis, CartesianGrid, Tooltip, Legend, ResponsiveContainer } from 'recharts'
import { invoke } from '@tauri-apps/api/core'

interface StackedAreaChartProps {
  compareType: 'api_keys' | 'providers' | 'models'
  ids: string[]
  timeRange: 'hour' | 'day' | 'week' | 'month'
  metricType: 'cost' | 'tokens' | 'requests'
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
      <div className="h-64 bg-gray-50 rounded-lg flex items-center justify-center">
        <p className="text-gray-500">Loading...</p>
      </div>
    )
  }

  if (error) {
    return (
      <div className="h-64 bg-red-50 rounded-lg flex items-center justify-center">
        <p className="text-red-600">{error}</p>
      </div>
    )
  }

  if (!data || data.datasets.length === 0) {
    return (
      <div className="h-64 bg-gray-50 rounded-lg flex items-center justify-center">
        <p className="text-gray-500">No data available</p>
      </div>
    )
  }

  // Format timestamp based on time range
  const formatTimestamp = (timestamp: string): string => {
    try {
      const date = new Date(timestamp)

      switch (timeRange) {
        case 'hour':
          // Show time only: "2:30 PM"
          return date.toLocaleTimeString('en-US', {
            hour: 'numeric',
            minute: '2-digit',
            hour12: true
          })
        case 'day':
          // Show time: "2:30 PM"
          return date.toLocaleTimeString('en-US', {
            hour: 'numeric',
            minute: '2-digit',
            hour12: true
          })
        case 'week':
          // Show day and time: "Mon 2PM"
          return date.toLocaleDateString('en-US', {
            weekday: 'short',
            hour: 'numeric',
            hour12: true
          })
        case 'month':
          // Show date: "Jan 15"
          return date.toLocaleDateString('en-US', {
            month: 'short',
            day: 'numeric'
          })
        default:
          return timestamp
      }
    } catch (e) {
      return timestamp
    }
  }

  // Transform to Recharts format
  const chartData = data.labels.map((label, index) => {
    const point: any = {
      time: label,
      timeFormatted: formatTimestamp(label)
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

  // Determine tick count based on time range to avoid overcrowding
  const getTickCount = () => {
    switch (timeRange) {
      case 'hour': return 6
      case 'day': return 8
      case 'week': return 7
      case 'month': return 6
      default: return 8
    }
  }

  return (
    <div className="bg-white dark:bg-gray-800 p-4 rounded-lg shadow">
      {title && <h3 className="text-lg font-semibold mb-4 text-gray-900 dark:text-gray-100">{title}</h3>}

      <ResponsiveContainer width="100%" height={400}>
        <AreaChart data={chartData}>
          <CartesianGrid strokeDasharray="3 3" stroke="#e5e7eb" />
          <XAxis
            dataKey="timeFormatted"
            tick={{ fontSize: 11, fill: '#6b7280' }}
            angle={-45}
            textAnchor="end"
            height={70}
            interval="preserveStartEnd"
            tickCount={getTickCount()}
          />
          <YAxis tick={{ fontSize: 11, fill: '#6b7280' }} />
          <Tooltip
            contentStyle={{
              backgroundColor: '#fff',
              border: '1px solid #e5e7eb',
              borderRadius: '0.5rem',
              fontSize: '12px'
            }}
          />
          <Legend wrapperStyle={{ fontSize: '12px' }} />
          {data.datasets.map((dataset, i) => (
            <Area
              key={i}
              type="monotone"
              dataKey={dataset.label}
              stackId="1"
              stroke={colors[i % colors.length]}
              fill={colors[i % colors.length]}
              fillOpacity={0.6}
            />
          ))}
        </AreaChart>
      </ResponsiveContainer>
    </div>
  )
}
