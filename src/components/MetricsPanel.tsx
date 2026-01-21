import { useState } from 'react'
import { MetricsChart } from './charts/MetricsChart'
import { McpMetricsChart } from './charts/McpMetricsChart'
import { McpMethodBreakdown } from './charts/McpMethodBreakdown'

type TimeRange = 'hour' | 'day' | 'week' | 'month'
type LlmMetricType = 'requests' | 'tokens' | 'cost' | 'latency' | 'successrate'
type McpMetricType = 'requests' | 'latency' | 'successrate'

interface MetricOption<T> {
  id: T
  label: string
}

interface MetricsPanelProps {
  /** Title of the metrics panel */
  title: string

  /** Type of metrics to display */
  chartType: 'llm' | 'mcp' | 'mcp-methods'

  /** Metric options to display as tabs */
  metricOptions: MetricOption<LlmMetricType>[] | MetricOption<McpMetricType>[]

  /** Scope for data fetching */
  scope: 'global' | 'api_key' | 'provider' | 'model' | 'strategy' | 'client' | 'server'

  /** Optional scope ID (required for non-global scopes) */
  scopeId?: string

  /** Default selected metric */
  defaultMetric?: string

  /** Default time range */
  defaultTimeRange?: TimeRange

  /** Refresh trigger for forcing data reload */
  refreshTrigger?: number

  /** Show method breakdown for MCP requests (only for chartType='mcp-methods') */
  showMethodBreakdown?: boolean
}

export default function MetricsPanel({
  title,
  chartType,
  metricOptions,
  scope,
  scopeId,
  defaultMetric,
  defaultTimeRange = 'day',
  refreshTrigger = 0,
  showMethodBreakdown = false
}: MetricsPanelProps) {
  const [timeRange, setTimeRange] = useState<TimeRange>(defaultTimeRange)
  const [selectedMetric, setSelectedMetric] = useState<string>(
    defaultMetric || metricOptions[0]?.id || 'requests'
  )

  const renderChart = () => {
    const key = `${chartType}-${scope}-${scopeId || 'global'}-${timeRange}-${selectedMetric}`

    switch (chartType) {
      case 'llm':
        return (
          <MetricsChart
            key={key}
            scope={scope as 'global' | 'api_key' | 'provider' | 'model' | 'strategy'}
            scopeId={scopeId}
            timeRange={timeRange}
            metricType={selectedMetric as LlmMetricType}
            refreshTrigger={refreshTrigger}
          />
        )

      case 'mcp':
        return (
          <McpMetricsChart
            key={key}
            scope={scope as 'global' | 'client' | 'server'}
            scopeId={scopeId}
            timeRange={timeRange}
            metricType={selectedMetric as McpMetricType}
            refreshTrigger={refreshTrigger}
          />
        )

      case 'mcp-methods':
        if (showMethodBreakdown && selectedMetric === 'requests') {
          return (
            <McpMethodBreakdown
              key={`${key}-breakdown`}
              scope={scopeId ? `${scope}:${scopeId}` : scope}
              timeRange={timeRange}
              refreshTrigger={refreshTrigger}
            />
          )
        }

        return (
          <McpMetricsChart
            key={key}
            scope={scope as 'global' | 'client' | 'server'}
            scopeId={scopeId}
            timeRange={timeRange}
            metricType={selectedMetric as McpMetricType}
            refreshTrigger={refreshTrigger}
          />
        )

      default:
        return null
    }
  }

  return (
    <div className="bg-white dark:bg-gray-800 rounded-lg shadow p-6">
      {/* Header with Time Range Selector */}
      <div className="flex items-center justify-between mb-6">
        <h2 className="text-xl font-semibold text-gray-900 dark:text-gray-100">{title}</h2>
        <select
          value={timeRange}
          onChange={(e) => setTimeRange(e.target.value as TimeRange)}
          className="px-4 py-2 border border-gray-300 dark:border-gray-600 rounded-lg bg-white dark:bg-gray-700 text-gray-900 dark:text-gray-100 hover:bg-gray-50 dark:hover:bg-gray-600 focus:outline-none focus:ring-2 focus:ring-blue-500 text-sm"
        >
          <option value="hour">Last Hour</option>
          <option value="day">Last 24 Hours</option>
          <option value="week">Last 7 Days</option>
          <option value="month">Last 30 Days</option>
        </select>
      </div>

      {/* Metric Type Tabs */}
      <div className="flex gap-2 mb-4">
        {metricOptions.map((metric) => (
          <button
            key={metric.id}
            onClick={() => setSelectedMetric(metric.id)}
            className={`px-3 py-1.5 text-sm rounded-md transition-colors ${
              selectedMetric === metric.id
                ? 'bg-blue-600 text-white'
                : 'bg-gray-100 dark:bg-gray-700 text-gray-700 dark:text-gray-300 hover:bg-gray-200 dark:hover:bg-gray-600'
            }`}
          >
            {metric.label}
          </button>
        ))}
      </div>

      {/* Chart */}
      {renderChart()}
    </div>
  )
}
