import { useState, useEffect } from 'react'
import { listen } from '@tauri-apps/api/event'
import Header from './components/Header'
import Sidebar from './components/Sidebar'
import HomeTab from './components/tabs/HomeTab'
import ApiKeysTab from './components/tabs/ApiKeysTab'
import ProvidersTab from './components/tabs/ProvidersTab'
import ServerTab from './components/tabs/ServerTab'
import { VisualizationTab } from './components/tabs/VisualizationTab'
import { PrioritizedListModal } from './components/PrioritizedListModal'
import { invoke } from '@tauri-apps/api/core'

type Tab = 'home' | 'api-keys' | 'providers' | 'server' | 'visualization'

interface ApiKeyInfo {
  id: string
  name: string
  enabled: boolean
  created_at: string
}

function App() {
  const [activeTab, setActiveTab] = useState<Tab>('home')
  const [prioritizedListModal, setPrioritizedListModal] = useState<{
    isOpen: boolean
    apiKeyId: string
    apiKeyName: string
  }>({
    isOpen: false,
    apiKeyId: '',
    apiKeyName: '',
  })

  useEffect(() => {
    // Subscribe to configuration changes
    const unsubscribeConfig = listen('config-changed', (event: any) => {
      console.log('Configuration changed:', event.payload)
    })

    // Subscribe to open-prioritized-list event from tray
    const unsubscribePrioritized = listen<string>('open-prioritized-list', async (event) => {
      const apiKeyId = event.payload
      console.log('Opening prioritized list for API key:', apiKeyId)

      try {
        // Get API key info to show the name
        const keys = await invoke<ApiKeyInfo[]>('list_api_keys')
        const key = keys.find((k) => k.id === apiKeyId)

        if (key) {
          // Switch to API keys tab
          setActiveTab('api-keys')

          // Open modal
          setPrioritizedListModal({
            isOpen: true,
            apiKeyId: key.id,
            apiKeyName: key.name,
          })
        }
      } catch (err) {
        console.error('Failed to load API key:', err)
      }
    })

    return () => {
      unsubscribeConfig.then((fn: any) => fn())
      unsubscribePrioritized.then((fn: any) => fn())
    }
  }, [])

  return (
    <div className="flex flex-col h-screen bg-gray-50">
      <Header />

      <div className="flex flex-1 overflow-hidden">
        <Sidebar activeTab={activeTab} onTabChange={setActiveTab} />

        <main className="flex-1 p-8 overflow-y-auto">
          {activeTab === 'home' && <HomeTab />}
          {activeTab === 'api-keys' && <ApiKeysTab />}
          {activeTab === 'providers' && <ProvidersTab />}
          {activeTab === 'server' && <ServerTab />}
          {activeTab === 'visualization' && <VisualizationTab />}
        </main>
      </div>

      {/* Prioritized List Modal */}
      <PrioritizedListModal
        isOpen={prioritizedListModal.isOpen}
        onClose={() =>
          setPrioritizedListModal({ isOpen: false, apiKeyId: '', apiKeyName: '' })
        }
        apiKeyId={prioritizedListModal.apiKeyId}
        apiKeyName={prioritizedListModal.apiKeyName}
        onSuccess={() => {
          console.log('Prioritized list saved successfully')
          // The modal will close itself and tray menu will rebuild
        }}
      />
    </div>
  )
}

export default App
