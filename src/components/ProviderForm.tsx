import { useState, useEffect, useCallback } from 'react'
import { invoke } from '@tauri-apps/api/core'
import { open } from '@tauri-apps/plugin-shell'
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

export type ProviderCategory = 'generic' | 'local' | 'subscription' | 'first_party' | 'third_party'

export interface ProviderType {
  provider_type: string
  display_name: string
  category: ProviderCategory
  description: string
  setup_parameters: SetupParameter[]
}

interface OAuthFlowResult {
  type: 'pending' | 'success' | 'error'
  user_code?: string
  verification_url?: string
  instructions?: string
  message?: string
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
// Map provider types to OAuth provider IDs
const OAUTH_PROVIDER_MAP: Record<string, string> = {
  'github-copilot': 'github-copilot',
  'openai-chatgpt-plus': 'openai-codex',
}

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

  // OAuth state
  const [oauthStatus, setOauthStatus] = useState<'idle' | 'pending' | 'success' | 'error'>('idle')
  const [oauthResult, setOauthResult] = useState<OAuthFlowResult | null>(null)
  const [oauthError, setOauthError] = useState<string | null>(null)

  // Check if this provider requires OAuth
  const hasOAuthParam = providerType.setup_parameters.some(p => p.param_type === 'oauth')
  const oauthProviderId = OAUTH_PROVIDER_MAP[providerType.provider_type]

  // Generate default instance name for create mode (fallback if none provided)
  useEffect(() => {
    if (mode === 'create' && !initialInstanceName) {
      setInstanceName(providerType.display_name)
    }
  }, [mode, providerType.display_name, initialInstanceName])

  // Check if OAuth is already authenticated
  useEffect(() => {
    if (hasOAuthParam && oauthProviderId) {
      invoke<string[]>('list_oauth_credentials')
        .then((providers) => {
          if (providers.includes(oauthProviderId)) {
            setOauthStatus('success')
            setConfig(prev => ({ ...prev, oauth: 'authenticated' }))
          }
        })
        .catch(console.error)
    }
  }, [hasOAuthParam, oauthProviderId])

  // Poll OAuth status when pending
  useEffect(() => {
    if (oauthStatus !== 'pending' || !oauthProviderId) return

    const pollInterval = setInterval(async () => {
      try {
        const result = await invoke<OAuthFlowResult>('poll_oauth_status', { providerId: oauthProviderId })
        if (result.type === 'success') {
          setOauthStatus('success')
          setOauthResult(null)
          setConfig(prev => ({ ...prev, oauth: 'authenticated' }))
          clearInterval(pollInterval)
        } else if (result.type === 'error') {
          setOauthStatus('error')
          setOauthError(result.message || 'OAuth failed')
          clearInterval(pollInterval)
        }
      } catch (err) {
        console.error('OAuth poll error:', err)
      }
    }, 2000)

    return () => clearInterval(pollInterval)
  }, [oauthStatus, oauthProviderId])

  const startOAuthFlow = useCallback(async () => {
    if (!oauthProviderId) return

    setOauthStatus('pending')
    setOauthError(null)

    try {
      const result = await invoke<OAuthFlowResult>('start_oauth_flow', { providerId: oauthProviderId })
      setOauthResult(result)

      if (result.type === 'success') {
        setOauthStatus('success')
        setConfig(prev => ({ ...prev, oauth: 'authenticated' }))
      } else if (result.type === 'error') {
        setOauthStatus('error')
        setOauthError(result.message || 'OAuth failed')
      } else if (result.type === 'pending' && result.verification_url) {
        // Automatically open the browser for OAuth (for PKCE flows)
        // Device code flows (like GitHub Copilot) show a code instead
        if (!result.user_code) {
          try {
            await open(result.verification_url)
          } catch (e) {
            console.error('Failed to open browser:', e)
          }
        }
      }
    } catch (err) {
      setOauthStatus('error')
      setOauthError(err instanceof Error ? err.message : 'Failed to start OAuth flow')
    }
  }, [oauthProviderId])

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

