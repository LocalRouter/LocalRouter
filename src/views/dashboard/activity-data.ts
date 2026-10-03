import type {
  GraphData,
  MonitorEventSummary,
  TimeRange,
} from '../../types/tauri-commands'

export const RANGES: Record<
  TimeRange,
  { label: string; description: string; bucket: string }
> = {
  hour: { label: '1h', description: 'last hour', bucket: '5 minutes' },
  day: { label: '24h', description: 'last 24 hours', bucket: 'hour' },
  week: { label: '7d', description: 'last 7 days', bucket: '6 hours' },
  month: { label: '30d', description: 'last 30 days', bucket: '12 hours' },
}

// Backend graph labels are UTC, without an explicit timezone. ISO labels are
// also accepted for demo data. Normalize before joining independently loaded series.
export function graphTimestamp(label: string): number {
  return Date.parse(
    /^\d{4}-\d\d-\d\d \d\d:\d\d$/.test(label)
      ? `${label.replace(' ', 'T')}:00Z`
      : label,
  )
}

export function graphValues(graph: GraphData | null): Map<number, number> {
  const values = new Map<number, number>()
  graph?.labels.forEach((label, index) => {
    const timestamp = graphTimestamp(label)
    if (!Number.isFinite(timestamp)) return
    values.set(
      timestamp,
      graph.datasets.reduce(
        (sum, dataset) => sum + (dataset.data[index] ?? 0),
        0,
      ),
    )
  })
  return values
}

export function requestTimeline(llm: GraphData | null, mcp: GraphData | null) {
  const llmValues = graphValues(llm)
  const mcpValues = graphValues(mcp)
  return [...new Set([...llmValues.keys(), ...mcpValues.keys()])]
    .sort((a, b) => a - b)
    .map((timestamp) => ({
      timestamp,
      // A missing bucket is unknown, not evidence of zero requests.
      llm: llmValues.get(timestamp) ?? null,
      mcp: mcpValues.get(timestamp) ?? null,
    }))
}

export function graphTotal(graph: GraphData | null): number | null {
  return graph
    ? [...graphValues(graph).values()].reduce((sum, value) => sum + value, 0)
    : null
}

const MCP_REQUESTS = new Set([
  'mcp_tool_call',
  'mcp_resource_read',
  'mcp_prompt_get',
  'mcp_sampling',
  'mcp_elicitation',
])

export function activityKind(
  event: MonitorEventSummary,
): 'LLM' | 'MCP' | 'System' {
  if (event.event_type === 'llm_call') return 'LLM'
  if (MCP_REQUESTS.has(event.event_type)) return 'MCP'
  return 'System'
}

export function isRequest(event: MonitorEventSummary): boolean {
  return (
    activityKind(event) !== 'System' &&
    !(event.duplicate_hop && event.duplicate_hop >= 2)
  )
}
