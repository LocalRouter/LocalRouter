import type { FreeTierKind } from "@/types/tauri-commands"

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

function PricingPill({ kind }: { kind: string }) {
  if (kind === "subscription") {
    return (
      <span className="text-[10px] leading-tight font-medium px-1.5 py-0.5 rounded-full bg-blue-100 text-blue-700 dark:bg-blue-950/60 dark:text-blue-400 whitespace-nowrap">
        Subscription
      </span>
    )
  }

  if (kind === "none") {
    return (
      <span className="text-[10px] leading-tight font-medium px-1.5 py-0.5 rounded-full bg-muted text-muted-foreground whitespace-nowrap">
        API
      </span>
    )
  }

  // rate_limited_free, credit_based, free_models_only
  return (
    <span className="text-[10px] leading-tight font-medium px-1.5 py-0.5 rounded-full bg-green-100 text-green-700 dark:bg-green-950/60 dark:text-green-400 whitespace-nowrap">
      Free tier
    </span>
  )
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

  // Local providers are always free — don't show any pricing info
  if (kind === "always_free_local") {
    return null
  }

  const priceStr = hasPricing
    ? `${formatPriceShort(inputPricePerMillion ?? 0)}/${formatPriceShort(outputPricePerMillion ?? 0)}`
    : null

  // Subscription or free-tier without pricing — just show the pill
  if (!hasPricing && kind !== "none") {
    return <PricingPill kind={kind} />
  }

  // No free tier and no pricing — show dash
  if (!hasPricing) {
    return <span className="text-xs text-muted-foreground">—</span>
  }

  // Has pricing — show pill + price
  return (
    <span className="flex items-center gap-1.5">
      <PricingPill kind={kind} />
      <span className="text-xs text-muted-foreground">{priceStr}</span>
    </span>
  )
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
  // Local providers are always free — minimal display
  if (kind === "always_free_local") {
    return (
      <p className="text-sm text-muted-foreground">
        Free — runs locally on your machine
      </p>
    )
  }

  return (
    <div className="space-y-3">
      {/* Pricing model pill */}
      <div className="flex items-center gap-2">
        <PricingPill kind={kind} />
        {kind === "subscription" && (
          <span className="text-sm text-muted-foreground">Included in subscription</span>
        )}
      </div>

      {/* Pricing */}
      {hasPricing && (
        <div className="grid grid-cols-2 gap-4 text-sm">
          <div>
            <p className="text-muted-foreground">Input</p>
            <p className="font-medium">{formatPriceLong(inputPricePerMillion ?? 0)}/M tokens</p>
          </div>
          <div>
            <p className="text-muted-foreground">Output</p>
            <p className="font-medium">{formatPriceLong(outputPricePerMillion ?? 0)}/M tokens</p>
          </div>
        </div>
      )}

      {/* Free tier detail */}
      {kind !== "none" && kind !== "subscription" && freeTierKind && (
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
