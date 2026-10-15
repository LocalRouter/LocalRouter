import type { ReplayCost } from '@/types/tauri-commands'
/** Partial sums retain separate accounting categories; null means no known value. */
export function sumReplayCosts(costs: ReplayCost[]): ReplayCost {
  const sum = (
    key:
      | 'reported_usd'
      | 'estimated_usd'
      | 'classifier_reported_usd'
      | 'classifier_estimated_usd',
  ) => {
    const values = costs
      .map((c) => c[key])
      .filter((n): n is number => n !== null && Number.isFinite(n) && n >= 0)
    return values.length ? values.reduce((a, b) => a + b, 0) : null
  }
  return {
    reported_usd: sum('reported_usd'),
    estimated_usd: sum('estimated_usd'),
    classifier_reported_usd: sum('classifier_reported_usd'),
    classifier_estimated_usd: sum('classifier_estimated_usd'),
    unknown_calls: costs.reduce((n, c) => n + c.unknown_calls, 0),
    classifier_cost_unknown: costs.some((c) => c.classifier_cost_unknown),
  }
}
