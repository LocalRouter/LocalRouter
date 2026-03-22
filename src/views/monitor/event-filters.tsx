import { useState, useEffect } from 'react'
import { invoke } from '@tauri-apps/api/core'
import { Button } from '@/components/ui/Button'
import { Input } from '@/components/ui/Input'
import { Popover, PopoverTrigger, PopoverContent } from '@/components/ui/popover'
import { Checkbox } from '@/components/ui/checkbox'
import { Separator } from '@/components/ui/separator'
import { Trash2, Search, Crosshair, Filter } from 'lucide-react'
import { InfoTooltip } from '@/components/ui/info-tooltip'
import type { MonitorEventFilter, MonitorEventType, InterceptCategory, InterceptRule, ClientInfo } from '@/types/tauri-commands'

interface EventFiltersProps {
  filter: MonitorEventFilter
  onFilterChange: (filter: MonitorEventFilter) => void
  onClear: () => void
  interceptRule: InterceptRule | null
  onInterceptRuleChange: (rule: InterceptRule | null) => void
}

const TYPE_GROUPS: { key: string; label: string; types: MonitorEventType[] }[] = [
  { key: 'llm', label: 'LLM', types: ['llm_call'] },
  { key: 'mcp', label: 'MCP', types: [
    'mcp_tool_call', 'mcp_resource_read', 'mcp_prompt_get',
    'mcp_elicitation', 'mcp_sampling', 'mcp_server_event',
  ]},
  { key: 'auth', label: 'Auth & Access', types: ['auth_error', 'access_denied', 'oauth_event'] },
  { key: 'security', label: 'Security', types: ['guardrail_scan', 'guardrail_response_scan', 'secret_scan'] },
  { key: 'routing', label: 'Routing', types: ['route_llm_classify', 'routing_decision'] },
  { key: 'errors', label: 'Errors', types: [
    'rate_limit_event', 'validation_error', 'internal_error',
    'moderation_event', 'connection_error',
  ]},
  { key: 'memory', label: 'Memory', types: ['memory_compaction'] },
  { key: 'other', label: 'Other', types: ['prompt_compression', 'firewall_decision', 'sse_connection'] },
]

const ALL_TYPE_GROUP_KEYS = TYPE_GROUPS.map(g => g.key)

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
  // Type filter state
  const [selectedGroups, setSelectedGroups] = useState<string[]>(ALL_TYPE_GROUP_KEYS)
  const [typeFilterOpen, setTypeFilterOpen] = useState(false)

  // Intercept state
  const [categories, setCategories] = useState<InterceptCategory[]>(ALL_CATEGORY_VALUES)
  const [clientIds, setClientIds] = useState<string[]>([])
  const [clients, setClients] = useState<ClientInfo[]>([])
  const [interceptPopoverOpen, setInterceptPopoverOpen] = useState(false)

  // Load clients when intercept popover opens
  useEffect(() => {
    if (interceptPopoverOpen) {
      invoke<ClientInfo[]>('list_clients')
        .then(setClients)
        .catch(() => setClients([]))
    }
  }, [interceptPopoverOpen])

  // Sync local state when rule changes externally
  useEffect(() => {
    if (interceptRule) {
      setCategories(interceptRule.categories)
      setClientIds(interceptRule.client_ids)
    }
  }, [interceptRule])

  // Apply type filter when selected groups change
  useEffect(() => {
    const allSelected = selectedGroups.length === ALL_TYPE_GROUP_KEYS.length
    if (allSelected) {
      onFilterChange({ ...filter, event_types: null })
    } else if (selectedGroups.length === 0) {
      onFilterChange({ ...filter, event_types: [] })
    } else {
      const types = TYPE_GROUPS
        .filter(g => selectedGroups.includes(g.key))
        .flatMap(g => g.types)
      onFilterChange({ ...filter, event_types: types })
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [selectedGroups])

  const toggleGroup = (key: string) => {
    setSelectedGroups(prev =>
      prev.includes(key) ? prev.filter(k => k !== key) : [...prev, key]
    )
  }

  const allGroupsSelected = selectedGroups.length === ALL_TYPE_GROUP_KEYS.length
  const someGroupsSelected = selectedGroups.length > 0 && !allGroupsSelected

  const typeFilterLabel = allGroupsSelected
    ? 'All Events'
    : selectedGroups.length === 0
      ? 'No Events'
      : selectedGroups.length === 1
        ? TYPE_GROUPS.find(g => g.key === selectedGroups[0])!.label
        : `${selectedGroups.length} types`

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
    setInterceptPopoverOpen(false)
  }

  const handleStopIntercept = () => {
    onInterceptRuleChange(null)
    setInterceptPopoverOpen(false)
  }

  const interceptLabel = interceptRule
    ? `Intercepting (${interceptRule.categories.length})`
    : 'Intercept'

  return (
    <div className="flex items-center gap-2 p-2">
      <Popover open={typeFilterOpen} onOpenChange={setTypeFilterOpen}>
        <PopoverTrigger asChild>
          <Button
            variant={allGroupsSelected ? 'ghost' : 'secondary'}
            size="sm"
            className="h-7 text-xs gap-1"
          >
            <Filter className="h-3 w-3" />
            {typeFilterLabel}
          </Button>
        </PopoverTrigger>
        <PopoverContent className="w-48 p-3" align="start">
          <div className="space-y-1">
            <div className="flex items-center gap-2 py-0.5">
              <Checkbox
                id="type-all"
                checked={allGroupsSelected ? true : someGroupsSelected ? 'indeterminate' : false}
                onCheckedChange={(checked) => {
                  setSelectedGroups(checked ? [...ALL_TYPE_GROUP_KEYS] : [])
                }}
                className="h-3.5 w-3.5"
              />
              <label htmlFor="type-all" className="text-xs cursor-pointer">All Events</label>
            </div>
            {TYPE_GROUPS.map(g => (
              <div key={g.key} className="flex items-center gap-2 py-0.5 pl-2">
                <Checkbox
                  id={`type-${g.key}`}
                  checked={selectedGroups.includes(g.key)}
                  onCheckedChange={() => toggleGroup(g.key)}
                  className="h-3.5 w-3.5"
                />
                <label htmlFor={`type-${g.key}`} className="text-xs cursor-pointer">{g.label}</label>
              </div>
            ))}
          </div>
        </PopoverContent>
      </Popover>

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

      <Popover open={interceptPopoverOpen} onOpenChange={setInterceptPopoverOpen}>
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
