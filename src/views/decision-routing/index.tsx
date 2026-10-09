import { useEffect, useState } from 'react'
import { invoke } from '@tauri-apps/api/core'
import { listenSafe } from '@/hooks/useTauriListener'
import { Card, CardHeader, CardTitle, CardDescription, CardContent } from '@/components/ui/Card'
import { Button } from '@/components/ui/Button'
import type { ClientFeatureStatus, GetFeatureClientsStatusParams } from '@/types/tauri-commands'

export function DecisionRoutingView({ onTabChange }: { activeSubTab: string | null; onTabChange: (view: string, subTab?: string | null) => void }) {
  const [clients, setClients] = useState<ClientFeatureStatus[]>([])
  const [error, setError] = useState<string | null>(null)
  useEffect(() => {
    let active = true
    const load = () => invoke<ClientFeatureStatus[]>('get_feature_clients_status', { feature: 'decision_routing' } satisfies GetFeatureClientsStatusParams)
      .then(v => { if (active) setClients(v) }).catch(e => { if (active) setError(String(e)) })
    void load()
    const listener = listenSafe('strategies-changed', load)
    return () => { active = false; listener.cleanup() }
  }, [])
  return <div className="space-y-5 p-6">
    <div><h1 className="text-2xl font-semibold">Decision Routing</h1><p className="text-muted-foreground mt-2">Choose when to use each model.</p></div>
    <Card><CardHeader><CardTitle>Policies configured per client</CardTitle><CardDescription>In a client's LLM tab, choose a decision model from your providers, write a routing question, and map each answer to a model or fallback list.</CardDescription></CardHeader>
      <CardContent className="space-y-3 text-sm"><p>Templates cover planning and implementation, specialist tasks, and quick or thorough responses. Exact client mode rules work without inference.</p>
        <p>For plan → Astra, otherwise → Sol, choose the Client mode template and assign those models to its two options. Your client must supply <code>metadata: {JSON.stringify({ 'localrouter.mode': 'plan' })}</code>. Missing mode uses the default route.</p>
        <p>Native decision models such as Jev, Laya and Kev use the existing provider configuration and model management. No separate routing model download is required.</p>
        <Button variant="outline" onClick={() => onTabChange('resources', 'providers')}>Manage providers</Button>
      </CardContent></Card>
    <Card><CardHeader><CardTitle>Clients</CardTitle></CardHeader><CardContent className="space-y-2">
      {error && <p role="alert" className="text-destructive text-sm">{error}</p>}
      {!clients.length && !error && <p className="text-sm text-muted-foreground">Configure routing in a client's LLM tab.</p>}
      {clients.map(c => <div key={c.client_id} className="flex items-center justify-between border rounded-md p-3"><div className="text-sm"><strong>{c.client_name}</strong><p className="text-muted-foreground">{c.active ? 'Policy enabled' : 'Uses default model order'}</p></div><Button variant="outline" size="sm" onClick={() => onTabChange('clients', `${c.client_id}|models`)}>Configure</Button></div>)}
    </CardContent></Card>
  </div>
}
