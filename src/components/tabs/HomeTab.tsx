import { useState, useEffect } from 'react'
import { invoke } from '@tauri-apps/api/core'
import Card from '../ui/StatCard'
import MetricsPanel from '../MetricsPanel'
import { useMetricsSubscription } from '../../hooks/useMetricsSubscription'

interface AggregateStats {
  total_requests: number
  total_tokens: number
  total_cost: number
  successful_requests: number
}

export default function HomeTab() {
  const refreshKey = useMetricsSubscription()
  const [stats, setStats] = useState<AggregateStats | null>(null)
  const [loading, setLoading] = useState(true)

  useEffect(() => {
    loadOverview()
  }, [refreshKey])

  const loadOverview = async () => {
    try {
      // Load aggregate stats
      try {
        const aggregateStats = await invoke<AggregateStats>('get_aggregate_stats')
        setStats(aggregateStats)
      } catch (error) {
        console.error('Failed to load aggregate stats:', error)
        setStats({ total_requests: 0, total_tokens: 0, total_cost: 0, successful_requests: 0 })
      }

    } catch (error) {
      console.error('Failed to load overview:', error)
    } finally {
      setLoading(false)
    }
  }

  if (loading) {
    return (
      <div className="flex items-center justify-center h-64">
        <div className="text-gray-500 dark:text-gray-400">Loading dashboard...</div>
      </div>
    )
  }

  const successRate = stats && stats.total_requests > 0
    ? ((stats.successful_requests / stats.total_requests) * 100).toFixed(1)
    : '0.0'

  return (
    <div className="p-6 space-y-6">
      {/* Header */}
      <div className="flex justify-between items-center">
        <h1 className="text-2xl font-bold text-gray-900 dark:text-gray-100">Dashboard</h1>
      </div>

      {/* Summary Stats */}
      {stats && (
        <div className="grid grid-cols-4 gap-4">
          <Card
            title="Total Requests"
            value={stats.total_requests.toLocaleString()}
          />
          <Card
            title="Total Tokens"
            value={stats.total_tokens.toLocaleString()}
          />
          <Card
            title="Total Cost"
            value={`$${stats.total_cost.toFixed(4)}`}
          />
          <Card
            title="Success Rate"
            value={`${successRate}%`}
          />
        </div>
      )}

      {/* LLM Metrics Panel */}
      <MetricsPanel
        title="LLM Metrics"
        chartType="llm"
        metricOptions={[
          { id: 'requests', label: 'Requests' },
          { id: 'tokens', label: 'Tokens' },
          { id: 'cost', label: 'Cost' },
          { id: 'latency', label: 'Latency' },
          { id: 'successrate', label: 'Success' },
        ]}
        scope="global"
        defaultMetric="requests"
        defaultTimeRange="day"
        refreshTrigger={refreshKey}
      />

      {/* MCP Metrics Panel */}
      <MetricsPanel
        title="MCP Metrics"
        chartType="mcp-methods"
        metricOptions={[
          { id: 'requests', label: 'Requests' },
          { id: 'latency', label: 'Latency' },
          { id: 'successrate', label: 'Success' },
        ]}
        scope="global"
        defaultMetric="requests"
        defaultTimeRange="day"
        refreshTrigger={refreshKey}
        showMethodBreakdown={true}
      />

    </div>
  )
}
