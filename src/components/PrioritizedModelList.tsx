import { useState, useEffect } from 'react'
import { invoke } from '@tauri-apps/api/core'
import WarningBox from './ui/WarningBox'
import { hasOfflineModel, ProviderConfig } from '../utils/modelHelpers'

export interface Model {
  id: string
  provider: string
}

interface PrioritizedModelListProps {
  models: Model[]
  prioritizedModels: [string, string][] // [provider, model_id]
  onChange: (prioritizedModels: [string, string][]) => void
}

export default function PrioritizedModelList({
  models,
  prioritizedModels,
  onChange
}: PrioritizedModelListProps) {
  const [prioritized, setPrioritized] = useState<[string, string][]>(prioritizedModels)
  const [providerConfigs, setProviderConfigs] = useState<Map<string, ProviderConfig>>(new Map())
  const [loadingConfigs, setLoadingConfigs] = useState(true)

  useEffect(() => {
    setPrioritized(prioritizedModels)
  }, [prioritizedModels])

  // Load provider configurations for offline detection
  useEffect(() => {
    loadProviderConfigs()
  }, [])

  const loadProviderConfigs = async () => {
    setLoadingConfigs(true)
    try {
      // Get list of all provider instances
      const instances = await invoke<Array<{
        instance_name: string
        provider_type: string
        enabled: boolean
      }>>('list_provider_instances')

      // Load config for each provider instance
      const configMap = new Map<string, ProviderConfig>()

      for (const instance of instances) {
        try {
          const config = await invoke<Record<string, string>>('get_provider_config', {
            instanceName: instance.instance_name
          })

          configMap.set(instance.instance_name, {
            instance_name: instance.instance_name,
            provider_type: instance.provider_type,
            config,
          })
        } catch (error) {
          console.warn(`Failed to load config for provider ${instance.instance_name}:`, error)
          // Continue loading other providers even if one fails
        }
      }

      setProviderConfigs(configMap)
    } catch (error) {
      console.error('Failed to load provider configurations:', error)
    } finally {
      setLoadingConfigs(false)
    }
  }

  const handleAdd = (provider: string, modelId: string) => {
    // Check if already in list
    const alreadyExists = prioritized.some(([p, m]) => p === provider && m === modelId)
    if (alreadyExists) return

    const newPrioritized = [...prioritized, [provider, modelId] as [string, string]]
    setPrioritized(newPrioritized)
    onChange(newPrioritized)
  }

  const handleRemove = (index: number) => {
    const newPrioritized = prioritized.filter((_, i) => i !== index)
    setPrioritized(newPrioritized)
    onChange(newPrioritized)
  }

  const handleMoveUp = (index: number) => {
    if (index === 0) return
    const newPrioritized = [...prioritized]
    const temp = newPrioritized[index - 1]
    newPrioritized[index - 1] = newPrioritized[index]
    newPrioritized[index] = temp
    setPrioritized(newPrioritized)
    onChange(newPrioritized)
  }

  const handleMoveDown = (index: number) => {
    if (index === prioritized.length - 1) return
    const newPrioritized = [...prioritized]
    const temp = newPrioritized[index + 1]
    newPrioritized[index + 1] = newPrioritized[index]
    newPrioritized[index] = temp
    setPrioritized(newPrioritized)
    onChange(newPrioritized)
  }

  // Group models by provider
  const groupedModels: Record<string, Model[]> = models.reduce((acc, model) => {
    if (!acc[model.provider]) acc[model.provider] = []
    acc[model.provider].push(model)
    return acc
  }, {} as Record<string, Model[]>)

  const providers = Object.keys(groupedModels).sort()

  // Check if a model is already in the prioritized list
  const isPrioritized = (provider: string, modelId: string): boolean => {
    return prioritized.some(([p, m]) => p === provider && m === modelId)
  }

  // Check if there are any offline models in the prioritized list
  const showOfflineWarning = !loadingConfigs && !hasOfflineModel(prioritized, providerConfigs) && prioritized.length > 0

  return (
    <div className="space-y-6">
      {/* Offline Model Warning */}
      {showOfflineWarning && (
        <WarningBox
          variant="warning"
          title="No offline models selected"
          message={
            <>
              Your current routing strategy only includes cloud-based models.
              Internet connectivity issues may interrupt your workflow.
            </>
          }
          action={
            <>
              Consider adding a local model (<strong>Ollama</strong> or <strong>LM Studio</strong>)
              as a fallback for offline use.
            </>
          }
        />
      )}

      {/* Prioritized Models List */}
      <div className="border border-gray-300 dark:border-gray-600 rounded-lg overflow-hidden">
        <div className="bg-gray-100 dark:bg-gray-800 border-b border-gray-300 dark:border-gray-600 px-4 py-3">
          <h4 className="font-semibold text-gray-900 dark:text-gray-100">Prioritized Models ({prioritized.length})</h4>
          <p className="text-xs text-gray-600 dark:text-gray-400 mt-1">
            Models are tried in order from top to bottom. If one fails, the next is tried automatically.
          </p>
        </div>
        <div className="max-h-96 overflow-y-auto">
          {prioritized.length === 0 ? (
            <div className="p-8 text-center text-gray-500 dark:text-gray-400 text-sm">
              No models in the prioritized list. Add models from below.
            </div>
          ) : (
            <div className="divide-y divide-gray-200 dark:divide-gray-700">
              {prioritized.map(([provider, modelId], index) => (
                <div
                  key={`${provider}/${modelId}/${index}`}
                  className="p-3 hover:bg-gray-50 dark:hover:bg-gray-700 flex items-center gap-2"
                >
                  <span className="text-sm font-mono text-gray-500 dark:text-gray-400 w-8">
                    {index + 1}.
                  </span>
                  <div className="flex-1 min-w-0">
                    <p className="text-sm font-medium text-gray-900 dark:text-gray-100 truncate">{modelId}</p>
                    <p className="text-xs text-gray-500 dark:text-gray-400 truncate">{provider}</p>
                  </div>
                  <div className="flex gap-1">
                    <button
                      onClick={() => handleMoveUp(index)}
                      disabled={index === 0}
                      className="px-2 py-1 text-xs text-gray-600 dark:text-gray-300 hover:bg-gray-200 dark:hover:bg-gray-600 rounded disabled:opacity-30 disabled:cursor-not-allowed"
                      title="Move up"
                    >
                      ↑
                    </button>
                    <button
                      onClick={() => handleMoveDown(index)}
                      disabled={index === prioritized.length - 1}
                      className="px-2 py-1 text-xs text-gray-600 dark:text-gray-300 hover:bg-gray-200 dark:hover:bg-gray-600 rounded disabled:opacity-30 disabled:cursor-not-allowed"
                      title="Move down"
                    >
                      ↓
                    </button>
                    <button
                      onClick={() => handleRemove(index)}
                      className="px-2 py-1 text-xs text-red-600 dark:text-red-400 hover:bg-red-100 dark:hover:bg-red-900/30 rounded"
                      title="Remove"
                    >
                      ✕
                    </button>
                  </div>
                </div>
              ))}
            </div>
          )}
        </div>
      </div>

      {/* Add Model Section */}
      <div className="border border-gray-300 dark:border-gray-600 rounded-lg overflow-hidden">
        <div className="bg-gray-100 dark:bg-gray-800 border-b border-gray-300 dark:border-gray-600 px-4 py-3">
          <h4 className="font-semibold text-gray-900 dark:text-gray-100">Add Models</h4>
          <p className="text-xs text-gray-600 dark:text-gray-400 mt-1">
            Click + to add a model to the prioritized list
          </p>
        </div>
        <div className="max-h-96 overflow-y-auto">
          <table className="w-full">
            <tbody>
              {providers.map((provider) => {
                const providerModels = groupedModels[provider]

                return (
                  <>
                    {/* Provider header row */}
                    <tr key={provider} className="border-b border-gray-200 dark:border-gray-700 bg-gray-50 dark:bg-gray-800">
                      <td className="px-4 py-2 pl-8">
                        <span className="font-medium text-gray-800 dark:text-gray-200">{provider}</span>
                      </td>
                      <td className="w-12"></td>
                    </tr>

                    {/* Model rows */}
                    {providerModels.map((model) => {
                      const isInList = isPrioritized(provider, model.id)

                      return (
                        <tr
                          key={`${provider}/${model.id}`}
                          className="border-b border-gray-100 dark:border-gray-800 hover:bg-gray-50 dark:hover:bg-gray-700"
                        >
                          <td className="px-4 py-2 pl-16">
                            <span className="text-gray-700 dark:text-gray-300 text-sm">{model.id}</span>
                          </td>
                          <td className="px-4 py-2 text-right">
                            <button
                              onClick={() => handleAdd(provider, model.id)}
                              disabled={isInList}
                              className="text-blue-600 dark:text-blue-400 hover:text-blue-700 dark:hover:text-blue-300 disabled:text-gray-400 dark:disabled:text-gray-600 disabled:cursor-not-allowed text-lg font-bold"
                              title={isInList ? 'Already in list' : 'Add to list'}
                            >
                              +
                            </button>
                          </td>
                        </tr>
                      )
                    })}
                  </>
                )
              })}
            </tbody>
          </table>
        </div>
      </div>
    </div>
  )
}
