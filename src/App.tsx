import { useState, useEffect } from 'react'
import { listen } from '@tauri-apps/api/event'
import Header from './components/Header'
import Sidebar from './components/Sidebar'
import HomeTab from './components/tabs/HomeTab'
import ApiKeysTab from './components/tabs/ApiKeysTab'
import ProvidersTab from './components/tabs/ProvidersTab'

type Tab = 'home' | 'api-keys' | 'providers'

function App() {
  const [activeTab, setActiveTab] = useState<Tab>('home')

  useEffect(() => {
    // Subscribe to configuration changes
    const unsubscribe = listen('config-changed', (event: any) => {
      console.log('Configuration changed:', event.payload)
    })

    return () => {
      unsubscribe.then((fn: any) => fn())
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
        </main>
      </div>
    </div>
  )
}

export default App
