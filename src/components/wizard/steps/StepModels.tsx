/**
 * Step 2: Select Models
 *
 * Model selection using AllowedModelsSelector.
 * Shows empty state with guidance if no models available.
 */

import { useState, useEffect } from "react"
import { invoke } from "@tauri-apps/api/core"
import { Loader2, Info } from "lucide-react"
import {
  AllowedModelsSelector,
  type AllowedModelsSelection,
  type Model,
} from "@/components/strategy/AllowedModelsSelector"

interface StepModelsProps {
  allowedModels: AllowedModelsSelection
  onChange: (selection: AllowedModelsSelection) => void
}

export function StepModels({ allowedModels, onChange }: StepModelsProps) {
  const [models, setModels] = useState<Model[]>([])
  const [loading, setLoading] = useState(true)

  useEffect(() => {
    loadModels()
  }, [])

  const loadModels = async () => {
    try {
      setLoading(true)
      const modelList = await invoke<Array<{ id: string; provider: string }>>("list_all_models")
      setModels(modelList.map(m => ({ id: m.id, provider: m.provider })))
    } catch (error) {
      console.error("Failed to load models:", error)
      setModels([])
    } finally {
      setLoading(false)
    }
  }

  if (loading) {
    return (
      <div className="flex items-center justify-center py-12">
        <Loader2 className="h-6 w-6 animate-spin text-muted-foreground" />
      </div>
    )
  }

  if (models.length === 0) {
    return (
      <div className="space-y-4">
        <div className="rounded-lg border border-amber-500/30 bg-amber-500/10 p-4">
          <div className="flex items-start gap-3">
            <Info className="h-5 w-5 text-amber-600 dark:text-amber-400 mt-0.5 shrink-0" />
            <div className="space-y-1">
              <p className="text-sm font-medium text-amber-700 dark:text-amber-300">
                No models available
              </p>
              <p className="text-sm text-amber-600/90 dark:text-amber-400/90">
                Add a provider in the Resources tab to get started with models.
                You can continue creating this client and configure models later.
              </p>
            </div>
          </div>
        </div>
        <p className="text-xs text-muted-foreground text-center">
          Default: All future models will be allowed.
        </p>
      </div>
    )
  }

  return (
    <div className="space-y-4">
      <p className="text-sm text-muted-foreground">
        Select which models this client can access.
      </p>

      <AllowedModelsSelector
        models={models}
        value={allowedModels}
        onChange={onChange}
      />
    </div>
  )
}
