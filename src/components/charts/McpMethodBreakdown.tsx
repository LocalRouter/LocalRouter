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
      <div className="flex items-center justify-center h-64 bg-gray-50 rounded-lg">
        <p className="text-gray-500">Loading method breakdown...</p>
      </div>
    )
  }

  if (error) {
    return (
      <div className="flex items-center justify-center h-64 bg-red-50 rounded-lg">
        <p className="text-red-600">{error}</p>
      </div>
    )
  }

  if (!data || data.datasets.length === 0) {
    return (
      <div className="flex items-center justify-center h-64 bg-gray-50 rounded-lg">
        <p className="text-gray-500">No MCP method data available</p>
      </div>
    )
  }

  // Transform GraphData to Recharts format
  const chartData = data.labels.map((label, index) => {
    const point: any = { time: label }
    data.datasets.forEach(dataset => {
      point[dataset.label] = dataset.data[index] || 0
    })
    return point
  })

  return (
    <div className="bg-white p-4 rounded-lg shadow">
      {title && <h3 className="text-lg font-semibold mb-4">{title}</h3>}

      <ResponsiveContainer width="100%" height={300}>
        <AreaChart data={chartData}>
          <CartesianGrid strokeDasharray="3 3" />
          <XAxis
            dataKey="time"
            tick={{ fontSize: 12 }}
            angle={-45}
            textAnchor="end"
            height={80}
          />
          <YAxis tick={{ fontSize: 12 }} />
          <Tooltip />
          <Legend />
          {data.datasets.map((dataset, i) => (
            <Area
              key={i}
              type="monotone"
              dataKey={dataset.label}
              stackId="1"
              stroke={dataset.border_color || `hsl(${i * 60}, 70%, 50%)`}
              fill={dataset.background_color || `hsl(${i * 60}, 70%, 80%)`}
              fillOpacity={0.6}
            />
          ))}
        </AreaChart>
      </ResponsiveContainer>
    </div>
  )
}
