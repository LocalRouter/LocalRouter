export interface Model {
  id: string
  provider: string
}

export interface ModelSelectionValue {
  selected_all: boolean
  selected_providers: string[]
  selected_models: [string, string][]
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
  const isAllSelected = value?.selected_all ?? true

  // Determine if a provider has all its models selected
  const isProviderSelected = (provider: string): boolean => {
    if (isAllSelected) return true
    return value?.selected_providers?.some(p => p.toLowerCase() === provider.toLowerCase()) || false
  }

  // Determine if a specific model is selected
  const isModelSelected = (provider: string, modelId: string): boolean => {
    if (isAllSelected) return true
    if (isProviderSelected(provider)) return true
    return value?.selected_models?.some(([p, m]) =>
      p.toLowerCase() === provider.toLowerCase() && m.toLowerCase() === modelId.toLowerCase()
    ) || false
  }

  // Handle "All" checkbox toggle
  const handleAllToggle = () => {
    if (isAllSelected) {
      // Uncheck all - keep existing selections but disable selected_all
      onChange({
        selected_all: false,
        selected_providers: value?.selected_providers || [],
        selected_models: value?.selected_models || [],
      })
    } else {
      // Check all
      onChange({
        selected_all: true,
        selected_providers: value?.selected_providers || [],
        selected_models: value?.selected_models || [],
      })
    }
  }

  // Handle provider checkbox toggle
  const handleProviderToggle = (provider: string) => {
    if (isAllSelected) {
      // Turn off selected_all and select all providers except this one
      onChange({
        selected_all: false,
        selected_providers: providers.filter(p => p.toLowerCase() !== provider.toLowerCase()),
        selected_models: [],
      })
      return
    }

    const currentProviders = value?.selected_providers || []
    const currentModels = value?.selected_models || []
    const providerLower = provider.toLowerCase()

    if (isProviderSelected(provider)) {
      // Uncheck provider - remove from selected_providers
      onChange({
        selected_all: false,
        selected_providers: currentProviders.filter(p => p.toLowerCase() !== providerLower),
        selected_models: currentModels,
      })
    } else {
      // Check provider - add to selected_providers and remove individual models from this provider
      onChange({
        selected_all: false,
        selected_providers: [...currentProviders, provider],
        selected_models: currentModels.filter(([p]) => p.toLowerCase() !== providerLower),
      })
    }
  }

  // Handle individual model checkbox toggle
  const handleModelToggle = (provider: string, modelId: string) => {
    const providerLower = provider.toLowerCase()
    const modelLower = modelId.toLowerCase()

    if (isAllSelected) {
      // Turn off selected_all and select everything except this model
      const otherProviders = providers.filter(p => p.toLowerCase() !== providerLower)
      const providerModels = groupedModels[provider] || []
      const otherModels = providerModels
        .filter(m => m.id.toLowerCase() !== modelLower)
        .map(m => [provider, m.id] as [string, string])

      onChange({
        selected_all: false,
        selected_providers: otherProviders,
        selected_models: otherModels,
      })
      return
    }

    if (isProviderSelected(provider)) {
      // Demote provider to individual models minus this one
      const providerModels = groupedModels[provider] || []
      const otherModels = providerModels
        .filter(m => m.id.toLowerCase() !== modelLower)
        .map(m => [provider, m.id] as [string, string])

      onChange({
        selected_all: false,
        selected_providers: (value?.selected_providers || []).filter(p => p.toLowerCase() !== providerLower),
        selected_models: [...(value?.selected_models || []), ...otherModels],
      })
      return
    }

    const currentProviders = value?.selected_providers || []
    const currentModels = value?.selected_models || []

    if (isModelSelected(provider, modelId)) {
      // Uncheck model
      onChange({
        selected_all: false,
        selected_providers: currentProviders,
        selected_models: currentModels.filter(([p, m]) =>
          !(p.toLowerCase() === providerLower && m.toLowerCase() === modelLower)
        ),
      })
    } else {
      // Check model
      const newSelectedModels = [...currentModels, [provider, modelId] as [string, string]]

      // Check if all models from this provider are now selected - promote to provider level
      const providerModels = groupedModels[provider] || []
      const selectedFromProvider = newSelectedModels.filter(
        ([p]) => p.toLowerCase() === providerLower
      ).length

      if (selectedFromProvider === providerModels.length) {
        // Promote to provider-level selection
        onChange({
          selected_all: false,
          selected_providers: [...currentProviders, provider],
          selected_models: newSelectedModels.filter(([p]) => p.toLowerCase() !== providerLower),
        })
      } else {
        onChange({
          selected_all: false,
          selected_providers: currentProviders,
          selected_models: newSelectedModels,
        })
      }
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
                <span className="ml-3 font-semibold text-gray-900 dark:text-gray-100">
                  All Providers & Models
                  {isAllSelected && (
                    <span className="ml-2 text-xs text-blue-600 dark:text-blue-400 font-normal">
                      (including future models)
                    </span>
                  )}
                </span>
              </label>
            </td>
          </tr>

          {/* Provider rows */}
          {providers.map((provider) => {
            const providerSelected = isProviderSelected(provider)
            const providerModels = groupedModels[provider]

            return (
              <div key={provider}>
                {/* Provider row */}
                <tr className={`border-b border-gray-200 dark:border-gray-700 ${isAllSelected ? 'bg-gray-100 dark:bg-gray-800' : 'hover:bg-gray-50 dark:hover:bg-gray-800'}`}>
                  <td className="px-4 py-2 pl-8">
                    <label className={`flex items-center ${isAllSelected ? 'opacity-60' : ''} cursor-pointer`}>
                      <input
                        type="checkbox"
                        checked={providerSelected}
                        onChange={() => handleProviderToggle(provider)}
                        className="w-4 h-4 text-blue-600 rounded focus:ring-blue-500 cursor-pointer"
                      />
                      <span className="ml-3 font-medium text-gray-800 dark:text-gray-200">{provider}</span>
                    </label>
                  </td>
                </tr>

                {/* Model rows */}
                {providerModels.map((model) => {
                  const modelSelected = isModelSelected(provider, model.id)
                  const canToggle = !isAllSelected && !providerSelected

                  return (
                    <tr
                      key={`${provider}/${model.id}`}
                      className={`border-b border-gray-100 dark:border-gray-700 ${!canToggle ? 'bg-gray-50 dark:bg-gray-800/50' : 'hover:bg-gray-50 dark:hover:bg-gray-800'}`}
                    >
                      <td className="px-4 py-2 pl-16">
                        <label className={`flex items-center ${!canToggle ? 'opacity-50' : ''} cursor-pointer`}>
                          <input
                            type="checkbox"
                            checked={modelSelected}
                            onChange={() => handleModelToggle(provider, model.id)}
                            disabled={!canToggle}
                            className={`w-4 h-4 text-blue-600 rounded focus:ring-blue-500 ${!canToggle ? 'cursor-not-allowed' : 'cursor-pointer'}`}
                          />
                          <span className="ml-3 text-gray-700 dark:text-gray-300 text-sm">{model.id}</span>
                        </label>
                      </td>
                    </tr>
                  )
                })}
              </div>
            )
          })}
        </tbody>
      </table>
    </div>
  )
}
