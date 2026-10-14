/** Rust: crates/lr-providers/src/systemone/decisions.rs. OpenAI Decisions wire types. */
export type DecisionValue = string | boolean
export type DecisionInput = string | Array<{
  role: 'user'
  type?: 'message'
  content: string | Array<
    | { type: 'input_text'; text: string }
    | { type: 'input_image'; image_url: string; detail?: 'low' | 'high' | 'auto' | 'original' | null }
  >
}>
export type DecisionQuestion =
  | { type: 'predicate'; name?: string | null; instructions: string }
  | { type: 'choice'; name?: string | null; instructions: string; choices: Array<{ value: DecisionValue; description?: string | null }> }
  | { type: 'score'; name?: string | null; instructions: string; levels: Array<{ label: string; description?: string | null }> }
export interface DecisionRequest {
  model: string
  input: DecisionInput
  questions: DecisionQuestion[]
  safety_identifier?: string | null
  [extension: string]: unknown
}
export type DecisionAnswer =
  | { type: 'predicate'; name: string | null; probability: number }
  | { type: 'choice'; name: string | null; choice: DecisionValue; confidence: number; probabilities: Array<{ value: DecisionValue; probability: number }> }
  | { type: 'score'; name: string | null; score: number; confidence: number; probabilities: Array<{ value: number; label: string; probability: number }> }
  | { type: 'refusal'; name: string | null }
export interface DecisionResponse {
  model: string
  answers: DecisionAnswer[]
  usage: { input_tokens: number; output_tokens: number; total_tokens: number; [detail: string]: unknown }
  [extension: string]: unknown
}
