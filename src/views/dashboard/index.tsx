import { useState, useEffect } from "react"
import { invoke } from "@tauri-apps/api/core"
import { Activity, DollarSign, Zap, CheckCircle, RefreshCw } from "lucide-react"
import { StatsCard, StatsRow } from "@/components/shared/stats-card"
import { MetricsChart } from "@/components/shared/metrics-chart"
import { useMetricsSubscription } from "@/hooks/useMetricsSubscription"
import { Tabs, TabsContent, TabsList, TabsTrigger } from "@/components/ui/tabs"
import { Card, CardContent } from "@/components/ui/Card"
import { Label } from "@/components/ui/label"
import { Button } from "@/components/ui/Button"
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/Select"

interface AggregateStats {
  total_requests: number
  total_tokens: number
  total_cost: number
  successful_requests: number
}

interface Client {
  client_id: string
  name: string
}

interface Provider {
  instance_name: string
  provider_type: string
}

interface Model {
  model_id: string
  provider_instance: string
}

interface McpServer {
  id: string
  name: string
}

type TimeRange = "hour" | "day" | "week" | "month"
type LlmScope = "global" | "client" | "provider" | "model"
type McpScope = "global" | "client" | "server"

// Special value for "All" selection
const ALL_ENTITIES = "__all__"

