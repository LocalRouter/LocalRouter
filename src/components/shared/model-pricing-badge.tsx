import type { FreeTierKind } from "@/types/tauri-commands"
import { cn } from "@/lib/utils"

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
  variant?: "compact" | "full"
}

function formatPricePart(price: number): string {
  if (price === 0) return "$0"
  if (price < 0.01) return `$${price.toFixed(4)}`
  if (price < 1) return `$${price.toFixed(3)}`
  return `$${price.toFixed(2)}`
}

export function ModelPricingBadge({
  inputPricePerMillion,
  outputPricePerMillion,
  freeTierKind,
  variant = "compact",
}: ModelPricingBadgeProps) {
  const kind = freeTierKind?.kind ?? "none"
  const hasInput = inputPricePerMillion != null && inputPricePerMillion > 0
  const hasOutput = outputPricePerMillion != null && outputPricePerMillion > 0
  const hasPricing = hasInput || hasOutput

  if (variant === "full") {
    return <FullVariant
      inputPricePerMillion={inputPricePerMillion}
      outputPricePerMillion={outputPricePerMillion}
      freeTierKind={freeTierKind}
      hasPricing={hasPricing}
      kind={kind}
    />
  }

  // Compact variant
  if (kind === "always_free_local" || kind === "subscription") {
    return <span className="text-xs font-medium text-green-600 dark:text-green-400">Free</span>
  }

  if (!hasPricing) {
    return <span className="text-xs text-muted-foreground">—</span>
  }

  const priceStr = `${formatPricePart(inputPricePerMillion ?? 0)}/${formatPricePart(outputPricePerMillion ?? 0)}`

  if (kind === "rate_limited_free" || kind === "credit_based" || kind === "free_models_only") {
    return (
      <span className="flex items-center gap-1 text-xs text-muted-foreground">
        <span>{priceStr}</span>
        <span className="inline-block h-1.5 w-1.5 rounded-full bg-green-500 flex-shrink-0" title="Free tier available" />
      </span>
    )
  }

  return <span className="text-xs text-muted-foreground">{priceStr}</span>
}

function FullVariant({
  inputPricePerMillion,
  outputPricePerMillion,
  freeTierKind,
  hasPricing,
  kind,
}: {
  inputPricePerMillion?: number | null
  outputPricePerMillion?: number | null
  freeTierKind?: FreeTierKind | null
  hasPricing: boolean
  kind: string
}) {
  return (
    <div className="space-y-3">
      {/* Pricing */}
      {hasPricing ? (
        <div className="grid grid-cols-2 gap-4 text-sm">
          <div>
            <p className="text-muted-foreground">Input</p>
            <p className="font-medium">{formatPricePart(inputPricePerMillion ?? 0)}/M tokens</p>
          </div>
          <div>
            <p className="text-muted-foreground">Output</p>
            <p className="font-medium">{formatPricePart(outputPricePerMillion ?? 0)}/M tokens</p>
          </div>
        </div>
      ) : kind === "always_free_local" || kind === "subscription" ? (
        <p className={cn("text-sm font-medium", "text-green-600 dark:text-green-400")}>
          Free — {FREE_TIER_LABELS[kind]}
        </p>
      ) : (
        <p className="text-sm text-muted-foreground">No pricing data available</p>
      )}

      {/* Free tier detail */}
      {kind !== "none" && kind !== "always_free_local" && kind !== "subscription" && freeTierKind && (
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
    return <p className="text-xs text-muted-foreground mt-0.5">{freeTierKind.free_model_patterns.length} free model pattern(s)</p>
  }

  return null
}
