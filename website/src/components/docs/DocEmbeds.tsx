/**
 * Registry mapping documentation section IDs to interactive demo components.
 * The DocContent component in Docs.tsx renders these after the markdown content
 * for matching sections.
 */
import { lazy, Suspense } from "react"
import type { ComponentType } from "react"

// Lazy-load demos to avoid bundling all of them upfront
const FirewallApprovalDemo = lazy(() =>
  import("../FirewallApprovalDemo").then((m) => ({ default: m.FirewallApprovalDemo }))
)
const GuardrailApprovalDemo = lazy(() =>
  import("../GuardrailApprovalDemo").then((m) => ({ default: m.GuardrailApprovalDemo }))
)
const ModelRoutingDemo = lazy(() =>
  import("./ModelRoutingDemo").then((m) => ({ default: m.ModelRoutingDemo }))
)
const MarketplaceDemo = lazy(() =>
  import("./MarketplaceDemo").then((m) => ({ default: m.MarketplaceDemo }))
)
const MarketplaceInstallDemo = lazy(() =>
  import("./MarketplaceInstallDemo").then((m) => ({ default: m.MarketplaceInstallDemo }))
)
const MetricsDemo = lazy(() =>
  import("./MetricsDemo").then((m) => ({ default: m.MetricsDemo }))
)

/** Map of section IDs to their demo components */
const docEmbeds: Record<string, ComponentType> = {
  "approval-flow": FirewallApprovalDemo,
  "content-safety-scanning": GuardrailApprovalDemo,
  "auto-routing": ModelRoutingDemo,
  "marketplace-overview": MarketplaceDemo,
  "gated-installation": MarketplaceInstallDemo,
  "graph-data": MetricsDemo,
}

/** Render the embed for a given section ID, if one exists */
export function DocEmbed({ id }: { id: string }) {
  const Component = docEmbeds[id]
  if (!Component) return null

  return (
    <div className="my-4">
      <div className="flex justify-center">
        <Suspense
          fallback={
            <div className="h-32 rounded-lg border border-dashed border-border flex items-center justify-center text-xs text-muted-foreground">
              Loading preview...
            </div>
          }
        >
          <Component />
        </Suspense>
      </div>
    </div>
  )
}
