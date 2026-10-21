import { profileModelRefs } from './profile-review'
import { useEffect, useState } from 'react'
import { invoke } from '@tauri-apps/api/core'
import { listenSafe } from '@/hooks/useTauriListener'
import { Card, CardContent, CardHeader, CardTitle, CardDescription } from '@/components/ui/Card'
import { Button } from '@/components/ui/Button'
import { Input } from '@/components/ui/Input'
import { AlertDialog, AlertDialogContent, AlertDialogHeader, AlertDialogTitle, AlertDialogDescription, AlertDialogFooter, AlertDialogCancel, AlertDialogAction } from '@/components/ui/alert-dialog'
import { RoutingPolicyEditor } from './RoutingPolicyEditor'
import { PolicyModelPicker, PolicyModelLabel } from './PolicyModelPicker'
import { AdaptiveRoutingEditor } from './AdaptiveRoutingEditor'
import { SessionRoutingEditor } from './SessionRoutingEditor'
import type { RoutingProfilesState, RoutingProfileStatus, RoutingProfileAction, RouteSnapshot, GetRoutingProfilesParams, CreateRoutingProfileParams, MutateRoutingProfileParams, DetailedModelInfo, FreeTierKind, ProviderFreeTierStatus } from '@/types/tauri-commands'

export function RoutingProfilesManager({ selectedId, onSelect, onClient }: { selectedId: string | null; onSelect: (id: string) => void; onClient: (id: string) => void }) {
  const [state, setState] = useState<RoutingProfilesState | null>(null)
  const [editing, setEditing] = useState<RoutingProfileStatus | null>(null)
  const [name, setName] = useState('')
  const [busy, setBusy] = useState(false)
  const [error, setError] = useState<string | null>(null)
  const [refresh, setRefresh] = useState(0)
  useEffect(() => {
    let live = true
    let generation = 0
    const load = async () => { const request = ++generation; try {
      const value = await invoke<RoutingProfilesState>('get_routing_profiles', {} satisfies GetRoutingProfilesParams)
      if (live && request === generation) { setState(value); setError(null) }
    } catch (e) { if (live && request === generation) setError(String(e)) } }
    const listeners = ['routing-profiles-changed', 'strategies-changed', 'clients-changed', 'providers-changed'].map(event => listenSafe(event, load))
    void Promise.all(listeners.map(l => l.promise)).then(() => { if (live) void load() })
    return () => { live = false; listeners.forEach(l => l.cleanup()) }
  }, [refresh])
  const latest = state?.profiles.find(p => p.profile.id === selectedId)
  const selected = editing?.profile.id === selectedId ? editing : latest
  useEffect(() => { if (latest && editing?.profile.id !== selectedId) setEditing(latest) }, [selectedId, latest, editing?.profile.id])
  async function create(source?: RoutingProfileStatus) {
    setBusy(true); setError(null)
    try {
      const value = await invoke<RoutingProfilesState>('create_routing_profile', { name: source ? `${source.profile.name.slice(0, 120)} copy` : name.trim(), sourceProfileId: source?.profile.id, expectedFingerprint: source?.fingerprint } satisfies CreateRoutingProfileParams)
      setState(value); setName(''); setEditing(value.profiles[value.profiles.length - 1]); onSelect(value.profiles[value.profiles.length - 1].profile.id)
    } catch (e) { setError(String(e)) } finally { setBusy(false) }
  }
  async function mutate(status: RoutingProfileStatus, operation: RoutingProfileAction) {
    const value = await invoke<RoutingProfilesState>('mutate_routing_profile', { profileId: status.profile.id, expectedFingerprint: status.fingerprint, operation } satisfies MutateRoutingProfileParams)
    setState(value)
    setEditing(value.profiles.find(p => p.profile.id === status.profile.id) ?? null)
  }
  return <div className="space-y-4">
    <Card><CardHeader><CardTitle>Shared routing profiles</CardTitle><CardDescription>Create Chatbot or Coding profiles and apply them in each client's LLM tab. Publishing updates linked routing choices; client model permissions, aliases, limits and spending settings stay independent.</CardDescription></CardHeader><CardContent className="space-y-3">
      <div className="flex gap-2"><Input aria-label="Profile name" value={name} maxLength={128} onChange={e => setName(e.target.value)} placeholder="e.g. Chatbot" className="max-w-xs" /><Button disabled={busy || !name.trim()} onClick={() => void create()}>Create profile</Button></div>
      {!state && !error && <p className="text-sm text-muted-foreground">Loading profiles…</p>}
      {state && !state.profiles.length && <p className="text-sm text-muted-foreground">No shared profiles yet. Create one here, or copy a client's existing routing from its LLM tab.</p>}
      {state?.profiles.map(status => <div key={status.profile.id} className="flex flex-wrap items-center justify-between rounded-md border p-3 gap-2"><div className="text-sm"><strong>{status.profile.name}</strong><p className="text-muted-foreground">Revision {status.profile.versions.active_revision} · {status.clients.length} linked clients{status.profile.versions.draft && ' · unpublished draft'}</p></div><div className="flex gap-2"><Button variant="outline" size="sm" onClick={() => { setEditing(status); onSelect(status.profile.id) }}>Edit profile</Button><Button variant="outline" size="sm" disabled={busy} onClick={() => void create(status)}>Duplicate</Button></div></div>)}
      {error && <p role="alert" className="text-destructive text-sm">{error} <Button variant="outline" size="sm" onClick={() => setRefresh(n => n + 1)}>Reload profiles</Button></p>}
    </CardContent></Card>
    {selected && latest && selected.fingerprint !== latest.fingerprint && <p role="status" className="text-sm text-yellow-600">This profile or its linked clients changed. Reload before saving or publishing. <Button variant="outline" size="sm" onClick={() => setEditing(latest)}>Reload latest profile</Button></p>}
    {selected && <ProfileEditor key={selected.profile.id} status={selected} onMutate={operation => mutate(selected, operation)} onClient={onClient} onReload={() => { setEditing(latest ?? null); setRefresh(n => n + 1) }} />}
  </div>
}

function ProfileEditor({ status, onMutate, onClient, onReload }: { status: RoutingProfileStatus; onMutate: (op: RoutingProfileAction) => Promise<void>; onClient: (id: string) => void; onReload: () => void }) {
  const profile = status.profile
  const saved = profile.versions.draft ?? profile.active
  const [draft, setDraft] = useState<RouteSnapshot>(saved)
  const [name, setName] = useState(profile.name)
  const [label, setLabel] = useState('')
  const [snapshotText, setSnapshotText] = useState(JSON.stringify(saved, null, 2))
  const [busy, setBusy] = useState(false)
  const [error, setError] = useState<string | null>(null)
  const [confirm, setConfirm] = useState<RoutingProfileAction | null>(null)
  const [models, setModels] = useState<DetailedModelInfo[]>([])
  const [freeTiers, setFreeTiers] = useState<Record<string, FreeTierKind>>({})
  const [previewClient, setPreviewClient] = useState(status.clients[0]?.client_id ?? '')
  const savedKey = JSON.stringify(saved)
  useEffect(() => { setDraft(saved); setSnapshotText(JSON.stringify(saved, null, 2)) }, [savedKey])
  useEffect(() => { setName(profile.name) }, [profile.name])
  useEffect(() => {
    let live = true
    let generation = 0
    const load = async () => { const request = ++generation; try {
      const [ms, statuses] = await Promise.all([invoke<DetailedModelInfo[]>('list_all_models_detailed'), invoke<ProviderFreeTierStatus[]>('get_free_tier_status')])
      if (live && request === generation) { setModels(ms); setFreeTiers(Object.fromEntries(statuses.map(s => [s.provider_instance, s.free_tier]))) }
    } catch (e) { if (live && request === generation) setError(String(e)) } }
    const listeners = ['models-changed', 'providers-changed', 'config-changed'].map(event => listenSafe(event, load))
    void Promise.all(listeners.map(l => l.promise)).then(() => { if (live) void load() })
    return () => { live = false; listeners.forEach(l => l.cleanup()) }
  }, [])
  const run = async (op: RoutingProfileAction) => {
    setBusy(true); setError(null)
    try { await onMutate(op) } catch (e) { setError(String(e)); throw e } finally { setBusy(false) }
  }
  const action = (op: RoutingProfileAction) => { void run(op).catch(() => {}) }
  const selectedClient = status.clients.find(c => c.client_id === previewClient) ?? status.clients[0]
  const impact = confirm?.action === 'rollback' ? status.revision_clients[String(confirm.revision)] ?? [] : status.clients
  const reviewedSnapshot = confirm?.action === 'rollback' ? profile.versions.revisions.find(r => r.revision === confirm.revision)?.snapshot : profile.versions.draft ?? profile.active
  const unavailable = reviewedSnapshot ? profileModelRefs(reviewedSnapshot).filter(([p, id]) => !models.some(m => m.provider_instance === p && m.model_id === id)) : []
  const dirty = JSON.stringify(draft) !== savedKey
  const chatModels = models.filter(m => m.capabilities.includes('chat'))
  return <Card><CardHeader><CardTitle>{profile.name}</CardTitle><CardDescription>Edit a draft, save it, then publish it to linked clients. Existing permissions can block selected models. Free eligibility does not guarantee quota; hosted classifiers can incur charges.</CardDescription></CardHeader><CardContent className="space-y-4">
    <div className="flex gap-2"><Input aria-label="Rename profile" value={name} maxLength={128} onChange={e => setName(e.target.value)} /><Button variant="outline" disabled={busy || !name.trim() || name === profile.name} onClick={() => action({ action: 'rename', name: name.trim() })}>Rename</Button></div>
    <div className="space-y-2"><h3 className="font-medium text-sm">Ordinary fallback priority</h3><p className="text-xs text-muted-foreground">After the selected and default route, these models are tried in order. Empty route lists use this list immediately. Models are deduplicated and checked against each client's access, capabilities, limits and free-tier settings.</p>
      {draft.prioritized_models.map((model, i) => <div key={`${JSON.stringify(model)}-${i}`} className="flex items-center gap-2"><span className="text-xs">{i + 1}.</span><PolicyModelLabel value={model} models={models} freeTiers={freeTiers} /><Button variant="outline" size="sm" disabled={busy || i === 0} onClick={() => { const ms = [...draft.prioritized_models]; [ms[i - 1], ms[i]] = [ms[i], ms[i - 1]]; setDraft({ ...draft, prioritized_models: ms }) }}>Up</Button><Button variant="outline" size="sm" disabled={busy} onClick={() => setDraft({ ...draft, prioritized_models: draft.prioritized_models.filter((_, j) => i !== j) })}>Remove</Button></div>)}
      <PolicyModelPicker models={chatModels.filter(m => !draft.prioritized_models.some(([p, id]) => p === m.provider_instance && id === m.model_id))} freeTiers={freeTiers} value={null} onChange={model => setDraft({ ...draft, prioritized_models: [...draft.prioritized_models, model], available_models: draft.available_models.filter(([p, id]) => p !== model[0] || id !== model[1]) })} placeholder="Add fallback model" disabled={busy} />
      <Button disabled={busy || !dirty} onClick={() => action({ action: 'save_draft', snapshot: draft })}>Save draft routing</Button>
    </div>
    <div className="space-y-2 text-sm"><h3 className="font-medium">Linked clients</h3>{!status.clients.length && <p className="text-muted-foreground">Apply this profile in a client's LLM tab.</p>}{status.clients.map(c => <div key={c.client_id} className="border rounded-md p-2"><Button variant="link" onClick={() => onClient(c.client_id)}>{c.client_name}</Button>{c.issues.map(issue => <p key={issue} className="text-xs text-yellow-600">{issue}</p>)}</div>)}
      {!!status.clients.length && <label className="flex gap-2 items-center">Preview as client<select aria-label="Profile preview client" className="border rounded bg-background p-1" value={selectedClient?.client_id ?? ''} onChange={e => setPreviewClient(e.target.value)}>{status.clients.map(c => <option key={c.client_id} value={c.client_id}>{c.client_name}</option>)}</select></label>}
    </div>
    <RoutingPolicyEditor strategyId={selectedClient?.strategy_id ?? ''} clientId={selectedClient?.client_id} value={draft.routing_policy} draftOnly readOnly={busy} previewSnapshot={draft} onSave={async policy => { const next = { ...draft, routing_policy: policy }; await run({ action: 'save_draft', snapshot: next }); setDraft(next) }} />
    <details><summary className="cursor-pointer text-sm">Routing tuning and advanced configuration</summary><div className="space-y-4 mt-3">
      <AdaptiveRoutingEditor value={draft.adaptive_routing} readOnly={busy} onSave={async adaptive_routing => { await run({ action: 'save_draft', snapshot: { ...draft, adaptive_routing } }) }} />
      <SessionRoutingEditor value={draft.session_routing} readOnly={busy} onSave={async session_routing => { await run({ action: 'save_draft', snapshot: { ...draft, session_routing } }) }} />
      <label className="block text-sm">Draft snapshot JSON<textarea aria-label="Profile draft JSON" value={snapshotText} onChange={e => setSnapshotText(e.target.value)} className="w-full h-64 border rounded bg-background p-2 text-xs font-mono" /></label>
      <Button variant="outline" disabled={busy} onClick={() => { try { action({ action: 'save_draft', snapshot: JSON.parse(snapshotText) }) } catch (e) { setError(String(e)) } }}>Save JSON draft</Button>
    </div></details>
    <div className="flex flex-wrap items-center gap-2"><Input aria-label="Publication label" placeholder="Publication label (optional)" value={label} maxLength={128} onChange={e => setLabel(e.target.value)} className="max-w-xs" /><Button disabled={busy || dirty || !profile.versions.draft} onClick={() => setConfirm({ action: 'publish', label })}>Publish draft</Button><Button variant="outline" disabled={busy || !profile.versions.draft} onClick={() => setConfirm({ action: 'discard_draft' })}>Discard draft</Button></div>
    {dirty && <p className="text-xs text-muted-foreground">Save draft changes before publishing.</p>}
    <details><summary className="cursor-pointer text-sm">Published revisions and profile deletion</summary><div className="space-y-2 mt-3 text-sm">{[...profile.versions.revisions].reverse().map(r => <div key={r.revision} className="flex items-center gap-2"><span className="flex-1">Revision {r.revision}: {r.label}{profile.versions.active_revision === r.revision && ' (active)'}</span><Button variant="outline" size="sm" disabled={busy || profile.versions.active_revision === r.revision} onClick={() => setConfirm({ action: 'rollback', revision: r.revision })}>Activate</Button><Button variant="outline" size="sm" disabled={busy || profile.versions.active_revision === r.revision} onClick={() => setConfirm({ action: 'remove_revision', revision: r.revision })}>Remove</Button></div>)}<p className="text-xs text-muted-foreground">History is limited to 32 revisions. Remove inactive revisions explicitly when needed. Deleting a profile with no clients keeps any unused strategies as independent routing copies.</p><Button variant="destructive" disabled={busy || !!status.clients.length} onClick={() => setConfirm({ action: 'delete' })}>Delete unattached profile</Button></div></details>
    {error && <p role="alert" className="text-destructive text-sm">{error} <Button variant="outline" size="sm" onClick={onReload}>Reload profile</Button></p>}
    <AlertDialog open={confirm !== null} onOpenChange={open => { if (!open) setConfirm(null) }}><AlertDialogContent><AlertDialogHeader><AlertDialogTitle>{confirm?.action === 'publish' ? 'Publish to linked clients?' : confirm?.action === 'rollback' ? `Activate revision ${confirm.revision}?` : 'Apply profile change?'}</AlertDialogTitle><AlertDialogDescription>{confirm?.action === 'publish' || confirm?.action === 'rollback' ? 'The following clients will use the selected routing revision. Their aliases, access, limits and free / paid fallback controls are preserved.' : 'Discarding a draft or removing a revision cannot be undone. Active routing is preserved unless you publish or activate a revision.'}</AlertDialogDescription></AlertDialogHeader>
      {(confirm?.action === 'publish' || confirm?.action === 'rollback') && <div className="max-h-60 overflow-auto space-y-2 text-sm">{unavailable.map(([p, id]) => <p key={`${p}/${id}`} className="text-yellow-600 text-xs">Unavailable in the current model catalog: {p}/{id}</p>)}{impact.length ? impact.map(c => <div key={c.client_id}><strong>{c.client_name}</strong>{c.issues.map(issue => <p className="text-yellow-600 text-xs" key={issue}>{issue}</p>)}</div>) : <p>No attached clients.</p>}</div>}
      <AlertDialogFooter><AlertDialogCancel>Cancel</AlertDialogCancel><AlertDialogAction onClick={() => { if (confirm) action(confirm) }}>Apply</AlertDialogAction></AlertDialogFooter>
    </AlertDialogContent></AlertDialog>
  </CardContent></Card>
}
