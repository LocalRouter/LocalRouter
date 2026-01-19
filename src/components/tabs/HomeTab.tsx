import { useState, useEffect } from 'react'
import { invoke } from '@tauri-apps/api/core'
import Card from '../ui/StatCard'
import { MetricsChart } from '../charts/MetricsChart'
import { StackedAreaChart } from '../charts/StackedAreaChart'
import { McpMetricsChart } from '../charts/McpMetricsChart'
import { McpMethodBreakdown } from '../charts/McpMethodBreakdown'
import { useMetricsSubscription } from '../../hooks/useMetricsSubscription'
import { CatalogStats } from '../../lib/catalog-types'

interface AggregateStats {
  total_requests: number
  total_tokens: number
  total_cost: number
  successful_requests: number
}

export default function HomeTab() {
  const refreshKey = useMetricsSubscription()
  const [stats, setStats] = useState<AggregateStats | null>(null)
  const [catalogStats, setCatalogStats] = useState<CatalogStats | null>(null)
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
        setStats({ total_requests: 0, total_tokens: 0, total_cost: 0, successful_requests: 0 })
      }

      // Load tracked providers for comparison charts
      try {
        const providers = await invoke<string[]>('list_tracked_providers')
        setTrackedProviders(providers)
      } catch (error) {
        console.error('Failed to load tracked providers:', error)
      }

      // Load catalog stats
      try {
        const catalog = await invoke<CatalogStats>('get_catalog_stats')
        setCatalogStats(catalog)
      } catch (error) {
        console.error('Failed to load catalog stats:', error)
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
      {/* Header with time range selector */}
      <div className="flex justify-between items-center">
        <h1 className="text-2xl font-bold text-gray-900 dark:text-gray-100">Dashboard</h1>

        <select
          value={timeRange}
          onChange={(e) => setTimeRange(e.target.value as any)}
          className="px-4 py-2 border border-gray-300 dark:border-gray-600 rounded-lg bg-white dark:bg-gray-800 text-gray-900 dark:text-gray-100 hover:bg-gray-50 dark:hover:bg-gray-700 focus:outline-none focus:ring-2 focus:ring-blue-500"
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

      {/* Model Catalog Info */}
      {catalogStats && (
        <div className="bg-white dark:bg-gray-800 rounded-lg shadow p-6">
          <div className="flex items-center justify-between mb-4">
            <h2 className="text-lg font-semibold text-gray-900 dark:text-gray-100">
              Model Catalog
            </h2>
            <span className="text-xs text-gray-500 dark:text-gray-400">
              Updated {new Date(catalogStats.fetch_date).toLocaleDateString()}
            </span>
          </div>

          <div className="grid grid-cols-3 gap-4 mb-4">
            <div>
              <p className="text-sm text-gray-500 dark:text-gray-400">Total Models</p>
              <p className="text-2xl font-bold text-gray-900 dark:text-gray-100">
                {catalogStats.total_models}
              </p>
            </div>
            <div>
              <p className="text-sm text-gray-500 dark:text-gray-400">Providers</p>
              <p className="text-2xl font-bold text-gray-900 dark:text-gray-100">
                {Object.keys(catalogStats.providers).length}
              </p>
            </div>
            <div>
              <p className="text-sm text-gray-500 dark:text-gray-400">Modalities</p>
              <p className="text-2xl font-bold text-gray-900 dark:text-gray-100">
                {Object.keys(catalogStats.modalities).length}
              </p>
            </div>
          </div>

          <div className="grid grid-cols-2 gap-4">
            <div>
              <p className="text-xs font-medium text-gray-500 dark:text-gray-400 mb-2">
                Top Providers
              </p>
              <div className="space-y-1">
                {Object.entries(catalogStats.providers)
                  .sort(([, a], [, b]) => b - a)
                  .slice(0, 5)
                  .map(([provider, count]) => (
                    <div key={provider} className="flex justify-between text-sm">
                      <span className="text-gray-700 dark:text-gray-300 capitalize">{provider}</span>
                      <span className="text-gray-500 dark:text-gray-400">{count} models</span>
                    </div>
                  ))}
              </div>
            </div>
            <div>
              <p className="text-xs font-medium text-gray-500 dark:text-gray-400 mb-2">
                Modality Distribution
              </p>
              <div className="space-y-1">
                {Object.entries(catalogStats.modalities).map(([modality, count]) => (
                  <div key={modality} className="flex justify-between text-sm">
                    <span className="text-gray-700 dark:text-gray-300 capitalize">{modality}</span>
                    <span className="text-gray-500 dark:text-gray-400">{count} models</span>
                  </div>
                ))}
              </div>
            </div>
          </div>

          <div className="mt-4 pt-4 border-t border-gray-200 dark:border-gray-700">
            <p className="text-xs text-gray-500 dark:text-gray-400">
              Pricing data from OpenRouter • Embedded at build time • Fully offline-capable
            </p>
          </div>
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
        <h2 className="text-xl font-semibold text-gray-900 dark:text-gray-100 mb-6">MCP (Model Context Protocol) Usage</h2>

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
      <div className="bg-blue-50 dark:bg-blue-900/20 border border-blue-200 dark:border-blue-700 rounded-lg p-4">
        <p className="text-sm text-blue-900 dark:text-blue-200">
          <strong>Note:</strong> Metrics are tracked in-memory for the last 24 hours with 1-minute granularity.
          Historical data beyond 24 hours is available from access logs.
        </p>
      </div>
    </div>
  )
}
