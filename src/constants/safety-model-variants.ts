export const MODEL_FAMILY_GROUPS = [
  { family: "Llama Guard", modelType: "llama_guard" },
  { family: "Granite Guardian", modelType: "granite_guardian" },
  { family: "ShieldGemma", modelType: "shield_gemma" },
  { family: "Nemotron", modelType: "nemotron" },
]

/** Models that produce logprobs-based confidence scores */
export const CONFIDENCE_MODEL_TYPES = new Set(["granite_guardian", "shield_gemma"])

/** Provider model name mappings for each model type (for building "via Provider" entries) */
export const PROVIDER_MODEL_NAMES: Record<string, Record<string, string>> = {
  llama_guard: {
    ollama: "llama-guard3:1b",
  },
  granite_guardian: {
    ollama: "granite3-guardian:2b",
  },
  shield_gemma: {
    ollama: "shieldgemma:2b",
  },
  nemotron: {
    ollama: "llama-3.1-nemotron-safety-guard:8b",
  },
}
