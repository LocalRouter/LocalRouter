import React, { Suspense, useEffect, useState } from 'react'
import { setupTauriMocks, stubTauriInternals } from '../components/demo/TauriMockSetup'

// Lazy load the main Tauri app
const TauriApp = React.lazy(() => import('@app/App'))

export default function Demo() {
  const [ready, setReady] = useState(false)

  useEffect(() => {
    // Initialize Tauri mocks
    stubTauriInternals()
    setupTauriMocks()
    console.log('[Demo] Mocks initialized')
    setReady(true)
  }, [])

  if (!ready) {
    return (
      <div className="h-screen flex items-center justify-center bg-zinc-900 text-white">
        Initializing demo...
      </div>
    )
  }

  return (
    <div className="h-screen w-screen overflow-hidden">
      <Suspense fallback={
        <div className="h-screen flex items-center justify-center bg-zinc-900 text-white">
          Loading LocalRouter...
        </div>
      }>
        <TauriApp />
      </Suspense>
    </div>
  )
}
