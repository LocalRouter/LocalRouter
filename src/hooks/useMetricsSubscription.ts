import { useEffect, useState } from 'react'
import { listenSafe } from '@/hooks/useTauriListener'

/**
 * Hook to subscribe to metrics updates and trigger re-renders
 *
 * Returns a refreshKey that increments whenever metrics are updated,
 * which can be passed to chart components to trigger data reloading.
 */
export function useMetricsSubscription() {
  const [refreshKey, setRefreshKey] = useState(0)

  useEffect(() => {
    const l = listenSafe('metrics-updated', (event) => {
      console.log('Metrics updated:', event.payload)
      setRefreshKey(prev => prev + 1)
    })

    return () => {
      l.cleanup()
    }
  }, [])

  return refreshKey
}
