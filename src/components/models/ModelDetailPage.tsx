import { useState, useEffect, useMemo } from 'react'
import { invoke } from '@tauri-apps/api/core'
import Card from '../ui/Card'
import Badge from '../ui/Badge'
import ProviderIcon from '../ProviderIcon'
import { ContextualChat } from '../chat/ContextualChat'
import DetailPageLayout from '../layouts/DetailPageLayout'
import FilteredAccessLogs from '../logs/FilteredAccessLogs'
import MetricsPanel from '../MetricsPanel'
import { useMetricsSubscription } from '../../hooks/useMetricsSubscription'
import { CatalogMetadata } from '../../lib/catalog-types'

interface ModelDetailPageProps {
  modelKey: string // format: "provider/model_id"
  onTabChange?: (tab: 'providers' | 'api-keys', subTab: string) => void
}

interface Model {
  model_id: string
  provider_instance: string
  provider_type?: string
  capabilities: string[]
  context_window: number
  supports_streaming: boolean
  input_price_per_million?: number
  output_price_per_million?: number
  parameter_count?: string
  pricing_source?: 'catalog' | 'override'
}

interface ApiKey {
  id: string
  name: string
  enabled: boolean
  model_selection: any
}

export default function ModelDetailPage({ modelKey, onTabChange }: ModelDetailPageProps) {
  const refreshKey = useMetricsSubscription()
  const [providerInstance, modelId] = modelKey.split('/')
  const [model, setModel] = useState<Model | null>(null)
  const [catalogMetadata, setCatalogMetadata] = useState<CatalogMetadata | null>(null)
  const [apiKeys, setApiKeys] = useState<ApiKey[]>([])
  const [loading, setLoading] = useState(true)
  const [activeTab, setActiveTab] = useState<string>('metrics')
  const [isEditingPricing, setIsEditingPricing] = useState(false)
  const [editInputPrice, setEditInputPrice] = useState<string>('')
  const [editOutputPrice, setEditOutputPrice] = useState<string>('')

  // Memoize context object to prevent re-renders
  // MUST be before any conditional returns (Rules of Hooks)
  const chatContext = useMemo(() => ({
    type: 'model' as const,
    providerInstance: model?.provider_instance || providerInstance,
    modelId: model?.model_id || modelId,
  }), [model?.provider_instance, model?.model_id, providerInstance, modelId]);

  useEffect(() => {
    loadModelData()
    loadCatalogMetadata()
  }, [modelKey])

  const loadCatalogMetadata = async () => {
    try {
      const metadata = await invoke<CatalogMetadata>('get_catalog_metadata')
      setCatalogMetadata(metadata)
    } catch (error) {
      console.error('Failed to load catalog metadata:', error)
    }
  }

  const loadModelData = async () => {
    setLoading(true)
    try {
      const [basicModels, keys] = await Promise.all([
        invoke<Array<{ id: string; provider: string }>>('list_all_models'),
        invoke<ApiKey[]>('list_api_keys').catch(() => []),
      ])

      const foundModel = basicModels.find((m) => m.provider === providerInstance && m.id === modelId)

      if (foundModel) {
        setModel({
          model_id: foundModel.id,
          provider_instance: foundModel.provider,
          capabilities: [],
          context_window: 0,
          supports_streaming: true,
        })
      }

      // Filter API keys that can use this model
      const filteredKeys = keys.filter((key) => {
        if (!key.model_selection) return false
        if (key.model_selection.type === 'all') return true
        if (key.model_selection.type === 'custom') {
          const providers = key.model_selection.all_provider_models || []
          const individualModels = key.model_selection.individual_models || []
          // Check if this provider is in the all_provider_models list
          if (providers.includes(providerInstance)) return true
          // Check if this specific model is in individual_models
          return individualModels.some(
            ([provider, model]: [string, string]) => provider === providerInstance && model === modelId
          )
        }
        return false
      })
      setApiKeys(filteredKeys)
    } catch (error) {
      console.error('Failed to load model data:', error)
    } finally {
      setLoading(false)
    }
  }

  const formatContextWindow = (tokens: number) => {
    if (tokens === 0) return 'N/A'
    if (tokens >= 1000000) {
      return `${(tokens / 1000000).toFixed(1)}M tokens`
    } else if (tokens >= 1000) {
      return `${(tokens / 1000).toFixed(0)}K tokens`
    }
    return `${tokens} tokens`
  }

  const formatPrice = (model: Model) => {
    if (!model.input_price_per_million && !model.output_price_per_million) {
      return 'N/A'
    }
    const input = model.input_price_per_million
      ? `$${model.input_price_per_million.toFixed(2)}`
      : '-'
    const output = model.output_price_per_million
      ? `$${model.output_price_per_million.toFixed(2)}`
      : '-'
    return `In: ${input} / Out: ${output} per 1M tokens`
  }

  if (loading || !model) {
    return (
      <div className="bg-white dark:bg-gray-800 rounded-lg shadow-sm p-6">
        <div className="text-center py-8 text-gray-500 dark:text-gray-400">Loading model details...</div>
      </div>
    )
  }

  const tabs = [
    {
      id: 'metrics',
      label: 'Metrics',
      content: (
        <MetricsPanel
          title="Model Metrics"
          chartType="llm"
          metricOptions={[
            { id: 'requests', label: 'Requests' },
            { id: 'tokens', label: 'Tokens' },
            { id: 'cost', label: 'Cost' },
            { id: 'latency', label: 'Latency' },
            { id: 'successrate', label: 'Success' },
          ]}
          scope="model"
          scopeId={modelKey}
          defaultMetric="requests"
          defaultTimeRange="day"
          refreshTrigger={refreshKey}
        />
      ),
    },
    {
      id: 'details',
      label: 'Details',
      content: (
        <div className="space-y-6">
          <Card>
            <h3 className="text-lg font-semibold text-gray-900 dark:text-gray-100 mb-4">Specifications</h3>
            <div className="space-y-2 text-sm">
              <div className="flex justify-between">
                <span className="text-gray-600 dark:text-gray-400">Provider:</span>
                <button
                  onClick={() => onTabChange?.('providers', model.provider_instance)}
                  className="font-medium text-blue-600 dark:text-blue-400 hover:text-blue-700 dark:hover:text-blue-300 hover:underline"
                >
                  {model.provider_instance}
                </button>
              </div>
              <div className="flex justify-between">
                <span className="text-gray-600 dark:text-gray-400">Model ID:</span>
                <span className="font-medium text-gray-900 dark:text-gray-100">{model.model_id}</span>
              </div>
              <div className="flex justify-between">
                <span className="text-gray-600 dark:text-gray-400">Context Window:</span>
                <span className="font-medium text-gray-900 dark:text-gray-100">
                  {formatContextWindow(model.context_window)}
                </span>
              </div>
              <div className="flex justify-between">
                <span className="text-gray-600 dark:text-gray-400">Streaming:</span>
                <span className="font-medium text-gray-900 dark:text-gray-100">
                  {model.supports_streaming ? 'Yes' : 'No'}
                </span>
              </div>
              {(model.input_price_per_million || model.output_price_per_million || isEditingPricing) && (
                <div className="space-y-2">
                  <div className="flex justify-between items-start">
                    <span className="text-gray-600 dark:text-gray-400">Pricing:</span>
                    {!isEditingPricing ? (
                      <div className="text-right">
                        <span className="font-medium text-gray-900 dark:text-gray-100 block">
                          {formatPrice(model)}
                        </span>
                        {model.pricing_source && (
                          <span className={`text-xs ${model.pricing_source === 'override' ? 'text-purple-600 dark:text-purple-400' : 'text-green-600 dark:text-green-400'}`}>
                            {model.pricing_source === 'override' ? '(Custom Override)' : '(OpenRouter Catalog)'}
                          </span>
                        )}
                      </div>
                    ) : (
                      <div className="space-y-2 flex-1 ml-4">
                        <div>
                          <label className="text-xs text-gray-600 dark:text-gray-400">Input ($/1M tokens):</label>
                          <input
                            type="number"
                            step="0.01"
                            value={editInputPrice}
                            onChange={(e) => setEditInputPrice(e.target.value)}
                            className="w-full px-2 py-1 border border-gray-300 dark:border-gray-600 rounded text-sm bg-white dark:bg-gray-700 text-gray-900 dark:text-gray-100"
                            placeholder="0.00"
                          />
                        </div>
                        <div>
                          <label className="text-xs text-gray-600 dark:text-gray-400">Output ($/1M tokens):</label>
                          <input
                            type="number"
                            step="0.01"
                            value={editOutputPrice}
                            onChange={(e) => setEditOutputPrice(e.target.value)}
                            className="w-full px-2 py-1 border border-gray-300 dark:border-gray-600 rounded text-sm bg-white dark:bg-gray-700 text-gray-900 dark:text-gray-100"
                            placeholder="0.00"
                          />
                        </div>
                      </div>
                    )}
                  </div>
                  <div className="flex gap-2 justify-end">
                    {!isEditingPricing ? (
                      <button
                        onClick={() => {
                          setEditInputPrice(model.input_price_per_million?.toString() || '')
                          setEditOutputPrice(model.output_price_per_million?.toString() || '')
                          setIsEditingPricing(true)
                        }}
                        className="text-xs text-blue-600 dark:text-blue-400 hover:text-blue-700 dark:hover:text-blue-300 hover:underline"
                      >
                        {model.pricing_source === 'override' ? 'Edit Override' : 'Override Pricing'}
                      </button>
                    ) : (
                      <>
                        <button
                          onClick={async () => {
                            try {
                              const inputPrice = parseFloat(editInputPrice) || 0
                              const outputPrice = parseFloat(editOutputPrice) || 0
                              await invoke('set_pricing_override', {
                                provider: providerInstance,
                                model: modelId,
                                inputPerMillion: inputPrice,
                                outputPerMillion: outputPrice,
                              })
                              setIsEditingPricing(false)
                              // Reload model data to show new pricing
                              await loadModelData()
                            } catch (err) {
                              console.error('Failed to set pricing override:', err)
                            }
                          }}
                          className="text-xs px-3 py-1 bg-blue-600 dark:bg-blue-500 text-white rounded hover:bg-blue-700 dark:hover:bg-blue-600"
                        >
                          Save
                        </button>
                        <button
                          onClick={() => setIsEditingPricing(false)}
                          className="text-xs px-3 py-1 bg-gray-200 dark:bg-gray-700 text-gray-700 dark:text-gray-300 rounded hover:bg-gray-300 dark:hover:bg-gray-600"
                        >
                          Cancel
                        </button>
                        {model.pricing_source === 'override' && (
                          <button
                            onClick={async () => {
                              try {
                                await invoke('delete_pricing_override', {
                                  provider: providerInstance,
                                  model: modelId,
                                })
                                setIsEditingPricing(false)
                                // Reload model data to show catalog pricing
                                await loadModelData()
                              } catch (err) {
                                console.error('Failed to delete pricing override:', err)
                              }
                            }}
                            className="text-xs px-3 py-1 bg-red-600 dark:bg-red-500 text-white rounded hover:bg-red-700 dark:hover:bg-red-600"
                          >
                            Delete Override
                          </button>
                        )}
                      </>
                    )}
                  </div>
                </div>
              )}
              {model.parameter_count && (
                <div className="flex justify-between">
                  <span className="text-gray-600 dark:text-gray-400">Parameters:</span>
                  <span className="font-medium text-gray-900 dark:text-gray-100">{model.parameter_count}</span>
                </div>
              )}
            </div>

            {model.capabilities.length > 0 && (
              <div className="mt-4 pt-4 border-t border-gray-200 dark:border-gray-700">
                <h4 className="text-sm font-semibold text-gray-900 dark:text-gray-100 mb-2">Capabilities</h4>
                <div className="flex flex-wrap gap-2">
                  {model.capabilities.map((cap) => (
                    <Badge key={cap} variant="warning">
                      {cap}
                    </Badge>
                  ))}
                </div>
              </div>
            )}

            {catalogMetadata && model.pricing_source === 'catalog' && (
              <div className="mt-4 pt-4 border-t border-gray-200 dark:border-gray-700">
                <div className="bg-green-50 dark:bg-green-900/20 border border-green-200 dark:border-green-800 rounded-lg p-3">
                  <p className="text-xs text-green-900 dark:text-green-100">
                    <strong>Pricing Source:</strong> OpenRouter model catalog embedded at build time •
                    Last updated: {new Date(catalogMetadata.fetch_date).toLocaleDateString()} •
                    Fully offline-capable
                  </p>
                </div>
              </div>
            )}
            {model.pricing_source === 'override' && (
              <div className="mt-4 pt-4 border-t border-gray-200 dark:border-gray-700">
                <div className="bg-purple-50 dark:bg-purple-900/20 border border-purple-200 dark:border-purple-800 rounded-lg p-3">
                  <p className="text-xs text-purple-900 dark:text-purple-100">
                    <strong>Pricing Source:</strong> Custom override set by you •
                    This pricing will be used for rate limiting and cost tracking
                  </p>
                </div>
              </div>
            )}

            <div className="mt-4 pt-4 border-t border-gray-200 dark:border-gray-700">
              <div className="bg-blue-50 dark:bg-blue-900/20 border border-blue-200 dark:border-blue-800 rounded-lg p-3">
                <p className="text-xs text-blue-900 dark:text-blue-100">
                  <strong>Note:</strong> This model can be accessed via the OpenAI-compatible API
                  using the model identifier: <code className="bg-blue-100 dark:bg-blue-800 px-1 rounded">{model.provider_instance}/{model.model_id}</code>
                </p>
              </div>
            </div>
          </Card>

          {apiKeys.length > 0 && (
            <Card>
              <h3 className="text-lg font-semibold text-gray-900 dark:text-gray-100 mb-4">
                API Keys Using This Model ({apiKeys.length})
              </h3>
              <div className="space-y-2">
                {apiKeys.map((key) => (
                  <div
                    key={key.id}
                    onClick={() => onTabChange?.('api-keys', key.id)}
                    className="bg-gray-50 dark:bg-gray-800 border border-gray-200 dark:border-gray-700 rounded-lg p-3 hover:bg-gray-100 dark:hover:bg-gray-700 transition-colors cursor-pointer"
                  >
                    <div className="flex justify-between items-center">
                      <div>
                        <h4 className="text-sm font-semibold text-gray-900 dark:text-gray-100">{key.name}</h4>
                      </div>
                      <Badge variant={key.enabled ? 'success' : 'warning'}>
                        {key.enabled ? 'Enabled' : 'Disabled'}
                      </Badge>
                    </div>
                  </div>
                ))}
              </div>
            </Card>
          )}
        </div>
      ),
    },
    {
      id: 'chat',
      label: 'Chat',
      content: (
        <Card>
          <h3 className="text-lg font-semibold text-gray-900 dark:text-gray-100 mb-4">Chat with Model</h3>
          <ContextualChat context={chatContext} />
        </Card>
      ),
    },
    {
      id: 'logs',
      label: 'Logs',
      content: (
        <FilteredAccessLogs
          type="llm"
          provider={providerInstance}
          model={modelId}
          active={activeTab === 'logs'}
        />
      ),
    },
  ]

  return (
    <DetailPageLayout
      icon={<ProviderIcon providerId={model.provider_type || model.provider_instance} size={48} />}
      title={model.model_id}
      subtitle={model.provider_instance}
      badges={[]}
      tabs={tabs}
      activeTab={activeTab}
      onTabChange={setActiveTab}
      loading={loading}
    />
  )
}
