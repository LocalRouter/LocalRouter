/**
 * Demo port of free-model rule evaluation (crates/lr-providers/src/free_models.rs)
 * and the demo providers' free tiers.
 *
 * JSONPath filters are only partly supported here: comparisons of `@.path`
 * with a literal, joined by && or ||. The app evaluates full RFC 9535.
 */
import type {
  FreeModelCondition,
  FreeModelRule,
  FreeModelRuleError,
  FreeModelRulesPreview,
  FreeTierKind,
  ModelFreeTierInfo,
} from '@app/types/tauri-commands'

/** Each demo model's entry in its provider's model list (raw JSON). */
export const MOCK_LISTING_METADATA: Record<string, Record<string, Record<string, unknown>>> = {
  'free-ai': {
    'llama-3.3-70b': { id: 'llama-3.3-70b', name: 'Llama 3.3 70B', type: 'chat', tier: 'free', pricing: { input: 0, output: 0 } },
    'qwen3-32b': { id: 'qwen3-32b', name: 'Qwen3 32B', type: 'chat', tier: 'free', pricing: { input: 0, output: 0 } },
    'gemma-3-27b-preview': { id: 'gemma-3-27b-preview', name: 'Gemma 3 27B (preview)', type: 'chat', tier: 'free', beta: true, pricing: { input: 0, output: 0 } },
    'gpt-oss-120b': { id: 'gpt-oss-120b', name: 'gpt-oss 120B', type: 'chat', tier: 'community', pricing: { input: '0', output: '0' } },
    'deepseek-r1': { id: 'deepseek-r1', name: 'DeepSeek R1', type: 'chat', tier: 'pro', pricing: { input: '0.00000055', output: '0.00000219' } },
    'mistral-large': { id: 'mistral-large', name: 'Mistral Large', type: 'chat', tier: 'pro', pricing: { input: '0.000002', output: '0.000006' } },
    'kimi-k2': { id: 'kimi-k2', name: 'Kimi K2', type: 'chat', tier: 'partner' },
    'bge-m3': { id: 'bge-m3', name: 'BGE M3', type: 'embeddings', tier: 'free', pricing: { input: 0 } },
    'flux-schnell': { id: 'flux-schnell', name: 'FLUX.1 schnell', type: 'image', tier: 'pro', pricing: { image: 0.003 } },
  },
}

/** The demo providers' free tiers: their own setting or the type default. */
export const mockFreeTiers: Record<string, { kind: FreeTierKind; override: boolean }> = {
  'openai-primary': { kind: { kind: 'none' }, override: false },
  'anthropic-main': { kind: { kind: 'none' }, override: false },
  'ollama-local': { kind: { kind: 'always_free_local' }, override: false },
  'ollaya-local': { kind: { kind: 'always_free_local' }, override: false },
  'von-local': { kind: { kind: 'always_free_local' }, override: false },
  'llamacpp-local': { kind: { kind: 'always_free_local' }, override: false },
  'fastflowlm-npu': { kind: { kind: 'always_free_local' }, override: false },
  'gemini-google': {
    kind: { kind: 'rate_limited_free', max_rpm: 10, max_rpd: 250, max_tpm: 250000, max_tpd: 0, max_monthly_calls: 0, max_monthly_tokens: 0 },
    override: false,
  },
  'groq-fast': {
    kind: { kind: 'rate_limited_free', max_rpm: 30, max_rpd: 14400, max_tpm: 6000, max_tpd: 500000, max_monthly_calls: 0, max_monthly_tokens: 0 },
    override: false,
  },
  'openrouter-backup': {
    kind: {
      kind: 'free_models_only',
      free_model_patterns: [],
      rules: [{ enabled: true, effect: 'include', when: { type: 'zero_price', sources: ['provider_listing'] }, label: "Priced at $0 in OpenRouter's model list" }],
      max_rpm: 20,
    },
    override: false,
  },
  // A self-hosted vLLM box on the LAN, marked always free by the user.
  'gpu-box-vllm': { kind: { kind: 'always_free_local' }, override: true },
  'free-ai': {
    kind: {
      kind: 'free_models_only',
      free_model_patterns: ['kimi-k2'],
      excluded_models: [],
      rules: [
        { enabled: true, effect: 'exclude', when: { type: 'model_id', pattern: '*preview*', syntax: 'glob', case_insensitive: true }, label: 'Previews change without notice' },
        { enabled: true, effect: 'include', when: { type: 'field', pointer: '/tier', op: 'equals', value: 'free' } },
        { enabled: true, effect: 'include', when: { type: 'zero_price', sources: ['pricing_override', 'provider_listing'] } },
      ],
      max_rpm: 20,
    },
    override: true,
  },
}

