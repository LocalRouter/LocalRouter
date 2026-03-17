import { useState, useEffect, useMemo } from "react"
import { invoke } from "@tauri-apps/api/core"
import { Cloud, Download, CircleAlert, Loader2 } from "lucide-react"
import { useIncrementalModels } from "@/hooks/useIncrementalModels"
import { Button } from "@/components/ui/Button"
import { Label } from "@/components/ui/label"
import {
  Select,
  SelectContent,
  SelectGroup,
  SelectItem,
  SelectLabel,
  SelectSeparator,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/Select"
import {
  MODEL_FAMILY_GROUPS,
  PROVIDER_MODEL_NAMES,
  CLOUD_PROVIDER_TYPES,
  CLOUD_MODEL_PRICING,
} from "@/constants/safety-model-variants"
import type { ProviderInstanceInfo } from "@/types/tauri-commands"

/** Provider types that support pulling models on-demand */
const PULLABLE_PROVIDER_TYPES = new Set(["ollama", "lmstudio", "localai"])

/** Selection result from the picker */
export type PickerSelection = {
  type: "provider"
  modelType: string
  providerId: string
  providerType: string
  modelName: string
  label: string
  needsPull: boolean
}

interface SafetyModelPickerProps {
  existingModelIds: string[]
  onSelect: (selection: PickerSelection) => void
}

interface ProviderModelEntry {
  provider: ProviderInstanceInfo
  modelName: string
  modelType: string
  available: boolean
}

interface DetailedModel {
  model_id: string
  provider_type: string
  input_price_per_million?: number | null
  output_price_per_million?: number | null
}

function formatPrice(input: number, output: number): string {
  if (input === 0 && output === 0) return "Free"
  const fmt = (v: number) => v < 0.01 ? `$${v.toFixed(3)}` : v < 1 ? `$${v.toFixed(2)}` : `$${Math.round(v)}`
  return input === output
    ? `${fmt(input)}/1M tokens`
    : `${fmt(input)}/${fmt(output)} per 1M tokens`
}

export function SafetyModelPicker({ existingModelIds, onSelect }: SafetyModelPickerProps) {
  const [selectedKey, setSelectedKey] = useState<string | null>(null)
  const [providers, setProviders] = useState<ProviderInstanceInfo[]>([])
  const [catalogPricing, setCatalogPricing] = useState<Map<string, string>>(new Map())
  const { models: incrementalModels, isFullyLoaded } = useIncrementalModels()
  const loading = !isFullyLoaded && incrementalModels.length === 0

  useEffect(() => {
    invoke<ProviderInstanceInfo[]>("list_provider_instances")
      .then(setProviders)
      .catch(() => {})

    // Load catalog pricing for cloud models
    invoke<DetailedModel[]>("list_all_models_detailed")
      .then(models => {
        const pricing = new Map<string, string>()
        for (const m of models) {
          if (m.input_price_per_million != null && m.output_price_per_million != null) {
            // Key: "providerType:modelId"
            pricing.set(`${m.provider_type}:${m.model_id}`, formatPrice(m.input_price_per_million, m.output_price_per_million))
          }
        }
        setCatalogPricing(pricing)
      })
      .catch(() => {})
  }, [])

  // Derive per-provider model sets from the incremental cache
  const providerModels = useMemo(() => {
    const modelsMap = new Map<string, Set<string>>()
    for (const model of incrementalModels) {
      if (!modelsMap.has(model.provider)) {
        modelsMap.set(model.provider, new Set())
      }
      modelsMap.get(model.provider)!.add(model.id)
    }
    return modelsMap
  }, [incrementalModels])

  // Build entries from providers × model families × available models
  const providerEntries = useMemo(() => {
    const enabledProviders = providers.filter(p => p.enabled)
    const entries: ProviderModelEntry[] = []
    for (const [modelType, providerMap] of Object.entries(PROVIDER_MODEL_NAMES)) {
      for (const provider of enabledProviders) {
        const expectedModelName = providerMap[provider.provider_type]
        if (!expectedModelName) continue

        const isCloud = CLOUD_PROVIDER_TYPES.has(provider.provider_type)
        const availableModels = providerModels.get(provider.instance_name)
        entries.push({
          provider,
          modelName: expectedModelName,
          modelType,
          available: isCloud || (availableModels?.has(expectedModelName) ?? false),
        })
      }
    }
    return entries
  }, [providers, providerModels])

  // Build the selected entry's metadata for the action handler
  const selectedEntry = useMemo(() => {
    if (!selectedKey) return null

    // Parse key format: "provider:<modelType>:<providerInstanceName>"
    const parts = selectedKey.split(":")
    const modelType = parts[1]
    const providerId = parts.slice(2).join(":")

    return providerEntries.find(
      e => e.modelType === modelType && e.provider.instance_name === providerId
    ) ?? null
  }, [selectedKey, providerEntries])

  /** Look up pricing: catalog first, then static fallback */
  const getPricingLabel = (entry: ProviderModelEntry): string | null => {
    // Try catalog (models.dev) pricing first
    const catalogKey = `${entry.provider.provider_type}:${entry.modelName}`
    const catalogPrice = catalogPricing.get(catalogKey)
    if (catalogPrice) return catalogPrice

    // Fall back to static known prices for specialty moderation models
    return CLOUD_MODEL_PRICING[entry.modelType]?.[entry.provider.provider_type] ?? null
  }

  const handleAction = () => {
    if (!selectedEntry) return

    const familyGroup = MODEL_FAMILY_GROUPS.find(g => g.modelType === selectedEntry.modelType)

    const isCloud = CLOUD_PROVIDER_TYPES.has(selectedEntry.provider.provider_type)
    onSelect({
      type: "provider",
      modelType: selectedEntry.modelType,
      providerId: selectedEntry.provider.instance_name,
      providerType: selectedEntry.provider.provider_type,
      modelName: selectedEntry.modelName,
      label: `${familyGroup?.family ?? selectedEntry.modelType} via ${selectedEntry.provider.instance_name}`,
      needsPull: !isCloud && !selectedEntry.available,
    })
    setSelectedKey(null)
  }

  const needsPull = selectedEntry && !selectedEntry.available
  const isPullable = selectedEntry && PULLABLE_PROVIDER_TYPES.has(selectedEntry.provider.provider_type)

  return (
    <div className="flex items-end gap-3">
      <div className="flex-1">
        <Label className="text-xs mb-1.5 block">Add Model</Label>
        <Select
          value={selectedKey ?? undefined}
          onValueChange={setSelectedKey}
        >
          <SelectTrigger className="h-9 text-xs">
            {loading ? (
              <span className="flex items-center gap-1.5 text-muted-foreground">
                <Loader2 className="h-3 w-3 animate-spin" />
                Loading models...
              </span>
            ) : (
              <SelectValue placeholder="Select a safety model..." />
            )}
          </SelectTrigger>
          <SelectContent>
            {loading && (
              <div className="flex items-center justify-center gap-1.5 px-3 py-4 text-xs text-muted-foreground">
                <Loader2 className="h-3.5 w-3.5 animate-spin" />
                Loading available models from providers...
              </div>
            )}
            {!loading && MODEL_FAMILY_GROUPS.map((group, groupIdx) => {
              const entries = providerEntries.filter(e => e.modelType === group.modelType)

              return (
                <SelectGroup key={group.modelType}>
                  {groupIdx > 0 && <SelectSeparator />}
                  <SelectLabel className="text-xs font-semibold pl-2">{group.family}</SelectLabel>
                  {entries.length > 0 ? (
                    entries.map((entry) => {
                      const key = `provider:${group.modelType}:${entry.provider.instance_name}`
                      const alreadyAdded = existingModelIds.includes(key)
                      const isCloudModel = CLOUD_PROVIDER_TYPES.has(entry.provider.provider_type)

                      if (entry.available) {
                        const pricing = getPricingLabel(entry)
                        const readyLabel = isCloudModel
                          ? ` — ${entry.provider.instance_name} (${pricing || 'Cloud'})`
                          : ` — Ready on ${entry.provider.instance_name}`
                        return (
                          <SelectItem key={key} value={key} className="text-xs pl-6" disabled={alreadyAdded}>
                            <span>{entry.modelName}</span>
                            <span className="text-muted-foreground">
                              {alreadyAdded ? " — Already added" : readyLabel}
                            </span>
                          </SelectItem>
                        )
                      }

                      const canPull = PULLABLE_PROVIDER_TYPES.has(entry.provider.provider_type)
                      return (
                        <SelectItem key={key} value={key} className="text-xs pl-6" disabled={alreadyAdded}>
                          <span>{entry.modelName}</span>
                          <span className="text-muted-foreground">
                            {alreadyAdded
                              ? " — Already added"
                              : canPull
                                ? ` — Pull via ${entry.provider.instance_name}`
                                : ` — Not found on ${entry.provider.instance_name}`}
                          </span>
                        </SelectItem>
                      )
                    })
                  ) : (
                    <SelectItem value={`__none:${group.modelType}`} disabled className="text-xs pl-6 text-muted-foreground italic">
                      No compatible provider configured
                    </SelectItem>
                  )}
                </SelectGroup>
              )
            })}
            {!loading && providers.filter(p => p.enabled).length === 0 && (
              <>
                <SelectSeparator />
                <div className="flex items-center gap-1.5 px-3 py-2 text-xs text-muted-foreground">
                  <CircleAlert className="h-3 w-3 shrink-0" />
                  No providers configured. Add one in Settings &rarr; Providers.
                </div>
              </>
            )}
          </SelectContent>
        </Select>
      </div>
      <Button
        size="sm"
        className="h-9"
        disabled={!selectedEntry}
        onClick={handleAction}
      >
        {needsPull && isPullable ? (
          <><Download className="h-3.5 w-3.5 mr-1.5" />Pull &amp; Add</>
        ) : (
          <><Cloud className="h-3.5 w-3.5 mr-1.5" />Add</>
        )}
      </Button>
    </div>
  )
}
