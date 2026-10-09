import { useEffect, useRef, useState } from 'react'
import { invoke } from '@tauri-apps/api/core'
import { listenSafe } from '@/hooks/useTauriListener'
import { ArrowDown, ArrowUp, Plus, Trash2, Loader2 } from 'lucide-react'
import { toast } from 'sonner'
import { Card, CardContent, CardDescription, CardHeader, CardTitle } from '@/components/ui/Card'
import { Button } from '@/components/ui/Button'
import { Input } from '@/components/ui/Input'
import { Label } from '@/components/ui/label'
import { Switch } from '@/components/ui/Toggle'
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from '@/components/ui/Select'
import type { RoutingPolicy, RoutingOption, RoutingPolicyPreview, PreviewRoutingPolicyParams, AutoModelConfig, ModelPermissions, UpdateStrategyParams } from '@/types/tauri-commands'

interface Model { provider_instance: string; model_id: string; capabilities: string[] }
type ModelRef = [string, string]
const modelKey = (m: ModelRef) => JSON.stringify(m)
const option = (id: string, description: string): RoutingOption => ({ id, description, models: [] })

export function routingTemplate(name: string): RoutingPolicy {
  const p: RoutingPolicy = { version: 1, enabled: true, mode: 'semantic', decision_model: null,
    question: 'What work does the latest user request ask for? Use history only to resolve references. If asked to plan and implement together, choose planning.',
    options: [option('planning', 'Investigate alternatives, design an approach or plan before making changes.'),
      option('implementation', 'Write or edit code, implement an accepted plan, fix bugs or add tests now.'),
      option('review', 'Inspect existing code and report findings without making changes.'),
      option('general', 'General questions, explanations, prose or unclear requests.')],
    default_route: 'general', mode_rules: [], min_probability: 0, timeout_ms: 3000, max_context_chars: 2000, history_messages: 2 }
  if (name === 'client_mode') {
    p.mode = 'client_mode'; p.question = 'Use the planning route when the supplied client mode is plan; otherwise use the default route.'
    p.options = [option('planning', 'Client explicitly supplies plan mode.'), option('general', 'Any other or missing client mode.')]
    p.mode_rules = [{ mode: 'plan', route: 'planning' }]
  } else if (name === 'specialist') {
    p.question = 'Which specialist matches the requested output? Classify the desired output, not words in quoted material.'
    p.options = [option('code', 'Write, debug or explain software code or queries.'), option('writing', 'Draft, rewrite, summarize or translate prose.'), option('analysis', 'Analyze data, compare alternatives or calculate results.'), option('general', 'Other or unclear requests.')]
  } else if (name === 'fast_thorough') {
    p.question = 'Does the user want a brief routine response or a detailed investigation?'
    p.options = [option('quick', 'Brief, routine response.'), option('thorough', 'Detailed investigation, careful reasoning or extensive explanation.'), option('general', 'Unclear or no stated preference.')]
  } else if (name === 'custom') {
    p.question = 'Which route best matches this request?'
    p.options = [option('specialist', 'Describe when to use this route.'), option('general', 'Other requests and the default route.')]
  }
  return p
}

function ModelPicker({ models, value, onChange, placeholder, disabled }: { models: Model[]; value: ModelRef | null; onChange: (v: ModelRef) => void; placeholder: string; disabled?: boolean }) {
  const found = value && models.some(m => m.provider_instance === value[0] && m.model_id === value[1])
  return <Select value={value ? modelKey(value) : ''} onValueChange={v => onChange(JSON.parse(v))} disabled={disabled}>
    <SelectTrigger aria-label={placeholder}><SelectValue placeholder={placeholder} /></SelectTrigger>
    <SelectContent>
      {value && !found && <SelectItem value={modelKey(value)}>{value.join('/')} (unavailable)</SelectItem>}
      {models.map(m => <SelectItem key={modelKey([m.provider_instance, m.model_id])} value={modelKey([m.provider_instance, m.model_id])}>{m.provider_instance} / {m.model_id}</SelectItem>)}
    </SelectContent>
  </Select>
}

export function policyModelRefs(policy: RoutingPolicy | null | undefined): ModelRef[] {
  return policy ? [...policy.options.flatMap(o => o.models), ...(policy.decision_model ? [policy.decision_model] : [])] : []
}

