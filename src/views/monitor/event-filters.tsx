import { Button } from '@/components/ui/Button'
import { Input } from '@/components/ui/Input'
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from '@/components/ui/Select'
import { Trash2, Search } from 'lucide-react'
import type { MonitorEventFilter, MonitorEventType } from '@/types/tauri-commands'

interface EventFiltersProps {
  filter: MonitorEventFilter
  onFilterChange: (filter: MonitorEventFilter) => void
  onClear: () => void
}

const typeGroups = [
  { label: 'All Events', value: 'all' },
  { label: 'LLM', value: 'llm' },
  { label: 'MCP', value: 'mcp' },
  { label: 'Auth & Access', value: 'auth' },
  { label: 'Security', value: 'security' },
  { label: 'Routing', value: 'routing' },
  { label: 'Errors', value: 'errors' },
  { label: 'Other', value: 'other' },
]

const typeGroupMap: Record<string, MonitorEventType[]> = {
  llm: ['llm_request', 'llm_request_transformed', 'llm_response', 'llm_error'],
  mcp: [
    'mcp_tool_call', 'mcp_tool_response',
    'mcp_resource_read', 'mcp_resource_response',
    'mcp_prompt_get', 'mcp_prompt_response',
    'mcp_elicitation_request', 'mcp_elicitation_response',
    'mcp_sampling_request', 'mcp_sampling_response',
    'mcp_server_event',
  ],
  auth: [
    'auth_error', 'access_denied', 'oauth_event',
  ],
  security: [
    'guardrail_request', 'guardrail_response',
    'guardrail_response_check_request', 'guardrail_response_check_response',
    'secret_scan_request', 'secret_scan_response',
  ],
  routing: ['route_llm_request', 'route_llm_response', 'routing_decision'],
  errors: [
    'rate_limit_event', 'validation_error', 'internal_error',
    'moderation_event', 'connection_error',
  ],
  other: ['prompt_compression', 'firewall_decision', 'sse_connection'],
}

export function EventFilters({ filter, onFilterChange, onClear }: EventFiltersProps) {
  const activeGroup = filter.event_types
    ? Object.entries(typeGroupMap).find(([, types]) =>
        types.length === filter.event_types!.length &&
        types.every(t => filter.event_types!.includes(t))
      )?.[0] ?? 'custom'
    : 'all'

  return (
    <div className="flex items-center gap-2 p-2 border-b">
      <Select
        value={activeGroup}
        onValueChange={(value) => {
          if (value === 'all') {
            onFilterChange({ ...filter, event_types: null })
          } else if (value in typeGroupMap) {
            onFilterChange({ ...filter, event_types: typeGroupMap[value] })
          }
        }}
      >
        <SelectTrigger className="w-[130px] h-7 text-xs">
          <SelectValue />
        </SelectTrigger>
        <SelectContent>
          {typeGroups.map(g => (
            <SelectItem key={g.value} value={g.value} className="text-xs">
              {g.label}
            </SelectItem>
          ))}
        </SelectContent>
      </Select>

      <div className="relative flex-1 max-w-[200px]">
        <Search className="absolute left-2 top-1/2 -translate-y-1/2 h-3 w-3 text-muted-foreground" />
        <Input
          placeholder="Search..."
          value={filter.search ?? ''}
          onChange={(e) => onFilterChange({ ...filter, search: e.target.value || null })}
          className="h-7 text-xs pl-7"
        />
      </div>

      <div className="flex-1" />

      <Button variant="ghost" size="sm" className="h-7 text-xs" onClick={onClear}>
        <Trash2 className="h-3 w-3 mr-1" />
        Clear
      </Button>
    </div>
  )
}
