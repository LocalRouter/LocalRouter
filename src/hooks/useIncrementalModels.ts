import { useState, useEffect, useCallback } from 'react'
import { invoke } from '@tauri-apps/api/core'
import { listenSafe } from '@/hooks/useTauriListener'

export interface Model {
  id: string
  name?: string
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

  const refresh = useCallback((force?: boolean) => {
    invoke('refresh_models_incremental', { force: force ?? false }).catch(() => {})
  }, [])

  useEffect(() => {
    let cancelled = false
    const refreshedProviders = new Set<string>()

    // Show cached models instantly
    invoke<Model[]>('get_cached_models')
      .then(cached => {
        if (cancelled || cached.length === 0) return
        // A provider may finish refreshing before the cache IPC reply arrives.
        // Keep those newer results, including a provider's newly empty list.
        setModels(prev => [
          ...cached.filter(model => !refreshedProviders.has(model.provider)),
          ...prev.filter(model => refreshedProviders.has(model.provider)),
        ])
      })
      .catch(() => {})

    const listeners = [
      listenSafe<ModelsRefreshStartedPayload>('models-refresh-started', (event) => {
        if (cancelled) return
        setLoadingProviders(new Set(event.payload.providers))
      }),
      listenSafe<ProviderModelsPayload>('models-provider-loaded', (event) => {
        if (cancelled) return
        const { provider, models: providerModels } = event.payload
        refreshedProviders.add(provider)
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
      listenSafe('models-changed', () => {
        if (cancelled) return
        setLoadingProviders(new Set())
      }),
    ]

    // Tauri listener registration is asynchronous. Starting the refresh first
    // can lose every event when the provider answers from its local cache.
    void Promise.all(listeners.map(listener => listener.promise)).then(() => {
      if (!cancelled && refreshOnMount) refresh()
    })

    return () => {
      cancelled = true
      listeners.forEach(l => l.cleanup())
    }
  }, [refreshOnMount, refresh])

  return {
    models,
    loadingProviders,
    isFullyLoaded: loadingProviders.size === 0,
    refresh,
  }
}
