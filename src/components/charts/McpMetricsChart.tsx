import { useEffect, useState } from 'react'
import { LineChart, Line, XAxis, YAxis, CartesianGrid, Tooltip, Legend, ResponsiveContainer } from 'recharts'
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
      <div className="flex items-center justify-center h-64 bg-gray-50 rounded-lg">
        <p className="text-gray-500">Loading MCP metrics...</p>
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
        <p className="text-gray-500">No MCP requests yet</p>
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
        <LineChart data={chartData}>
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
            <Line
              key={i}
              type="monotone"
              dataKey={dataset.label}
              stroke={dataset.border_color || `hsl(${i * 60}, 70%, 50%)`}
              strokeWidth={2}
              dot={false}
            />
          ))}
        </LineChart>
      </ResponsiveContainer>
    </div>
  )
}
