import * as React from "react"
import {
  BarChart,
  Bar,
  XAxis,
  YAxis,
  CartesianGrid,
  Tooltip,
  Legend,
  ResponsiveContainer,
  ReferenceLine,
} from "recharts"
import { invoke } from "@tauri-apps/api/core"
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/Card"
import { Button } from "@/components/ui/Button"
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/Select"
import { Skeleton } from "@/components/ui/skeleton"
import { RefreshCw, Expand, Minimize } from "lucide-react"
import { cn } from "@/lib/utils"

type TimeRange = "hour" | "day" | "week" | "month"
type MetricType = "tokens" | "cost" | "requests" | "latency" | "successrate"
type McpMetricType = "requests" | "latency" | "successrate"
type ChartType = "line" | "area" | "bar"
type Scope = "global" | "api_key" | "provider" | "model" | "strategy"
type McpScope = "global" | "client" | "server"
type DataSourceType = "llm" | "mcp"

interface RateLimitInfo {
  limit_type: string
  value: number
  time_window_seconds: number
}

interface GraphData {
  labels: string[]
  datasets: {
    label: string
    data: number[]
    border_color?: string
    background_color?: string
  }[]
  rate_limits?: RateLimitInfo[]
}

interface MultiScopeEntity {
  id: string
  label: string
  scope: Scope | McpScope
}

interface MetricsChartProps {
  title: string
  scope: Scope | McpScope
  scopeId?: string
  defaultMetricType?: MetricType | McpMetricType
  defaultTimeRange?: TimeRange
  chartType?: ChartType
  metricOptions?: { id: MetricType | McpMetricType; label: string }[]
  refreshTrigger?: number
  showControls?: boolean
  className?: string
  height?: number
  /** Data source type - 'llm' for LLM metrics, 'mcp' for MCP metrics. Default: 'llm' */
  dataSource?: DataSourceType
  /** Show method breakdown for MCP requests (only for dataSource='mcp') */
  showMethodBreakdown?: boolean
  /** Multiple entities to show on a single chart (overrides scope/scopeId) */
  multiScope?: MultiScopeEntity[]
}

const CHART_COLORS = [
  "hsl(var(--chart-1))",
  "hsl(var(--chart-2))",
  "hsl(var(--chart-3))",
  "hsl(var(--chart-4))",
  "hsl(var(--chart-5))",
]

