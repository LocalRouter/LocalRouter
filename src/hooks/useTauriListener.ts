import { useEffect, useRef } from 'react'
import { listen, type EventCallback } from '@tauri-apps/api/event'

/**
 * Safe wrapper around Tauri's `listen()` that handles React StrictMode
 * double-mount/unmount and the Tauri 2.x bug where `unlisten_js_script`
 * doesn't null-check `listeners[eventId]` before accessing `.handlerId`.
 *
 * Without this wrapper, the unlisten throws synchronously inside the async
 * `_unlisten`, producing an unhandled promise rejection and leaving zombie
 * listeners on the Rust side.
 */
export function useTauriListener<T>(
  event: string,
  handler: EventCallback<T>,
  deps: React.DependencyList = [],
) {
  const handlerRef = useRef(handler)
  handlerRef.current = handler

  useEffect(() => {
    let cancelled = false
    let unlisten: (() => void) | null = null

    listen<T>(event, (e) => {
      if (!cancelled) handlerRef.current(e)
    })
      .then((fn) => {
        if (cancelled) {
          // Component already unmounted — clean up immediately
          try { fn() } catch { /* Tauri unlisten bug — safe to ignore */ }
        } else {
          unlisten = fn
        }
      })
      .catch(() => {
        // listen() itself failed (e.g. stale IPC) — nothing to clean up
      })

    return () => {
      cancelled = true
      if (unlisten) {
        try { unlisten() } catch { /* Tauri unlisten bug — safe to ignore */ }
      }
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, deps)
}

/**
 * Set up multiple Tauri event listeners at once. Returns a cleanup function.
 * Use this inside a useEffect when you need several listeners with a shared cleanup.
 */
export function listenSafe<T>(
  event: string,
  handler: EventCallback<T>,
): { promise: Promise<void>; cleanup: () => void } {
  let cancelled = false
  let unlisten: (() => void) | null = null

  const promise = listen<T>(event, (e) => {
    if (!cancelled) handler(e)
  })
    .then((fn) => {
      if (cancelled) {
        try { fn() } catch { /* ignore */ }
      } else {
        unlisten = fn
      }
    })
    .catch(() => {})

  return {
    promise,
    cleanup: () => {
      cancelled = true
      if (unlisten) {
        try { unlisten() } catch { /* ignore */ }
      }
    },
  }
}
