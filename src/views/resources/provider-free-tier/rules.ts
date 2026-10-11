import type {
  FieldOp,
  FreeModelCondition,
  FreeModelRule,
  FreeTierKind,
} from "@/types/tauri-commands"

export type FreeModelsOnly = Extract<FreeTierKind, { kind: "free_models_only" }>
export type ConditionType = FreeModelCondition["type"]

export { FREE_TIER_LABELS } from "@/components/shared/model-pricing-badge"

export const FREE_TIER_DESCRIPTIONS: Record<string, string> = {
  none: "This provider has no free tier. All requests are treated as paid. The provider is always skipped when a strategy has free-tier mode enabled.",
  always_free_local: "Local or self-hosted provider with no external billing. Always treated as free with no usage limits tracked. A model with a price set on the Models tab counts as paid.",
  subscription: "Included in an existing subscription (e.g. GitHub Copilot). Always treated as free. No usage counters are tracked.",
  rate_limited_free: "Free access within rate limits (requests per minute/day, tokens per minute/day, monthly caps). The router tracks usage against these limits and skips the provider when any limit is reached. Used by Gemini, Groq, Cerebras, Mistral, and Cohere.",
  credit_based: "Dollar-budget credits that are consumed per request. The router estimates cost from token usage and compares against the budget. When credits run out, the provider is skipped. Used by xAI, DeepInfra, and Perplexity.",
  free_models_only: "Only some models from this provider are free. Lists and rules decide which: by model ID, by fields in the provider's model list, or by a $0 price. Other models are treated as paid and skipped in free-tier mode. Used by OpenRouter and Together AI.",
}

export const CONDITION_LABELS: Record<ConditionType, string> = {
  model_id: "Model ID",
  field: "Listing field",
  json_path: "JSONPath filter",
  zero_price: "$0 price",
  unsupported: "Unsupported",
}

export const CONDITION_HELP: Record<ConditionType, string> = {
  model_id: "Match the model ID exactly, with a glob (* matches anything), or with a regular expression.",
  field: "Compare a field of the model's entry in the provider's model list, addressed by a JSON Pointer such as /tier or /pricing/prompt.",
  json_path: "An RFC 9535 filter expression where @ is the model's entry, e.g. @.tier == 'free' or @.tags[?@ == 'free'].",
  zero_price: "The model costs nothing: input and output price are $0 in the first source that has a price for it.",
  unsupported: "This rule was written by a newer version of LocalRouter and is ignored (never makes a model free).",
}

export const FIELD_OPS: { op: FieldOp; label: string; needsValue: boolean }[] = [
  { op: "equals", label: "equals", needsValue: true },
  { op: "not_equals", label: "does not equal", needsValue: true },
  { op: "contains", label: "contains", needsValue: true },
  { op: "matches", label: "matches regex", needsValue: true },
  { op: "lt", label: "<", needsValue: true },
  { op: "lte", label: "≤", needsValue: true },
  { op: "gt", label: ">", needsValue: true },
  { op: "gte", label: "≥", needsValue: true },
  { op: "exists", label: "is set", needsValue: false },
  { op: "not_exists", label: "is not set", needsValue: false },
]

export function opNeedsValue(op: FieldOp): boolean {
  return FIELD_OPS.find((o) => o.op === op)?.needsValue ?? false
}

export function newCondition(type: ConditionType): FreeModelCondition {
  switch (type) {
    case "model_id":
      return { type: "model_id", pattern: "", syntax: "glob", case_insensitive: false }
    case "field":
      return { type: "field", pointer: "", op: "equals", value: "" }
    case "json_path":
      return { type: "json_path", expression: "" }
    case "zero_price":
      return { type: "zero_price", sources: ["pricing_override", "provider_listing"] }
    default:
      return { type: "unsupported" }
  }
}

export function newRule(type: ConditionType): FreeModelRule {
  return { enabled: true, effect: "include", when: newCondition(type) }
}

export interface RulePreset {
  id: string
  label: string
  description: string
  rule: FreeModelRule
}

export const RULE_PRESETS: RulePreset[] = [
  {
    id: "zero-price",
    label: "$0 in the provider's model list",
    description: "OpenRouter, LLM gateways and other lists with a pricing object",
    rule: {
      enabled: true,
      effect: "include",
      when: { type: "zero_price", sources: ["provider_listing"] },
      label: "Priced at $0 in the model list",
    },
  },
  {
    id: "free-suffix",
    label: "ID ends with :free",
    description: "OpenRouter-style free variants",
    rule: {
      enabled: true,
      effect: "include",
      when: { type: "model_id", pattern: "*:free", syntax: "glob", case_insensitive: false },
    },
  },
  {
    id: "dash-free",
    label: "ID ends with -free (any case)",
    description: "Together AI style, e.g. …-Instruct-Turbo-Free",
    rule: {
      enabled: true,
      effect: "include",
      when: { type: "model_id", pattern: "*-free", syntax: "glob", case_insensitive: true },
    },
  },
  {
    id: "tier-free",
    label: "Listing field tier is \"free\"",
    description: "Lists that mark each model's tier",
    rule: {
      enabled: true,
      effect: "include",
      when: { type: "field", pointer: "/tier", op: "equals", value: "free" },
    },
  },
  {
    id: "exclude-preview",
    label: "Exclude preview models",
    description: "Never free when the ID contains \"preview\"",
    rule: {
      enabled: true,
      effect: "exclude",
      when: { type: "model_id", pattern: "*preview*", syntax: "glob", case_insensitive: true },
    },
  },
]

/** Escape one key for a JSON Pointer (RFC 6901). */
export function pointerToken(key: string): string {
  return key.replace(/~/g, "~0").replace(/\//g, "~1")
}

/**
 * A rule value typed in a text box: numbers, booleans and null as JSON,
 * anything else as a string.
 */
export function parseValue(text: string): unknown {
  const t = text.trim()
  if (t === "true") return true
  if (t === "false") return false
  if (t !== "" && /^-?\d+(\.\d+)?([eE][-+]?\d+)?$/.test(t)) return Number(t)
  return text
}

export function valueToText(value: unknown): string {
  if (value === undefined || value === null) return ""
  if (typeof value === "string") return value
  return JSON.stringify(value)
}

export function asFreeModelsOnly(kind: FreeTierKind): FreeModelsOnly | null {
  return kind.kind === "free_models_only" ? kind : null
}

/** The per-model choice in the preview: lists first, as the router reads them. */
export type ModelChoice = "auto" | "always" | "never"

export function modelChoice(kind: FreeModelsOnly, modelId: string): ModelChoice {
  if ((kind.excluded_models ?? []).includes(modelId)) return "never"
  if (kind.free_model_patterns.includes(modelId)) return "always"
  return "auto"
}

export function setModelChoice(kind: FreeModelsOnly, modelId: string, choice: ModelChoice): FreeModelsOnly {
  const included = kind.free_model_patterns.filter((p) => p !== modelId)
  const excluded = (kind.excluded_models ?? []).filter((p) => p !== modelId)
  if (choice === "always") included.push(modelId)
  if (choice === "never") excluded.push(modelId)
  return { ...kind, free_model_patterns: included, excluded_models: excluded }
}
