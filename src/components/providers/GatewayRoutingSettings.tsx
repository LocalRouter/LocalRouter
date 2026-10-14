import { useEffect, useState } from 'react'
import { Input } from '@/components/ui/Input'
import type { GatewayPreferences } from '@/types/gateway'

export const GATEWAY_CONFIG_KEY = 'gateway_preferences'

export function GatewayRoutingSettings({ providerType: rawProviderType, value, onChange }: {
  providerType: string; value: string; onChange: (value: string) => void
}) {
  const providerType = rawProviderType === 'vercel_ai_gateway' ? 'vercel' : rawProviderType
  const [preferences, setPreferences] = useState<GatewayPreferences>({})
  const [error, setError] = useState<string | null>(null)
  useEffect(() => {
    try {
      const parsed: unknown = JSON.parse(value || '{}')
      if (!parsed || typeof parsed !== 'object' || Array.isArray(parsed)) throw new Error('Invalid settings')
      const preferences = parsed as GatewayPreferences
      for (const key of ['only', 'order', 'ignore'] as const) {
        if (preferences[key] !== undefined && (!Array.isArray(preferences[key]) || !preferences[key]!.every(v => typeof v === 'string'))) throw new Error('Invalid provider list')
      }
      if (preferences.max_price !== undefined && (!preferences.max_price || typeof preferences.max_price !== 'object' || Array.isArray(preferences.max_price))) throw new Error('Invalid price ceilings')
      setPreferences(preferences)
      setError(null)
    } catch {
      setError('Saved gateway settings could not be read. Correct the configuration before changing them.')
    }
  }, [value])
  const update = (patch: Partial<GatewayPreferences>) => {
    const next = { ...preferences, ...patch }
    setPreferences(next)
    onChange(JSON.stringify(next))
  }
  const lists: Array<['only' | 'order' | 'ignore', string]> = [
    ['only', providerType === 'llmgateway' ? 'Pinned inference provider (one)' : 'Allowed inference providers'],
    ['order', 'Preferred inference provider order'],
    ...(providerType === 'openrouter' ? [['ignore', 'Excluded inference providers'] as ['ignore', string]] : []),
  ]
  if (error) return <p role="alert" className="text-sm text-destructive">{error}</p>
  return <fieldset className="space-y-3 rounded border p-4">
    <legend className="text-sm font-medium">Upstream gateway routing</legend>
    <p className="text-xs text-muted-foreground">These select inference providers inside this gateway. Request overrides can only tighten saved restrictions.</p>
    {lists.map(([key, label]) => <label key={key} className="block text-sm space-y-1">
      <span>{label}</span>
      <Input key={`${key}-${value}`} defaultValue={preferences[key]?.join(', ') ?? ''} placeholder="Comma-separated provider slugs" onBlur={e => {
        const names = [...new Set(e.target.value.split(',').map(v => v.trim()).filter(Boolean))]
        update({ [key]: names.length ? names : undefined })
      }} />
    </label>)}
    <label className="block text-sm space-y-1"><span>Sort endpoints by</span>
      <select className="block w-full rounded border bg-background p-2" value={preferences.sort ?? ''} onChange={e => update({ sort: (e.target.value || undefined) as GatewayPreferences['sort'] })}>
        <option value="">Gateway default</option><option value="price">Price</option><option value="latency">Latency</option><option value="throughput">Throughput</option>
      </select>
    </label>
    {([
      ['allow_fallbacks', 'Disable upstream provider fallback', true],
      ['zdr', 'Require zero data retention', providerType !== 'llmgateway'],
      ['require_parameters', 'Require support for all request parameters', providerType === 'openrouter'],
    ] as const).filter(([, , visible]) => visible).map(([key, label]) => <label key={key} className="flex items-center gap-2 text-sm">
      <input type="checkbox" checked={key === 'allow_fallbacks' ? preferences[key] === false : preferences[key] === true} onChange={e => update({ [key]: e.target.checked ? key !== 'allow_fallbacks' : undefined })} />{label}
    </label>)}
    {providerType !== 'llmgateway' && <label className="flex items-center gap-2 text-sm"><input type="checkbox" checked={preferences.data_collection === 'deny'} onChange={e => update({ data_collection: e.target.checked ? 'deny' : undefined })} />Disallow provider data collection{providerType === 'vercel' ? ' (requires ZDR and no prompt training)' : ''}</label>}
    {providerType === 'openrouter' && <>
      <label className="block text-sm space-y-1"><span>Maximum reasoning effort</span><select className="block w-full rounded border bg-background p-2" value={preferences.reasoning_effort ?? ''} onChange={e => update({ reasoning_effort: e.target.value || undefined })}>
        <option value="">Model default</option>{['none', 'minimal', 'low', 'medium', 'high', 'xhigh', 'max'].map(effort => <option key={effort} value={effort}>{effort}</option>)}
      </select></label>
      <label className="block text-sm space-y-1"><span>Service tier</span><select className="block w-full rounded border bg-background p-2" value={preferences.service_tier ?? ''} onChange={e => update({ service_tier: e.target.value || undefined })}>
        <option value="">Gateway default</option>{['default', 'flex', 'priority', 'ultrafast'].map(tier => <option key={tier} value={tier}>{tier}</option>)}
      </select></label>
      {(['prompt', 'completion', 'request', 'image'] as const).map(key => <label key={key} className="block text-sm space-y-1"><span>Maximum {key} price ({key === 'prompt' || key === 'completion' ? 'USD / million tokens' : `USD / ${key}`})</span>
        <Input key={`${key}-${value}`} type="number" min="0" step="any" defaultValue={preferences.max_price?.[key] ?? ''} onBlur={e => {
          const raw = e.target.value.trim()
          const amount = raw ? Number(raw) : undefined
          if (amount !== undefined && (!Number.isFinite(amount) || amount < 0)) return
          const prices = { ...preferences.max_price, [key]: amount }
          update({ max_price: Object.values(prices).some(v => v !== undefined) ? prices : undefined })
        }} />
      </label>)}
      <p className="text-xs text-muted-foreground">The OpenRouter Alpha Decisions endpoint currently requires default routing preferences. Regional routing uses the provider's base URL.</p>
    </>}
    {providerType === 'vercel' && <p className="text-xs text-muted-foreground">Disabling fallback requires a single allowed provider or an eligible preferred provider.</p>}
  </fieldset>
}