    // Validate required parameters (skip OAuth params - they're handled separately)
    for (const param of providerType.setup_parameters) {
      if (param.param_type === 'oauth') {
        // OAuth validation: must be authenticated
        if (oauthStatus !== 'success') {
          alert('Please complete authentication first')
          return
        }
        continue
      }
      if (param.required && !config[param.key]?.trim()) {
        alert(`${param.description} is required`)
        return
      }
    }

    await onSubmit(instanceName, config)
  }

  // Check if form can be submitted
  const canSubmit = !isSubmitting && (
    !hasOAuthParam || oauthStatus === 'success'
  )

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
        placeholder="e.g., OpenAI, Groq"
        value={instanceName}
        onChange={(e) => setInstanceName(e.target.value)}
        required
        helperText="Unique name to identify this provider instance"
      />

      {/* Dynamic Parameter Fields */}
      {providerType.setup_parameters.map((param) => {
        // Handle OAuth parameters
        if (param.param_type === 'oauth') {
          return (
            <div key={param.key} className="space-y-3 p-4 border rounded-lg bg-muted/30">
              <div className="flex items-center justify-between">
                <div>
                  <p className="font-medium text-sm">Authentication</p>
                  <p className="text-xs text-muted-foreground">{param.description}</p>
                </div>
                {oauthStatus === 'success' && (
                  <span className="text-xs text-green-600 dark:text-green-400 font-medium">
                    ✓ Connected
                  </span>
                )}
              </div>

              {oauthStatus === 'idle' && (
                <Button type="button" onClick={startOAuthFlow} variant="secondary">
                  Connect with {providerType.display_name}
                </Button>
              )}

              {oauthStatus === 'pending' && (
                <div className="space-y-2 text-sm">
                  {/* Device code flow (GitHub Copilot) - shows code to enter */}
                  {oauthResult?.user_code && (
                    <>
                      <div className="p-3 bg-background rounded border">
                        <p className="text-muted-foreground mb-1">Enter this code:</p>
                        <p className="font-mono text-lg font-bold">{oauthResult.user_code}</p>
                      </div>
                      {oauthResult.verification_url && (
                        <p className="text-muted-foreground">
                          Visit:{' '}
                          <a
                            href={oauthResult.verification_url}
                            target="_blank"
                            rel="noopener noreferrer"
                            className="text-blue-500 hover:underline"
                          >
                            {oauthResult.verification_url}
                          </a>
                        </p>
                      )}
                    </>
                  )}
                  {/* Browser PKCE flow - browser opened automatically */}
                  {!oauthResult?.user_code && (
                    <div className="p-3 bg-background rounded border text-center">
                      <p className="text-muted-foreground">A browser window has been opened.</p>
                      <p className="text-muted-foreground">Complete the authorization there.</p>
                    </div>
                  )}
                  <p className="text-muted-foreground animate-pulse text-center">
                    Waiting for authentication...
                  </p>
                </div>
              )}

              {oauthStatus === 'error' && (
                <div className="space-y-2">
                  <p className="text-sm text-red-500">{oauthError}</p>
                  <Button type="button" onClick={startOAuthFlow} variant="secondary" size="sm">
                    Try Again
                  </Button>
                </div>
              )}

              {oauthStatus === 'success' && (
                <p className="text-sm text-green-600 dark:text-green-400">
                  Successfully authenticated! You can now create the provider.
                </p>
              )}
            </div>
          )
        }

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
        <Button type="submit" disabled={!canSubmit}>
          {isSubmitting ? 'Saving...' : mode === 'create' ? 'Add Provider' : 'Save Changes'}
        </Button>
        <Button type="button" variant="secondary" onClick={onCancel} disabled={isSubmitting}>
          Cancel
        </Button>
      </div>
    </form>
  )
}
