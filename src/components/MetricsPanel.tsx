import UnifiedChart from './charts/UnifiedChart'

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
  return (
    <UnifiedChart
      title={title}
      chartType={chartType}
      scope={scope}
      scopeId={scopeId}
      metricOptions={metricOptions}
      defaultMetric={defaultMetric}
      defaultTimeRange={defaultTimeRange}
      refreshTrigger={refreshTrigger}
      showMethodBreakdown={showMethodBreakdown}
    />
  )
}
