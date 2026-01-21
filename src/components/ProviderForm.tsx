import { useState, useEffect } from 'react'
import Button from './ui/Button'
import Input from './ui/Input'
import { EyeIcon, EyeSlashIcon } from '@heroicons/react/24/outline'

export interface SetupParameter {
  key: string
  param_type: string
  required: boolean
  description: string
  default_value?: string
  sensitive: boolean
}

export interface ProviderType {
  provider_type: string
  description: string
  setup_parameters: SetupParameter[]
}

interface ProviderFormProps {
  mode: 'create' | 'edit'
  providerType: ProviderType
  initialInstanceName?: string
  initialConfig?: Record<string, string>
  onSubmit: (instanceName: string, config: Record<string, string>) => Promise<void>
  onCancel: () => void
  isSubmitting?: boolean
}

/**
 * Unified provider form component for creating and editing provider instances.
 * Dynamically renders form fields based on the provider type's setup parameters.
 */
export default function ProviderForm({
  mode,
  providerType,
  initialInstanceName = '',
  initialConfig = {},
  onSubmit,
  onCancel,
  isSubmitting = false,
}: ProviderFormProps) {
  const [instanceName, setInstanceName] = useState(initialInstanceName)
  const [config, setConfig] = useState<Record<string, string>>(initialConfig)
  const [visibleFields, setVisibleFields] = useState<Set<string>>(new Set())

  // Generate default instance name for create mode
  useEffect(() => {
    if (mode === 'create' && !initialInstanceName) {
      setInstanceName(`${providerType.provider_type}-${Date.now()}`)
    }
  }, [mode, providerType.provider_type, initialInstanceName])

  // Update config when initialConfig changes (edit mode)
  useEffect(() => {
    if (mode === 'edit' && initialConfig) {
      setConfig(initialConfig)
    }
  }, [mode, initialConfig])

  const handleSubmit = async (e: React.FormEvent) => {
    e.preventDefault()

    if (!instanceName.trim()) {
      alert('Instance name is required')
      return
    }

    // Validate required parameters
    for (const param of providerType.setup_parameters) {
      if (param.required && !config[param.key]?.trim()) {
        alert(`${param.description} is required`)
        return
      }
    }

    await onSubmit(instanceName, config)
  }

  const handleConfigChange = (key: string, value: string) => {
    setConfig((prev) => ({ ...prev, [key]: value }))
  }

  const toggleFieldVisibility = (key: string) => {
    setVisibleFields((prev) => {
      const newSet = new Set(prev)
      if (newSet.has(key)) {
        newSet.delete(key)
      } else {
        newSet.add(key)
      }
      return newSet
    })
  }

  return (
    <form onSubmit={handleSubmit} className="space-y-4">
      {/* Instance Name Field */}
      <Input
        label="Instance Name *"
        placeholder="e.g., my-openai, groq-prod"
        value={instanceName}
        onChange={(e) => setInstanceName(e.target.value)}
        required
        helperText="Unique name to identify this provider instance"
      />

      {/* Dynamic Parameter Fields */}
      {providerType.setup_parameters.map((param) => {
        const isFieldVisible = visibleFields.has(param.key)
        const isSensitive = param.sensitive
        const fieldType = isSensitive && !isFieldVisible
          ? 'password'
          : param.param_type === 'number'
          ? 'number'
          : param.param_type === 'boolean'
          ? 'checkbox'
          : 'text'

        const label = `${param.description}${param.required ? ' *' : ' (Optional)'}`

        // Handle boolean/checkbox parameters differently
        if (param.param_type === 'boolean') {
          return (
            <div key={param.key} className="flex items-center gap-2">
              <input
                type="checkbox"
                id={param.key}
                checked={config[param.key] === 'true'}
                onChange={(e) => handleConfigChange(param.key, e.target.checked.toString())}
                className="w-4 h-4 text-blue-600 rounded focus:ring-blue-500"
              />
              <label htmlFor={param.key} className="text-sm font-medium text-gray-700 dark:text-gray-300">
                {param.description}
              </label>
            </div>
          )
        }

        // Render text/password/number fields with optional visibility toggle
        return (
          <div key={param.key} className="relative">
            <Input
              label={label}
              type={fieldType}
              placeholder={param.default_value || ''}
              value={config[param.key] || ''}
              onChange={(e) => handleConfigChange(param.key, e.target.value)}
              required={param.required}
            />
            {isSensitive && (
              <button
                type="button"
                onClick={() => toggleFieldVisibility(param.key)}
                className="absolute right-3 top-9 text-gray-500 dark:text-gray-400 hover:text-gray-700 dark:hover:text-gray-300 focus:outline-none"
                title={isFieldVisible ? 'Hide' : 'Show'}
              >
                {isFieldVisible ? (
                  <EyeSlashIcon className="h-5 w-5" />
                ) : (
                  <EyeIcon className="h-5 w-5" />
                )}
              </button>
            )}
          </div>
        )
      })}

      {/* Form Actions */}
      <div className="flex gap-2 mt-6">
        <Button type="submit" disabled={isSubmitting}>
          {isSubmitting ? 'Saving...' : mode === 'create' ? 'Add Provider' : 'Save Changes'}
        </Button>
        <Button type="button" variant="secondary" onClick={onCancel} disabled={isSubmitting}>
          Cancel
        </Button>
      </div>
    </form>
  )
}
