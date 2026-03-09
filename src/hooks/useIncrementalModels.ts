import { useState, useEffect, useCallback, useRef } from 'react'
import { invoke } from '@tauri-apps/api/core'
import { listen } from '@tauri-apps/api/event'

export interface Model {
  id: string
  provider: string
}

interface ProviderModelsPayload {
  provider: string
  models: Model[]
}

interface ModelsRefreshStartedPayload {
  providers: string[]
}

interface UseIncrementalModelsOptions {
  /** Trigger a refresh on mount (default: true) */
  refreshOnMount?: boolean
}

interface UseIncrementalModelsResult {
  models: Model[]
  /** Set of provider instance names still loading */
  loadingProviders: Set<string>
  /** True once all providers have responded (or no refresh is in progress) */
  isFullyLoaded: boolean
  /** Manually trigger an incremental refresh. Pass true to force-bypass cache. */
  refresh: (force?: boolean) => void
}

export function useIncrementalModels(
  options: UseIncrementalModelsOptions = {},
): UseIncrementalModelsResult {
  const { refreshOnMount = true } = options
  const [models, setModels] = useState<Model[]>([])
  const [loadingProviders, setLoadingProviders] = useState<Set<string>>(new Set())
  const mountedRef = useRef(true)

  const refresh = useCallback((force?: boolean) => {
    invoke('refresh_models_incremental', { force: force ?? false }).catch(() => {})
  }, [])

  useEffect(() => {
    mountedRef.current = true

    // Show cached models instantly
    invoke<Model[]>('get_cached_models')
      .then(cached => {
        if (mountedRef.current && cached.length > 0) setModels(cached)
      })
      .catch(() => {})

    if (refreshOnMount) {
      refresh()
    }

    const unsubscribers = [
      listen<ModelsRefreshStartedPayload>('models-refresh-started', (event) => {
        if (!mountedRef.current) return
        setLoadingProviders(new Set(event.payload.providers))
      }),
      listen<ProviderModelsPayload>('models-provider-loaded', (event) => {
        if (!mountedRef.current) return
        const { provider, models: providerModels } = event.payload
        setModels(prev => [
          ...prev.filter(m => m.provider !== provider),
          ...providerModels,
        ])
        setLoadingProviders(prev => {
          const next = new Set(prev)
          next.delete(provider)
          return next
        })
      }),
      listen('models-changed', () => {
        if (!mountedRef.current) return
        setLoadingProviders(new Set())
      }),
    ]

    return () => {
      mountedRef.current = false
      unsubscribers.forEach((unsub) => {
        unsub.then((fn) => fn()).catch(() => {})
      })
    }
  }, [])

  return {
    models,
    loadingProviders,
    isFullyLoaded: loadingProviders.size === 0,
    refresh,
  }
}
