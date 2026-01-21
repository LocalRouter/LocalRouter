export interface Model {
  id: string
  provider: string
}

export interface ModelSelectionValue {
  type: 'all' | 'custom'
  all_provider_models?: string[]
  individual_models?: [string, string][]
}

interface ModelSelectionTableProps {
  models: Model[]
  value: ModelSelectionValue | null
  onChange: (selection: ModelSelectionValue) => void
}

export default function ModelSelectionTable({ models, value, onChange }: ModelSelectionTableProps) {
  // Group models by provider
  const groupedModels: Record<string, Model[]> = models.reduce((acc, model) => {
    if (!acc[model.provider]) acc[model.provider] = []
    acc[model.provider].push(model)
    return acc
  }, {} as Record<string, Model[]>)

  const providers = Object.keys(groupedModels).sort()

  // Determine if "All" is selected
  const isAllSelected = value?.type === 'all'

  // Determine if a provider has all its models selected
  const isProviderSelected = (provider: string): boolean => {
    if (isAllSelected) return true
    if (value?.type !== 'custom') return false
    return value.all_provider_models?.includes(provider) || false
  }

  // Determine if a specific model is selected
  const isModelSelected = (provider: string, modelId: string): boolean => {
    if (isAllSelected) return true
    if (isProviderSelected(provider)) return true
    if (value?.type !== 'custom') return false
    return value.individual_models?.some(([p, m]) => p === provider && m === modelId) || false
  }

  // Handle "All" checkbox toggle
  const handleAllToggle = () => {
    if (isAllSelected) {
      // Uncheck all - set to empty custom selection
      onChange({
        type: 'custom',
        all_provider_models: [],
        individual_models: [],
      })
    } else {
      // Check all
      onChange({ type: 'all' })
    }
  }

  // Handle provider checkbox toggle
  const handleProviderToggle = (provider: string) => {
    if (isAllSelected) return // Can't toggle providers when "All" is selected

    const currentProviders = value?.all_provider_models || []
    const currentModels = value?.individual_models || []

    if (isProviderSelected(provider)) {
      // Uncheck provider - remove from all_provider_models
      onChange({
        type: 'custom',
        all_provider_models: currentProviders.filter(p => p !== provider),
        individual_models: currentModels.filter(([p]) => p !== provider),
      })
    } else {
      // Check provider - add to all_provider_models and remove individual models from this provider
      onChange({
        type: 'custom',
        all_provider_models: [...currentProviders, provider],
        individual_models: currentModels.filter(([p]) => p !== provider),
      })
    }
  }

  // Handle individual model checkbox toggle
  const handleModelToggle = (provider: string, modelId: string) => {
    if (isAllSelected) return // Can't toggle models when "All" is selected
    if (isProviderSelected(provider)) return // Can't toggle individual models when provider is selected

    const currentProviders = value?.all_provider_models || []
    const currentModels = value?.individual_models || []

    if (isModelSelected(provider, modelId)) {
      // Uncheck model
      onChange({
        type: 'custom',
        all_provider_models: currentProviders,
        individual_models: currentModels.filter(([p, m]) => !(p === provider && m === modelId)),
      })
    } else {
      // Check model
      onChange({
        type: 'custom',
        all_provider_models: currentProviders,
        individual_models: [...currentModels, [provider, modelId]],
      })
    }
  }

  return (
    <div className="border border-gray-300 dark:border-gray-600 rounded-lg overflow-hidden">
      <table className="w-full">
        <thead className="bg-gray-100 dark:bg-gray-800 border-b border-gray-300 dark:border-gray-600">
          <tr>
            <th className="text-left px-4 py-3 font-semibold text-gray-900 dark:text-gray-100">Model Selection</th>
          </tr>
        </thead>
        <tbody>
          {/* All row */}
          <tr className="border-b border-gray-200 dark:border-gray-700 hover:bg-gray-50 dark:hover:bg-gray-800">
            <td className="px-4 py-2">
              <label className="flex items-center cursor-pointer">
                <input
                  type="checkbox"
                  checked={isAllSelected}
                  onChange={handleAllToggle}
                  className="w-4 h-4 text-blue-600 rounded focus:ring-blue-500 cursor-pointer"
                />
                <span className="ml-3 font-semibold text-gray-900 dark:text-gray-100">All Providers & Models</span>
              </label>
            </td>
          </tr>

          {/* Provider rows */}
          {providers.map((provider) => {
            const providerSelected = isProviderSelected(provider)
            const providerModels = groupedModels[provider]

            return (
              <>
                {/* Provider row */}
                <tr key={provider} className={`border-b border-gray-200 dark:border-gray-700 ${isAllSelected ? 'bg-gray-100 dark:bg-gray-800' : 'hover:bg-gray-50 dark:hover:bg-gray-800'}`}>
                  <td className="px-4 py-2 pl-8">
                    <label className={`flex items-center ${isAllSelected ? 'cursor-not-allowed opacity-60' : 'cursor-pointer'}`}>
                      <input
                        type="checkbox"
                        checked={providerSelected}
                        onChange={() => handleProviderToggle(provider)}
                        disabled={isAllSelected}
                        className={`w-4 h-4 text-blue-600 rounded focus:ring-blue-500 ${isAllSelected ? 'cursor-not-allowed' : 'cursor-pointer'}`}
                      />
                      <span className="ml-3 font-medium text-gray-800 dark:text-gray-200">{provider}</span>
                    </label>
                  </td>
                </tr>

                {/* Model rows */}
                {providerModels.map((model) => {
                  const modelSelected = isModelSelected(provider, model.id)
                  const disabled = isAllSelected || providerSelected

                  return (
                    <tr
                      key={`${provider}/${model.id}`}
                      className={`border-b border-gray-100 dark:border-gray-700 ${disabled ? 'bg-gray-50 dark:bg-gray-800/50' : 'hover:bg-gray-50 dark:hover:bg-gray-800'}`}
                    >
                      <td className="px-4 py-2 pl-16">
                        <label className={`flex items-center ${disabled ? 'cursor-not-allowed opacity-50' : 'cursor-pointer'}`}>
                          <input
                            type="checkbox"
                            checked={modelSelected}
                            onChange={() => handleModelToggle(provider, model.id)}
                            disabled={disabled}
                            className={`w-4 h-4 text-blue-600 rounded focus:ring-blue-500 ${disabled ? 'cursor-not-allowed' : 'cursor-pointer'}`}
                          />
                          <span className="ml-3 text-gray-700 dark:text-gray-300 text-sm">{model.id}</span>
                        </label>
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
  )
}
