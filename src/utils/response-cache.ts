export interface LocalCacheReceipt {
  scope: 'resolved_model_call'
  hit: true
  upstream_spend_usd: 0
  billed_input_tokens: 0
  billed_output_tokens: 0
  age_ms: number | null
  avoided_cost_estimate_usd: number | null
  served_usage: unknown
  original_gateway: unknown
}
/** Captured provider data must explicitly support a zero-new-spend claim. */
export function localCacheReceipt(body: unknown): LocalCacheReceipt | null {
  if (!body || typeof body !== 'object') return null
  const extensions = (body as Record<string, unknown>).extensions
  if (!extensions || typeof extensions !== 'object') return null
  const value = (extensions as Record<string, unknown>)[
    'localrouter.response_cache'
  ]
  if (!value || typeof value !== 'object' || Array.isArray(value)) return null
  const receipt = value as Record<string, unknown>
  if (
    receipt.scope !== 'resolved_model_call' ||
    receipt.hit !== true ||
    receipt.upstream_spend_usd !== 0 ||
    receipt.billed_input_tokens !== 0 ||
    receipt.billed_output_tokens !== 0
  )
    return null
  const number = (v: unknown) =>
    typeof v === 'number' && Number.isFinite(v) && v >= 0 ? v : null
  return {
    scope: 'resolved_model_call',
    hit: true,
    upstream_spend_usd: 0,
    billed_input_tokens: 0,
    billed_output_tokens: 0,
    age_ms: number(receipt.age_ms),
    avoided_cost_estimate_usd: number(receipt.avoided_cost_estimate_usd),
    served_usage: receipt.served_usage ?? null,
    original_gateway: receipt.original_gateway ?? null,
  }
}
