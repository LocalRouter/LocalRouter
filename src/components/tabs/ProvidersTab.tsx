import { useState, useEffect } from 'react'
import { invoke } from '@tauri-apps/api/core'
import Card from '../ui/Card'
import Button from '../ui/Button'
import Badge from '../ui/Badge'
import Modal from '../ui/Modal'
import { OAuthModal } from '../OAuthModal'
import ProviderForm, { ProviderType as ProviderTypeInfo } from '../ProviderForm'
import ProviderIcon from '../ProviderIcon'

interface ProviderInstance {
  instance_name: string
  provider_type: string
  provider_name: string
  enabled: boolean
  created_at: string
}

interface ProviderHealth {
  status: string
  latency_ms?: number
}

// ProviderType and SetupParameter are now imported from ProviderForm

interface OAuthProvider {
  provider_id: string
  provider_name: string
}

const OAUTH_PROVIDER_DISPLAY: Record<string, { icon: string; description: string }> = {
  'github-copilot': {
    icon: '🐙',
    description: 'GitHub Copilot subscription access via OAuth',
  },
  'openai-codex': {
    icon: '🤖',
    description: 'OpenAI ChatGPT Plus/Pro subscription via OAuth',
  },
  'anthropic-claude': {
    icon: '🧠',
    description: 'Anthropic Claude Pro subscription via OAuth',
  },
}

const PROVIDER_DISPLAY_INFO: Record<string, { name: string; icon: string; category: string }> = {
  ollama: { name: 'Ollama', icon: '🦙', category: 'Local' },
  lmstudio: { name: 'LM Studio', icon: '💻', category: 'Local' },
  openai: { name: 'OpenAI', icon: '🤖', category: 'Cloud' },
  anthropic: { name: 'Anthropic', icon: '🧠', category: 'Cloud' },
  gemini: { name: 'Google Gemini', icon: '✨', category: 'Cloud' },
  groq: { name: 'Groq', icon: '⚡', category: 'Cloud' },
  mistral: { name: 'Mistral AI', icon: '🌬️', category: 'Cloud' },
  cohere: { name: 'Cohere', icon: '🎯', category: 'Cloud' },
  togetherai: { name: 'Together AI', icon: '🤝', category: 'Cloud' },
  perplexity: { name: 'Perplexity', icon: '🔍', category: 'Cloud' },
  deepinfra: { name: 'DeepInfra', icon: '🏗️', category: 'Cloud' },
  cerebras: { name: 'Cerebras', icon: '🧮', category: 'Cloud' },
  xai: { name: 'xAI (Grok)', icon: '🚀', category: 'Cloud' },
  openrouter: { name: 'OpenRouter', icon: '🌐', category: 'Gateway' },
  openai_compatible: { name: 'OpenAI Compatible', icon: '🔌', category: 'Custom' },
}

