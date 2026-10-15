import { useEffect, useState } from 'react'
import { Card, CardContent, CardHeader, CardTitle } from '@/components/ui/Card'
import { Button } from '@/components/ui/Button'
import { Input } from '@/components/ui/Input'
import { Label } from '@/components/ui/label'
import { Switch } from '@/components/ui/switch'
import type { SessionRoutingConfig } from '@/types/tauri-commands'
const DEFAULT: SessionRoutingConfig = { enabled: false, classifier_reuse: false, forward_cache_key: false, ttl_secs: 900 }
export function SessionRoutingEditor({ value, readOnly, onSave }: {
  value?: SessionRoutingConfig
  readOnly?: boolean
  onSave: (config: SessionRoutingConfig) => Promise<void>
}) {
  const saved = JSON.stringify(value ?? DEFAULT)
  const [draft, setDraft] = useState(value ?? DEFAULT)
  const [saving, setSaving] = useState(false)
  const [error, setError] = useState<string | null>(null)
  useEffect(() => setDraft(JSON.parse(saved) as SessionRoutingConfig), [saved])
  const toggle = (key: 'enabled' | 'classifier_reuse' | 'forward_cache_key', label: string) => <div className="flex items-center gap-2">
    <Switch id={`session-${key}`} disabled={readOnly || saving} checked={draft[key]} onCheckedChange={checked => setDraft({ ...draft, [key]: checked })} />
    <Label htmlFor={`session-${key}`}>{label}</Label>
  </div>
  return <Card><CardHeader><CardTitle>Session routing</CardTitle></CardHeader><CardContent className="space-y-4">
    <p className="text-xs text-muted-foreground">Clients can send x-session-id or extensions.localrouter.session.id. Pins are private to each client and policy, stay in their route group, and expire on restart. Permissions and limits are checked every time.</p>
    {toggle('enabled', 'Prefer the same provider and model within a conversation')}
    {toggle('classifier_reuse', 'Reuse classification for an unchanged routing context')}
    {toggle('forward_cache_key', 'Send a private cache routing key to supported providers')}
    <div className="space-y-1"><Label htmlFor="session-ttl">Session idle expiry (seconds)</Label><Input id="session-ttl" disabled={readOnly || saving} type="number" min={1} max={3600} value={draft.ttl_secs} onChange={event => setDraft({ ...draft, ttl_secs: Number(event.target.value) })} /></div>
    <p className="text-xs text-muted-foreground">Set reset or reclassify in the session options to start over or reconsider the route. Use require_continuity for provider-bound state: missing or changed pins require a new conversation. Temporary limits still apply. A local pin selects a LocalRouter instance; upstream gateway provider selection follows its own routing controls. Prompt caching reuses provider computation; it does not replay a stored answer.</p>
    {error && <p className="text-sm text-destructive">{error}</p>}
    <Button disabled={readOnly || saving} onClick={async () => { setSaving(true); setError(null); try { await onSave(draft) } catch (error) { setError(String(error)) } finally { setSaving(false) } }}>{saving ? 'Saving…' : 'Save session routing'}</Button>
  </CardContent></Card>
}
