import { useEffect, useState } from 'react'
import { listen } from '@tauri-apps/api/event'

/**
 * Hook to subscribe to metrics updates and trigger re-renders
 *
 * Returns a refreshKey that increments whenever metrics are updated,
 * which can be passed to chart components to trigger data reloading.
 */
export function useMetricsSubscription() {
  const [refreshKey, setRefreshKey] = useState(0)

  useEffect(() => {
    let unlisten: (() => void) | undefined

    // Listen for metrics-updated events from the backend
    listen('metrics-updated', (event) => {
      console.log('Metrics updated:', event.payload)
      setRefreshKey(prev => prev + 1)
    }).then(fn => {
      unlisten = fn
    })

    // Cleanup on unmount
    return () => {
      if (unlisten) {
        unlisten()
      }
    }
  }, [])

  return refreshKey
}
