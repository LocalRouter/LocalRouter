import { Shield, KeyRound, Wrench, Minimize2, Cpu, BookText, Database, Brain } from "lucide-react"
import type { LucideIcon } from "lucide-react"

export interface FeatureDefinition {
  icon: LucideIcon
  color: string
  borderColor: string
  name: string
  shortName: string
  viewId: string
}

export const FEATURES = {
  guardrails:         { icon: Shield,    color: "text-red-500",     borderColor: "border-red-500/30",     name: "GuardRails",          shortName: "GuardRails",      viewId: "guardrails" },
  secretScanning:     { icon: KeyRound,  color: "text-orange-500",  borderColor: "border-orange-500/30",  name: "Secret Scanning",     shortName: "Secret Scanning", viewId: "secret-scanning" },
  jsonRepair:         { icon: Wrench,    color: "text-amber-500",   borderColor: "border-amber-500/30",   name: "JSON Repair",         shortName: "JSON Repair",     viewId: "json-repair" },
  compression:        { icon: Minimize2, color: "text-blue-500",    borderColor: "border-blue-500/30",    name: "Prompt Compression",  shortName: "Compression",     viewId: "compression" },
  routing:            { icon: Cpu,       color: "text-purple-500",  borderColor: "border-purple-500/30",  name: "Strong/Weak Routing", shortName: "Strong/Weak",     viewId: "strong-weak" },
  catalogCompression: { icon: BookText,  color: "text-teal-500",    borderColor: "border-teal-500/30",    name: "Catalog Compression", shortName: "Catalog",         viewId: "catalog-compression" },
  responseRag:        { icon: Database,  color: "text-emerald-500", borderColor: "border-emerald-500/30", name: "Response RAG",        shortName: "RAG",             viewId: "response-rag" },
  memory:             { icon: Brain,    color: "text-pink-500",    borderColor: "border-pink-500/30",    name: "Memory",              shortName: "Memory",          viewId: "memory" },
} as const satisfies Record<string, FeatureDefinition>

export type FeatureKey = keyof typeof FEATURES

/** Backward-compatible alias — maps feature keys to their text color class. */
export const OPTIMIZE_COLORS = Object.fromEntries(
  Object.entries(FEATURES).map(([key, def]) => [key, def.color])
) as { readonly [K in FeatureKey]: string }
