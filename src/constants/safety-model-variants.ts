export const MODEL_FAMILY_GROUPS = [
  { family: "Llama Guard", modelType: "llama_guard" },
  { family: "Granite Guardian", modelType: "granite_guardian" },
  { family: "ShieldGemma", modelType: "shield_gemma" },
  { family: "Nemotron", modelType: "nemotron" },
  { family: "OpenAI Moderation", modelType: "openai_moderation" },
  { family: "Mistral Moderation", modelType: "mistral_moderation" },
]

/** Models that produce logprobs-based confidence scores */
export const CONFIDENCE_MODEL_TYPES = new Set(["granite_guardian", "shield_gemma", "openai_moderation", "mistral_moderation"])

/** Provider types that are cloud-only (no pull needed, model always available) */
export const CLOUD_PROVIDER_TYPES = new Set(["openai", "mistral", "deepinfra", "groq", "togetherai"])

/**
 * Provider model name mappings for each model type.
 * For local models: maps providerType → modelName (e.g., ollama → "llama-guard3:1b")
 * For cloud models: maps providerType → modelName (e.g., openai → "omni-moderation-latest")
 */
export const PROVIDER_MODEL_NAMES: Record<string, Record<string, string>> = {
  llama_guard: {
    ollama: "llama-guard3:1b",
    localai: "llama-guard3",
    deepinfra: "meta-llama/Llama-Guard-4-12B",
    groq: "meta-llama/llama-guard-4-12b",
    togetherai: "meta-llama/Llama-Guard-4-12B",
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
    // No longer available on Ollama (removed from registry)
  },
  openai_moderation: {
    openai: "omni-moderation-latest",
  },
  mistral_moderation: {
    mistral: "mistral-moderation-latest",
  },
}

/** Pricing labels for cloud-hosted safety models */
export const CLOUD_MODEL_PRICING: Record<string, Record<string, string>> = {
  openai_moderation: { openai: "Free" },
  mistral_moderation: { mistral: "~$0.10/1M tokens" },
  llama_guard: {
    deepinfra: "$0.18/1M tokens",
    groq: "$0.20/1M tokens",
    togetherai: "$0.20/1M tokens",
  },
}