export default function ProvidersTab() {
  const [providerInstances, setProviderInstances] = useState<ProviderInstance[]>([])
  const [providerTypes, setProviderTypes] = useState<ProviderTypeInfo[]>([])
  const [providersHealth, setProvidersHealth] = useState<Record<string, ProviderHealth>>({})
  const [loading, setLoading] = useState(true)

  const [showProviderModal, setShowProviderModal] = useState(false)
  const [modalMode, setModalMode] = useState<'create' | 'edit'>('create')
  const [selectedProviderType, setSelectedProviderType] = useState<string | null>(null)
  const [selectedInstanceName, setSelectedInstanceName] = useState<string | null>(null)
  const [providerConfig, setProviderConfig] = useState<Record<string, string>>({})
  const [isSubmitting, setIsSubmitting] = useState(false)

  // OAuth state
  const [oauthProviders, setOAuthProviders] = useState<OAuthProvider[]>([])
  const [authenticatedOAuthProviders, setAuthenticatedOAuthProviders] = useState<string[]>([])
  const [showOAuthModal, setShowOAuthModal] = useState(false)
  const [selectedOAuthProvider, setSelectedOAuthProvider] = useState<OAuthProvider | null>(null)

  useEffect(() => {
    loadProviders()
  }, [])

  const loadProviders = async () => {
    setLoading(true)
    try {
      const [instances, types, health, oauthList, oauthAuth] = await Promise.all([
        invoke<ProviderInstance[]>('list_provider_instances'),
        invoke<ProviderTypeInfo[]>('list_provider_types'),
        invoke<Record<string, ProviderHealth>>('get_providers_health'),
        invoke<OAuthProvider[]>('list_oauth_providers'),
        invoke<string[]>('list_oauth_credentials'),
      ])

      setProviderInstances(instances)
      setProviderTypes(types)
      setProvidersHealth(health)
      setOAuthProviders(oauthList)
      setAuthenticatedOAuthProviders(oauthAuth)
    } catch (error) {
      console.error('Failed to load providers:', error)
      alert(`Error loading providers: ${error}`)
    } finally {
      setLoading(false)
    }
  }

  const handleOpenCreateModal = (providerType: string) => {
    setModalMode('create')
    setSelectedProviderType(providerType)
    setSelectedInstanceName(null)
    setProviderConfig({})
    setShowProviderModal(true)
  }

  const handleOpenEditModal = async (instanceName: string, providerType: string) => {
    try {
      // Fetch the current config
      const config = await invoke<Record<string, string>>('get_provider_config', { instanceName })

      setModalMode('edit')
      setSelectedProviderType(providerType)
      setSelectedInstanceName(instanceName)
      setProviderConfig(config)
      setShowProviderModal(true)
    } catch (error) {
      console.error('Failed to load provider config:', error)
      alert(`Error loading provider config: ${error}`)
    }
  }

  const handleProviderSubmit = async (instanceName: string, config: Record<string, string>) => {
    if (!selectedProviderType) {
      alert('Provider type is required')
      return
    }

    setIsSubmitting(true)
    try {
      if (modalMode === 'create') {
        await invoke('create_provider_instance', {
          instanceName,
          providerType: selectedProviderType,
          config,
        })
        alert(`Provider "${instanceName}" added successfully!`)
      } else {
        // Check if name has changed - if so, we need to delete old and create new
        const nameChanged = selectedInstanceName && instanceName !== selectedInstanceName

        if (nameChanged) {
          // Delete the old instance
          await invoke('remove_provider_instance', { instanceName: selectedInstanceName })
          // Create new instance with new name
          await invoke('create_provider_instance', {
            instanceName,
            providerType: selectedProviderType,
            config,
          })
          alert(`Provider renamed from "${selectedInstanceName}" to "${instanceName}" successfully!`)
        } else {
          // Just update the existing instance
          await invoke('update_provider_instance', {
            instanceName,
            providerType: selectedProviderType,
            config,
          })
          alert(`Provider "${instanceName}" updated successfully!`)
        }
      }

      setShowProviderModal(false)
      setSelectedProviderType(null)
      setSelectedInstanceName(null)
      setProviderConfig({})
      await loadProviders()
    } catch (error) {
      console.error(`Failed to ${modalMode} provider:`, error)
      alert(`Error ${modalMode === 'create' ? 'adding' : 'updating'} provider: ${error}`)
    } finally {
      setIsSubmitting(false)
    }
  }

  const handleModalCancel = () => {
    setShowProviderModal(false)
    setSelectedProviderType(null)
    setSelectedInstanceName(null)
    setProviderConfig({})
  }

  const handleToggleProviderEnabled = async (instanceName: string, enabled: boolean) => {
    try {
      await invoke('set_provider_enabled', { instanceName, enabled })
      await loadProviders()
    } catch (error) {
      console.error('Failed to toggle provider:', error)
      alert(`Error toggling provider: ${error}`)
    }
  }

  const handleRemoveProvider = async (instanceName: string) => {
    if (!confirm(`Remove provider "${instanceName}"? This action cannot be undone.`)) {
      return
    }

    try {
      await invoke('remove_provider_instance', { instanceName })
      await loadProviders()
      alert(`Provider "${instanceName}" removed successfully!`)
    } catch (error) {
      console.error('Failed to remove provider:', error)
      alert(`Error removing provider: ${error}`)
    }
  }

  const handleConnectOAuth = (provider: OAuthProvider) => {
    setSelectedOAuthProvider(provider)
    setShowOAuthModal(true)
  }

  const handleDisconnectOAuth = async (providerId: string) => {
    if (!confirm('Disconnect this OAuth provider? You will need to re-authenticate to use it again.')) {
      return
    }

    try {
      await invoke('delete_oauth_credentials', { providerId })
      await loadProviders()
      alert('OAuth provider disconnected successfully!')
    } catch (error) {
      console.error('Failed to disconnect OAuth provider:', error)
      alert(`Error disconnecting OAuth provider: ${error}`)
    }
  }

  const handleOAuthSuccess = async () => {
    await loadProviders()
  }

  const getProviderTypeInfo = (typeId: string) => {
    return PROVIDER_DISPLAY_INFO[typeId] || { name: typeId, icon: '📦', category: 'Other' }
  }

  const getProviderTypeObject = (typeId: string) => {
    return providerTypes.find((t) => t.provider_type === typeId)
  }

  const groupedProviders = providerTypes.reduce((acc, type) => {
    const info = getProviderTypeInfo(type.provider_type)
    if (!acc[info.category]) {
      acc[info.category] = []
    }
    acc[info.category].push(type)
    return acc
  }, {} as Record<string, ProviderTypeInfo[]>)

  const selectedProviderTypeObject = selectedProviderType
    ? getProviderTypeObject(selectedProviderType)
    : null

  return (
    <div className="space-y-6">
      <Card>
        <div className="mb-4">
          <h2 className="text-2xl font-bold text-gray-900">LLM Providers</h2>
          <p className="text-sm text-gray-500 mt-1">
            Configure providers to access various LLM services. Each provider can have multiple instances.
          </p>
        </div>

        {loading ? (
          <div className="text-center py-8 text-gray-500">Loading providers...</div>
        ) : (
          <div className="space-y-6">
            {/* Active Provider Instances */}
            {providerInstances.length > 0 && (
              <div>
                <h3 className="text-lg font-semibold text-gray-700 mb-3 border-b pb-2">
                  Active Provider Instances
                </h3>
                <div className="overflow-x-auto">
                  <table className="min-w-full divide-y divide-gray-200">
                    <thead className="bg-gray-50">
                      <tr>
                        <th className="px-4 py-3 text-left text-xs font-medium text-gray-500 uppercase tracking-wider">
                          Provider
                        </th>
                        <th className="px-4 py-3 text-left text-xs font-medium text-gray-500 uppercase tracking-wider">
                          Instance Name
                        </th>
                        <th className="px-4 py-3 text-left text-xs font-medium text-gray-500 uppercase tracking-wider">
                          Health
                        </th>
                        <th className="px-4 py-3 text-left text-xs font-medium text-gray-500 uppercase tracking-wider">
                          Status
                        </th>
                        <th className="px-4 py-3 text-left text-xs font-medium text-gray-500 uppercase tracking-wider">
                          Created
                        </th>
                        <th className="px-4 py-3 text-right text-xs font-medium text-gray-500 uppercase tracking-wider">
                          Actions
                        </th>
                      </tr>
                    </thead>
                    <tbody className="bg-white divide-y divide-gray-200">
                      {providerInstances.map((instance) => {
                        const info = getProviderTypeInfo(instance.provider_type)
                        const health = providersHealth[instance.instance_name]
                        const healthStatus = health?.status || 'Unknown'
                        const healthVariant =
                          healthStatus === 'Healthy'
                            ? 'success'
                            : healthStatus === 'Degraded'
                            ? 'warning'
                            : 'error'

                        return (
                          <tr key={instance.instance_name} className="hover:bg-gray-50">
                            <td className="px-4 py-3 whitespace-nowrap">
                              <div className="flex items-center gap-2">
                                <ProviderIcon providerId={instance.provider_type} size={24} />
                                <span className="text-sm font-medium text-gray-900">{info.name}</span>
                              </div>
                            </td>
                            <td className="px-4 py-3 whitespace-nowrap">
                              <span className="text-sm text-gray-900">{instance.instance_name}</span>
                            </td>
                            <td className="px-4 py-3 whitespace-nowrap">
                              <Badge variant={healthVariant}>{healthStatus}</Badge>
                              {health?.latency_ms && (
                                <div className="text-xs text-gray-500 mt-1">{health.latency_ms}ms</div>
                              )}
                            </td>
                            <td className="px-4 py-3 whitespace-nowrap">
                              <Badge variant={instance.enabled ? 'success' : 'warning'}>
                                {instance.enabled ? 'Enabled' : 'Disabled'}
                              </Badge>
                            </td>
                            <td className="px-4 py-3 whitespace-nowrap text-sm text-gray-500">
                              {new Date(instance.created_at).toLocaleDateString()}
                            </td>
                            <td className="px-4 py-3 whitespace-nowrap text-right text-sm">
                              <div className="flex gap-1 justify-end">
                                <Button
                                  variant="secondary"
                                  onClick={() => handleOpenEditModal(instance.instance_name, instance.provider_type)}
                                  className="px-2 py-1 text-xs"
                                >
                                  Edit
                                </Button>
                                <Button
                                  variant="secondary"
                                  onClick={() =>
                                    handleToggleProviderEnabled(instance.instance_name, !instance.enabled)
                                  }
                                  className="px-2 py-1 text-xs"
                                >
                                  {instance.enabled ? 'Disable' : 'Enable'}
                                </Button>
                                <Button
                                  variant="danger"
                                  onClick={() => handleRemoveProvider(instance.instance_name)}
                                  className="px-2 py-1 text-xs"
                                >
                                  Remove
                                </Button>
                              </div>
                            </td>
                          </tr>
                        )
                      })}
                    </tbody>
                  </table>
                </div>
              </div>
            )}

            {/* Available Provider Types */}
            <div>
              <h3 className="text-lg font-semibold text-gray-700 mb-3 border-b pb-2">
                Add New Provider
              </h3>
              {Object.entries(groupedProviders).map(([category, types]) => (
                <div key={category} className="mb-4">
                  <h4 className="text-sm font-semibold text-gray-600 mb-2">{category} Providers</h4>
                  <div className="grid grid-cols-1 md:grid-cols-2 lg:grid-cols-3 gap-3">
                    {types.map((type) => {
                      const info = getProviderTypeInfo(type.provider_type)

                      return (
                        <button
                          key={type.provider_type}
                          onClick={() => handleOpenCreateModal(type.provider_type)}
                          className="flex items-start gap-3 p-4 bg-gray-50 border border-gray-200 rounded-lg hover:bg-gray-100 hover:border-gray-300 transition-colors text-left"
                        >
                          <ProviderIcon providerId={type.provider_type} size={32} />
                          <div className="flex-1 min-w-0">
                            <h5 className="text-sm font-semibold text-gray-900">{info.name}</h5>
                            <p className="text-xs text-gray-600 mt-0.5 line-clamp-2">{type.description}</p>
                          </div>
                        </button>
                      )
                    })}
                  </div>
                </div>
              ))}
            </div>

            {/* OAuth Providers Section */}
            {oauthProviders.length > 0 && (
              <div>
                <h3 className="text-lg font-semibold text-gray-700 mb-3 border-b pb-2">
                  Subscription Providers (OAuth)
                </h3>
                <div className="grid grid-cols-1 md:grid-cols-2 lg:grid-cols-3 gap-3">
                  {oauthProviders.map((provider) => {
                    const isAuthenticated = authenticatedOAuthProviders.includes(provider.provider_id)
                    const displayInfo = OAUTH_PROVIDER_DISPLAY[provider.provider_id] || {
                      icon: '🔐',
                      description: provider.provider_name,
                    }

                    return (
                      <div
                        key={provider.provider_id}
                        className="bg-gray-50 border border-gray-200 rounded-lg p-4"
                      >
                        <div className="flex items-start gap-3 mb-3">
                          <ProviderIcon providerId={provider.provider_id} size={32} />
                          <div className="flex-1 min-w-0">
                            <div className="flex items-center gap-2 mb-1">
                              <h5 className="text-sm font-semibold text-gray-900">
                                {provider.provider_name}
                              </h5>
                              {isAuthenticated && <Badge variant="success">Connected</Badge>}
                            </div>
                            <p className="text-xs text-gray-600">{displayInfo.description}</p>
                          </div>
                        </div>
                        <div className="flex justify-end">
                          {isAuthenticated ? (
                            <Button
                              variant="danger"
                              onClick={() => handleDisconnectOAuth(provider.provider_id)}
                              className="px-3 py-1.5 text-xs"
                            >
                              Disconnect
                            </Button>
                          ) : (
                            <Button
                              onClick={() => handleConnectOAuth(provider)}
                              className="px-3 py-1.5 text-xs"
                            >
                              Connect
                            </Button>
                          )}
                        </div>
                      </div>
                    )
                  })}
                </div>
              </div>
            )}
          </div>
        )}
      </Card>

      {/* Provider Modal (Create/Edit) */}
      <Modal
        isOpen={showProviderModal}
        onClose={handleModalCancel}
        title={
          modalMode === 'create'
            ? `Add ${selectedProviderTypeObject ? getProviderTypeInfo(selectedProviderTypeObject.provider_type).name : 'Provider'}`
            : `Edit ${selectedInstanceName}`
        }
      >
        {selectedProviderTypeObject && (
          <ProviderForm
            mode={modalMode}
            providerType={selectedProviderTypeObject}
            initialInstanceName={selectedInstanceName || undefined}
            initialConfig={providerConfig}
            onSubmit={handleProviderSubmit}
            onCancel={handleModalCancel}
            isSubmitting={isSubmitting}
          />
        )}
      </Modal>

      {/* OAuth Modal */}
      {selectedOAuthProvider && (
        <OAuthModal
          isOpen={showOAuthModal}
          onClose={() => {
            setShowOAuthModal(false)
            setSelectedOAuthProvider(null)
          }}
          providerId={selectedOAuthProvider.provider_id}
          providerName={selectedOAuthProvider.provider_name}
          onSuccess={handleOAuthSuccess}
        />
      )}
    </div>
  )
}
