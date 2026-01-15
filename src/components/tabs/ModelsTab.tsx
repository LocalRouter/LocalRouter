import { useState, useEffect } from 'react'
import { invoke } from '@tauri-apps/api/core'
import Card from '../ui/Card'
import Badge from '../ui/Badge'
import Button from '../ui/Button'
import ProviderIcon from '../ProviderIcon'
import ModelDetailPage from '../models/ModelDetailPage'

interface Model {
  model_id: string
  provider_instance: string
  provider_type: string
  capabilities: string[]
  context_window: number
  supports_streaming: boolean
  input_price_per_million?: number
  output_price_per_million?: number
  parameter_count?: string
}

type SortField = 'name' | 'provider' | 'price' | 'context' | 'parameters'
type SortDirection = 'asc' | 'desc'

interface ModelsTabProps {
  activeSubTab: string | null
}

export default function ModelsTab({ activeSubTab }: ModelsTabProps) {
  const [models, setModels] = useState<Model[]>([])
  const [loading, setLoading] = useState(true)
  const [searchTerm, setSearchTerm] = useState('')
  const [sortField, setSortField] = useState<SortField>('name')
  const [sortDirection, setSortDirection] = useState<SortDirection>('asc')
  const [filterCapability, setFilterCapability] = useState<string>('all')

  useEffect(() => {
    loadModels()
  }, [])

  const loadModels = async () => {
    setLoading(true)
    try {
      const modelList = await invoke<Model[]>('list_all_models_detailed')
      setModels(modelList)
    } catch (error) {
      console.error('Failed to load models:', error)
      // Fallback to basic list
      try {
        const basicList = await invoke<Array<{ id: string; provider: string }>>('list_all_models')
        const detailedModels: Model[] = basicList.map((m) => ({
          model_id: m.id,
          provider_instance: m.provider,
          provider_type: m.provider.split('/')[0] || 'unknown',
          capabilities: [],
          context_window: 0,
          supports_streaming: true,
        }))
        setModels(detailedModels)
      } catch (fallbackError) {
        console.error('Failed to load models (fallback):', fallbackError)
      }
    } finally {
      setLoading(false)
    }
  }

  const handleSort = (field: SortField) => {
    if (sortField === field) {
      setSortDirection(sortDirection === 'asc' ? 'desc' : 'asc')
    } else {
      setSortField(field)
      setSortDirection('asc')
    }
  }

  const getNumericPrice = (model: Model): number => {
    if (!model.input_price_per_million && !model.output_price_per_million) return Infinity
    const inputPrice = model.input_price_per_million || 0
    const outputPrice = model.output_price_per_million || 0
    return (inputPrice + outputPrice) / 2
  }

  const getNumericParameters = (model: Model): number => {
    if (!model.parameter_count) return 0
    const match = model.parameter_count.match(/(\d+(?:\.\d+)?)\s*([BM])/i)
    if (!match) return 0
    const value = parseFloat(match[1])
    const unit = match[2].toUpperCase()
    return unit === 'B' ? value : value / 1000
  }

  const filteredAndSortedModels = models
    .filter((model) => {
      const matchesSearch =
        model.model_id.toLowerCase().includes(searchTerm.toLowerCase()) ||
        model.provider_instance.toLowerCase().includes(searchTerm.toLowerCase())

      if (filterCapability === 'all') return matchesSearch

      return matchesSearch && model.capabilities.includes(filterCapability)
    })
    .sort((a, b) => {
      let comparison = 0

      switch (sortField) {
        case 'name':
          comparison = a.model_id.localeCompare(b.model_id)
          break
        case 'provider':
          comparison = a.provider_instance.localeCompare(b.provider_instance)
          break
        case 'price':
          comparison = getNumericPrice(a) - getNumericPrice(b)
          break
        case 'context':
          comparison = a.context_window - b.context_window
          break
        case 'parameters':
          comparison = getNumericParameters(a) - getNumericParameters(b)
          break
      }

      return sortDirection === 'asc' ? comparison : -comparison
    })

  const allCapabilities = Array.from(new Set(models.flatMap((m) => m.capabilities)))

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
    return `${input} / ${output}`
  }

  const formatContextWindow = (tokens: number) => {
    if (tokens === 0) return 'N/A'
    if (tokens >= 1000000) {
      return `${(tokens / 1000000).toFixed(1)}M`
    } else if (tokens >= 1000) {
      return `${(tokens / 1000).toFixed(0)}K`
    }
    return `${tokens}`
  }

  const SortButton = ({ field, label }: { field: SortField; label: string }) => (
    <button
      onClick={() => handleSort(field)}
      className={`flex items-center gap-1 px-2 py-1 rounded hover:bg-gray-100 transition-colors ${
        sortField === field ? 'text-blue-600 font-semibold' : 'text-gray-700'
      }`}
    >
      {label}
      {sortField === field && (
        <span className="text-xs">{sortDirection === 'asc' ? '↑' : '↓'}</span>
      )}
    </button>
  )

  // If a sub-tab is selected, show detail page for that specific model
  if (activeSubTab) {
    return <ModelDetailPage modelKey={activeSubTab} />
  }

  return (
    <div>
      <Card>
        <div className="mb-6">
          <h2 className="text-2xl font-bold text-gray-900 mb-2">Models</h2>
          <p className="text-sm text-gray-500">
            Browse all available models across all providers
          </p>
        </div>

        {/* Filters and Search */}
        <div className="flex flex-wrap gap-4 mb-6">
          <div className="flex-1 min-w-[200px]">
            <input
              type="text"
              placeholder="Search models or providers..."
              value={searchTerm}
              onChange={(e) => setSearchTerm(e.target.value)}
              className="w-full px-4 py-2 border border-gray-300 rounded-lg focus:outline-none focus:ring-2 focus:ring-blue-500"
            />
          </div>

          <div className="flex items-center gap-2">
            <label className="text-sm font-medium text-gray-700">Capability:</label>
            <select
              value={filterCapability}
              onChange={(e) => setFilterCapability(e.target.value)}
              className="px-3 py-2 border border-gray-300 rounded-lg focus:outline-none focus:ring-2 focus:ring-blue-500"
            >
              <option value="all">All</option>
              {allCapabilities.map((cap) => (
                <option key={cap} value={cap}>
                  {cap}
                </option>
              ))}
            </select>
          </div>

          <Button variant="secondary" onClick={loadModels}>
            Refresh
          </Button>
        </div>

        {/* Sort Controls */}
        <div className="flex gap-2 mb-4 pb-2 border-b border-gray-200">
          <span className="text-sm text-gray-600 mr-2">Sort by:</span>
          <SortButton field="name" label="Name" />
          <SortButton field="provider" label="Provider" />
          <SortButton field="price" label="Price" />
          <SortButton field="context" label="Context" />
          <SortButton field="parameters" label="Parameters" />
        </div>

        {loading ? (
          <div className="text-center py-12 text-gray-500">Loading models...</div>
        ) : filteredAndSortedModels.length === 0 ? (
          <div className="text-center py-12 text-gray-500">
            <p>No models found matching your criteria.</p>
          </div>
        ) : (
          <div className="space-y-2">
            {filteredAndSortedModels.map((model) => (
              <div
                key={`${model.provider_instance}-${model.model_id}`}
                className="bg-gray-50 border border-gray-200 rounded-lg p-4 hover:bg-gray-100 transition-colors"
              >
                <div className="flex items-start justify-between">
                  <div className="flex-1">
                    <div className="flex items-center gap-3 mb-2">
                      <ProviderIcon providerId={model.provider_type} size={24} />
                      <div>
                        <h3 className="text-base font-semibold text-gray-900">
                          {model.model_id}
                        </h3>
                        <p className="text-sm text-gray-500">{model.provider_instance}</p>
                      </div>
                    </div>

                    <div className="flex flex-wrap gap-3 text-sm text-gray-600 mt-2">
                      {model.context_window > 0 && (
                        <div className="flex items-center gap-1">
                          <span className="font-medium">Context:</span>
                          <span>{formatContextWindow(model.context_window)} tokens</span>
                        </div>
                      )}

                      {(model.input_price_per_million || model.output_price_per_million) && (
                        <div className="flex items-center gap-1">
                          <span className="font-medium">Price/M tokens:</span>
                          <span>{formatPrice(model)}</span>
                        </div>
                      )}

                      {model.parameter_count && (
                        <div className="flex items-center gap-1">
                          <span className="font-medium">Params:</span>
                          <span>{model.parameter_count}</span>
                        </div>
                      )}
                    </div>

                    {model.capabilities.length > 0 && (
                      <div className="flex flex-wrap gap-1 mt-2">
                        {model.capabilities.map((cap) => (
                          <Badge key={cap} variant="warning">
                            {cap}
                          </Badge>
                        ))}
                        {model.supports_streaming && (
                          <Badge variant="warning">Streaming</Badge>
                        )}
                      </div>
                    )}
                  </div>

                  <div className="ml-4">
                    <Button
                      variant="secondary"
                      className="px-3 py-1.5 text-xs"
                      onClick={() => {
                        /* TODO: Open chat modal */
                      }}
                    >
                      Chat
                    </Button>
                  </div>
                </div>
              </div>
            ))}
          </div>
        )}

        <div className="mt-4 text-sm text-gray-500 text-center">
          Showing {filteredAndSortedModels.length} of {models.length} models
        </div>
      </Card>
    </div>
  )
}
