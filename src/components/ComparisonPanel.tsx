import UnifiedChart from './charts/UnifiedChart'

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
  return (
    <UnifiedChart
      title={title}
      chartType="comparison"
      compareType={compareType}
      compareIds={ids}
      metricOptions={metricOptions}
      defaultMetric={defaultMetric}
      defaultTimeRange={defaultTimeRange}
      refreshTrigger={refreshTrigger}
    />
  )
}
