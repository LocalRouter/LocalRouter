import { profileModelRefs, profilePermissionIssues } from './profile-review'
import { useEffect, useState } from 'react'
import { invoke } from '@tauri-apps/api/core'
import { listenSafe } from '@/hooks/useTauriListener'
import { Card, CardHeader, CardTitle, CardDescription, CardContent } from '@/components/ui/Card'
import { Button } from '@/components/ui/Button'
import { Input } from '@/components/ui/Input'
import { AlertDialog, AlertDialogContent, AlertDialogHeader, AlertDialogTitle, AlertDialogDescription, AlertDialogFooter, AlertDialogCancel, AlertDialogAction } from '@/components/ui/alert-dialog'
import type { RoutingProfilesState, GetRoutingProfilesParams, BindRoutingProfileParams, CreateRoutingProfileParams, DetailedModelInfo } from '@/types/tauri-commands'

export function RoutingProfileBindingEditor({ strategyId, disabled, beforeChange, onUpdate, onEdit }: {
  strategyId: string; disabled?: boolean; beforeChange: () => Promise<void>; onUpdate: () => void; onEdit?: (id: string) => void
}) {
  const [models, setModels] = useState<DetailedModelInfo[]>([])
  const [state, setState] = useState<RoutingProfilesState | null>(null)
  const [selected, setSelected] = useState('')
  const [name, setName] = useState('')
  const [busy, setBusy] = useState(false)
  const [error, setError] = useState<string | null>(null)
  const [confirm, setConfirm] = useState<{ profileId: string; fingerprint?: string; clients: string[] } | null>(null)
  const [refresh, setRefresh] = useState(0)
  useEffect(() => {
    let live = true
    let generation = 0
    let loadedBinding: string | null = null
    setState(null)
    const load = async () => { const request = ++generation; try {
      const [value, catalog] = await Promise.all([invoke<RoutingProfilesState>('get_routing_profiles', { strategyId } satisfies GetRoutingProfilesParams), invoke<DetailedModelInfo[]>('list_all_models_detailed')])
      if (live && request === generation) { setState(value); setModels(catalog); const binding = value.strategy?.routing_profile?.profile_id ?? ''; if (binding !== loadedBinding) setSelected(binding); loadedBinding = binding; setError(null) }
    } catch (e) { if (live && request === generation) setError(String(e)) } }
    const listeners = ['strategies-changed', 'routing-profiles-changed'].map(event => listenSafe(event, load))
    void Promise.all(listeners.map(l => l.promise)).then(() => { if (live) void load() })
    return () => { live = false; listeners.forEach(l => l.cleanup()) }
  }, [strategyId, refresh])
  const current = state?.strategy?.routing_profile?.profile_id ?? ''
  const bound = state?.profiles.find(p => p.profile.id === current)
  const target = state?.profiles.find(p => p.profile.id === selected)
  const attachIssues = target && state?.strategy ? [...profilePermissionIssues(target.profile.active, state.strategy.model_permissions), ...profileModelRefs(target.profile.active).filter(([p, id]) => !models.some(m => m.provider_instance === p && m.model_id === id)).map(([p, id]) => `Unavailable in the current model catalog: ${p}/${id}`)] : []
  async function attach() {
    if (!state?.strategy_fingerprint || !confirm) return
    const reviewed = confirm
    setBusy(true); setError(null)
    try {
      await beforeChange()
      const fresh = await invoke<RoutingProfilesState>('get_routing_profiles', { strategyId } satisfies GetRoutingProfilesParams)
      const next = fresh.profiles.find(p => p.profile.id === reviewed.profileId)
      if (reviewed.profileId && (!next || next.fingerprint !== reviewed.fingerprint)) throw new Error('Profile changed. Reload and review before attaching.')
      if (JSON.stringify(fresh.strategy_clients.map(c => c.client_id)) !== JSON.stringify(reviewed.clients)) throw new Error('Clients using this strategy changed. Reload and review before applying.')
      const value = await invoke<RoutingProfilesState>('bind_routing_profile', { strategyId, profileId: reviewed.profileId || null, expectedStrategyFingerprint: fresh.strategy_fingerprint!, expectedProfileFingerprint: next?.fingerprint } satisfies BindRoutingProfileParams)
      setState(value); onUpdate()
    } catch (e) { setError(String(e)) } finally { setBusy(false); setConfirm(null) }
  }
  async function create() {
    setBusy(true); setError(null)
    try {
      await beforeChange()
      const fresh = await invoke<RoutingProfilesState>('get_routing_profiles', { strategyId } satisfies GetRoutingProfilesParams)
      const value = await invoke<RoutingProfilesState>('create_routing_profile', { name: name.trim(), sourceStrategyId: strategyId, expectedFingerprint: fresh.strategy_fingerprint } satisfies CreateRoutingProfileParams)
      setState(value); setName(''); setSelected(value.profiles[value.profiles.length - 1].profile.id)
    } catch (e) { setError(String(e)) } finally { setBusy(false) }
  }
  return <Card><CardHeader><CardTitle className="text-base">Routing profile</CardTitle><CardDescription>Share routing choices between clients. Model access, rate limits and free / paid fallback controls stay on each strategy.</CardDescription></CardHeader><CardContent className="space-y-3">
    {!state && !error && <p className="text-sm text-muted-foreground">Loading profiles…</p>}
    {state && <>
      <div className="flex flex-wrap items-center gap-2"><select aria-label="Routing profile" className="rounded-md border bg-background p-2 text-sm" value={selected} disabled={busy || disabled} onChange={e => setSelected(e.target.value)}><option value="">Custom routing (independent)</option>{state.profiles.map(p => <option key={p.profile.id} value={p.profile.id}>{p.profile.name} · revision {p.profile.versions.active_revision}</option>)}</select>
        <Button variant="outline" disabled={busy || disabled || selected === current} onClick={() => setConfirm({ profileId: selected, fingerprint: target?.fingerprint, clients: state.strategy_clients.map(c => c.client_id) })}>Apply profile</Button>
        {bound && <><Button variant="outline" disabled={busy || disabled} onClick={() => { setSelected(''); setConfirm({ profileId: '', clients: state.strategy_clients.map(c => c.client_id) }) }}>Make independent copy</Button><Button variant="outline" onClick={() => onEdit?.(bound.profile.id)}>Edit shared profile</Button></>}
      </div>
      <p className="text-xs text-muted-foreground">{state.strategy?.free_tier_only ? `Free-first routing; paid fallback: ${state.strategy.free_tier_fallback === 'allow' ? 'allowed' : state.strategy.free_tier_fallback === 'ask' ? 'ask before paid' : 'off (free only)'}.` : 'Free-only mode is off; permitted paid models may be used.'} Change spending controls below.</p>
      {bound && <p className="text-sm text-muted-foreground">Using {bound.profile.name}, revision {state.strategy?.routing_profile?.revision}. Published routing changes apply to all linked clients.</p>}
      {state.strategy_clients.length > 1 && <p className="text-sm text-muted-foreground">This existing strategy is shared by {state.strategy_clients.map(c => c.client_name).join(', ')}. Changing its binding affects all of them.</p>}
      {state.strategy_clients.flatMap(c => c.issues.map(issue => <p key={`${c.client_id}-${issue}`} className="text-xs text-yellow-600">{c.client_name}: {issue}</p>))}
      <div className="flex flex-wrap gap-2"><Input aria-label="New routing profile name" placeholder="e.g. Coding or Chatbot" value={name} onChange={e => setName(e.target.value)} className="max-w-xs" maxLength={128} /><Button variant="outline" disabled={busy || disabled || !name.trim()} onClick={() => void create()}>Create profile from current routing</Button></div>
      <p className="text-xs text-muted-foreground">Creating a profile copies routing choices. Select and apply it to link clients. Attaching a profile does not grant access to its models or enable paid fallback.</p>
    </>}
    {error && <div role="alert" className="text-sm text-destructive">{error}<Button variant="outline" size="sm" className="ml-2" onClick={() => setRefresh(n => n + 1)}>Reload</Button></div>}
    <AlertDialog open={confirm !== null} onOpenChange={open => { if (!open) setConfirm(null) }}><AlertDialogContent><AlertDialogHeader><AlertDialogTitle>{selected ? `Use ${target?.profile.name ?? 'shared routing'}?` : 'Make an independent copy?'}</AlertDialogTitle><AlertDialogDescription>{selected ? 'Routing models and rules will follow this profile. Existing model access, aliases, spending choices and limits are preserved. Some models may be blocked by those settings.' : 'Keep the current effective routing and stop following profile publications.'} Affected clients: {state?.strategy_clients.map(c => c.client_name).join(', ') || 'none'}.</AlertDialogDescription></AlertDialogHeader>{confirm?.profileId && <div className="max-h-48 overflow-auto text-xs space-y-1">{attachIssues.map(issue => <p className="text-yellow-600" key={issue}>{issue}</p>)}</div>}<AlertDialogFooter><AlertDialogCancel>Cancel</AlertDialogCancel><AlertDialogAction onClick={e => { e.preventDefault(); void attach() }} disabled={busy}>Apply</AlertDialogAction></AlertDialogFooter></AlertDialogContent></AlertDialog>
  </CardContent></Card>
}
