import { useState, useEffect } from 'react'
import { invoke } from '@tauri-apps/api/core'
import { Button } from '@/components/ui/Button'
import { Input } from '@/components/ui/Input'
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from '@/components/ui/Select'
import { Popover, PopoverTrigger, PopoverContent } from '@/components/ui/popover'
import { Checkbox } from '@/components/ui/checkbox'
import { Separator } from '@/components/ui/separator'
import { Trash2, Search, Crosshair } from 'lucide-react'
import { InfoTooltip } from '@/components/ui/info-tooltip'
import type { MonitorEventFilter, MonitorEventType, InterceptCategory, InterceptRule, ClientInfo } from '@/types/tauri-commands'

interface EventFiltersProps {
  filter: MonitorEventFilter
  onFilterChange: (filter: MonitorEventFilter) => void
  onClear: () => void
  interceptRule: InterceptRule | null
  onInterceptRuleChange: (rule: InterceptRule | null) => void
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

const ALL_INTERCEPT_CATEGORIES: { label: string; value: InterceptCategory }[] = [
  { label: 'LLM', value: 'llm' },
  { label: 'MCP Tools', value: 'mcp' },
  { label: 'Skills', value: 'skill' },
  { label: 'Marketplace', value: 'marketplace' },
  { label: 'Coding Agent', value: 'coding_agent' },
  { label: 'Guardrails', value: 'guardrails' },
  { label: 'Secret Scan', value: 'secret_scan' },
  { label: 'Sampling', value: 'sampling' },
  { label: 'Elicitation', value: 'elicitation' },
]

const ALL_CATEGORY_VALUES = ALL_INTERCEPT_CATEGORIES.map(c => c.value)

export function EventFilters({ filter, onFilterChange, onClear, interceptRule, onInterceptRuleChange }: EventFiltersProps) {
  const [categories, setCategories] = useState<InterceptCategory[]>(ALL_CATEGORY_VALUES)
  const [clientIds, setClientIds] = useState<string[]>([])
  const [clients, setClients] = useState<ClientInfo[]>([])
  const [popoverOpen, setPopoverOpen] = useState(false)

  // Load clients when popover opens
  useEffect(() => {
    if (popoverOpen) {
      invoke<ClientInfo[]>('list_clients')
        .then(setClients)
        .catch(() => setClients([]))
    }
  }, [popoverOpen])

  // Sync local state when rule changes externally
  useEffect(() => {
    if (interceptRule) {
      setCategories(interceptRule.categories)
      setClientIds(interceptRule.client_ids)
    }
  }, [interceptRule])

  const activeGroup = filter.event_types
    ? Object.entries(typeGroupMap).find(([, types]) =>
        types.length === filter.event_types!.length &&
        types.every(t => filter.event_types!.includes(t))
      )?.[0] ?? 'custom'
    : 'all'

  const toggleCategory = (value: InterceptCategory) => {
    setCategories(prev =>
      prev.includes(value) ? prev.filter(c => c !== value) : [...prev, value]
    )
  }

  const toggleClient = (id: string) => {
    setClientIds(prev =>
      prev.includes(id) ? prev.filter(c => c !== id) : [...prev, id]
    )
  }

  const allCategoriesSelected = categories.length === ALL_CATEGORY_VALUES.length
  const someCategoriesSelected = categories.length > 0 && !allCategoriesSelected

  const enabledClients = clients.filter(c => c.enabled)
  const allClientsSelected = clientIds.length === 0
  const someClientsSelected = clientIds.length > 0 && clientIds.length < enabledClients.length

  const handleStartIntercept = () => {
    if (categories.length === 0) return
    onInterceptRuleChange({ categories, client_ids: clientIds })
    setPopoverOpen(false)
  }

  const handleStopIntercept = () => {
    onInterceptRuleChange(null)
    setPopoverOpen(false)
  }

  const interceptLabel = interceptRule
    ? `Intercepting (${interceptRule.categories.length})`
    : 'Intercept'

  return (
    <div className="flex items-center gap-2 p-2">
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

      <Popover open={popoverOpen} onOpenChange={setPopoverOpen}>
        <PopoverTrigger asChild>
          <Button
            variant={interceptRule ? 'destructive' : 'ghost'}
            size="sm"
            className="h-7 text-xs gap-1"
          >
            <Crosshair className="h-3 w-3" />
            {interceptLabel}
          </Button>
        </PopoverTrigger>
        <PopoverContent className="w-64 p-3" align="end">
          <div className="space-y-3">
            <div className="text-xs font-medium flex items-center gap-1">
              Intercept Requests
              <InfoTooltip content="Forces the firewall popup for matching requests, letting you inspect and approve/deny them in real time." />
            </div>
            <div className="text-[11px] text-muted-foreground">
              Force firewall popup for matching requests regardless of permissions.
            </div>

            {/* Category checkboxes */}
            <div className="space-y-1">
              <label className="text-[11px] text-muted-foreground font-medium">Types</label>
              <div className="flex items-center gap-2 py-0.5">
                <Checkbox
                  id="cat-all"
                  checked={allCategoriesSelected ? true : someCategoriesSelected ? 'indeterminate' : false}
                  onCheckedChange={(checked) => {
                    setCategories(checked ? [...ALL_CATEGORY_VALUES] : [])
                  }}
                  className="h-3.5 w-3.5"
                />
                <label htmlFor="cat-all" className="text-xs cursor-pointer">Select All</label>
              </div>
              {ALL_INTERCEPT_CATEGORIES.map(c => (
                <div key={c.value} className="flex items-center gap-2 py-0.5 pl-2">
                  <Checkbox
                    id={`cat-${c.value}`}
                    checked={categories.includes(c.value)}
                    onCheckedChange={() => toggleCategory(c.value)}
                    className="h-3.5 w-3.5"
                  />
                  <label htmlFor={`cat-${c.value}`} className="text-xs cursor-pointer">{c.label}</label>
                </div>
              ))}
            </div>

            <Separator />

            {/* Client checkboxes */}
            <div className="space-y-1">
              <label className="text-[11px] text-muted-foreground font-medium">Clients</label>
              <div className="flex items-center gap-2 py-0.5">
                <Checkbox
                  id="client-all"
                  checked={allClientsSelected ? true : someClientsSelected ? 'indeterminate' : false}
                  onCheckedChange={(checked) => {
                    setClientIds(checked ? [] : enabledClients.length > 0 ? [enabledClients[0].id] : [])
                  }}
                  className="h-3.5 w-3.5"
                />
                <label htmlFor="client-all" className="text-xs cursor-pointer">All Clients</label>
              </div>
              {enabledClients.map(c => (
                <div key={c.id} className="flex items-center gap-2 py-0.5 pl-2">
                  <Checkbox
                    id={`client-${c.id}`}
                    checked={allClientsSelected || clientIds.includes(c.id)}
                    onCheckedChange={() => {
                      if (allClientsSelected) {
                        // Switch from "all" to "only this one"
                        setClientIds([c.id])
                      } else {
                        toggleClient(c.id)
                      }
                    }}
                    className="h-3.5 w-3.5"
                  />
                  <label htmlFor={`client-${c.id}`} className="text-xs cursor-pointer">{c.name}</label>
                </div>
              ))}
            </div>

            {/* Toggle button */}
            {interceptRule ? (
              <Button
                variant="outline"
                size="sm"
                className="w-full h-7 text-xs"
                onClick={handleStopIntercept}
              >
                Stop Intercepting
              </Button>
            ) : (
              <Button
                variant="default"
                size="sm"
                className="w-full h-7 text-xs"
                onClick={handleStartIntercept}
                disabled={categories.length === 0}
              >
                Start Intercepting
              </Button>
            )}
          </div>
        </PopoverContent>
      </Popover>

      <Button variant="ghost" size="sm" className="h-7 text-xs" onClick={onClear}>
        <Trash2 className="h-3 w-3 mr-1" />
        Clear
      </Button>
    </div>
  )
}