export function mockFreeTier(instance: string): FreeTierKind {
  return mockFreeTiers[instance]?.kind ?? { kind: 'none' }
}

// ---------------------------------------------------------------------------
// Evaluation
// ---------------------------------------------------------------------------

interface PriceOverride {
  input_per_million: number
  output_per_million: number
}

export interface MockFreeInput {
  modelId: string
  metadata?: Record<string, unknown> | null
  priceOverride?: PriceOverride | null
}

type Outcome = { kind: 'match' | 'no_match' | 'unknown'; why: string }

const NO_LISTING = 'no listing data for this model'

function globMatch(pattern: string, text: string, ci: boolean): boolean {
  const escaped = pattern.split('*').map(p => p.replace(/[.+?^${}()|[\]\\]/g, '\\$&')).join('.*')
  return new RegExp(`^${escaped}$`, ci ? 'is' : 's').test(text)
}

function pointerGet(value: unknown, pointer: string): unknown {
  if (pointer === '') return value
  let cur: unknown = value
  for (const raw of pointer.slice(1).split('/')) {
    const key = raw.replace(/~1/g, '/').replace(/~0/g, '~')
    if (Array.isArray(cur)) cur = cur[Number(key)]
    else if (cur && typeof cur === 'object') cur = (cur as Record<string, unknown>)[key]
    else return undefined
  }
  return cur
}

function asNumber(v: unknown): number | null {
  if (typeof v === 'number') return Number.isFinite(v) ? v : null
  if (typeof v === 'string' && v.trim() !== '') {
    const n = Number(v.trim())
    return Number.isFinite(n) ? n : null
  }
  return null
}

function looseEq(a: unknown, b: unknown): boolean {
  if (JSON.stringify(a) === JSON.stringify(b)) return true
  if (typeof a === 'number' || typeof b === 'number') {
    const x = asNumber(a)
    const y = asNumber(b)
    return x != null && y != null && x === y
  }
  if (typeof a === 'boolean' && typeof b === 'string') return b.trim().toLowerCase() === String(a)
  if (typeof b === 'boolean' && typeof a === 'string') return a.trim().toLowerCase() === String(b)
  return false
}

function show(v: unknown): string {
  const s = JSON.stringify(v)
  return s.length > 60 ? `${s.slice(0, 57)}…` : s
}

type Compiled = { error?: { field: string; message: string }; eval: (input: MockFreeInput) => Outcome }

function compileRegex(pattern: string, ci: boolean): RegExp | string {
  try {
    return new RegExp(pattern, ci ? 'i' : '')
  } catch (e) {
    // "Invalid regular expression: /(free/: Unterminated group" → the cause
    const cause = (e as Error).message.split(': ').pop() ?? ''
    return `Invalid regular expression: ${cause.charAt(0).toLowerCase()}${cause.slice(1)}`
  }
}

