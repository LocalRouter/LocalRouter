import { useState, useEffect } from "react"
import { invoke } from "@tauri-apps/api/core"
import { Activity, DollarSign, Zap, CheckCircle } from "lucide-react"
import { StatsCard, StatsRow } from "@/components/shared/stats-card"
import { MetricsChart } from "@/components/shared/metrics-chart"
import { ActivityLog } from "./activity-log"
import { useMetricsSubscription } from "@/hooks/useMetricsSubscription"

interface AggregateStats {
  total_requests: number
  total_tokens: number
  total_cost: number
  successful_requests: number
}

export function DashboardView() {
  const refreshKey = useMetricsSubscription()
  const [stats, setStats] = useState<AggregateStats | null>(null)
  const [loading, setLoading] = useState(true)

  useEffect(() => {
    loadStats()
  }, [refreshKey])

  const loadStats = async () => {
    try {
      const aggregateStats = await invoke<AggregateStats>("get_aggregate_stats")
      setStats(aggregateStats)
    } catch (error) {
      console.error("Failed to load aggregate stats:", error)
      setStats({
        total_requests: 0,
        total_tokens: 0,
        total_cost: 0,
        successful_requests: 0,
      })
    } finally {
      setLoading(false)
    }
  }

  const successRate =
    stats && stats.total_requests > 0
      ? ((stats.successful_requests / stats.total_requests) * 100).toFixed(1)
      : "0.0"

  return (
    <div className="space-y-6">
      {/* Stats Row */}
      <StatsRow>
        <StatsCard
          title="Total Requests"
          value={loading ? "-" : stats?.total_requests.toLocaleString() ?? "0"}
          icon={<Activity className="h-5 w-5" />}
          loading={loading}
        />
        <StatsCard
          title="Total Tokens"
          value={loading ? "-" : stats?.total_tokens.toLocaleString() ?? "0"}
          icon={<Zap className="h-5 w-5" />}
          loading={loading}
        />
        <StatsCard
          title="Total Cost"
          value={loading ? "-" : `$${stats?.total_cost.toFixed(4) ?? "0.00"}`}
          icon={<DollarSign className="h-5 w-5" />}
          loading={loading}
        />
        <StatsCard
          title="Success Rate"
          value={loading ? "-" : `${successRate}%`}
          icon={<CheckCircle className="h-5 w-5" />}
          loading={loading}
        />
      </StatsRow>

      {/* LLM Metrics */}
      <MetricsChart
        title="LLM Metrics"
        scope="global"
        chartType="bar"
        defaultMetricType="requests"
        defaultTimeRange="day"
        metricOptions={[
          { id: "requests", label: "Requests" },
          { id: "tokens", label: "Tokens" },
          { id: "cost", label: "Cost" },
          { id: "latency", label: "Latency" },
          { id: "successrate", label: "Success" },
        ]}
        refreshTrigger={refreshKey}
      />

      {/* MCP Metrics */}
      <MetricsChart
        title="MCP Metrics"
        scope="global"
        chartType="bar"
        defaultMetricType="requests"
        defaultTimeRange="day"
        metricOptions={[
          { id: "requests", label: "Requests" },
          { id: "latency", label: "Latency" },
          { id: "successrate", label: "Success" },
        ]}
        refreshTrigger={refreshKey}
        dataSource="mcp"
        showMethodBreakdown={true}
      />

      {/* Live Activity Log */}
      <ActivityLog refreshTrigger={refreshKey} />
    </div>
  )
}
