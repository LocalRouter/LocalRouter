import { knownPrice } from '@/components/strategy/policy-model-catalog'
import type { FreeTierKind, ModelFreeTierInfo } from "@/types/tauri-commands"

export const FREE_TIER_LABELS: Record<string, string> = {
  none: 'No Free Tier',
  always_free_local: 'Always Free (Local)',
  subscription: 'Subscription',
  rate_limited_free: 'Rate Limited',
  credit_based: 'Credit Based',
  free_models_only: 'Free Models Only',
}

interface ModelPricingBadgeProps {
  inputPricePerMillion?: number | null
  outputPricePerMillion?: number | null
  freeTierKind?: FreeTierKind | null
  /** Whether this model is free under its provider's free tier (per model) */
  freeModel?: ModelFreeTierInfo | null
  variant?: "compact" | "full"
}

function formatPriceShort(price: number): string {
  if (price === 0) return "$0"
  if (price < 0.01) return `$${price.toFixed(3)}`
  if (price < 1) return `$${price.toFixed(2)}`
  return `$${Math.round(price)}`
}

function formatPriceLong(price: number): string {
  if (price === 0) return "$0"
  if (price < 0.01) return `$${price.toFixed(4)}`
  if (price < 1) return `$${price.toFixed(2)}`
  return `$${price.toFixed(2)}`
}

const GREEN_PILL = "text-[10px] leading-tight font-medium px-1.5 py-0.5 rounded-full bg-green-100 text-green-700 dark:bg-green-950/60 dark:text-green-400 whitespace-nowrap"
const MUTED_PILL = "text-[10px] leading-tight font-medium px-1.5 py-0.5 rounded-full bg-muted text-muted-foreground whitespace-nowrap"

/** A model that is not free although its provider has a free tier. */
function isPaidHere(kind: string, freeModel?: ModelFreeTierInfo | null): boolean {
  return freeModel != null && !freeModel.is_free && kind !== "none"
}

function PricingPill({ kind, freeModel }: { kind: string; freeModel?: ModelFreeTierInfo | null }) {
  const title = freeModel?.reason
  if (isPaidHere(kind, freeModel)) {
    // A priced model of a local provider is paid; a model a free-models
    // provider does not count as free is just not free.
    const label = kind === "always_free_local" || kind === "subscription" ? "Paid" : "Not free"
    return <span className={MUTED_PILL} title={title}>{label}</span>
  }

  if (kind === "always_free_local") {
    return <span className={GREEN_PILL} title={title}>Local</span>
  }

  if (kind === "subscription" || kind === "none") {
    const label = kind === "subscription" ? "Subscription" : "API"
    return <span className={MUTED_PILL} title={title}>{label}</span>
  }

  if (kind === "free_models_only" && freeModel?.is_free) {
    return <span className={GREEN_PILL} title={title}>Free</span>
  }

  // rate_limited_free, credit_based, free_models_only without per-model info
  return <span className={GREEN_PILL} title={title}>Free tier</span>
}

export function ModelPricingBadge({
  inputPricePerMillion,
  outputPricePerMillion,
  freeTierKind,
  freeModel,
  variant = "compact",
}: ModelPricingBadgeProps) {
  const kind = freeTierKind?.kind ?? "none"
  // A known price is shown, $0 included (e.g. a provider listing a free model)
  const hasInput = knownPrice(inputPricePerMillion)
  const hasOutput = knownPrice(outputPricePerMillion)
  const hasPricing = hasInput || hasOutput
  if (variant === "full") {
    return <FullVariant
      inputPricePerMillion={inputPricePerMillion}
      outputPricePerMillion={outputPricePerMillion}
      freeTierKind={freeTierKind}
      freeModel={freeModel}
      hasPricing={hasPricing}
      kind={kind}
    />
  }

  // Compact variant

  // Local providers — just show the pill, no price
  if (kind === "always_free_local" && !isPaidHere(kind, freeModel)) {
    return <PricingPill kind={kind} freeModel={freeModel} />
  }

  const priceStr = hasPricing
    ? `${(knownPrice(inputPricePerMillion) ? formatPriceShort(inputPricePerMillion) : '?')}/${(knownPrice(outputPricePerMillion) ? formatPriceShort(outputPricePerMillion) : '?')}`
    : null

  // Subscription or free-tier without pricing — just show the pill
  if (!hasPricing && kind !== "none") {
    return <PricingPill kind={kind} freeModel={freeModel} />
  }

  // No free tier and no pricing — show dash
  if (!hasPricing) {
    return <span className="text-xs text-muted-foreground">—</span>
  }

  // Has pricing — show pill + price
  return (
    <span className="flex items-center gap-1.5">
      <PricingPill kind={kind} freeModel={freeModel} />
      <span className="text-xs text-muted-foreground">{priceStr}</span>
    </span>
  )
}

