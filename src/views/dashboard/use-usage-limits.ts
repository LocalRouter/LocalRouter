import { useCallback, useEffect, useRef, useState } from 'react'
import { invoke } from '@tauri-apps/api/core'
import { useTauriListener } from '@/hooks/useTauriListener'
import type { UsageSnapshot } from '@/types/tauri-commands'

/** Countdowns and staleness are recomputed on this clock. */
const CLOCK_MS = 30_000

/**
 * The usage tracker's snapshot, refetched on `usage-limits-changed`
 * (debounced) and on a slow clock so countdowns stay current.
 */
export function useUsageLimits() {
  const [snapshot, setSnapshot] = useState<UsageSnapshot | null>(null)
  const [error, setError] = useState<string | null>(null)
  const [now, setNow] = useState(() => Date.now())
  const timer = useRef<number | null>(null)

  const load = useCallback(async () => {
    try {
      setSnapshot(await invoke<UsageSnapshot>('get_usage_limits'))
      setError(null)
    } catch (e) {
      setError(String(e))
    }
    setNow(Date.now())
  }, [])

  useEffect(() => {
    load()
    const clock = window.setInterval(load, CLOCK_MS)
    return () => {
      window.clearInterval(clock)
      if (timer.current !== null) window.clearTimeout(timer.current)
    }
  }, [load])

  useTauriListener('usage-limits-changed', () => {
    if (timer.current !== null) return
    timer.current = window.setTimeout(() => {
      timer.current = null
      load()
    }, 500)
  })

  const refresh = useCallback(async () => {
    try {
      await invoke('refresh_usage_limits')
    } finally {
      // Polls finish asynchronously and announce themselves; load what we
      // have now for immediate feedback.
      await load()
    }
  }, [load])

  return { snapshot, error, now, reload: load, refresh }
}
