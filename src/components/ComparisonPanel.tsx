import { useState } from 'react'
import { StackedAreaChart } from './charts/StackedAreaChart'

type TimeRange = 'hour' | 'day' | 'week' | 'month'
type ComparisonMetricType = 'requests' | 'cost' | 'tokens'

interface MetricOption {
  id: ComparisonMetricType
  label: string
}

interface ComparisonPanelProps {
  /** Title of the comparison panel */
  title: string

  /** Type of comparison (providers, models, api_keys) */
  compareType: 'providers' | 'models' | 'api_keys'

  /** IDs of entities to compare */
  ids: string[]

  /** Metric options to display as tabs */
  metricOptions: MetricOption[]

  /** Default selected metric */
  defaultMetric?: ComparisonMetricType

  /** Default time range */
  defaultTimeRange?: TimeRange

  /** Refresh trigger for forcing data reload */
  refreshTrigger?: number
}

export default function ComparisonPanel({
  title,
  compareType,
  ids,
  metricOptions,
  defaultMetric = 'requests',
  defaultTimeRange = 'day',
  refreshTrigger = 0
}: ComparisonPanelProps) {
  const [timeRange, setTimeRange] = useState<TimeRange>(defaultTimeRange)
  const [selectedMetric, setSelectedMetric] = useState<ComparisonMetricType>(defaultMetric)

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
      <StackedAreaChart
        compareType={compareType}
        ids={ids}
        timeRange={timeRange}
        metricType={selectedMetric}
        title=""
        refreshTrigger={refreshTrigger}
      />
    </div>
  )
}