function FullVariant({
  inputPricePerMillion,
  outputPricePerMillion,
  freeTierKind,
  freeModel,
  hasPricing,
  kind,
}: {
  inputPricePerMillion?: number | null
  outputPricePerMillion?: number | null
  freeTierKind?: FreeTierKind | null
  freeModel?: ModelFreeTierInfo | null
  hasPricing: boolean
  kind: string
}) {
  const paidHere = isPaidHere(kind, freeModel)
  // Local providers — show pill with explanation
  if (kind === "always_free_local" && !paidHere) {
    return (
      <div className="flex items-center gap-2">
        <PricingPill kind={kind} freeModel={freeModel} />
        <span className="text-sm text-muted-foreground">Runs locally on your machine</span>
      </div>
    )
  }

  return (
    <div className="space-y-3">
      {/* Pricing model pill */}
      <div className="flex items-center gap-2">
        <PricingPill kind={kind} freeModel={freeModel} />
        {kind === "subscription" && !paidHere && (
          <span className="text-sm text-muted-foreground">Included in subscription</span>
        )}
      </div>

      {/* Pricing */}
      {hasPricing && (
        <div className="grid grid-cols-2 gap-4 text-sm">
          <div>
            <p className="text-muted-foreground">Input</p>
            <p className="font-medium">{knownPrice(inputPricePerMillion) ? formatPriceLong(inputPricePerMillion) : 'Unknown'}/M tokens</p>
          </div>
          <div>
            <p className="text-muted-foreground">Output</p>
            <p className="font-medium">{knownPrice(outputPricePerMillion) ? formatPriceLong(outputPricePerMillion) : 'Unknown'}/M tokens</p>
          </div>
        </div>
      )}

      {/* Free tier detail */}
      {kind !== "none" && kind !== "subscription" && freeTierKind && !paidHere && (
        <div className="text-sm">
          <p className="text-muted-foreground mb-1">Free Tier</p>
          <p className="font-medium text-green-600 dark:text-green-400">{FREE_TIER_LABELS[kind]}</p>
          <FreeTierLimitsDetail freeTierKind={freeTierKind} />
        </div>
      )}
    </div>
  )
}

function FreeTierLimitsDetail({ freeTierKind }: { freeTierKind: FreeTierKind }) {
  if (freeTierKind.kind === "rate_limited_free") {
    const parts: string[] = []
    if (freeTierKind.max_rpm > 0) parts.push(`${freeTierKind.max_rpm} RPM`)
    if (freeTierKind.max_rpd > 0) parts.push(`${(freeTierKind.max_rpd / 1000).toFixed(1)}K RPD`)
    if (freeTierKind.max_tpm > 0) parts.push(`${(freeTierKind.max_tpm / 1000).toFixed(0)}K TPM`)
    if (freeTierKind.max_monthly_calls > 0) parts.push(`${freeTierKind.max_monthly_calls} monthly`)
    return parts.length > 0 ? <p className="text-xs text-muted-foreground mt-0.5">{parts.join(", ")}</p> : null
  }

  if (freeTierKind.kind === "credit_based") {
    return <p className="text-xs text-muted-foreground mt-0.5">${freeTierKind.budget_usd.toFixed(2)} budget</p>
  }

  if (freeTierKind.kind === "free_models_only") {
    const listed = freeTierKind.free_model_patterns.length
    const rules = freeTierKind.rules?.length ?? 0
    const parts = [
      listed > 0 ? `${listed} always free` : null,
      rules > 0 ? `${rules} rule${rules === 1 ? "" : "s"}` : null,
      freeTierKind.max_rpm > 0 ? `${freeTierKind.max_rpm} RPM` : null,
    ].filter(Boolean)
    return parts.length > 0 ? <p className="text-xs text-muted-foreground mt-0.5">{parts.join(", ")}</p> : null
  }

  return null
}

/** How a model's price was found, for display. */
export function pricingSourceLabel(source?: string | null): string | null {
  switch (source) {
    case "catalog": return "Catalog (models.dev)"
    case "override": return "Set on the Models tab"
    case "listing": return "Provider's model list"
    default: return source ?? null
  }
}