export function DashboardView() {
  const metricsRefreshKey = useMetricsSubscription()
  const [manualRefreshKey, setManualRefreshKey] = useState(0)
  const refreshKey = metricsRefreshKey + manualRefreshKey
  const [stats, setStats] = useState<AggregateStats | null>(null)
  const [loading, setLoading] = useState(true)

  // Active tab
  const [activeTab, setActiveTab] = useState<"llm" | "mcp">("llm")

  // Unified controls
  const [timeRange, setTimeRange] = useState<TimeRange>("day")

  // LLM controls
  const [llmScope, setLlmScope] = useState<LlmScope>("global")
  const [llmScopeId, setLlmScopeId] = useState<string>("")

  // MCP controls
  const [mcpScope, setMcpScope] = useState<McpScope>("global")
  const [mcpScopeId, setMcpScopeId] = useState<string>("")

  // Entity lists for selectors
  const [clients, setClients] = useState<Client[]>([])
  const [providers, setProviders] = useState<Provider[]>([])
  const [models, setModels] = useState<Model[]>([])
  const [mcpServers, setMcpServers] = useState<McpServer[]>([])

  useEffect(() => {
    loadStats()
    loadEntities()
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

  const loadEntities = async () => {
    try {
      const [clientList, providerList, modelList, mcpServerList] = await Promise.all([
        invoke<Client[]>("list_clients").catch(() => []),
        invoke<Provider[]>("list_provider_instances").catch(() => []),
        invoke<Array<{ id: string; provider: string }>>("list_all_models").catch(() => []),
        invoke<McpServer[]>("list_mcp_servers").catch(() => []),
      ])

      setClients(clientList)
      setProviders(providerList)
      setModels(modelList.map(m => ({ model_id: m.id, provider_instance: m.provider })))
      setMcpServers(mcpServerList)
    } catch (error) {
      console.error("Failed to load entities:", error)
    }
  }

  const successRate =
    stats && stats.total_requests > 0
      ? ((stats.successful_requests / stats.total_requests) * 100).toFixed(1)
      : "0.0"

  // Set to "All" by default when scope changes
  useEffect(() => {
    if (llmScope !== "global") {
      setLlmScopeId(ALL_ENTITIES)
    } else {
      setLlmScopeId("")
    }
  }, [llmScope])

  useEffect(() => {
    if (mcpScope !== "global") {
      setMcpScopeId(ALL_ENTITIES)
    } else {
      setMcpScopeId("")
    }
  }, [mcpScope])

  // Check if "All" is selected
  const llmShowAll = llmScopeId === ALL_ENTITIES
  const mcpShowAll = mcpScopeId === ALL_ENTITIES

  // Check if we have a valid selection for non-global scopes
  const llmHasValidSelection = llmScope === "global" || (llmScopeId && llmScopeId !== "")
  const mcpHasValidSelection = mcpScope === "global" || (mcpScopeId && mcpScopeId !== "")

  // Get entities for multiScope when "All" is selected
  const getLlmMultiScope = () => {
    if (llmScope === "client") return clients.map(c => ({ id: c.client_id, label: c.name, scope: "api_key" as const }))
    if (llmScope === "provider") return providers.map(p => ({ id: p.instance_name, label: p.instance_name, scope: "provider" as const }))
    if (llmScope === "model") return models.map(m => ({ id: `${m.provider_instance}/${m.model_id}`, label: m.model_id, scope: "model" as const }))
    return []
  }

  const getMcpMultiScope = () => {
    if (mcpScope === "client") return clients.map(c => ({ id: c.client_id, label: c.name, scope: "client" as const }))
    if (mcpScope === "server") return mcpServers.map(s => ({ id: s.id, label: s.name, scope: "server" as const }))
    return []
  }

  // Get scope string for single entity view
  const getLlmChartScope = () => {
    if (llmScope === "global") return "global" as const
    if (llmScope === "client") return "api_key" as const
    if (llmScope === "provider") return "provider" as const
    if (llmScope === "model") return "model" as const
    return "global" as const
  }

  const getMcpChartScope = () => {
    if (mcpScope === "global") return "global" as const
    if (mcpScope === "client") return "client" as const
    if (mcpScope === "server") return "server" as const
    return "global" as const
  }

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

      {/* Metrics Tabs */}
      <Tabs value={activeTab} onValueChange={(v) => setActiveTab(v as "llm" | "mcp")}>
        <TabsList>
          <TabsTrigger value="llm">LLM Metrics</TabsTrigger>
          <TabsTrigger value="mcp">MCP Metrics</TabsTrigger>
        </TabsList>

        {/* LLM Metrics Tab */}
        <TabsContent value="llm" className="space-y-4">
          {/* Controls */}
          <Card>
            <CardContent className="pt-4">
              <div className="flex flex-wrap items-end gap-4">
                <div className="space-y-1.5">
                  <Label>Scope</Label>
                  <Select value={llmScope} onValueChange={(v) => setLlmScope(v as LlmScope)}>
                    <SelectTrigger className="w-[140px]">
                      <SelectValue />
                    </SelectTrigger>
                    <SelectContent>
                      <SelectItem value="global">Global</SelectItem>
                      <SelectItem value="client">Client</SelectItem>
                      <SelectItem value="provider">Provider</SelectItem>
                      <SelectItem value="model">Model</SelectItem>
                    </SelectContent>
                  </Select>
                </div>

                {llmScope !== "global" && (
                  <div className="space-y-1.5">
                    <Label>
                      {llmScope === "client" ? "Client" : llmScope === "provider" ? "Provider" : "Model"}
                    </Label>
                    <Select value={llmScopeId} onValueChange={setLlmScopeId}>
                      <SelectTrigger className="w-[200px]">
                        <SelectValue placeholder={`Select ${llmScope}...`} />
                      </SelectTrigger>
                      <SelectContent>
                        <SelectItem value={ALL_ENTITIES}>All {llmScope}s</SelectItem>
                        {llmScope === "client" && clients.map((c) => (
                          <SelectItem key={c.client_id} value={c.client_id}>
                            {c.name}
                          </SelectItem>
                        ))}
                        {llmScope === "provider" && providers.map((p) => (
                          <SelectItem key={p.instance_name} value={p.instance_name}>
                            {p.instance_name}
                          </SelectItem>
                        ))}
                        {llmScope === "model" && models.map((m) => (
                          <SelectItem
                            key={`${m.provider_instance}/${m.model_id}`}
                            value={`${m.provider_instance}/${m.model_id}`}
                          >
                            {m.model_id}
                          </SelectItem>
                        ))}
                      </SelectContent>
                    </Select>
                  </div>
                )}

                <div className="space-y-1.5">
                  <Label>Time Range</Label>
                  <Select value={timeRange} onValueChange={(v) => setTimeRange(v as TimeRange)}>
                    <SelectTrigger className="w-[120px]">
                      <SelectValue />
                    </SelectTrigger>
                    <SelectContent>
                      <SelectItem value="hour">Hour</SelectItem>
                      <SelectItem value="day">Day</SelectItem>
                      <SelectItem value="week">Week</SelectItem>
                      <SelectItem value="month">Month</SelectItem>
                    </SelectContent>
                  </Select>
                </div>

                <Button
                  variant="outline"
                  size="icon"
                  onClick={() => {
                    loadStats()
                    setManualRefreshKey(k => k + 1)
                  }}
                  title="Refresh"
                >
                  <RefreshCw className="h-4 w-4" />
                </Button>
              </div>
            </CardContent>
          </Card>

          {/* LLM Charts Grid */}
          {llmHasValidSelection ? (
            <div className="grid grid-cols-1 lg:grid-cols-2 gap-4">
              <MetricsChart
                title="Requests"
                scope={getLlmChartScope()}
                scopeId={llmScope === "global" || llmShowAll ? undefined : llmScopeId}
                multiScope={llmShowAll ? getLlmMultiScope() : undefined}
                chartType="bar"
                defaultMetricType="requests"
                defaultTimeRange={timeRange}
                showControls={false}
                refreshTrigger={refreshKey}
                height={250}
              />
              <MetricsChart
                title="Tokens"
                scope={getLlmChartScope()}
                scopeId={llmScope === "global" || llmShowAll ? undefined : llmScopeId}
                multiScope={llmShowAll ? getLlmMultiScope() : undefined}
                chartType="bar"
                defaultMetricType="tokens"
                defaultTimeRange={timeRange}
                showControls={false}
                refreshTrigger={refreshKey}
                height={250}
              />
              <MetricsChart
                title="Cost"
                scope={getLlmChartScope()}
                scopeId={llmScope === "global" || llmShowAll ? undefined : llmScopeId}
                multiScope={llmShowAll ? getLlmMultiScope() : undefined}
                chartType="bar"
                defaultMetricType="cost"
                defaultTimeRange={timeRange}
                showControls={false}
                refreshTrigger={refreshKey}
                height={250}
              />
              <MetricsChart
                title="Latency"
                scope={getLlmChartScope()}
                scopeId={llmScope === "global" || llmShowAll ? undefined : llmScopeId}
                multiScope={llmShowAll ? getLlmMultiScope() : undefined}
                chartType="bar"
                defaultMetricType="latency"
                defaultTimeRange={timeRange}
                showControls={false}
                refreshTrigger={refreshKey}
                height={250}
              />
              <MetricsChart
                title="Success Rate"
                scope={getLlmChartScope()}
                scopeId={llmScope === "global" || llmShowAll ? undefined : llmScopeId}
                multiScope={llmShowAll ? getLlmMultiScope() : undefined}
                chartType="bar"
                defaultMetricType="successrate"
                defaultTimeRange={timeRange}
                showControls={false}
                refreshTrigger={refreshKey}
                height={250}
                className="lg:col-span-2"
              />
            </div>
          ) : (
            <Card>
              <CardContent className="py-12 text-center text-muted-foreground">
                Select a {llmScope} to view metrics
              </CardContent>
            </Card>
          )}
        </TabsContent>

        {/* MCP Metrics Tab */}
        <TabsContent value="mcp" className="space-y-4">
          {/* Controls */}
          <Card>
            <CardContent className="pt-4">
              <div className="flex flex-wrap items-end gap-4">
                <div className="space-y-1.5">
                  <Label>Scope</Label>
                  <Select value={mcpScope} onValueChange={(v) => setMcpScope(v as McpScope)}>
                    <SelectTrigger className="w-[140px]">
                      <SelectValue />
                    </SelectTrigger>
                    <SelectContent>
                      <SelectItem value="global">Global</SelectItem>
                      <SelectItem value="client">Client</SelectItem>
                      <SelectItem value="server">Server</SelectItem>
                    </SelectContent>
                  </Select>
                </div>

                {mcpScope !== "global" && (
                  <div className="space-y-1.5">
                    <Label>
                      {mcpScope === "client" ? "Client" : "Server"}
                    </Label>
                    <Select value={mcpScopeId} onValueChange={setMcpScopeId}>
                      <SelectTrigger className="w-[200px]">
                        <SelectValue placeholder={`Select ${mcpScope}...`} />
                      </SelectTrigger>
                      <SelectContent>
                        <SelectItem value={ALL_ENTITIES}>All {mcpScope}s</SelectItem>
                        {mcpScope === "client" && clients.map((c) => (
                          <SelectItem key={c.client_id} value={c.client_id}>
                            {c.name}
                          </SelectItem>
                        ))}
                        {mcpScope === "server" && mcpServers.map((s) => (
                          <SelectItem key={s.id} value={s.id}>
                            {s.name}
                          </SelectItem>
                        ))}
                      </SelectContent>
                    </Select>
                  </div>
                )}

                <div className="space-y-1.5">
                  <Label>Time Range</Label>
                  <Select value={timeRange} onValueChange={(v) => setTimeRange(v as TimeRange)}>
                    <SelectTrigger className="w-[120px]">
                      <SelectValue />
                    </SelectTrigger>
                    <SelectContent>
                      <SelectItem value="hour">Hour</SelectItem>
                      <SelectItem value="day">Day</SelectItem>
                      <SelectItem value="week">Week</SelectItem>
                      <SelectItem value="month">Month</SelectItem>
                    </SelectContent>
                  </Select>
                </div>

                <Button
                  variant="outline"
                  size="icon"
                  onClick={() => {
                    loadStats()
                    setManualRefreshKey(k => k + 1)
                  }}
                  title="Refresh"
                >
                  <RefreshCw className="h-4 w-4" />
                </Button>
              </div>
            </CardContent>
          </Card>

          {/* MCP Charts Grid */}
          {mcpHasValidSelection ? (
            <div className="grid grid-cols-1 lg:grid-cols-2 gap-4">
              <MetricsChart
                title="Requests"
                scope={getMcpChartScope()}
                scopeId={mcpScope === "global" || mcpShowAll ? undefined : mcpScopeId}
                multiScope={mcpShowAll ? getMcpMultiScope() : undefined}
                chartType="bar"
                defaultMetricType="requests"
                defaultTimeRange={timeRange}
                showControls={false}
                refreshTrigger={refreshKey}
                height={250}
                dataSource="mcp"
              />
              <MetricsChart
                title="Latency"
                scope={getMcpChartScope()}
                scopeId={mcpScope === "global" || mcpShowAll ? undefined : mcpScopeId}
                multiScope={mcpShowAll ? getMcpMultiScope() : undefined}
                chartType="bar"
                defaultMetricType="latency"
                defaultTimeRange={timeRange}
                showControls={false}
                refreshTrigger={refreshKey}
                height={250}
                dataSource="mcp"
              />
              <MetricsChart
                title="Success Rate"
                scope={getMcpChartScope()}
                scopeId={mcpScope === "global" || mcpShowAll ? undefined : mcpScopeId}
                multiScope={mcpShowAll ? getMcpMultiScope() : undefined}
                chartType="bar"
                defaultMetricType="successrate"
                defaultTimeRange={timeRange}
                showControls={false}
                refreshTrigger={refreshKey}
                height={250}
                dataSource="mcp"
              />
              <MetricsChart
                title="Method Breakdown"
                scope={mcpShowAll ? "global" : getMcpChartScope()}
                scopeId={mcpScope === "global" || mcpShowAll ? undefined : mcpScopeId}
                chartType="bar"
                defaultMetricType="requests"
                defaultTimeRange={timeRange}
                showControls={false}
                refreshTrigger={refreshKey}
                height={250}
                dataSource="mcp"
                showMethodBreakdown={true}
              />
            </div>
          ) : (
            <Card>
              <CardContent className="py-12 text-center text-muted-foreground">
                Select a {mcpScope} to view metrics
              </CardContent>
            </Card>
          )}
        </TabsContent>
      </Tabs>
    </div>
  )
}
