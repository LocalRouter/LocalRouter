/** GGUF download variants for safety model types */
export interface SafetyModelVariant {
  /** Unique key for this variant */
  key: string
  /** Display label */
  label: string
  /** Model type this variant belongs to */
  modelType: string
  /** HuggingFace repository ID */
  hfRepoId: string
  /** GGUF filename to download */
  ggufFilename: string
  /** Approximate download size */
  size: string
  /** Whether this is the recommended/default variant for its model type */
  recommended?: boolean
}

export const SAFETY_MODEL_VARIANTS: SafetyModelVariant[] = [
  // Llama Guard
  {
    key: "llama_guard_3_1b",
    label: "Llama Guard 3 1B",
    modelType: "llama_guard",
    hfRepoId: "QuantFactory/Llama-Guard-3-1B-GGUF",
    ggufFilename: "Llama-Guard-3-1B.Q4_K_M.gguf",
    size: "~955 MB",
    recommended: true,
  },
  {
    key: "llama_guard_3_8b",
    label: "Llama Guard 3 8B",
    modelType: "llama_guard",
    hfRepoId: "QuantFactory/Llama-Guard-3-8B-GGUF",
    ggufFilename: "Llama-Guard-3-8B.Q4_K_M.gguf",
    size: "~4.9 GB",
  },
  {
    key: "llama_guard_4_12b",
    label: "Llama Guard 4 12B (f16)",
    modelType: "llama_guard",
    hfRepoId: "DevQuasar/meta-llama.Llama-Guard-4-12B-GGUF",
    ggufFilename: "meta-llama.Llama-Guard-4-12B.f16.gguf",
    size: "~22.3 GB",
  },

  // Granite Guardian
  {
    key: "granite_guardian_2b",
    label: "Granite Guardian 3.0 2B",
    modelType: "granite_guardian",
    hfRepoId: "mradermacher/granite-guardian-3.0-2b-GGUF",
    ggufFilename: "granite-guardian-3.0-2b.Q4_K_M.gguf",
    size: "~1.5 GB",
    recommended: true,
  },
  {
    key: "granite_guardian_5b",
    label: "Granite Guardian 3.2 5B",
    modelType: "granite_guardian",
    hfRepoId: "ibm-research/granite-guardian-3.2-5b-GGUF",
    ggufFilename: "granite-guardian-3.2-5b.Q4_K_M.gguf",
    size: "~3.5 GB",
  },
  {
    key: "granite_guardian_8b",
    label: "Granite Guardian 3.3 8B",
    modelType: "granite_guardian",
    hfRepoId: "ibm-granite/granite-guardian-3.3-8b-GGUF",
    ggufFilename: "granite-guardian-3.3-8b.Q4_K_M.gguf",
    size: "~4.9 GB",
  },

  // ShieldGemma
  {
    key: "shieldgemma_2b",
    label: "ShieldGemma 2B",
    modelType: "shield_gemma",
    hfRepoId: "QuantFactory/shieldgemma-2b-GGUF",
    ggufFilename: "shieldgemma-2b.Q4_K_M.gguf",
    size: "~1.7 GB",
    recommended: true,
  },

  // Nemotron
  {
    key: "nemotron_safety_8b",
    label: "Nemotron Safety 8B",
    modelType: "nemotron",
    hfRepoId: "AXONVERTEX-AI-RESEARCH/Llama-3.1-Nemotron-Safety-Guard-8B-v3-Q8_0-GGUF",
    ggufFilename: "llama-3.1-nemotron-safety-guard-8b-v3-q8_0.gguf",
    size: "~8.5 GB",
    recommended: true,
  },
]

export const MODEL_FAMILY_GROUPS = [
  { family: "Llama Guard", modelType: "llama_guard" },
  { family: "Granite Guardian", modelType: "granite_guardian" },
  { family: "ShieldGemma", modelType: "shield_gemma" },
  { family: "Nemotron", modelType: "nemotron" },
]

/** Models that produce logprobs-based confidence scores */
export const CONFIDENCE_MODEL_TYPES = new Set(["granite_guardian", "shield_gemma"])

/** Get all variants for a given model type */
export function getVariantsForModelType(modelType: string): SafetyModelVariant[] {
  return SAFETY_MODEL_VARIANTS.filter((v) => v.modelType === modelType)
}

/** Get the recommended (default) variant for a model type */
export function getRecommendedVariant(modelType: string): SafetyModelVariant | undefined {
  return SAFETY_MODEL_VARIANTS.find((v) => v.modelType === modelType && v.recommended)
}

/** Find a specific variant by key */
export function findVariant(key: string): SafetyModelVariant | undefined {
  return SAFETY_MODEL_VARIANTS.find((v) => v.key === key)
}
