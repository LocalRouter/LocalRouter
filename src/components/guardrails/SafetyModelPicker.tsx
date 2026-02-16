import { useState, useEffect } from "react"
import { invoke } from "@tauri-apps/api/core"
import { Download, Cloud } from "lucide-react"
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
  getVariantsForModelType,
} from "@/constants/safety-model-variants"
import type { ProviderInstanceInfo, SafetyModelDownloadStatus } from "@/types/tauri-commands"

/** Selection result from the picker */
export type PickerSelection =
  | { type: "direct_download"; variantKey: string }
  | { type: "provider"; modelType: string; providerId: string; providerType: string; modelName: string; label: string }
  | { type: "custom" }

interface SafetyModelPickerProps {
  existingModelIds: string[]
  downloadStatuses: Record<string, SafetyModelDownloadStatus>
  onSelect: (selection: PickerSelection) => void
}

interface ProviderModelMatch {
  provider: ProviderInstanceInfo
  modelName: string
  modelType: string
}

export function SafetyModelPicker({ existingModelIds, downloadStatuses, onSelect }: SafetyModelPickerProps) {
  const [selectedKey, setSelectedKey] = useState<string | null>(null)
  const [providers, setProviders] = useState<ProviderInstanceInfo[]>([])
  const [providerModelMatches, setProviderModelMatches] = useState<ProviderModelMatch[]>([])

  useEffect(() => {
    invoke<ProviderInstanceInfo[]>("list_provider_instances")
      .then(setProviders)
      .catch(() => {})
  }, [])

  // Check which provider models actually exist
  useEffect(() => {
    const enabledProviders = providers.filter(p => p.enabled)
    if (enabledProviders.length === 0) {
      setProviderModelMatches([])
      return
    }

    const checkProviders = async () => {
      const matches: ProviderModelMatch[] = []

      for (const provider of enabledProviders) {
        try {
          const models = await invoke<{ id: string }[]>("list_provider_models", {
            instanceName: provider.instance_name,
          })
          const modelIds = new Set(models.map(m => m.id))

          for (const [modelType, providerMap] of Object.entries(PROVIDER_MODEL_NAMES)) {
            const expectedModelName = providerMap[provider.provider_type]
            if (expectedModelName && modelIds.has(expectedModelName)) {
              matches.push({
                provider,
                modelName: expectedModelName,
                modelType,
              })
            }
          }
        } catch {
          // Provider model listing failed, skip
        }
      }

      setProviderModelMatches(matches)
    }

    checkProviders()
  }, [providers])

  const handleChange = (value: string) => {
    if (value === "custom") {
      setSelectedKey(null)
      onSelect({ type: "custom" })
      return
    }
    setSelectedKey(value)
  }

  const handleAction = () => {
    if (!selectedKey) return

    if (selectedKey.startsWith("provider:")) {
      const parts = selectedKey.split(":")
      const modelType = parts[1]
      const providerId = parts.slice(2).join(":")
      const match = providerModelMatches.find(
        m => m.modelType === modelType && m.provider.instance_name === providerId
      )
      const familyGroup = MODEL_FAMILY_GROUPS.find(g => g.modelType === modelType)

      onSelect({
        type: "provider",
        modelType,
        providerId,
        providerType: match?.provider.provider_type ?? "",
        modelName: match?.modelName ?? "",
        label: `${familyGroup?.family ?? modelType} via ${providerId}`,
      })
      setSelectedKey(null)
    } else {
      onSelect({ type: "direct_download", variantKey: selectedKey })
      setSelectedKey(null)
    }
  }

  const isProvider = selectedKey?.startsWith("provider:")

  return (
    <div className="flex items-end gap-3">
      <div className="flex-1">
        <Label className="text-xs mb-1.5 block">Add Model</Label>
        <Select
          value={selectedKey ?? undefined}
          onValueChange={handleChange}
        >
          <SelectTrigger className="h-9 text-xs">
            <SelectValue placeholder="Select a safety model..." />
          </SelectTrigger>
          <SelectContent>
            {MODEL_FAMILY_GROUPS.map((group) => {
              const variants = getVariantsForModelType(group.modelType)
              const matchingProviders = providerModelMatches.filter(
                m => m.modelType === group.modelType
              )

              if (variants.length === 0 && matchingProviders.length === 0) return null

              return (
                <SelectGroup key={group.modelType}>
                  <SelectLabel className="text-xs font-semibold pl-2">{group.family}</SelectLabel>
                  {variants.map((v) => {
                    const isAdded = existingModelIds.includes(v.key)
                    const isDownloaded = downloadStatuses[v.key]?.downloaded
                    return (
                      <SelectItem
                        key={v.key}
                        value={v.key}
                        className="text-xs pl-10"
                        disabled={isAdded}
                      >
                        {v.label} ({v.size})
                        {v.recommended ? " (Recommended)" : ""}
                        {isAdded ? " — Added" : isDownloaded ? " — Downloaded" : ""}
                      </SelectItem>
                    )
                  })}
                  {matchingProviders.map((m) => {
                    const key = `provider:${group.modelType}:${m.provider.instance_name}`
                    return (
                      <SelectItem
                        key={key}
                        value={key}
                        className="text-xs pl-10"
                      >
                        {m.modelName} — via {m.provider.instance_name}
                      </SelectItem>
                    )
                  })}
                </SelectGroup>
              )
            })}
            <SelectSeparator />
            <SelectItem value="custom" className="text-xs">
              Custom...
            </SelectItem>
          </SelectContent>
        </Select>
      </div>
      <Button
        size="sm"
        className="h-9"
        disabled={!selectedKey}
        onClick={handleAction}
      >
        {isProvider ? (
          <><Cloud className="h-3.5 w-3.5 mr-1.5" />Use</>
        ) : (
          <><Download className="h-3.5 w-3.5 mr-1.5" />Download</>
        )}
      </Button>
    </div>
  )
}
