export interface Model {
  id: string
  provider: string
}

interface ForcedModelSelectorProps {
  models: Model[]
  selectedModel: [string, string] | null // [provider, model_id]
  onChange: (model: [string, string] | null) => void
}

export default function ForcedModelSelector({ models, selectedModel, onChange }: ForcedModelSelectorProps) {
  // Group models by provider
  const groupedModels: Record<string, Model[]> = models.reduce((acc, model) => {
    if (!acc[model.provider]) acc[model.provider] = []
    acc[model.provider].push(model)
    return acc
  }, {} as Record<string, Model[]>)

  const providers = Object.keys(groupedModels).sort()

  const isModelSelected = (provider: string, modelId: string): boolean => {
    if (!selectedModel) return false
    return selectedModel[0] === provider && selectedModel[1] === modelId
  }

  const handleModelSelect = (provider: string, modelId: string) => {
    if (isModelSelected(provider, modelId)) {
      onChange(null) // Deselect if clicking the same model
    } else {
      onChange([provider, modelId])
    }
  }

  return (
    <div className="border border-gray-300 rounded-lg overflow-hidden">
      <table className="w-full">
        <thead className="bg-gray-100 border-b border-gray-300">
          <tr>
            <th className="text-left px-4 py-3 font-semibold text-gray-900">Select One Model</th>
          </tr>
        </thead>
        <tbody>
          {providers.map((provider) => {
            const providerModels = groupedModels[provider]

            return (
              <>
                {/* Provider header row */}
                <tr key={provider} className="border-b border-gray-200 bg-gray-50">
                  <td className="px-4 py-2 pl-8">
                    <span className="font-medium text-gray-800">{provider}</span>
                  </td>
                </tr>

                {/* Model rows */}
                {providerModels.map((model) => {
                  const modelSelected = isModelSelected(provider, model.id)

                  return (
                    <tr
                      key={`${provider}/${model.id}`}
                      className="border-b border-gray-100 hover:bg-gray-50"
                    >
                      <td className="px-4 py-2 pl-16">
                        <label className="flex items-center cursor-pointer">
                          <input
                            type="radio"
                            name="forced-model"
                            checked={modelSelected}
                            onChange={() => handleModelSelect(provider, model.id)}
                            className="w-4 h-4 text-blue-600 cursor-pointer"
                          />
                          <span className="ml-3 text-gray-700 text-sm">{model.id}</span>
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
