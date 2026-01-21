import { useState, useEffect } from 'react'
import { invoke } from '@tauri-apps/api/core'
import Card from '../ui/Card'
import Button from '../ui/Button'
import Toggle from '../ui/Toggle'
import ModelSelectionTable, { Model, ModelSelectionValue } from '../ModelSelectionTable'
import RateLimitEditor, { StrategyRateLimit } from './RateLimitEditor'

export interface Strategy {
  id: string
  name: string
  parent: string | null
  allowed_models: {
    type: 'all' | 'custom'
    all_provider_models?: string[]
    individual_models?: [string, string][]
  }
  auto_config: AutoModelConfig | null
  rate_limits: StrategyRateLimit[]
}

export interface AutoModelConfig {
  enabled: boolean
  prioritized_models: [string, string][]
  available_models: [string, string][]
  routellm_config?: {
    enabled: boolean
    threshold: number
    strong_models: [string, string][]
    weak_models: [string, string][]
  }
}

interface StrategyConfigEditorProps {
  strategyId: string
  readOnly?: boolean
  onSave?: () => void
}

export default function StrategyConfigEditor({
  strategyId,
  readOnly = false,
  onSave,
}: StrategyConfigEditorProps) {
  const [strategy, setStrategy] = useState<Strategy | null>(null)
  const [models, setModels] = useState<Model[]>([])
  const [selectedModels, setSelectedModels] = useState<string[]>([])
  const [loading, setLoading] = useState(true)
  const [saving, setSaving] = useState(false)

  useEffect(() => {
    loadData()
  }, [strategyId])

  const loadData = async () => {
    setLoading(true)
    try {
      const [strategyData, modelsData] = await Promise.all([
        invoke<Strategy>('get_strategy', { strategy_id: strategyId }),
        invoke<any[]>('list_all_models'),
      ])

      setStrategy(strategyData)
      setModels(modelsData.map((m) => ({ id: m.id, provider: m.provider })))
    } catch (error) {
      console.error('Failed to load strategy:', error)
    } finally {
      setLoading(false)
    }
  }

  const updateStrategy = async (updates: Partial<Strategy>) => {
    if (!strategy || readOnly) return

    setSaving(true)
    try {
      await invoke('update_strategy', {
        strategy_id: strategy.id,
        name: updates.name !== undefined ? updates.name : null,
        allowed_models: updates.allowed_models || null,
        auto_config: updates.auto_config !== undefined ? updates.auto_config : null,
        rate_limits: updates.rate_limits || null,
      })

      // Reload strategy to get updated state
      await loadData()
      onSave?.()
    } catch (error) {
      console.error('Failed to update strategy:', error)
      alert(`Failed to update strategy: ${error}`)
    } finally {
      setSaving(false)
    }
  }

  const handleAllowedModelsChange = (selection: ModelSelectionValue) => {
    updateStrategy({ allowed_models: selection })
  }

  const handleAutoConfigToggle = (enabled: boolean) => {
    if (!strategy) return

    const newAutoConfig: AutoModelConfig = {
      enabled,
      prioritized_models: strategy.auto_config?.prioritized_models || [],
      available_models: strategy.auto_config?.available_models || [],
    }

    updateStrategy({ auto_config: newAutoConfig })
  }

  const handleAddToPrioritized = () => {
    if (!strategy || selectedModels.length === 0) return

    const newModels = selectedModels.map((modelStr) => {
      const [provider, model] = modelStr.split('/')
      return [provider, model] as [string, string]
    })

    const updatedPrioritized = [
      ...(strategy.auto_config?.prioritized_models || []),
      ...newModels,
    ]

    updateStrategy({
      auto_config: {
        ...strategy.auto_config!,
        prioritized_models: updatedPrioritized,
      },
    })

    setSelectedModels([])
  }

  const handleRemoveFromPrioritized = (index: number) => {
    if (!strategy || !strategy.auto_config) return

    const updatedPrioritized = strategy.auto_config.prioritized_models.filter((_, i) => i !== index)

    updateStrategy({
      auto_config: {
        ...strategy.auto_config,
        prioritized_models: updatedPrioritized,
      },
    })
  }

  const handleMovePrioritized = (index: number, direction: 'up' | 'down') => {
    if (!strategy || !strategy.auto_config) return

    const models = [...strategy.auto_config.prioritized_models]
    const newIndex = direction === 'up' ? index - 1 : index + 1

    if (newIndex < 0 || newIndex >= models.length) return

    ;[models[index], models[newIndex]] = [models[newIndex], models[index]]

    updateStrategy({
      auto_config: {
        ...strategy.auto_config,
        prioritized_models: models,
      },
    })
  }

  const handleRateLimitsChange = (limits: StrategyRateLimit[]) => {
    updateStrategy({ rate_limits: limits })
  }

  if (loading) {
    return (
      <div className="flex items-center justify-center py-12">
        <div className="text-gray-500 dark:text-gray-400">Loading strategy configuration...</div>
      </div>
    )
  }

  if (!strategy) {
    return (
      <div className="flex items-center justify-center py-12">
        <div className="text-red-500 dark:text-red-400">Strategy not found</div>
      </div>
    )
  }

  // Get available models (not in prioritized list)
  const prioritizedSet = new Set(
    (strategy.auto_config?.prioritized_models || []).map(([p, m]) => `${p}/${m}`)
  )
  const availableModels = models.filter((m) => !prioritizedSet.has(`${m.provider}/${m.id}`))

  // Group available models by provider
  const groupedAvailable: Record<string, Model[]> = availableModels.reduce((acc, model) => {
    if (!acc[model.provider]) acc[model.provider] = []
    acc[model.provider].push(model)
    return acc
  }, {} as Record<string, Model[]>)

  const providers = Object.keys(groupedAvailable).sort()

  return (
    <div className="space-y-6">
      {/* Allowed Models Section */}
      <Card>
        <h3 className="text-lg font-semibold text-gray-900 dark:text-gray-100 mb-4">Allowed Models</h3>
        <p className="text-sm text-gray-600 dark:text-gray-400 mb-4">
          Select which models clients using this strategy can access. This applies to all requests,
          including auto-routing.
        </p>
        <ModelSelectionTable
          models={models}
          value={strategy.allowed_models}
          onChange={handleAllowedModelsChange}
        />
      </Card>

      {/* Auto Model Configuration */}
      <Card>
        <div className="flex items-center justify-between mb-4">
          <div>
            <h3 className="text-lg font-semibold text-gray-900 dark:text-gray-100">Auto Model Configuration</h3>
            <p className="text-sm text-gray-600 dark:text-gray-400">
              Enable the <code className="bg-gray-100 dark:bg-gray-800 text-gray-900 dark:text-gray-100 px-1 py-0.5 rounded text-xs">localrouter/auto</code> virtual
              model with intelligent fallback
            </p>
          </div>
          <Toggle
            enabled={strategy.auto_config?.enabled || false}
            onChange={handleAutoConfigToggle}
            disabled={readOnly || saving}
            label="Enable auto-routing"
          />
        </div>

        {strategy.auto_config?.enabled && (
          <div className="mt-6">
            <div className="grid grid-cols-5 gap-4">
              {/* Prioritized Models (left) */}
              <div className="col-span-2">
                <h4 className="font-medium text-gray-900 dark:text-gray-100 mb-2 text-sm">Prioritized Models (In Order)</h4>
                <p className="text-xs text-gray-600 dark:text-gray-400 mb-2">
                  Models are tried in this order until one succeeds
                </p>
                <div className="border border-gray-300 dark:border-gray-600 rounded p-2 min-h-[300px] max-h-[500px] overflow-y-auto bg-gray-50 dark:bg-gray-800/50">
                  {strategy.auto_config.prioritized_models.length === 0 ? (
                    <div className="text-center text-gray-400 dark:text-gray-500 py-8 text-sm">
                      No models added. Select from available models →
                    </div>
                  ) : (
                    <div className="space-y-1">
                      {strategy.auto_config.prioritized_models.map(([provider, model], idx) => (
                        <div
                          key={idx}
                          className="flex items-center gap-2 p-2 bg-white dark:bg-gray-700 hover:bg-gray-100 dark:hover:bg-gray-600 rounded border border-gray-300 dark:border-gray-600"
                        >
                          <span className="text-xs text-gray-500 dark:text-gray-400 font-mono w-6">{idx + 1}.</span>
                          <span className="flex-1 text-sm font-mono text-gray-900 dark:text-gray-100">
                            {provider}/{model}
                          </span>
                          <div className="flex gap-1">
                            <Button
                              variant="secondary"
                              onClick={() => handleMovePrioritized(idx, 'up')}
                              disabled={idx === 0 || readOnly || saving}
                              className="px-2 py-1 text-xs"
                            >
                              ↑
                            </Button>
                            <Button
                              variant="secondary"
                              onClick={() => handleMovePrioritized(idx, 'down')}
                              disabled={idx === strategy.auto_config!.prioritized_models.length - 1 || readOnly || saving}
                              className="px-2 py-1 text-xs"
                            >
                              ↓
                            </Button>
                            <Button
                              variant="danger"
                              onClick={() => handleRemoveFromPrioritized(idx)}
                              disabled={readOnly || saving}
                              className="px-2 py-1 text-xs"
                            >
                              ×
                            </Button>
                          </div>
                        </div>
                      ))}
                    </div>
                  )}
                </div>
              </div>

              {/* Action Buttons (center) */}
              <div className="flex flex-col justify-center gap-2">
                <Button
                  onClick={handleAddToPrioritized}
                  disabled={selectedModels.length === 0 || readOnly || saving}
                >
                  Add →
                </Button>
                <Button
                  onClick={() => {
                    if (!strategy || !strategy.auto_config) return
                    updateStrategy({
                      auto_config: {
                        ...strategy.auto_config,
                        prioritized_models: [],
                      },
                    })
                  }}
                  disabled={
                    !strategy.auto_config?.prioritized_models?.length || readOnly || saving
                  }
                  variant="secondary"
                >
                  ← Clear
                </Button>
              </div>

              {/* Available Models (right) */}
              <div className="col-span-2">
                <h4 className="font-medium text-gray-900 dark:text-gray-100 mb-2 text-sm">Available Models</h4>
                <p className="text-xs text-gray-600 dark:text-gray-400 mb-2">
                  Select models to add to prioritized list
                </p>
                <div className="border border-gray-300 dark:border-gray-600 rounded p-2 min-h-[300px] max-h-[500px] overflow-y-auto bg-gray-50 dark:bg-gray-800/50">
                  {providers.length === 0 ? (
                    <div className="text-center text-gray-400 dark:text-gray-500 py-8 text-sm">
                      All models are in the prioritized list
                    </div>
                  ) : (
                    <div className="space-y-3">
                      {providers.map((provider) => (
                        <div key={provider}>
                          <div className="font-semibold text-xs text-gray-700 dark:text-gray-300 mb-1 px-1">
                            {provider}
                          </div>
                          {groupedAvailable[provider].map((model) => {
                            const modelStr = `${provider}/${model.id}`
                            const isSelected = selectedModels.includes(modelStr)

                            return (
                              <div
                                key={model.id}
                                className={`
                                  flex items-center gap-2 pl-3 py-1.5 hover:bg-white dark:hover:bg-gray-700 rounded cursor-pointer
                                  ${isSelected ? 'bg-blue-50 dark:bg-blue-900/30' : ''}
                                `}
                                onClick={() => {
                                  if (readOnly || saving) return
                                  if (isSelected) {
                                    setSelectedModels(selectedModels.filter((m) => m !== modelStr))
                                  } else {
                                    setSelectedModels([...selectedModels, modelStr])
                                  }
                                }}
                              >
                                <input
                                  type="checkbox"
                                  checked={isSelected}
                                  readOnly
                                  className="cursor-pointer"
                                />
                                <span className="text-xs font-mono text-gray-900 dark:text-gray-100">{model.id}</span>
                              </div>
                            )
                          })}
                        </div>
                      ))}
                    </div>
                  )}
                </div>
              </div>
            </div>

            {/* Fallback Note */}
            <div className="mt-4 p-3 bg-blue-50 dark:bg-blue-900/30 border border-blue-200 dark:border-blue-800 rounded">
              <p className="text-sm text-blue-800 dark:text-blue-300">
                <strong>Fallback Behavior:</strong> Models will be tried in order. Fallback occurs
                on: rate limits, policy violations, context length exceeded, unreachable, and other
                errors.
              </p>
            </div>
          </div>
        )}
      </Card>

      {/* Rate Limits Section */}
      <Card>
        <h3 className="text-lg font-semibold text-gray-900 dark:text-gray-100 mb-2">Rate Limits</h3>
        <p className="text-sm text-gray-600 dark:text-gray-400 mb-4">
          Set usage limits to control costs and prevent abuse. Limits are checked before each
          request using metrics from the database.
        </p>
        <RateLimitEditor
          limits={strategy.rate_limits}
          onChange={handleRateLimitsChange}
          disabled={readOnly || saving}
        />
        <div className="mt-3 text-xs text-gray-600 dark:text-gray-400">
          <strong>Note:</strong> Cost rate limits have no effect on models with zero pricing (e.g.,
          local models like Ollama).
        </div>
      </Card>
    </div>
  )
}