export function MetricsChart({
  title,
  scope,
  scopeId,
  defaultMetricType = "requests",
  defaultTimeRange = "day",
  chartType: _chartType = "bar",
  metricOptions = [
    { id: "requests", label: "Requests" },
    { id: "tokens", label: "Tokens" },
    { id: "cost", label: "Cost" },
    { id: "latency", label: "Latency" },
    { id: "successrate", label: "Success" },
  ],
  refreshTrigger = 0,
  showControls = true,
  className,
  height = 300,
  dataSource = "llm",
  showMethodBreakdown = false,
  multiScope,
}: MetricsChartProps) {
  const [data, setData] = React.useState<GraphData | null>(null)
  const [loading, setLoading] = React.useState(true)
  const [error, setError] = React.useState<string | null>(null)
  const [metricType, setMetricType] = React.useState<MetricType | McpMetricType>(defaultMetricType)
  const [timeRange, setTimeRange] = React.useState<TimeRange>(defaultTimeRange)
  const [expanded, setExpanded] = React.useState(false)

  // Sync internal state with props when they change (for controlled usage)
  React.useEffect(() => {
    setTimeRange(defaultTimeRange)
  }, [defaultTimeRange])

  React.useEffect(() => {
    setMetricType(defaultMetricType)
  }, [defaultMetricType])

  // Helper to get command and args for a single scope
  const getCommandForScope = React.useCallback((entityScope: Scope | McpScope, entityScopeId?: string) => {
    let command = ""
    const args: Record<string, unknown> = { timeRange, metricType }

    if (dataSource === "mcp") {
      if (showMethodBreakdown && metricType === "requests") {
        command = "get_mcp_method_breakdown"
        args.scope = entityScopeId ? `${entityScope}:${entityScopeId}` : entityScope
      } else {
        switch (entityScope) {
          case "global":
            command = "get_global_mcp_metrics"
            break
          case "client":
            command = "get_client_mcp_metrics"
            args.clientId = entityScopeId
            break
          case "server":
            command = "get_mcp_server_metrics"
            args.serverId = entityScopeId
            break
        }
      }
    } else {
      switch (entityScope) {
        case "global":
          command = "get_global_metrics"
          break
        case "api_key":
          command = "get_api_key_metrics"
          args.apiKeyId = entityScopeId
          break
        case "provider":
          command = "get_provider_metrics"
          args.provider = entityScopeId
          break
        case "model":
          command = "get_model_metrics"
          args.model = entityScopeId
          break
        case "strategy":
          command = "get_strategy_metrics"
          args.strategyId = entityScopeId
          break
      }
    }

    return { command, args }
  }, [timeRange, metricType, dataSource, showMethodBreakdown])

  const loadMetrics = React.useCallback(async () => {
    try {
      setLoading(true)
      setError(null)

      if (multiScope && multiScope.length > 0) {
        // Fetch data for multiple entities and combine
        const results = await Promise.all(
          multiScope.map(async (entity) => {
            const { command, args } = getCommandForScope(entity.scope, entity.id)
            if (!command) return null
            try {
              const result = await invoke<GraphData>(command, args)
              return { entity, result }
            } catch {
              return null
            }
          })
        )

        // Combine results into a single GraphData
        const validResults = results.filter((r): r is { entity: MultiScopeEntity; result: GraphData } =>
          r !== null && r.result.labels.length > 0
        )

        if (validResults.length === 0) {
          setData({ labels: [], datasets: [] })
          return
        }

        // Use the labels from the first result (they should all be the same time buckets)
        const labels = validResults[0].result.labels

        // Create a dataset for each entity
        const datasets = validResults.map((r, index) => {
          // Build a map of timestamp -> value for this entity's data
          const dataMap = new Map<string, number>()
          r.result.labels.forEach((label, labelIndex) => {
            const value = r.result.datasets.reduce((sum, ds) => sum + (ds.data[labelIndex] || 0), 0)
            dataMap.set(label, value)
          })

          // Map to common labels, using 0 for missing data points
          const combinedData = labels.map((label) => dataMap.get(label) || 0)

          return {
            label: r.entity.label,
            data: combinedData,
            background_color: CHART_COLORS[index % CHART_COLORS.length],
            border_color: CHART_COLORS[index % CHART_COLORS.length],
          }
        })

        setData({ labels, datasets })
      } else {
        // Single scope - original behavior
        const { command, args } = getCommandForScope(scope, scopeId)

        if (!command) {
          console.error(`No command for dataSource=${dataSource}, scope=${scope}`)
          setError(`Invalid configuration: dataSource=${dataSource}, scope=${scope}`)
          return
        }

        const result = await invoke<GraphData>(command, args)
        setData(result)
      }
    } catch (err) {
      console.error("Failed to load metrics:", err)
      setError(err instanceof Error ? err.message : "Failed to load metrics")
    } finally {
      setLoading(false)
    }
  }, [scope, scopeId, timeRange, metricType, dataSource, showMethodBreakdown, multiScope, getCommandForScope])

  React.useEffect(() => {
    loadMetrics()
  }, [loadMetrics, refreshTrigger])

  // Generate time domain based on selected time range
  const getTimeDomain = React.useCallback(() => {
    const now = Date.now()
    let rangeMs: number
    switch (timeRange) {
      case "hour":
        rangeMs = 60 * 60 * 1000
        break
      case "day":
        rangeMs = 24 * 60 * 60 * 1000
        break
      case "week":
        rangeMs = 7 * 24 * 60 * 60 * 1000
        break
      case "month":
        rangeMs = 30 * 24 * 60 * 60 * 1000
        break
      default:
        rangeMs = 24 * 60 * 60 * 1000
    }
    return { start: now - rangeMs, end: now }
  }, [timeRange])

  // Transform GraphData to Recharts format
  const chartData = React.useMemo(() => {
    if (!data || data.labels.length === 0) return []

    return data.labels.map((label, index) => {
      const timestamp = new Date(label).getTime()
      const point: Record<string, unknown> = {
        timestamp,
        timeLabel: label,
      }
      data.datasets.forEach((dataset) => {
        point[dataset.label] = dataset.data[index] || 0
      })
      return point
    })
  }, [data])

  const formatXAxis = (tickItem: number) => {
    const date = new Date(tickItem)
    switch (timeRange) {
      case "hour":
      case "day":
        return date.toLocaleTimeString("en-US", {
          hour: "numeric",
          minute: "2-digit",
        })
      case "week":
        return date.toLocaleDateString("en-US", {
          weekday: "short",
          hour: "numeric",
        })
      case "month":
        return date.toLocaleDateString("en-US", {
          month: "short",
          day: "numeric",
        })
      default:
        return date.toLocaleString()
    }
  }

  const formatTooltipLabel = (timestamp: number) => {
    const date = new Date(timestamp)
    return date.toLocaleString("en-US", {
      month: "short",
      day: "numeric",
      hour: "numeric",
      minute: "2-digit",
      hour12: true,
    })
  }

  const renderChart = () => {
    const timeDomain = getTimeDomain()
    const hasData = data && data.datasets.length > 0 && chartData.length > 0

    const commonProps = {
      data: hasData ? chartData : [],
      margin: { top: 10, right: 30, left: 0, bottom: 60 },
    }

    const commonAxisProps = {
      xAxis: (
        <XAxis
          dataKey="timestamp"
          type="number"
          domain={hasData ? ["dataMin", "dataMax"] : [timeDomain.start, timeDomain.end]}
          tickFormatter={formatXAxis}
          tick={{ fontSize: 11, fill: "hsl(var(--muted-foreground))" }}
          angle={-45}
          textAnchor="end"
          height={60}
          scale="time"
        />
      ),
      yAxis: (
        <YAxis
          tick={{ fontSize: 11, fill: "hsl(var(--muted-foreground))" }}
          width={50}
          domain={hasData ? ["auto", "auto"] : [0, 10]}
        />
      ),
      grid: (
        <CartesianGrid strokeDasharray="3 3" stroke="hsl(var(--border))" />
      ),
      tooltip: (
        <Tooltip
          labelFormatter={formatTooltipLabel}
          contentStyle={{
            backgroundColor: "hsl(var(--background))",
            border: "1px solid hsl(var(--border))",
            borderRadius: "var(--radius)",
            fontSize: "12px",
          }}
        />
      ),
      legend: (
        <Legend
          wrapperStyle={{ fontSize: "12px" }}
          verticalAlign="top"
          height={36}
        />
      ),
    }

    const renderContent = () => {
      if (!hasData || !data) return null
      return (
        <>
          {data.datasets.map((dataset, i) => {
            const color = dataset.border_color || CHART_COLORS[i % CHART_COLORS.length]

            return (
              <Bar
                key={dataset.label}
                dataKey={dataset.label}
                fill={color}
                animationDuration={300}
              />
            )
          })}
          {data.rate_limits?.map((limit, i) => (
            <ReferenceLine
              key={`limit-${i}`}
              y={limit.value}
              stroke="hsl(var(--destructive))"
              strokeDasharray="5 5"
              strokeWidth={2}
              label={{
                value: `${limit.limit_type} Limit: ${limit.value}`,
                position: "insideTopRight",
                fill: "hsl(var(--destructive))",
                fontSize: 10,
              }}
            />
          ))}
        </>
      )
    }

    return (
      <ResponsiveContainer width="100%" height={expanded ? 500 : height}>
        <BarChart {...commonProps}>
          {commonAxisProps.grid}
          {commonAxisProps.xAxis}
          {commonAxisProps.yAxis}
          {commonAxisProps.tooltip}
          {hasData && commonAxisProps.legend}
          {renderContent()}
        </BarChart>
      </ResponsiveContainer>
    )
  }

  return (
    <Card className={cn(expanded && "fixed inset-4 z-50", className)}>
      <CardHeader className="flex flex-row items-center justify-between space-y-0 pb-2">
        <CardTitle className="text-base font-medium">{title}</CardTitle>
        {showControls && (
          <div className="flex items-center gap-2">
            <Select
              value={timeRange}
              onValueChange={(v) => setTimeRange(v as TimeRange)}
            >
              <SelectTrigger className="w-[100px]">
                <SelectValue />
              </SelectTrigger>
              <SelectContent>
                <SelectItem value="hour">Hour</SelectItem>
                <SelectItem value="day">Day</SelectItem>
                <SelectItem value="week">Week</SelectItem>
                <SelectItem value="month">Month</SelectItem>
              </SelectContent>
            </Select>

            <Select
              value={metricType}
              onValueChange={(v) => setMetricType(v as MetricType)}
            >
              <SelectTrigger className="w-[100px]">
                <SelectValue />
              </SelectTrigger>
              <SelectContent>
                {metricOptions.map((option) => (
                  <SelectItem key={option.id} value={option.id}>
                    {option.label}
                  </SelectItem>
                ))}
              </SelectContent>
            </Select>

            <Button
              variant="ghost"
              size="icon"
              onClick={loadMetrics}
              disabled={loading}
            >
              <RefreshCw
                className={cn("h-4 w-4", loading && "animate-spin")}
              />
            </Button>

            <Button
              variant="ghost"
              size="icon"
              onClick={() => setExpanded(!expanded)}
            >
              {expanded ? (
                <Minimize className="h-4 w-4" />
              ) : (
                <Expand className="h-4 w-4" />
              )}
            </Button>
          </div>
        )}
      </CardHeader>
      <CardContent>
        {loading ? (
          <Skeleton style={{ height: expanded ? 500 : height }} className="w-full" />
        ) : error ? (
          <div style={{ height: expanded ? 500 : height }} className="flex items-center justify-center text-destructive">
            {error}
          </div>
        ) : (
          renderChart()
        )}
      </CardContent>
    </Card>
  )
}
