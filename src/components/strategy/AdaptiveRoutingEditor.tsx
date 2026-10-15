import { useEffect, useState } from 'react'
import { Card, CardContent, CardHeader, CardTitle } from '@/components/ui/Card'
import { Button } from '@/components/ui/Button'
import { Input } from '@/components/ui/Input'
import { Label } from '@/components/ui/label'
import { Switch } from '@/components/ui/switch'
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from '@/components/ui/Select'
import type { AdaptiveRoutingConfig, CandidateOrdering } from '@/types/tauri-commands'

export const DEFAULT_ADAPTIVE_ROUTING: AdaptiveRoutingConfig = {
  ordering: 'priority', min_samples: 5, observation_window_secs: 300,
  max_attempts: null, attempt_timeout_ms: null, total_timeout_ms: null,
  infrastructure_cooldown: false,
}

export function AdaptiveRoutingEditor({ value, readOnly, onSave }: {
  value?: AdaptiveRoutingConfig
  readOnly?: boolean
  onSave: (config: AdaptiveRoutingConfig) => Promise<void>
}) {
  const [draft, setDraft] = useState(value ?? DEFAULT_ADAPTIVE_ROUTING)
  const [saving, setSaving] = useState(false)
  const [error, setError] = useState<string | null>(null)
  const savedValue = JSON.stringify(value ?? DEFAULT_ADAPTIVE_ROUTING)
  useEffect(() => setDraft(JSON.parse(savedValue) as AdaptiveRoutingConfig), [savedValue])
  const save = async () => {
    setSaving(true)
    setError(null)
    try { await onSave(draft) } catch (error) { setError(String(error)) } finally { setSaving(false) }
  }
  const optionalNumber = (key: 'max_attempts' | 'attempt_timeout_ms' | 'total_timeout_ms', label: string, min: number, max: number) => (
    <div className="space-y-1"><Label htmlFor={`routing-${key}`}>{label}</Label>
      <Input id={`routing-${key}`} type="number" min={min} max={max} step={1} placeholder="No local limit" value={draft[key] ?? ''}
        disabled={readOnly || saving} onChange={event => setDraft({ ...draft, [key]: event.target.value === '' ? null : Number(event.target.value) })} />
    </div>
  )
  return <Card><CardHeader><CardTitle>Ordering and retries</CardTitle></CardHeader><CardContent className="space-y-4">
    <p className="text-xs text-muted-foreground">Reorder only the models you chose within each route. Measurements come from your local requests. Fallback routes retain their order.</p>
    <div className="space-y-1"><Label>Candidate ordering</Label>
      <Select value={draft.ordering} disabled={readOnly || saving} onValueChange={ordering => setDraft({ ...draft, ordering: ordering as CandidateOrdering })}>
        <SelectTrigger><SelectValue /></SelectTrigger><SelectContent>
          <SelectItem value="priority">Configured priority</SelectItem>
          <SelectItem value="estimated_cost">Lowest estimated cost</SelectItem>
          <SelectItem value="observed_ttft">Lowest average time to first token</SelectItem>
          <SelectItem value="observed_throughput">Highest average output throughput</SelectItem>
          <SelectItem value="least_in_flight">Least in-flight load</SelectItem>
        </SelectContent>
      </Select>
    </div>
    <p className="text-xs text-muted-foreground">Missing prices or insufficient measurements remain unknown. Cost estimates use the current input and token limit, or 1,024 output tokens when absent. Output-speed measurements require completed streams with reported usage.</p>
    <div className="grid gap-3 sm:grid-cols-2">
      <div className="space-y-1"><Label>Minimum observations</Label><Input type="number" min={1} max={128} value={draft.min_samples} disabled={readOnly || saving} onChange={event => setDraft({ ...draft, min_samples: Number(event.target.value) })} /></div>
      <div className="space-y-1"><Label>Observation window (seconds)</Label><Input type="number" min={1} max={3600} value={draft.observation_window_secs} disabled={readOnly || saving} onChange={event => setDraft({ ...draft, observation_window_secs: Number(event.target.value) })} /></div>
      {optionalNumber('max_attempts', 'Maximum model requests', 1, 64)}
      {optionalNumber('attempt_timeout_ms', 'Per-attempt deadline (ms)', 100, 3600000)}
      {optionalNumber('total_timeout_ms', 'Total routing deadline (ms)', 100, 3600000)}
    </div>
    <div className="flex items-center gap-2"><Switch id="routing-cooldown" checked={draft.infrastructure_cooldown} disabled={readOnly || saving} onCheckedChange={infrastructure_cooldown => setDraft({ ...draft, infrastructure_cooldown })} /><Label htmlFor="routing-cooldown">Pause failing providers for bounded recovery</Label></div>
    <p className="text-xs text-muted-foreground">A timeout can leave an upstream charge unknown. Model discovery and classifier calls are separate. Upstream gateways may make their own fallback attempts. Turn off gateway fallback in provider preferences when LocalRouter should manage failover. Streams stop at their deadline and never automatically restart after output.</p>
    {error && <p className="text-sm text-destructive">{error}</p>}
    <Button disabled={readOnly || saving} onClick={save}>{saving ? 'Saving…' : 'Save ordering and limits'}</Button>
  </CardContent></Card>
}