/** Minimal JSONPath filter support for the demo. */
function compileJsonPath(expr: string): ((m: Record<string, unknown>) => boolean) | string {
  const term = /^\s*@((?:\.[A-Za-z_][\w-]*|\[\d+\])*)\s*(?:(==|!=|<=|>=|<|>)\s*('(?:[^'\\]|\\.)*'|"(?:[^"\\]|\\.)*"|-?\d+(?:\.\d+)?|true|false|null))?\s*$/
  const orParts = expr.split('||')
  const compiled = orParts.map(part =>
    part.split('&&').map(t => {
      const m = term.exec(t)
      if (!m) return null
      const path = m[1].replace(/\[(\d+)\]/g, '.$1').split('.').filter(Boolean)
      const op = m[2]
      const lit = m[3] == null ? undefined : m[3].startsWith("'") ? m[3].slice(1, -1) : JSON.parse(m[3])
      return (model: Record<string, unknown>) => {
        let v: unknown = model
        for (const k of path) v = v && typeof v === 'object' ? (v as Record<string, unknown>)[k] : undefined
        if (op == null) return v !== undefined
        if (op === '==') return JSON.stringify(v) === JSON.stringify(lit)
        if (op === '!=') return JSON.stringify(v) !== JSON.stringify(lit)
        if (typeof v !== 'number' || typeof lit !== 'number') return false
        return op === '<' ? v < lit : op === '<=' ? v <= lit : op === '>' ? v > lit : v >= lit
      }
    }),
  )
  if (compiled.some(and => and.some(t => t == null))) {
    return 'Invalid JSONPath filter: expected comparisons like @.tier == \'free\''
  }
  return m => compiled.some(and => and.every(t => t!(m)))
}

function listedZero(meta: Record<string, unknown>, inPtr?: string | null, outPtr?: string | null): boolean | null {
  let input: unknown
  let output: unknown
  let request: unknown
  if (inPtr) {
    input = pointerGet(meta, inPtr)
    if (input === undefined) return null
    if (outPtr) {
      output = pointerGet(meta, outPtr)
      if (output === undefined) return null
    }
  } else {
    const p = meta.pricing as Record<string, unknown> | undefined
    if (!p) return null
    input = p.prompt ?? p.input
    if (input === undefined) return null
    output = p.completion ?? p.output
    request = p.request
  }
  const zero = (v: unknown) => {
    const n = asNumber(v)
    return n == null ? null : n === 0
  }
  let isZero = zero(input)
  if (isZero == null) return null
  if (output !== undefined) {
    const z = zero(output)
    if (z == null) return null
    isZero = isZero && z
  }
  if (request !== undefined) isZero = isZero && zero(request) === true
  return isZero
}

