import { useState, useEffect } from 'react'
import { invoke } from '@tauri-apps/api/core'
import Card from '../ui/StatCard'
import { MetricsChart } from '../charts/MetricsChart'
import { StackedAreaChart } from '../charts/StackedAreaChart'
import { McpMetricsChart } from '../charts/McpMetricsChart'
import { McpMethodBreakdown } from '../charts/McpMethodBreakdown'
import { useMetricsSubscription } from '../../hooks/useMetricsSubscription'

interface AggregateStats {
  total_requests: number
  total_tokens: number
  total_cost: number
}

export default function HomeTab() {
  const refreshKey = useMetricsSubscription()
  const [stats, setStats] = useState<AggregateStats | null>(null)
  const [trackedProviders, setTrackedProviders] = useState<string[]>([])
  const [timeRange, setTimeRange] = useState<'hour' | 'day' | 'week' | 'month'>('day')
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
        setStats({ total_requests: 0, total_tokens: 0, total_cost: 0 })
      }

      // Load tracked providers for comparison charts
      try {
        const providers = await invoke<string[]>('list_tracked_providers')
        setTrackedProviders(providers)
      } catch (error) {
        console.error('Failed to load tracked providers:', error)
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
        <div className="text-gray-500">Loading dashboard...</div>
      </div>
    )
  }

  const successRate = stats && stats.total_requests > 0
    ? ((stats.total_requests / stats.total_requests) * 100).toFixed(1)
    : '0.0'

  return (
    <div className="p-6 space-y-6">
      {/* Header with time range selector */}
      <div className="flex justify-between items-center">
        <h1 className="text-2xl font-bold">Dashboard</h1>

        <select
          value={timeRange}
          onChange={(e) => setTimeRange(e.target.value as any)}
          className="px-4 py-2 border border-gray-300 rounded-lg bg-white hover:bg-gray-50 focus:outline-none focus:ring-2 focus:ring-blue-500"
        >
          <option value="hour">Last Hour</option>
          <option value="day">Last 24 Hours</option>
          <option value="week">Last 7 Days</option>
          <option value="month">Last 30 Days</option>
        </select>
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

      {/* Cost Breakdown by Provider (Stacked Area Chart) */}
      {trackedProviders.length > 0 && (
        <StackedAreaChart
          compareType="providers"
          ids={trackedProviders}
          timeRange={timeRange}
          metricType="cost"
          title="Cost Breakdown by Provider"
          refreshTrigger={refreshKey}
        />
      )}

      {/* Request Volume & Token Usage */}
      <div className="grid grid-cols-2 gap-6">
        <MetricsChart
          scope="global"
          timeRange={timeRange}
          metricType="requests"
          title="Request Volume"
          refreshTrigger={refreshKey}
        />

        <MetricsChart
          scope="global"
          timeRange={timeRange}
          metricType="tokens"
          title="Token Usage"
          refreshTrigger={refreshKey}
        />
      </div>

      {/* Latency & Success Rate */}
      <div className="grid grid-cols-2 gap-6">
        <MetricsChart
          scope="global"
          timeRange={timeRange}
          metricType="latency"
          title="Average Latency"
          refreshTrigger={refreshKey}
        />

        <MetricsChart
          scope="global"
          timeRange={timeRange}
          metricType="successrate"
          title="Success Rate"
          refreshTrigger={refreshKey}
        />
      </div>

      {/* Provider Comparison - Tokens */}
      {trackedProviders.length > 0 && (
        <StackedAreaChart
          compareType="providers"
          ids={trackedProviders}
          timeRange={timeRange}
          metricType="tokens"
          title="Token Usage by Provider"
          refreshTrigger={refreshKey}
        />
      )}

      {/* Cost Over Time */}
      <MetricsChart
        scope="global"
        timeRange={timeRange}
        metricType="cost"
        title="Total Cost Over Time"
        refreshTrigger={refreshKey}
      />

      {/* MCP Metrics Section */}
      <div className="mt-12">
        <h2 className="text-xl font-semibold mb-6">MCP (Model Context Protocol) Usage</h2>

        {/* Method Breakdown */}
        <McpMethodBreakdown
          scope="global"
          timeRange={timeRange}
          title="MCP Methods Over Time"
          refreshTrigger={refreshKey}
        />

        {/* MCP Request Volume & Latency */}
        <div className="grid grid-cols-2 gap-6 mt-6">
          <McpMetricsChart
            scope="global"
            timeRange={timeRange}
            metricType="requests"
            title="MCP Request Volume"
            refreshTrigger={refreshKey}
          />

          <McpMetricsChart
            scope="global"
            timeRange={timeRange}
            metricType="latency"
            title="MCP Average Latency"
            refreshTrigger={refreshKey}
          />
        </div>

        {/* MCP Success Rate */}
        <div className="mt-6">
          <McpMetricsChart
            scope="global"
            timeRange={timeRange}
            metricType="successrate"
            title="MCP Success Rate"
            refreshTrigger={refreshKey}
          />
        </div>
      </div>

      {/* Info Note */}
      <div className="bg-blue-50 border border-blue-200 rounded-lg p-4">
        <p className="text-sm text-blue-900">
          <strong>Note:</strong> Metrics are tracked in-memory for the last 24 hours with 1-minute granularity.
          Historical data beyond 24 hours is available from access logs.
        </p>
      </div>
    </div>
  )
}