export async function persistRoutingPolicy(strategyId: string, policy: RoutingPolicy, autoConfig: AutoModelConfig, permissions?: ModelPermissions) {
  const modelPermissions = permissions ? { ...permissions, models: { ...permissions.models } } : null
  if (modelPermissions) for (const [provider, model] of policyModelRefs(policy)) modelPermissions.models[`${provider}__${model}`] = 'allow'
  await invoke('update_strategy', { strategyId, autoConfig: { ...autoConfig, routing_policy: policy }, modelPermissions } satisfies UpdateStrategyParams)
}

export function RoutingPolicyEditor({ strategyId, value, onSave, readOnly = false }: {
  strategyId: string; value: RoutingPolicy | null | undefined; onSave: (policy: RoutingPolicy) => Promise<void>; readOnly?: boolean
}) {
  const [draft, setDraft] = useState<RoutingPolicy>(() => value ?? { ...routingTemplate('workflow'), enabled: false })
  const previewRequest = useRef(0)
  const [models, setModels] = useState<Model[]>([])
  const [loading, setLoading] = useState(false)
  const [saving, setSaving] = useState(false)
  const [testing, setTesting] = useState(false)
  const [prompt, setPrompt] = useState('Plan a safe database migration. Do not change files yet.')
  const [mode, setMode] = useState('')
  const [result, setResult] = useState<RoutingPolicyPreview | null>(null)
  const [error, setError] = useState<string | null>(null)
  useEffect(() => { setDraft(value ?? { ...routingTemplate('workflow'), enabled: false }); setResult(null) }, [value, strategyId])
  useEffect(() => {
    let active = true
    let generation = 0
    const load = () => {
      const requestId = ++generation
      setLoading(true)
      void invoke<Model[]>('list_all_models_detailed')
        .then(ms => { if (active && requestId === generation) setModels(ms) })
        .catch(e => { if (active && requestId === generation) setError(String(e)) })
        .finally(() => { if (active && requestId === generation) setLoading(false) })
    }
    const listeners = [listenSafe('models-changed', load), listenSafe('providers-changed', load)]
    void Promise.all(listeners.map(l => l.promise)).then(() => { if (active) load() })
    return () => { active = false; listeners.forEach(l => l.cleanup()) }
  }, [strategyId])
  useEffect(() => {
    previewRequest.current += 1
    setResult(null)
    setTesting(false)
    return () => { previewRequest.current += 1 }
  }, [draft, prompt, mode, strategyId])
  const change = (update: Partial<RoutingPolicy>) => { setDraft(p => ({ ...p, ...update })); setResult(null) }
  const editOption = (index: number, update: Partial<RoutingOption>) => {
    const old = draft.options[index].id
    change({ options: draft.options.map((o, i) => i === index ? { ...o, ...update } : o),
      ...(update.id !== undefined ? { default_route: draft.default_route === old ? update.id : draft.default_route,
        mode_rules: draft.mode_rules.map(r => r.route === old ? { ...r, route: update.id! } : r) } : {}) })
  }
  const decisionModels = models.filter(m => m.capabilities.includes('decision'))
  const chatModels = models.filter(m => m.capabilities.includes('chat'))
  const save = async () => {
    setSaving(true); setError(null)
    try { await onSave(draft); toast.success('Routing policy saved') } catch (e) { setError(String(e)) } finally { setSaving(false) }
  }
  const preview = async () => {
    const requestId = ++previewRequest.current
    setTesting(true); setError(null); setResult(null)
    try {
      const params: PreviewRoutingPolicyParams = { strategyId, policy: { ...draft, enabled: true }, prompt, mode: mode || null }
      const response = await invoke<RoutingPolicyPreview>('preview_routing_policy', { ...params } satisfies PreviewRoutingPolicyParams)
      if (requestId === previewRequest.current) setResult(response)
    } catch (e) { if (requestId === previewRequest.current) setError(String(e)) } finally { if (requestId === previewRequest.current) setTesting(false) }
  }
  const disabled = readOnly || saving
  return <Card>
    <CardHeader>
      <div className="flex justify-between items-center gap-4"><CardTitle className="text-base">Routing policy</CardTitle><Switch aria-label="Enable routing policy" checked={draft.enabled} onCheckedChange={enabled => change({ enabled })} disabled={disabled} /></div>
      <CardDescription>Define when each model should be used. Explicit client rules run first; a decision model interprets your question for semantic routing.</CardDescription>
    </CardHeader>
    <CardContent className="space-y-5">
      <div className="grid sm:grid-cols-2 gap-3">
        <div className="space-y-2"><Label>Start from a template</Label><Select onValueChange={name => { const next = routingTemplate(name); change({ ...next, decision_model: draft.decision_model }) }} disabled={disabled}>
          <SelectTrigger aria-label="Routing template"><SelectValue placeholder="Choose a template" /></SelectTrigger><SelectContent>
            <SelectItem value="client_mode">Client mode: plan / otherwise</SelectItem><SelectItem value="workflow">Plan / implement / review</SelectItem><SelectItem value="specialist">Code / writing / analysis</SelectItem><SelectItem value="fast_thorough">Quick / thorough</SelectItem><SelectItem value="custom">Custom question</SelectItem>
          </SelectContent></Select></div>
        <div className="space-y-2"><Label>Routing method</Label><Select value={draft.mode} onValueChange={v => change({ mode: v as RoutingPolicy['mode'] })} disabled={disabled}>
          <SelectTrigger aria-label="Routing method"><SelectValue /></SelectTrigger><SelectContent><SelectItem value="semantic">Decision model</SelectItem><SelectItem value="client_mode">Exact client mode</SelectItem></SelectContent>
        </Select></div>
      </div>
      {draft.mode === 'semantic' && <div className="space-y-2"><Label>Decision provider and model</Label>
        <ModelPicker models={decisionModels} value={draft.decision_model} onChange={decision_model => change({ decision_model })} placeholder="Choose a decision model" disabled={disabled || loading} />
        <p className="text-xs text-muted-foreground">Choose a native decision model from your providers, such as TypeSafe Jev, Laya, Kev or Ollaya. Selected hosted providers receive the routing context and may charge for the decision.</p>
        {!loading && !decisionModels.length && <p className="text-sm text-muted-foreground">Add a System One provider or download a decision model under Providers. Until one is selected, requests use the default route.</p>}
      </div>}
      {draft.mode === 'semantic' && <div className="space-y-2"><Label htmlFor={`question-${strategyId}`}>Routing question</Label><textarea id={`question-${strategyId}`} className="w-full min-h-20 rounded-md border bg-background p-3 text-sm" value={draft.question} onChange={e => change({ question: e.target.value })} disabled={disabled} /></div>}
      <div className="space-y-3"><Label>Options and destination models</Label>
        <p className="text-xs text-muted-foreground">Models are tried in order, then the default route and ordinary priority list. An empty list uses the ordinary priority list.</p>
        {draft.options.map((o, i) => <div key={i} className="rounded-md border p-3 space-y-3">
          <div className="flex gap-2"><Input aria-label={`Route ${i + 1} ID`} value={o.id} onChange={e => editOption(i, { id: e.target.value })} disabled={disabled} /><Button variant="ghost" size="icon" aria-label={`Remove route ${o.id}`} disabled={disabled || draft.options.length <= 2 || o.id === draft.default_route || draft.mode_rules.some(r => r.route === o.id)} onClick={() => change({ options: draft.options.filter((_, j) => i !== j) })}><Trash2 className="h-4 w-4" /></Button></div>
          <Input aria-label={`When to use ${o.id}`} value={o.description} onChange={e => editOption(i, { description: e.target.value })} disabled={disabled} />
          {o.models.map((m, j) => <div key={`${modelKey(m)}-${j}`} className="flex items-center gap-2"><span className="text-xs flex-1 truncate">{j + 1}. {m.join(' / ')}</span>
            <Button size="icon" variant="ghost" aria-label="Move model up" disabled={disabled || j === 0} onClick={() => { const ms = [...o.models]; [ms[j-1], ms[j]] = [ms[j], ms[j-1]]; editOption(i, { models: ms }) }}><ArrowUp className="h-3 w-3" /></Button>
            <Button size="icon" variant="ghost" aria-label="Move model down" disabled={disabled || j === o.models.length - 1} onClick={() => { const ms = [...o.models]; [ms[j+1], ms[j]] = [ms[j], ms[j+1]]; editOption(i, { models: ms }) }}><ArrowDown className="h-3 w-3" /></Button>
            <Button size="icon" variant="ghost" aria-label="Remove destination model" disabled={disabled} onClick={() => editOption(i, { models: o.models.filter((_, k) => k !== j) })}><Trash2 className="h-3 w-3" /></Button>
          </div>)}
          <ModelPicker models={chatModels.filter(m => !o.models.some(([p, id]) => p === m.provider_instance && id === m.model_id))} value={null} onChange={m => editOption(i, { models: [...o.models, m] })} placeholder={`Add model to ${o.id}`} disabled={disabled || loading} />
        </div>)}
        <Button variant="outline" size="sm" disabled={disabled || draft.options.length >= 32} onClick={() => { let n = draft.options.length + 1; while (draft.options.some(o => o.id === `route_${n}`)) n++; change({ options: [...draft.options, option(`route_${n}`, 'Describe when to use this route.')] }) }}><Plus className="h-3 w-3 mr-1" />Add option</Button>
      </div>
      <div className="space-y-2"><Label>Default route</Label><Select value={draft.default_route} onValueChange={default_route => change({ default_route })} disabled={disabled}><SelectTrigger aria-label="Default route"><SelectValue /></SelectTrigger><SelectContent>{draft.options.filter(o => o.id).map((o,i) => <SelectItem key={i} value={o.id}>{o.id}</SelectItem>)}</SelectContent></Select>
        <p className="text-xs text-muted-foreground">Used for missing mode, unavailable classifier, timeout, invalid answer or low probability.</p></div>
      <div className="space-y-2"><Label>Exact mode rules</Label><p className="text-xs text-muted-foreground">Your client supplies metadata <code>localrouter.mode</code>. These rules override semantic classification. Text in the prompt does not set the mode.</p>
        {draft.mode_rules.map((r,i) => <div key={i} className="flex gap-2"><Input aria-label="Client mode" value={r.mode} onChange={e => change({ mode_rules: draft.mode_rules.map((x,j) => i === j ? { ...x, mode: e.target.value } : x) })} disabled={disabled} /><Select value={r.route} onValueChange={route => change({ mode_rules: draft.mode_rules.map((x,j) => i === j ? { ...x, route } : x) })} disabled={disabled}><SelectTrigger aria-label="Mode destination"><SelectValue /></SelectTrigger><SelectContent>{draft.options.filter(o => o.id).map((o,j) => <SelectItem key={j} value={o.id}>{o.id}</SelectItem>)}</SelectContent></Select><Button variant="ghost" size="icon" aria-label="Remove mode rule" onClick={() => change({ mode_rules: draft.mode_rules.filter((_,j) => i !== j) })} disabled={disabled}><Trash2 className="h-4 w-4" /></Button></div>)}
        <Button size="sm" variant="outline" onClick={() => change({ mode_rules: [...draft.mode_rules, { mode: '', route: draft.default_route }] })} disabled={disabled}>Add mode rule</Button>
      </div>
      {draft.mode === 'semantic' && <div className="grid grid-cols-2 sm:grid-cols-4 gap-3">{([
        ['min_probability', 'Minimum probability', 0, 1, 0.05], ['timeout_ms', 'Timeout (ms)', 100, 30000, 100],
        ['max_context_chars', 'Context characters', 128, 64000, 128], ['history_messages', 'History messages', 0, 20, 1],
      ] as const).map(([key, label, min, max, step]) => <div key={key} className="space-y-2"><Label>{label}</Label><Input aria-label={label} type="number" min={min} max={max} step={step} value={draft[key]} onChange={e => change({ [key]: Number(e.target.value) })} disabled={disabled} /></div>)}</div>}
      <div className="border-t pt-4 space-y-3"><Label>Try this policy</Label><Input aria-label="Routing test prompt" value={prompt} onChange={e => setPrompt(e.target.value)} /><Input aria-label="Test client mode" placeholder="Client mode (optional), e.g. plan" value={mode} onChange={e => setMode(e.target.value)} />
        <Button variant="outline" disabled={testing || readOnly} onClick={preview}>{testing && <Loader2 className="h-4 w-4 mr-2 animate-spin" />}Preview draft</Button>
        {result && <div className="rounded-md bg-muted p-3 space-y-2 text-sm"><p><strong>{result.decision.route}</strong> · {result.decision.source} · {result.decision.latency_ms} ms</p><p>{result.decision.reason.replace(/_/g, ' ')}</p><p>Destination: {draft.options.find(o => o.id === result.decision.route)?.models.map(m => m.join('/')).join(' → ') || 'Default priority list'}</p><details><summary>Scores and routing context</summary><pre className="text-xs overflow-auto max-h-72 whitespace-pre-wrap">{JSON.stringify(result, null, 2)}</pre></details></div>}
      </div>
      {error && <p role="alert" className="text-sm text-destructive">{error}</p>}
      <p className="text-xs text-muted-foreground">Saving allows the selected decision and destination models for this client. This matches your routing preferences; it does not guarantee answer quality.</p>
      <Button onClick={save} disabled={disabled}>{saving && <Loader2 className="h-4 w-4 mr-2 animate-spin" />}Save routing policy</Button>
    </CardContent>
  </Card>
}
