import { Shield, KeyRound, Wrench, Minimize2, Cpu, BookText, Database, Brain } from "lucide-react"
import type { LucideIcon } from "lucide-react"

export interface FeatureDefinition {
  icon: LucideIcon
  color: string
  borderColor: string
  name: string
  shortName: string
  viewId: string
  experimental?: boolean
}

export const FEATURES: Record<string, FeatureDefinition> & {
  guardrails: FeatureDefinition
  secretScanning: FeatureDefinition
  jsonRepair: FeatureDefinition
  compression: FeatureDefinition
  routing: FeatureDefinition
  catalogCompression: FeatureDefinition
  responseRag: FeatureDefinition
  memory: FeatureDefinition
} = {
  guardrails:         { icon: Shield,    color: "text-red-500",     borderColor: "border-red-500/30",     name: "GuardRails",                 shortName: "GuardRails",      viewId: "guardrails" },
  secretScanning:     { icon: KeyRound,  color: "text-orange-500",  borderColor: "border-orange-500/30",  name: "Secret Scanning",            shortName: "Secret Scanning", viewId: "secret-scanning" },
  jsonRepair:         { icon: Wrench,    color: "text-amber-500",   borderColor: "border-amber-500/30",   name: "JSON Repair",                shortName: "JSON Repair",     viewId: "json-repair" },
  compression:        { icon: Minimize2, color: "text-blue-500",    borderColor: "border-blue-500/30",    name: "Prompt Compression",         shortName: "Compression",     viewId: "compression" },
  routing:            { icon: Cpu,       color: "text-purple-500",  borderColor: "border-purple-500/30",  name: "Strong/Weak Routing",        shortName: "Strong/Weak",     viewId: "strong-weak" },
  catalogCompression: { icon: BookText,  color: "text-teal-500",    borderColor: "border-teal-500/30",    name: "MCP Catalog Indexing",       shortName: "Catalog",         viewId: "catalog-compression" },
  responseRag:        { icon: Database,  color: "text-emerald-500", borderColor: "border-emerald-500/30", name: "Tool Responses Indexing",    shortName: "Responses",       viewId: "response-rag" },
  memory:             { icon: Brain,     color: "text-pink-500",    borderColor: "border-pink-500/30",    name: "Indexed Conversation Memory", shortName: "Memory",         viewId: "memory", experimental: true },
}

/** Non-feature experimental flags (client modes, etc.) */
export const EXPERIMENTAL = {
  mcpViaLlm: true,
} as const

export type FeatureKey = keyof typeof FEATURES

/** Backward-compatible alias — maps feature keys to their text color class. */
export const OPTIMIZE_COLORS = Object.fromEntries(
  Object.entries(FEATURES).map(([key, def]) => [key, def.color])
) as { readonly [K in FeatureKey]: string }
