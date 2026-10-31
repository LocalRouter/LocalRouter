import { useCallback, useMemo, useSyncExternalStore } from 'react'
import { invoke } from '@tauri-apps/api/core'
import type { SetUiStateParams, UiStateMap } from '@/types/tauri-commands'
import { UiStateStore } from '@/lib/ui-state-store'

function browserStorage(): Storage | null {
  try {
    return typeof window !== 'undefined' ? window.localStorage : null
  } catch {
    return null
  }
}

/** The app's durable UI state, backed by `ui_state.json` and mirrored to localStorage. */
export const uiState = new UiStateStore(
  {
    load: () => invoke<UiStateMap>('get_ui_state'),
    save: (key, value) =>
      invoke<void>('set_ui_state', { key, value } satisfies SetUiStateParams),
  },
  browserStorage(),
)

/** Turns a stored value into `T`, or `undefined` when it is unusable. */
export type UiStateDecoder<T> = (value: unknown) => T | undefined

function sameType<T>(fallback: T): UiStateDecoder<T> {
  return (value) => (typeof value === typeof fallback ? (value as T) : undefined)
}

/** Reads a UI state value outside React (for one-off initial values). */
export function readUiState<T>(key: string, fallback: T, decode: UiStateDecoder<T> = sameType(fallback)): T {
  const value = uiState.get(key)
  if (value === undefined) return fallback
  return decode(value) ?? fallback
}

/** Writes a UI state value outside React. */
export function writeUiState(key: string, value: unknown) {
  uiState.set(key, value)
}

/**
 * A piece of UI state remembered across restarts. Reads come from the store
 * loaded at startup, so the first render already shows the saved value; writes
 * go to the backend file at once and to localStorage.
 */
export function usePersistentUiState<T>(
  key: string,
  fallback: T,
  decode: UiStateDecoder<T> = sameType(fallback),
): [T, (next: T | ((current: T) => T)) => void] {
  const subscribe = useCallback((listener: () => void) => uiState.subscribe(key, listener), [key])
  const getSnapshot = useCallback(() => uiState.get(key), [key])
  const stored = useSyncExternalStore(subscribe, getSnapshot)
  // `decode` and `fallback` are expected to be stable in meaning for a key.
  // eslint-disable-next-line react-hooks/exhaustive-deps
  const value = useMemo(() => (stored === undefined ? fallback : decode(stored) ?? fallback), [stored])

  const setValue = useCallback(
    (next: T | ((current: T) => T)) => {
      const current = readUiState(key, fallback, decode)
      const resolved = typeof next === 'function' ? (next as (current: T) => T)(current) : next
      uiState.set(key, resolved)
    },
    // eslint-disable-next-line react-hooks/exhaustive-deps
    [key],
  )

  return [value, setValue]
}
