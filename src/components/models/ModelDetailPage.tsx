import { useState, useEffect, useMemo } from 'react'
import { invoke } from '@tauri-apps/api/core'
import Card from '../ui/Card'
import Badge from '../ui/Badge'
import ProviderIcon from '../ProviderIcon'
import { ContextualChat } from '../chat/ContextualChat'
import DetailPageLayout from '../layouts/DetailPageLayout'
import { MetricsChart } from '../charts/MetricsChart'
import { useMetricsSubscription } from '../../hooks/useMetricsSubscription'

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
  const [apiKeys, setApiKeys] = useState<ApiKey[]>([])
  const [loading, setLoading] = useState(true)
  const [activeTab, setActiveTab] = useState<string>('metrics')

  useEffect(() => {
    loadModelData()
  }, [modelKey])

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
      <div className="bg-white rounded-lg shadow-sm p-6">
        <div className="text-center py-8 text-gray-500">Loading model details...</div>
      </div>
    )
  }

  // Memoize context object to prevent re-renders
  const chatContext = useMemo(() => ({
    type: 'model' as const,
    providerInstance: model.provider_instance,
    modelId: model.model_id,
  }), [model.provider_instance, model.model_id]);

  const tabs = [
    {
      id: 'metrics',
      label: 'Metrics',
      content: (
        <div className="space-y-6">
          <div className="grid grid-cols-2 gap-6">
            <MetricsChart
              scope="model"
              scopeId={modelKey}
              timeRange="day"
              metricType="requests"
              title="Requests"
              refreshTrigger={refreshKey}
            />

            <MetricsChart
              scope="model"
              scopeId={modelKey}
              timeRange="day"
              metricType="tokens"
              title="Tokens"
              refreshTrigger={refreshKey}
            />

            <MetricsChart
              scope="model"
              scopeId={modelKey}
              timeRange="day"
              metricType="cost"
              title="Cost"
              refreshTrigger={refreshKey}
            />

            <MetricsChart
              scope="model"
              scopeId={modelKey}
              timeRange="day"
              metricType="latency"
              title="Latency"
              refreshTrigger={refreshKey}
            />
          </div>
        </div>
      ),
    },
    {
      id: 'details',
      label: 'Details',
      content: (
        <div className="space-y-6">
          <Card>
            <h3 className="text-lg font-semibold text-gray-900 mb-4">Specifications</h3>
            <div className="space-y-2 text-sm">
              <div className="flex justify-between">
                <span className="text-gray-600">Provider:</span>
                <button
                  onClick={() => onTabChange?.('providers', model.provider_instance)}
                  className="font-medium text-blue-600 hover:text-blue-700 hover:underline"
                >
                  {model.provider_instance}
                </button>
              </div>
              <div className="flex justify-between">
                <span className="text-gray-600">Model ID:</span>
                <span className="font-medium text-gray-900">{model.model_id}</span>
              </div>
              <div className="flex justify-between">
                <span className="text-gray-600">Context Window:</span>
                <span className="font-medium text-gray-900">
                  {formatContextWindow(model.context_window)}
                </span>
              </div>
              <div className="flex justify-between">
                <span className="text-gray-600">Streaming:</span>
                <span className="font-medium text-gray-900">
                  {model.supports_streaming ? 'Yes' : 'No'}
                </span>
              </div>
              {(model.input_price_per_million || model.output_price_per_million) && (
                <div className="flex justify-between">
                  <span className="text-gray-600">Pricing:</span>
                  <span className="font-medium text-gray-900 text-right">
                    {formatPrice(model)}
                  </span>
                </div>
              )}
              {model.parameter_count && (
                <div className="flex justify-between">
                  <span className="text-gray-600">Parameters:</span>
                  <span className="font-medium text-gray-900">{model.parameter_count}</span>
                </div>
              )}
            </div>

            {model.capabilities.length > 0 && (
              <div className="mt-4 pt-4 border-t border-gray-200">
                <h4 className="text-sm font-semibold text-gray-900 mb-2">Capabilities</h4>
                <div className="flex flex-wrap gap-2">
                  {model.capabilities.map((cap) => (
                    <Badge key={cap} variant="warning">
                      {cap}
                    </Badge>
                  ))}
                </div>
              </div>
            )}

            <div className="mt-4 pt-4 border-t border-gray-200">
              <div className="bg-blue-50 border border-blue-200 rounded-lg p-3">
                <p className="text-xs text-blue-900">
                  <strong>Note:</strong> This model can be accessed via the OpenAI-compatible API
                  using the model identifier: <code className="bg-blue-100 px-1 rounded">{model.provider_instance}/{model.model_id}</code>
                </p>
              </div>
            </div>
          </Card>

          {apiKeys.length > 0 && (
            <Card>
              <h3 className="text-lg font-semibold text-gray-900 mb-4">
                API Keys Using This Model ({apiKeys.length})
              </h3>
              <div className="space-y-2">
                {apiKeys.map((key) => (
                  <div
                    key={key.id}
                    onClick={() => onTabChange?.('api-keys', key.id)}
                    className="bg-gray-50 border border-gray-200 rounded-lg p-3 hover:bg-gray-100 transition-colors cursor-pointer"
                  >
                    <div className="flex justify-between items-center">
                      <div>
                        <h4 className="text-sm font-semibold text-gray-900">{key.name}</h4>
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
          <h3 className="text-lg font-semibold text-gray-900 mb-4">Chat with Model</h3>
          <ContextualChat context={chatContext} />
        </Card>
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
