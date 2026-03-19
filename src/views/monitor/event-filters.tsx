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
  llm: ['llm_call'],
  mcp: [
    'mcp_tool_call', 'mcp_resource_read', 'mcp_prompt_get',
    'mcp_elicitation', 'mcp_sampling', 'mcp_server_event',
  ],
  auth: ['auth_error', 'access_denied', 'oauth_event'],
  security: ['guardrail_scan', 'guardrail_response_scan', 'secret_scan'],
  routing: ['route_llm_classify', 'routing_decision'],
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
