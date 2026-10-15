import { ArrowUp, ArrowDown, Plus, Trash2 } from 'lucide-react'
import { Button } from '@/components/ui/Button'
import { Input } from '@/components/ui/Input'
import { Label } from '@/components/ui/label'
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from '@/components/ui/Select'
import type {
  RouteCondition,
  RouteRule,
  RoutingOption,
} from '@/types/tauri-commands'
const emptyCondition = (): RouteCondition => ({
  client_ids: [],
  has_tools: null,
  has_images: null,
  min_context_chars: null,
  max_context_chars: null,
  max_estimated_cost_usd: null,
})
export function RouteRulesEditor({
  value,
  options,
  onChange,
  disabled,
}: {
  value: RouteRule[]
  options: RoutingOption[]
  onChange: (rules: RouteRule[]) => void
  disabled?: boolean
}) {
  const edit = (i: number, patch: Partial<RouteRule>) =>
    onChange(value.map((rule, j) => (i === j ? { ...rule, ...patch } : rule)))
  const condition = (i: number, patch: Partial<RouteCondition>) =>
    edit(i, { condition: { ...value[i].condition, ...patch } })
  const move = (i: number, delta: number) => {
    const next = [...value]
    ;[next[i], next[i + delta]] = [next[i + delta], next[i]]
    onChange(next)
  }
  return (
    <div className="space-y-3">
      <Label>Deterministic rules</Label>
      <p className="text-xs text-muted-foreground">
        Exact mode rules run first, then the first matching rule below, then
        semantic/default routing. Client IDs come from authentication. Text size
        counts message text characters. Cost conditions require known estimates
        for every eligible fallback; they do not replace spending limits.
      </p>
      {value.map((rule, i) => (
        <div key={i} className="space-y-3 rounded-md border p-3">
          <div className="flex gap-2">
            <Input
              aria-label={`Rule ${i + 1} ID`}
              value={rule.id}
              onChange={(e) => edit(i, { id: e.target.value })}
              disabled={disabled}
            />
            <Button
              variant="ghost"
              size="icon"
              aria-label="Move rule up"
              disabled={disabled || i === 0}
              onClick={() => move(i, -1)}
            >
              <ArrowUp className="h-4 w-4" />
            </Button>
            <Button
              variant="ghost"
              size="icon"
              aria-label="Move rule down"
              disabled={disabled || i === value.length - 1}
              onClick={() => move(i, 1)}
            >
              <ArrowDown className="h-4 w-4" />
            </Button>
            <Button
              variant="ghost"
              size="icon"
              aria-label="Remove rule"
              disabled={disabled}
              onClick={() => onChange(value.filter((_, j) => i !== j))}
            >
              <Trash2 className="h-4 w-4" />
            </Button>
          </div>
          <div className="space-y-1">
            <Label>
              Client IDs (comma separated; empty matches any client)
            </Label>
            <Input
              key={JSON.stringify(rule.condition.client_ids)}
              defaultValue={rule.condition.client_ids.join(', ')}
              disabled={disabled}
              onBlur={(e) =>
                condition(i, {
                  client_ids: e.target.value
                    .split(',')
                    .map((id) => id.trim())
                    .filter(Boolean),
                })
              }
            />
          </div>
          <div className="grid gap-2 sm:grid-cols-2">
            {(['has_tools', 'has_images'] as const).map((key) => (
              <div key={key} className="space-y-1">
                <Label>
                  {key === 'has_tools' ? 'Tools requested' : 'Images present'}
                </Label>
                <Select
                  value={
                    rule.condition[key] === null
                      ? 'any'
                      : String(rule.condition[key])
                  }
                  disabled={disabled}
                  onValueChange={(v) =>
                    condition(i, { [key]: v === 'any' ? null : v === 'true' })
                  }
                >
                  <SelectTrigger>
                    <SelectValue />
                  </SelectTrigger>
                  <SelectContent>
                    <SelectItem value="any">Any</SelectItem>
                    <SelectItem value="true">Yes</SelectItem>
                    <SelectItem value="false">No</SelectItem>
                  </SelectContent>
                </Select>
              </div>
            ))}
          </div>
          <div className="grid gap-2 sm:grid-cols-3">
            {(
              [
                'min_context_chars',
                'max_context_chars',
                'max_estimated_cost_usd',
              ] as const
            ).map((key) => (
              <div key={key} className="space-y-1">
                <Label>
                  {key === 'min_context_chars'
                    ? 'Minimum text characters'
                    : key === 'max_context_chars'
                      ? 'Maximum text characters'
                      : 'Maximum estimated USD'}
                </Label>
                <Input
                  type="number"
                  min="0"
                  step={key === 'max_estimated_cost_usd' ? 'any' : '1'}
                  value={rule.condition[key] ?? ''}
                  disabled={disabled}
                  onChange={(e) =>
                    condition(i, {
                      [key]:
                        e.target.value === '' ? null : Number(e.target.value),
                    })
                  }
                />
              </div>
            ))}
          </div>
          <p className="text-xs text-muted-foreground">
            Branch weights total 10,000 (100%). Splits use a stable
            client/session assignment and make one normal routed request.
          </p>
          {rule.branches.map((branch, j) => (
            <div key={j} className="flex gap-2">
              <Select
                value={branch.route}
                disabled={disabled}
                onValueChange={(route) =>
                  edit(i, {
                    branches: rule.branches.map((b, k) =>
                      j === k ? { ...b, route } : b,
                    ),
                  })
                }
              >
                <SelectTrigger aria-label="Branch route">
                  <SelectValue />
                </SelectTrigger>
                <SelectContent>
                  {options.map((option) => (
                    <SelectItem key={option.id} value={option.id}>
                      {option.id}
                    </SelectItem>
                  ))}
                </SelectContent>
              </Select>
              <Input
                aria-label="Branch weight"
                type="number"
                min="1"
                max="10000"
                step="1"
                value={branch.weight}
                disabled={disabled}
                onChange={(e) =>
                  edit(i, {
                    branches: rule.branches.map((b, k) =>
                      j === k ? { ...b, weight: Number(e.target.value) } : b,
                    ),
                  })
                }
              />
              <Button
                variant="ghost"
                size="icon"
                aria-label="Remove branch"
                disabled={disabled || rule.branches.length === 1}
                onClick={() =>
                  edit(i, { branches: rule.branches.filter((_, k) => j !== k) })
                }
              >
                <Trash2 className="h-4 w-4" />
              </Button>
            </div>
          ))}
          <Button
            variant="outline"
            size="sm"
            disabled={disabled || rule.branches.length >= 16}
            onClick={() =>
              edit(i, {
                branches: [
                  ...rule.branches,
                  { route: options[0]?.id ?? '', weight: 1 },
                ],
              })
            }
          >
            Add branch
          </Button>
        </div>
      ))}
      <Button
        variant="outline"
        size="sm"
        disabled={disabled || value.length >= 64 || !options.length}
        onClick={() => {
          let n = value.length + 1
          while (value.some((rule) => rule.id === `rule_${n}`)) n++
          onChange([
            ...value,
            {
              id: `rule_${n}`,
              condition: emptyCondition(),
              branches: [{ route: options[0].id, weight: 10000 }],
            },
          ])
        }}
      >
        <Plus className="mr-2 h-4 w-4" />
        Add rule
      </Button>
    </div>
  )
}
