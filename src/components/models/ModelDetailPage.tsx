import { useState, useEffect } from 'react'
import { invoke } from '@tauri-apps/api/core'
import Card from '../ui/Card'
import Badge from '../ui/Badge'
import ProviderIcon from '../ProviderIcon'
import { ContextualChat } from '../chat/ContextualChat'

interface ModelDetailPageProps {
  modelKey: string // format: "provider/model_id"
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

export default function ModelDetailPage({ modelKey }: ModelDetailPageProps) {
  const [providerInstance, modelId] = modelKey.split('/')
  const [model, setModel] = useState<Model | null>(null)
  const [loading, setLoading] = useState(true)

  useEffect(() => {
    loadModelData()
  }, [modelKey])

  const loadModelData = async () => {
    setLoading(true)
    try {
      const basicModels = await invoke<Array<{ id: string; provider: string }>>('list_all_models')
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

  return (
    <div className="grid grid-cols-2 gap-6 h-full">
      {/* Left Column: Model Details */}
      <div className="space-y-6">
        <Card>
          <div className="flex items-center gap-4 mb-6">
            <ProviderIcon providerId={model.provider_type || model.provider_instance} size={48} />
            <div>
              <h2 className="text-2xl font-bold text-gray-900">{model.model_id}</h2>
              <p className="text-sm text-gray-500">{model.provider_instance}</p>
            </div>
          </div>

          <div className="space-y-4">
            <div className="border-t border-gray-200 pt-4">
              <h3 className="text-lg font-semibold text-gray-900 mb-3">Specifications</h3>
              <div className="space-y-2 text-sm">
                <div className="flex justify-between">
                  <span className="text-gray-600">Provider:</span>
                  <span className="font-medium text-gray-900">{model.provider_instance}</span>
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
            </div>

            {model.capabilities.length > 0 && (
              <div className="border-t border-gray-200 pt-4">
                <h3 className="text-lg font-semibold text-gray-900 mb-3">Capabilities</h3>
                <div className="flex flex-wrap gap-2">
                  {model.capabilities.map((cap) => (
                    <Badge key={cap} variant="warning">
                      {cap}
                    </Badge>
                  ))}
                </div>
              </div>
            )}

            <div className="border-t border-gray-200 pt-4">
              <div className="bg-blue-50 border border-blue-200 rounded-lg p-3">
                <p className="text-xs text-blue-900">
                  <strong>Note:</strong> This model can be accessed via the OpenAI-compatible API
                  using the model identifier: <code className="bg-blue-100 px-1 rounded">{model.provider_instance}/{model.model_id}</code>
                </p>
              </div>
            </div>
          </div>
        </Card>
      </div>

      {/* Right Column: Chat Interface */}
      <div className="space-y-6">
        <Card>
          <h3 className="text-lg font-semibold text-gray-900 mb-4">Chat with Model</h3>
          <ContextualChat
            context={{
              type: 'model',
              providerInstance: model.provider_instance,
              modelId: model.model_id,
            }}
          />
        </Card>
      </div>
    </div>
  )
}