function compileCondition(when: FreeModelCondition): Compiled {
  const bad = (field: string, message: string): Compiled => ({
    error: { field, message },
    eval: () => ({ kind: 'unknown', why: 'the rule has an error' }),
  })
  const checkPointer = (p: string) => (p !== '' && !p.startsWith('/') ? 'A JSON Pointer starts with "/" (e.g. /tier)' : null)
  switch (when.type) {
    case 'model_id': {
      const pattern = when.pattern.trim()
      if (!pattern) return bad('pattern', 'Enter a pattern')
      const ci = when.case_insensitive ?? false
      const syntax = when.syntax ?? 'glob'
      if (syntax === 'regex') {
        const re = compileRegex(pattern, ci)
        if (typeof re === 'string') return bad('pattern', re)
        return { eval: i => (re.test(i.modelId) ? { kind: 'match', why: `id matches /${pattern}/` } : { kind: 'no_match', why: `not: id matches /${pattern}/` }) }
      }
      if (syntax === 'exact') {
        return {
          eval: i => ((ci ? i.modelId.toLowerCase() === pattern.toLowerCase() : i.modelId === pattern)
            ? { kind: 'match', why: `id is "${pattern}"` }
            : { kind: 'no_match', why: `not: id is "${pattern}"` }),
        }
      }
      return { eval: i => (globMatch(pattern, i.modelId, ci) ? { kind: 'match', why: `id matches "${pattern}"` } : { kind: 'no_match', why: `not: id matches "${pattern}"` }) }
    }
    case 'field': {
      const ptrErr = checkPointer(when.pointer)
      if (ptrErr) return bad('pointer', ptrErr)
      const needsValue = ['equals', 'not_equals', 'contains'].includes(when.op)
      if (needsValue && (when.value === undefined || when.value === null || when.value === '')) return bad('value', 'Enter a value')
      let re: RegExp | null = null
      if (when.op === 'matches') {
        if (typeof when.value !== 'string' || !when.value) return bad('value', 'Enter a regular expression')
        const r = compileRegex(when.value, false)
        if (typeof r === 'string') return bad('value', r)
        re = r
      }
      if (['lt', 'lte', 'gt', 'gte'].includes(when.op) && asNumber(when.value) == null) return bad('value', 'Enter a number')
      return {
        eval: i => {
          if (!i.metadata) return { kind: 'unknown', why: NO_LISTING }
          const name = when.pointer || '(model)'
          const v = pointerGet(i.metadata, when.pointer)
          if (v === undefined || v === null) {
            return when.op === 'not_exists' ? { kind: 'match', why: `${name} is missing` } : { kind: 'no_match', why: `${name} is missing` }
          }
          const is = `${name} is ${show(v)}`
          let hit = false
          switch (when.op) {
            case 'exists': hit = true; break
            case 'equals': hit = looseEq(v, when.value); break
            case 'not_equals': hit = !looseEq(v, when.value); break
            case 'contains':
              hit = typeof v === 'string' ? typeof when.value === 'string' && v.includes(when.value) : Array.isArray(v) && v.some(x => looseEq(x, when.value))
              break
            case 'matches': hit = re != null && ['string', 'number', 'boolean'].includes(typeof v) && re.test(String(v)); break
            default: {
              const x = asNumber(v)
              const y = asNumber(when.value)
              if (x == null || y == null) break
              hit = when.op === 'lt' ? x < y : when.op === 'lte' ? x <= y : when.op === 'gt' ? x > y : x >= y
            }
          }
          return { kind: hit ? 'match' : 'no_match', why: is }
        },
      }
    }
    case 'json_path': {
      const expr = when.expression.trim().replace(/^\?/, '').trim()
      if (!expr) return bad('expression', 'Enter a filter expression')
      const f = compileJsonPath(expr)
      if (typeof f === 'string') return bad('expression', f)
      return {
        eval: i => {
          if (!i.metadata) return { kind: 'unknown', why: NO_LISTING }
          return f(i.metadata) ? { kind: 'match', why: 'JSONPath filter matches' } : { kind: 'no_match', why: 'JSONPath filter does not match' }
        },
      }
    }
    case 'zero_price': {
      const sources = (when.sources ?? ['pricing_override', 'provider_listing']).filter(s => s !== 'unknown')
      if (sources.length === 0) return bad('sources', 'Choose where the price comes from')
      for (const [field, p] of [['input_pointer', when.input_pointer], ['output_pointer', when.output_pointer]] as const) {
        const e = p ? checkPointer(p) : null
        if (e) return bad(field, e)
      }
      if (when.output_pointer && !when.input_pointer) return bad('input_pointer', 'Set the input price path too (or neither)')
      return {
        eval: i => {
          for (const s of sources) {
            if (s === 'pricing_override' && i.priceOverride) {
              const zero = i.priceOverride.input_per_million === 0 && i.priceOverride.output_per_million === 0
              return zero ? { kind: 'match', why: '$0 price set on the Models tab' } : { kind: 'no_match', why: 'not $0 (price set on the Models tab)' }
            }
            if (s === 'provider_listing' && i.metadata) {
              const zero = listedZero(i.metadata, when.input_pointer, when.output_pointer)
              if (zero != null) {
                return zero ? { kind: 'match', why: "$0 price in the provider's listing" } : { kind: 'no_match', why: "not $0 (price in the provider's listing)" }
              }
            }
          }
          return { kind: 'unknown', why: 'no price known for this model' }
        },
      }
    }
    default:
      return bad('type', 'This rule type is not supported by this version')
  }
}

export interface MockEvaluator {
  errors: FreeModelRuleError[]
  ruleCount: number
  ruleMatches: (index: number, input: MockFreeInput) => boolean
  decide: (input: MockFreeInput) => ModelFreeTierInfo
}

