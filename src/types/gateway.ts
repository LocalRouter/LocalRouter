/** Rust: lr-providers/src/gateway.rs. Request overrides may only tighten saved policy. */
export interface GatewayPreferences {
  only?: string[]
  order?: string[]
  ignore?: string[]
  sort?: 'price' | 'latency' | 'throughput'
  allow_fallbacks?: boolean
  require_parameters?: boolean
  data_collection?: 'allow' | 'deny'
  zdr?: boolean
  max_price?: { prompt?: number; completion?: number; request?: number; image?: number }
  service_tier?: string
  reasoning_effort?: string
}
export interface GatewayAttribution {
  gateway: 'openrouter' | 'llmgateway' | 'vercel'
  upstream_request_id: string | null
  inference_provider: string | null
  resolved_provider: string | null
  region: string | null
  service_tier: string | null
  cost_usd: number | null
  cost_source: 'reported' | 'unknown'
  cost_scope: 'upstream_call' | 'request_total' | 'unknown'
  routing: { provider: string | null; model: string | null; succeeded: boolean | null; status_code: number | null; region: string | null }[]
  applied_preferences: GatewayPreferences
}
