import type { GatewayAttribution } from '@/types/gateway'

/** Captured responses are untrusted and may be partial. */
export function gatewayAttribution(body: unknown): GatewayAttribution | null {
  if (!body || typeof body !== 'object') return null
  const extensions = (body as Record<string, unknown>).extensions
  if (!extensions || typeof extensions !== 'object') return null
  const value = (extensions as Record<string, unknown>)['localrouter.gateway']
  if (!value || typeof value !== 'object' || Array.isArray(value)) return null
  const v = value as Record<string, unknown>
  if (!['openrouter', 'llmgateway', 'vercel'].includes(String(v.gateway))) return null
  const text = (s: unknown) => typeof s === 'string' ? s : null
  const number = (n: unknown) => typeof n === 'number' && Number.isFinite(n) && n >= 0 ? n : null
  const cost = number(v.cost_usd)
  return {
    gateway: v.gateway as GatewayAttribution['gateway'],
    upstream_request_id: text(v.upstream_request_id),
    resolved_provider: text(v.resolved_provider),
    inference_provider: text(v.inference_provider), region: text(v.region), service_tier: text(v.service_tier),
    cost_usd: cost, cost_source: cost !== null && v.cost_source === 'reported' ? 'reported' : 'unknown',
    cost_scope: v.cost_scope === 'upstream_call' || v.cost_scope === 'request_total' ? v.cost_scope : 'unknown',
    routing: Array.isArray(v.routing) ? v.routing.flatMap(a => {
      if (!a || typeof a !== 'object' || Array.isArray(a)) return []
      const attempt = a as Record<string, unknown>
      return [{ provider: text(attempt.provider), model: text(attempt.model), succeeded: typeof attempt.succeeded === 'boolean' ? attempt.succeeded : null, status_code: number(attempt.status_code), region: text(attempt.region) }]
    }) : [],
    applied_preferences: v.applied_preferences && typeof v.applied_preferences === 'object' && !Array.isArray(v.applied_preferences) ? v.applied_preferences : {},
  }
}
