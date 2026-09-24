/**
 * System One (TypeSafe Jev wire format) request/response types for
 * `POST /v1/systemone`.
 *
 * Rust: crates/lr-providers/src/systemone/ - SystemOneRequest / SystemOneResponse
 */

/** A choice option's description: plain text, `null` (key only) or structured. */
export type SystemOneOptionDescription = string | null | Record<string, unknown>

export interface SystemOneChoiceQuestion {
  type: 'choice'
  instructions: string
  /** Option key → description, in the order the options are offered (1-255). */
  criteria: Record<string, SystemOneOptionDescription>
}

export interface SystemOneScoreQuestion {
  type: 'score'
  instructions: string
  /** Ordered level descriptions, lowest first (2-10 levels). */
  criteria: string[]
}

export interface SystemOneNoulQuestion {
  type: 'noul'
  instructions: string
  criteria?: { true?: string; false?: string } | null
}

export type SystemOneQuestion =
  | SystemOneChoiceQuestion
  | SystemOneScoreQuestion
  | SystemOneNoulQuestion

export type SystemOneQuestionType = SystemOneQuestion['type']

export interface SystemOneRequest {
  /** `provider/model`, a bare model id, `localrouter/auto`, or omitted. */
  model?: string
  /** The situation being judged: text or any JSON value. */
  state: unknown
  questions: Record<string, SystemOneQuestion>
}

export interface SystemOneChoiceAnswer {
  type: 'choice'
  choice: string
  confidence: number
  probabilities: Record<string, number>
}

export interface SystemOneScoreAnswer {
  type: 'score'
  /** Expected level (Σ i·pᵢ). */
  score: number
  confidence: number
  /** Level index → level description. */
  legend: Record<string, string>
  probabilities: Record<string, number>
}

export interface SystemOneNoulAnswer {
  type: 'noul'
  /** Probability of "yes / true". */
  noul: number
}

export type SystemOneAnswer =
  | SystemOneChoiceAnswer
  | SystemOneScoreAnswer
  | SystemOneNoulAnswer

export interface SystemOneUsage {
  input_tokens: number | null
  output_tokens: number | null
}

export interface SystemOneResponse {
  model: string
  answers: Record<string, SystemOneAnswer>
  usage?: SystemOneUsage | null
  [extra: string]: unknown
}

/**
 * Value of the `x-localrouter-systemone-backend` response header: `native`
 * when the provider speaks System One, otherwise the translation mode used on
 * a chat model.
 */
export type SystemOneBackend = 'native' | 'letter_logprobs' | 'json'

export const SYSTEMONE_BACKEND_HEADER = 'x-localrouter-systemone-backend'
