/**
 * Demo wrapper that mirrors the real MetricsChart component's UI
 * but uses hardcoded data instead of Tauri invoke calls.
 * Reuses the same Card/Select/Button components and Recharts setup.
 */
import { useState } from "react"
import {
  BarChart,
  Bar,
  XAxis,
  YAxis,
  CartesianGrid,
  Tooltip,
  ResponsiveContainer,
} from "recharts"
import { Card, CardContent, CardHeader, CardTitle } from "@app/components/ui/Card"
import { Button } from "@app/components/ui/Button"
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@app/components/ui/Select"
import { RefreshCw } from "lucide-react"

type TimeRange = "hour" | "day" | "week" | "month"
type MetricType = "requests" | "tokens" | "cost" | "latency"

// Pre-generated realistic data for each combination
function generateData(metric: MetricType, range: TimeRange) {
  const now = Date.now()
  const points: { timestamp: number; value: number }[] = []

  let count: number
  let stepMs: number
  switch (range) {
    case "hour":
      count = 12; stepMs = 5 * 60 * 1000; break
    case "day":
      count = 24; stepMs = 60 * 60 * 1000; break
    case "week":
      count = 28; stepMs = 6 * 60 * 60 * 1000; break
    case "month":
      count = 30; stepMs = 24 * 60 * 60 * 1000; break
  }

  const bases: Record<MetricType, number> = {
    requests: 200,
    tokens: 45000,
    cost: 1.2,
    latency: 350,
  }
  const base = bases[metric]

  // Use a seeded-ish pattern (deterministic from index) so it doesn't jump on re-render
  for (let i = count - 1; i >= 0; i--) {
    const t = now - i * stepMs
    const hour = new Date(t).getHours()
    const dayMultiplier = hour >= 9 && hour <= 18 ? 1.5 : hour >= 6 && hour <= 21 ? 1.0 : 0.3
    const wave = Math.sin(i / 3) * 0.3
    const jitter = Math.sin(i * 7.3 + 2.1) * 0.2 // deterministic pseudo-random
    const value = Math.max(1, base * dayMultiplier * (1 + wave + jitter))
    points.push({
      timestamp: t,
      value: metric === "cost" ? Math.round(value * 100) / 100 : Math.round(value),
    })
  }
  return points
}

function formatXAxis(timestamp: number, range: TimeRange) {
  const date = new Date(timestamp)
  switch (range) {
    case "hour":
    case "day":
      return date.toLocaleTimeString("en-US", { hour: "numeric", minute: "2-digit" })
    case "week":
      return date.toLocaleDateString("en-US", { weekday: "short", hour: "numeric" })
    case "month":
      return date.toLocaleDateString("en-US", { month: "short", day: "numeric" })
  }
}

const METRIC_OPTIONS = [
  { id: "requests" as const, label: "Requests" },
  { id: "tokens" as const, label: "Tokens" },
  { id: "cost" as const, label: "Cost" },
  { id: "latency" as const, label: "Latency" },
]

export function MetricsDemo() {
  const [metricType, setMetricType] = useState<MetricType>("requests")
  const [timeRange, setTimeRange] = useState<TimeRange>("day")

  const data = generateData(metricType, timeRange)

  return (
    <div className="dark max-w-lg">
      <Card>
        <CardHeader className="flex flex-row items-center justify-between space-y-0 pb-2">
          <CardTitle className="text-base font-medium">Requests</CardTitle>
          <div className="flex items-center gap-2">
            <Select value={timeRange} onValueChange={(v) => setTimeRange(v as TimeRange)}>
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

            <Select value={metricType} onValueChange={(v) => setMetricType(v as MetricType)}>
              <SelectTrigger className="w-[100px]">
                <SelectValue />
              </SelectTrigger>
              <SelectContent>
                {METRIC_OPTIONS.map((opt) => (
                  <SelectItem key={opt.id} value={opt.id}>{opt.label}</SelectItem>
                ))}
              </SelectContent>
            </Select>

            <Button variant="ghost" size="icon" onClick={() => {}}>
              <RefreshCw className="h-4 w-4" />
            </Button>
          </div>
        </CardHeader>
        <CardContent>
          <ResponsiveContainer width="100%" height={250}>
            <BarChart data={data} margin={{ top: 10, right: 30, left: 0, bottom: 60 }}>
              <CartesianGrid strokeDasharray="3 3" stroke="hsl(var(--border))" />
              <XAxis
                dataKey="timestamp"
                type="number"
                domain={["dataMin", "dataMax"]}
                tickFormatter={(t) => formatXAxis(t, timeRange)}
                tick={{ fontSize: 11, fill: "hsl(var(--muted-foreground))" }}
                angle={-45}
                textAnchor="end"
                height={60}
                scale="time"
              />
              <YAxis
                tick={{ fontSize: 11, fill: "hsl(var(--muted-foreground))" }}
                width={50}
              />
              <Tooltip
                labelFormatter={(label) => {
                  const ts = typeof label === "string" ? parseInt(label, 10) : Number(label)
                  if (isNaN(ts)) return String(label)
                  return new Date(ts).toLocaleString("en-US", {
                    month: "short", day: "numeric", hour: "numeric", minute: "2-digit", hour12: true,
                  })
                }}
                contentStyle={{
                  backgroundColor: "hsl(var(--background))",
                  border: "1px solid hsl(var(--border))",
                  borderRadius: "var(--radius)",
                  fontSize: "12px",
                }}
              />
              <Bar dataKey="value" fill="hsl(var(--chart-1))" animationDuration={300} />
            </BarChart>
          </ResponsiveContainer>
        </CardContent>
      </Card>
    </div>
  )
}
