import { useSyncExternalStore } from 'react'
import { invoke } from '@tauri-apps/api/core'
import type { DiscoveredResourceIcon, DiscoverResourceIconParams } from '@/types/tauri-commands'

const STORAGE_KEY = 'localrouter-resource-icons-v1'
interface CachedIcon { origin: string; data: DiscoveredResourceIcon }
const cache = new Map<string, CachedIcon>()
const pending = new Map<string, { origin: string }>()
const listeners = new Set<() => void>()
let loaded = false

/** Only the origin is retained; API paths, queries and credentials are discarded. */
export function resourceIconOrigin(endpoint?: string): string | undefined {
  try {
    const url = new URL(endpoint ?? '')
    if (url.protocol === 'https:' || url.protocol === 'http:') return url.origin
  } catch { /* Local command resources have no website to discover. */ }
  return undefined
}

function load() {
  if (loaded) return
  loaded = true
  try {
    const entries: unknown = JSON.parse(localStorage.getItem(STORAGE_KEY) ?? '[]')
    if (Array.isArray(entries)) {
      for (const entry of entries.slice(-100)) {
        if (!Array.isArray(entry) || entry.length !== 2) continue
        const [key, value] = entry
        if (typeof key === 'string' && typeof value?.origin === 'string'
          && (value.data === null || (typeof value.data === 'string'
            && value.data.startsWith('data:image/png;base64,') && value.data.length < 40000))) {
          cache.set(key, value)
        }
      }
    }
  } catch { /* Unavailable storage or invalid cache must never break rendering. */ }
}

function notify() {
  while (cache.size > 100) cache.delete(cache.keys().next().value!)
  try { localStorage.setItem(STORAGE_KEY, JSON.stringify([...cache])) } catch { /* Best effort. */ }
  listeners.forEach(listener => listener())
}

/** Call only from a successful user add/edit action. Never from an icon render. */
export async function discoverResourceIcon(resourceKey: string, endpoint?: string): Promise<void> {
  load()
  const origin = resourceIconOrigin(endpoint)
  if (!origin) {
    pending.delete(resourceKey)
    if (cache.delete(resourceKey)) notify()
    return
  }
  if (cache.get(resourceKey)?.origin === origin || pending.get(resourceKey)?.origin === origin) return
  const request = { origin }
  pending.set(resourceKey, request)
  // Clear a previous endpoint's branding immediately while its replacement loads.
  cache.delete(resourceKey)
  notify()
  try {
    const data = await invoke<DiscoveredResourceIcon>('discover_resource_icon', {
      endpointUrl: origin,
    } satisfies DiscoverResourceIconParams)
    if (pending.get(resourceKey) !== request) return
    cache.set(resourceKey, { origin, data })
    notify()
  } catch { /* Discovery failure must never fail resource creation or editing. */ }
  finally {
    if (pending.get(resourceKey) === request) pending.delete(resourceKey)
  }
}

/** Keep the local image cache consistent with resource rename/delete actions. */
export function forgetResourceIcon(resourceKey: string) {
  load()
  pending.delete(resourceKey)
  if (cache.delete(resourceKey)) notify()
}

export function renameResourceIcon(oldKey: string, newKey: string) {
  load()
  const icon = cache.get(oldKey)
  pending.delete(oldKey)
  cache.delete(oldKey)
  if (icon) cache.set(newKey, icon)
  notify()
}

const subscribe = (listener: () => void) => {
  listeners.add(listener)
  return () => { listeners.delete(listener) }
}

export function useResourceIcon(resourceKey?: string): string | undefined {
  load()
  const snapshot = () => resourceKey ? cache.get(resourceKey)?.data ?? undefined : undefined
  return useSyncExternalStore(subscribe, snapshot, snapshot)
}
