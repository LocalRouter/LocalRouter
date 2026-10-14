import type { SystemOneAnswer, SystemOneQuestion } from '@/types/systemone'

const record = (v: unknown): Record<string, unknown> | null => v != null && typeof v === 'object' && !Array.isArray(v) ? v as Record<string, unknown> : null
const valueKey = (v: unknown): string | null => typeof v === 'string' || typeof v === 'boolean' ? JSON.stringify(v) : null
const label = (v: Record<string, unknown>, i: number) => `${typeof v.name === 'string' ? v.name : 'Question'} (${i + 1})`

/** Captured traffic can be malformed. Preserve typed values without crashing the monitor. */
export function decisionQuestionsForView(value: unknown): Record<string, SystemOneQuestion> {
  const result: Record<string, SystemOneQuestion> = {}
  if (!Array.isArray(value)) return result
  value.forEach((raw, i) => {
    const q = record(raw)
    if (!q || typeof q.instructions !== 'string') return
    const instructions = q.instructions
    const id = label(q, i)
    if (q.type === 'predicate') result[id] = { type: 'noul', instructions }
    if (q.type === 'choice' && Array.isArray(q.choices)) {
      const entries = q.choices.flatMap(rawChoice => {
        const c = record(rawChoice)
        const key = c && valueKey(c.value)
        return key == null ? [] : [[key, typeof c?.description === 'string' ? c.description : null] as const]
      })
      result[id] = { type: 'choice', instructions, criteria: Object.fromEntries(entries) }
    }
    if (q.type === 'score' && Array.isArray(q.levels)) result[id] = {
      type: 'score', instructions, criteria: q.levels.flatMap(rawLevel => {
        const level = record(rawLevel)
        return typeof level?.label === 'string' ? [level.label] : []
      }),
    }
  })
  return result
}

export function decisionAnswersForView(value: unknown): Record<string, SystemOneAnswer> {
  const result: Record<string, SystemOneAnswer> = {}
  if (!Array.isArray(value)) return result
  value.forEach((raw, i) => {
    const a = record(raw)
    if (!a) return
    const id = label(a, i)
    if (a.type === 'refusal') result[id] = { type: 'refusal' }
    if (a.type === 'predicate' && typeof a.probability === 'number') result[id] = { type: 'noul', noul: a.probability }
    if (a.type === 'choice' && typeof a.confidence === 'number' && Array.isArray(a.probabilities)) {
      const choice = valueKey(a.choice)
      if (choice == null) return
      const entries = a.probabilities.flatMap(rawProbability => {
        const p = record(rawProbability)
        const key = p && valueKey(p.value)
        return key == null || typeof p?.probability !== 'number' ? [] : [[key, p.probability] as const]
      })
      result[id] = { type: 'choice', choice, confidence: a.confidence, probabilities: Object.fromEntries(entries) }
    }
    if (a.type === 'score' && typeof a.score === 'number' && typeof a.confidence === 'number' && Array.isArray(a.probabilities)) {
      const probabilities: Record<string, number> = {}
      const legend: Record<string, string> = {}
      a.probabilities.forEach(rawProbability => {
        const p = record(rawProbability)
        if (!p || typeof p.value !== 'number' || typeof p.probability !== 'number' || typeof p.label !== 'string') return
        probabilities[String(p.value)] = p.probability
        legend[String(p.value)] = p.label
      })
      result[id] = { type: 'score', score: a.score, confidence: a.confidence, probabilities, legend }
    }
  })
  return result
}