export function mockEvaluator(kind: FreeTierKind): MockEvaluator {
  const errors: FreeModelRuleError[] = []
  const decision = (is_free: boolean, decided_by: ModelFreeTierInfo['decided_by'], rule_index: number | null, reason: string): ModelFreeTierInfo =>
    ({ is_free, decided_by, rule_index, reason })
  if (kind.kind !== 'free_models_only') {
    return {
      errors,
      ruleCount: 0,
      ruleMatches: () => false,
      decide: input => {
        switch (kind.kind) {
          case 'none':
            return decision(false, 'no_free_tier', null, 'This provider has no free tier')
          case 'always_free_local':
          case 'subscription': {
            const p = input.priceOverride
            if (p && (p.input_per_million > 0 || p.output_per_million > 0)) {
              return decision(false, 'price_override', null, `Priced on the Models tab ($${p.input_per_million.toFixed(2)} in / $${p.output_per_million.toFixed(2)} out per 1M tokens)`)
            }
            return decision(true, 'provider', null, kind.kind === 'subscription' ? 'Included in the subscription' : 'Local provider: every model is free')
          }
          case 'rate_limited_free':
            return decision(true, 'provider', null, "Free within the provider's rate limits")
          default:
            return decision(true, 'provider', null, 'Free while the free credits last')
        }
      },
    }
  }
  const rules: (FreeModelRule & { compiled: Compiled })[] = (kind.rules ?? []).map((r, i) => {
    const compiled = compileCondition(r.when)
    if (compiled.error) errors.push({ rule_index: i, ...compiled.error })
    return { ...r, compiled }
  })
  const included = kind.free_model_patterns.map(p => p.trim()).filter(Boolean)
  const excluded = (kind.excluded_models ?? []).map(p => p.trim()).filter(Boolean)
  return {
    errors,
    ruleCount: rules.length,
    ruleMatches: (index, input) => rules[index]?.compiled.eval(input).kind === 'match',
    decide: input => {
      const ex = excluded.find(p => globMatch(p, input.modelId, false))
      if (ex) return decision(false, 'excluded_list', null, `Never free (matches "${ex}")`)
      const inc = included.find(p => globMatch(p, input.modelId, false))
      if (inc) return decision(true, 'included_list', null, `Always free (matches "${inc}")`)
      for (let i = 0; i < rules.length; i++) {
        const r = rules[i]
        if (r.enabled === false) continue
        const include = (r.effect ?? 'include') === 'include'
        const o = r.compiled.eval(input)
        if (include && o.kind === 'match') return decision(true, 'rule', i, `Rule ${i + 1}: ${o.why}`)
        if (!include && o.kind === 'match') return decision(false, 'rule', i, `Rule ${i + 1} (exclude): ${o.why}`)
        if (!include && o.kind === 'unknown') return decision(false, 'rule', i, `Rule ${i + 1} (exclude) applies: ${o.why}`)
      }
      return decision(false, 'no_rule', null, rules.some(r => r.enabled !== false) ? 'No rule matches' : 'Not on the free list')
    },
  }
}

/** The demo's preview_free_model_rules. */
export function mockPreview(
  kind: FreeTierKind,
  models: { id: string; name: string; enabled: boolean; priceOverride?: PriceOverride | null }[],
  metadata: Record<string, Record<string, unknown>>,
): FreeModelRulesPreview {
  const ev = mockEvaluator(kind)
  const ruleMatches = new Array(ev.ruleCount).fill(0)
  const rows = models.map(m => {
    const input: MockFreeInput = { modelId: m.id, metadata: metadata[m.id] ?? null, priceOverride: m.priceOverride }
    for (let i = 0; i < ev.ruleCount; i++) if (ev.ruleMatches(i, input)) ruleMatches[i]++
    return { model_id: m.id, name: m.name, enabled: m.enabled, has_metadata: !!metadata[m.id], free: ev.decide(input) }
  })
  return {
    models: rows,
    errors: ev.errors,
    rule_matches: ruleMatches,
    free_count: rows.filter(r => r.free.is_free).length,
    discovery_error: null,
    metadata_available: Object.keys(metadata).length > 0,
  }
}
