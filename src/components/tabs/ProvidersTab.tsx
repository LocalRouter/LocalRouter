import { useState, useEffect } from 'react'
import { invoke } from '@tauri-apps/api/core'
import Card from '../ui/Card'
import Button from '../ui/Button'
import Badge from '../ui/Badge'
import Modal from '../ui/Modal'
import { OAuthModal } from '../OAuthModal'
import ProviderForm, { ProviderType as ProviderTypeInfo } from '../ProviderForm'
import ProviderIcon from '../ProviderIcon'
import ProviderDetailPage from '../providers/ProviderDetailPage'
import { StackedAreaChart } from '../charts/StackedAreaChart'
import { useMetricsSubscription } from '../../hooks/useMetricsSubscription'

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

interface ProvidersTabProps {
  activeSubTab: string | null
  onTabChange?: (tab: 'providers', subTab: string) => void
}

export default function ProvidersTab({ activeSubTab, onTabChange }: ProvidersTabProps) {
  const refreshKey = useMetricsSubscription()
  const [providerInstances, setProviderInstances] = useState<ProviderInstance[]>([])
  const [providerTypes, setProviderTypes] = useState<ProviderTypeInfo[]>([])
  const [providersHealth, setProvidersHealth] = useState<Record<string, ProviderHealth>>({})
  const [trackedProviders, setTrackedProviders] = useState<string[]>([])
  const [loading, setLoading] = useState(true)
  const [activeTab, setActiveTab] = useState<'add' | 'instances'>('instances')
  const [timeRange, setTimeRange] = useState<'hour' | 'day' | 'week' | 'month'>('day')

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
    loadTrackedProviders()
  }, [refreshKey])

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

  const loadTrackedProviders = async () => {
    try {
      const providers = await invoke<string[]>('list_tracked_providers')
      setTrackedProviders(providers)
    } catch (error) {
      console.error('Failed to load tracked providers:', error)
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

  // If a sub-tab is selected, show detail page for that specific provider instance
  if (activeSubTab) {
    const instance = providerInstances.find(p => p.instance_name === activeSubTab)

    if (!instance && !loading) {
      return (
        <div className="bg-white rounded-lg shadow-sm p-6">
          <h2 className="text-2xl font-bold text-gray-800 mb-4">Provider Not Found</h2>
          <p className="text-gray-600">The requested provider instance could not be found.</p>
        </div>
      )
    }

    if (loading || !instance) {
      return (
        <div className="bg-white rounded-lg shadow-sm p-6">
          <div className="text-center py-8 text-gray-500">Loading provider details...</div>
        </div>
      )
    }

    return (
      <ProviderDetailPage
        instanceName={instance.instance_name}
        providerType={instance.provider_type}
        onTabChange={onTabChange}
      />
    )
  }

  return (
    <div className="space-y-6">
      {/* Metrics Overview */}
      {!loading && trackedProviders.length > 0 && (
        <Card>
          <div className="flex justify-between items-center mb-4">
            <h3 className="text-lg font-semibold text-gray-900 dark:text-gray-100">Provider Usage Overview</h3>
            <select
              value={timeRange}
              onChange={(e) => setTimeRange(e.target.value as any)}
              className="px-4 py-2 border border-gray-300 dark:border-gray-600 rounded-lg bg-white dark:bg-gray-800 text-gray-900 dark:text-gray-100 hover:bg-gray-50 dark:hover:bg-gray-700 focus:outline-none focus:ring-2 focus:ring-blue-500"
            >
              <option value="hour">Last Hour</option>
              <option value="day">Last 24 Hours</option>
              <option value="week">Last 7 Days</option>
              <option value="month">Last 30 Days</option>
            </select>
          </div>

          <div className="space-y-6">
            <StackedAreaChart
              compareType="providers"
              ids={trackedProviders}
              timeRange={timeRange}
              metricType="requests"
              title="Request Volume by Provider"
              refreshTrigger={refreshKey}
            />

            <StackedAreaChart
              compareType="providers"
              ids={trackedProviders}
              timeRange={timeRange}
              metricType="cost"
              title="Cost by Provider"
              refreshTrigger={refreshKey}
            />

            <StackedAreaChart
              compareType="providers"
              ids={trackedProviders}
              timeRange={timeRange}
              metricType="tokens"
              title="Token Usage by Provider"
              refreshTrigger={refreshKey}
            />
          </div>
        </Card>
      )}

      <Card>
        <div className="mb-4">
          <h2 className="text-2xl font-bold text-gray-900">LLM Providers</h2>
          <p className="text-sm text-gray-500 mt-1">
            Configure providers to access various LLM services. Each provider can have multiple instances.
          </p>
        </div>

        {/* Tab Navigation */}
        <div className="flex border-b border-gray-200 mb-6">
          <button
            onClick={() => setActiveTab('add')}
            className={`px-6 py-3 font-medium transition-colors ${
              activeTab === 'add'
                ? 'border-b-2 border-blue-500 text-blue-600'
                : 'text-gray-600 hover:text-gray-900'
            }`}
          >
            Add Provider
          </button>
          <button
            onClick={() => setActiveTab('instances')}
            className={`px-6 py-3 font-medium transition-colors ${
              activeTab === 'instances'
                ? 'border-b-2 border-blue-500 text-blue-600'
                : 'text-gray-600 hover:text-gray-900'
            }`}
          >
            Active Instances ({providerInstances.length})
          </button>
        </div>

        {loading ? (
          <div className="text-center py-8 text-gray-500">Loading providers...</div>
        ) : (
          <>
            {/* Add Provider Tab */}
            {activeTab === 'add' && (
              <div className="space-y-6">
                {/* Available Provider Types */}
                <div>
                  <h3 className="text-lg font-semibold text-gray-700 mb-3">
                    Standard Providers
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
                    <h3 className="text-lg font-semibold text-gray-700 mb-3">
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

            {/* Active Instances Tab */}
            {activeTab === 'instances' && (
              <div>
                {providerInstances.length === 0 ? (
                  <div className="text-center py-12 text-gray-500">
                    <p>No provider instances configured yet. Add a provider to get started.</p>
                  </div>
                ) : (
                  <div className="grid gap-3">
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
                        <div
                          key={instance.instance_name}
                          onClick={() => onTabChange?.('providers', instance.instance_name)}
                          className="bg-gray-50 border border-gray-200 rounded-lg p-4 hover:bg-gray-100 transition-colors cursor-pointer"
                        >
                          <div className="flex items-center justify-between">
                            <div className="flex items-center gap-3">
                              <ProviderIcon providerId={instance.provider_type} size={32} />
                              <div>
                                <h3 className="text-base font-semibold text-gray-900">{instance.instance_name}</h3>
                                <p className="text-sm text-gray-500">{info.name}</p>
                              </div>
                            </div>
                            <div className="flex items-center gap-2">
                              <Badge variant={healthVariant}>
                                {healthStatus}
                                {health?.latency_ms && ` (${health.latency_ms}ms)`}
                              </Badge>
                            </div>
                          </div>
                        </div>
                      )
                    })}
                  </div>
                )}
              </div>
            )}
          </>
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
