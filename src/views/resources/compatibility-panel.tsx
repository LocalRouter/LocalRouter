import { useState, useEffect } from "react"
import { invoke } from "@tauri-apps/api/core"
import { listen } from "@tauri-apps/api/event"
import { Loader2 } from "lucide-react"
import {
  Tooltip,
  TooltipContent,
  TooltipProvider,
  TooltipTrigger,
} from "@/components/ui/tooltip"
import { Card, CardContent, CardHeader, CardTitle, CardDescription } from "@/components/ui/Card"
import { SupportLevelBadge } from "@/components/shared/support-level-badge"
import ProviderIcon from "@/components/ProviderIcon"
import type { ProviderFeatureSupport, SupportLevel } from "@/types/tauri-commands"

// Which sections of ProviderFeatureSupport to render as matrix rows
interface MatrixSection {
  title: string
  key: "endpoints" | "model_features" | "optimization_features"
}

const SECTIONS: MatrixSection[] = [
  { title: "API Endpoints", key: "endpoints" },
  { title: "Model Features", key: "model_features" },
  { title: "Optimization Features", key: "optimization_features" },
]

function getFeatureName(item: { name: string }): string {
  return item.name
}

function getSupport(item: { support: SupportLevel }): SupportLevel {
  return item.support
}

function getNotes(item: { notes?: string | null }): string | null {
  return item.notes ?? null
}

export function CompatibilityPanel() {
  const [allSupport, setAllSupport] = useState<ProviderFeatureSupport[] | null>(null)
  const [loading, setLoading] = useState(true)

  const loadData = async () => {
    try {
      setLoading(true)
      const data = await invoke<ProviderFeatureSupport[]>("get_all_provider_feature_support")
      setAllSupport(data)
    } catch (err) {
      console.error("Failed to load provider feature support:", err)
    } finally {
      setLoading(false)
    }
  }

  useEffect(() => {
    loadData()
    const unlisten = listen("providers-changed", () => loadData())
    return () => { unlisten.then((fn) => fn()) }
  }, [])

  if (loading) {
    return (
      <div className="flex items-center gap-2 text-muted-foreground p-8">
        <Loader2 className="h-4 w-4 animate-spin" />
        <span>Loading compatibility data...</span>
      </div>
    )
  }

  if (!allSupport || allSupport.length === 0) {
    return (
      <div className="text-sm text-muted-foreground p-8">
        No providers configured. Add a provider to see compatibility data.
      </div>
    )
  }

  return (
    <div className="space-y-6 overflow-y-auto">
      {SECTIONS.map((section) => (
        <CompatibilityMatrix
          key={section.key}
          title={section.title}
          sectionKey={section.key}
          providers={allSupport}
        />
      ))}

      <div className="flex items-center gap-4 text-[10px] text-muted-foreground pb-4">
        <SupportLevelBadge level="supported" />
        <SupportLevelBadge level="partial" />
        <SupportLevelBadge level="translated" />
        <SupportLevelBadge level="not_supported" />
        <SupportLevelBadge level="not_implemented" />
      </div>
    </div>
  )
}

interface CompatibilityMatrixProps {
  title: string
  sectionKey: "endpoints" | "model_features" | "optimization_features"
  providers: ProviderFeatureSupport[]
}

function CompatibilityMatrix({ title, sectionKey, providers }: CompatibilityMatrixProps) {
  // Collect all unique feature names from the first provider (all providers have the same set)
  const featureNames = providers[0]?.[sectionKey]?.map(getFeatureName) ?? []

  // Build a lookup: providerInstance -> featureName -> {support, notes}
  const lookup = new Map<string, Map<string, { support: SupportLevel; notes: string | null }>>()
  for (const p of providers) {
    const featureMap = new Map<string, { support: SupportLevel; notes: string | null }>()
    for (const item of p[sectionKey]) {
      featureMap.set(getFeatureName(item), {
        support: getSupport(item),
        notes: getNotes(item),
      })
    }
    lookup.set(p.provider_instance, featureMap)
  }

  return (
    <Card>
      <CardHeader className="pb-3">
        <CardTitle className="text-sm">{title}</CardTitle>
        <CardDescription className="text-xs">
          Hover over cells for details
        </CardDescription>
      </CardHeader>
      <CardContent className="p-0">
        <TooltipProvider>
          <div className="overflow-x-auto">
            <table className="w-full text-xs">
              <thead>
                <tr className="border-b bg-muted/50">
                  <th className="text-left px-3 py-2 font-medium text-muted-foreground sticky left-0 bg-muted/50 z-10 min-w-[160px]">
                    Feature
                  </th>
                  {providers.map((p) => (
                    <th
                      key={p.provider_instance}
                      className="px-2 py-2 font-medium text-muted-foreground text-center whitespace-nowrap"
                    >
                      <Tooltip>
                        <TooltipTrigger asChild>
                          <div className="flex flex-col items-center gap-1 cursor-default">
                            <ProviderIcon providerId={p.provider_type} className="h-4 w-4" />
                            <span className="max-w-[80px] truncate text-[10px]">{p.provider_instance}</span>
                          </div>
                        </TooltipTrigger>
                        <TooltipContent side="bottom">
                          <p>{p.provider_instance} ({p.provider_type})</p>
                        </TooltipContent>
                      </Tooltip>
                    </th>
                  ))}
                </tr>
              </thead>
              <tbody className="divide-y">
                {featureNames.map((featureName) => (
                  <tr key={featureName} className="hover:bg-muted/30">
                    <td className="px-3 py-1.5 font-medium whitespace-nowrap sticky left-0 bg-background z-10">
                      {featureName}
                    </td>
                    {providers.map((p) => {
                      const cell = lookup.get(p.provider_instance)?.get(featureName)
                      return (
                        <td key={p.provider_instance} className="px-2 py-1.5 text-center">
                          {cell ? (
                            <SupportLevelBadge
                              level={cell.support}
                              notes={cell.notes}
                              compact
                            />
                          ) : (
                            <span className="text-muted-foreground/40">{"\u2014"}</span>
                          )}
                        </td>
                      )
                    })}
                  </tr>
                ))}
              </tbody>
            </table>
          </div>
        </TooltipProvider>
      </CardContent>
    </Card>
  )
}
