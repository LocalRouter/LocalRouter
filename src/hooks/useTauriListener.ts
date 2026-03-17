import { useEffect, useRef } from 'react'
import { listen, type EventCallback } from '@tauri-apps/api/event'

/**
 * Safely call an unlisten function, catching both sync throws and async
 * rejections from the Tauri 2.x bug where `unlisten_js_script` accesses
 * `listeners[eventId].handlerId` without a null-check.
 */
function safeUnlisten(fn: () => void) {
  try {
    const result = fn() as unknown
    // Catch async rejections (Tauri unlisten may return a rejecting promise)
    if (result && typeof (result as Promise<void>).catch === 'function') {
      ;(result as Promise<void>).catch(() => {})
    }
  } catch {
    /* Tauri unlisten bug — safe to ignore */
  }
}

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
          safeUnlisten(fn)
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
        safeUnlisten(unlisten)
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
        safeUnlisten(fn)
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
        safeUnlisten(unlisten)
      }
    },
  }
}
