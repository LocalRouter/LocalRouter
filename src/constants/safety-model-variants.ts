export const MODEL_FAMILY_GROUPS = [
  { family: "Llama Guard", modelType: "llama_guard" },
  { family: "Granite Guardian", modelType: "granite_guardian" },
  { family: "ShieldGemma", modelType: "shield_gemma" },
  { family: "Nemotron", modelType: "nemotron" },
  { family: "OpenAI Moderation", modelType: "openai_moderation" },
]

/** Models that produce logprobs-based confidence scores */
export const CONFIDENCE_MODEL_TYPES = new Set(["granite_guardian", "shield_gemma", "openai_moderation"])

/** Provider types that are cloud-only (no pull needed, model always available) */
export const CLOUD_PROVIDER_TYPES = new Set(["openai"])

/**
 * Provider model name mappings for each model type.
 * For local models: maps providerType → modelName (e.g., ollama → "llama-guard3:1b")
 * For cloud models: maps providerType → modelName (e.g., openai → "omni-moderation-latest")
 */
export const PROVIDER_MODEL_NAMES: Record<string, Record<string, string>> = {
  llama_guard: {
    ollama: "llama-guard3:1b",
    localai: "llama-guard3",
  },
  granite_guardian: {
    ollama: "granite3-guardian:2b",
    localai: "granite3-guardian",
  },
  shield_gemma: {
    ollama: "shieldgemma:2b",
    localai: "shieldgemma",
  },
  nemotron: {
    ollama: "llama-3.1-nemotron-safety-guard:8b",
  },
  openai_moderation: {
    openai: "omni-moderation-latest",
  },
}
