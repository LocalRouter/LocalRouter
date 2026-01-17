import { useEffect, useState } from 'react'
import { AreaChart, Area, XAxis, YAxis, CartesianGrid, Tooltip, Legend, ResponsiveContainer } from 'recharts'
import { invoke } from '@tauri-apps/api/core'

interface StackedAreaChartProps {
  compareType: 'api_keys' | 'providers'
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

        const command = compareType === 'api_keys' ? 'compare_api_keys' : 'compare_providers'
        const args = {
          [compareType === 'api_keys' ? 'api_key_ids' : 'providers']: ids,
          time_range: timeRange,
          metric_type: metricType,
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

  // Transform to Recharts format
  const chartData = data.labels.map((label, index) => {
    const point: any = { time: label }
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
    <div className="bg-white p-4 rounded-lg shadow">
      {title && <h3 className="text-lg font-semibold mb-4">{title}</h3>}

      <ResponsiveContainer width="100%" height={400}>
        <AreaChart data={chartData}>
          <CartesianGrid strokeDasharray="3 3" />
          <XAxis dataKey="time" tick={{ fontSize: 12 }} angle={-45} textAnchor="end" height={80} />
          <YAxis tick={{ fontSize: 12 }} />
          <Tooltip />
          <Legend />
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
