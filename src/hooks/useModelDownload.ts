import { useState, useEffect, useCallback, useRef } from 'react'
import { invoke } from '@tauri-apps/api/core'
import { listenSafe } from '@/hooks/useTauriListener'

export type DownloadStatus = 'idle' | 'downloading' | 'downloaded' | 'failed'

export interface UseModelDownloadConfig {
  /** Whether the model is already downloaded (from parent status query) */
  isDownloaded: boolean
  /** Tauri command name to start the download */
  downloadCommand: string
  /** Arguments to pass to the download command */
  downloadArgs?: Record<string, unknown>
  /** Event name for progress updates (payload must have progress 0-1) */
  progressEvent?: string
  /** Event name for download completion (if absent, invoke resolution = completion) */
  completeEvent?: string
  /** Event name for download failure (if absent, invoke rejection = failure) */
  failedEvent?: string
  /** Extract 0-1 progress from event payload. Default: (p) => p.progress */
  normalizeProgress?: (payload: any) => number
  /** Filter events (e.g. match provider_id/model_name for provider pulls) */
  eventFilter?: (payload: any) => boolean
  /** Called on successful download */
  onComplete?: () => void
  /** Called on failed download */
  onFailed?: (error: string) => void
}

export interface UseModelDownloadReturn {
  status: DownloadStatus
  /** Progress 0-100 */
  progress: number
  /** Error message when status === 'failed' */
  error: string | null
  /** Start the download */
  startDownload: () => void
  /** Retry after failure (resets error and starts download) */
  retry: () => void
}

export function useModelDownload(config: UseModelDownloadConfig): UseModelDownloadReturn {
  const {
    isDownloaded,
    downloadCommand,
    downloadArgs,
    progressEvent,
    completeEvent,
    failedEvent,
    normalizeProgress = (p: any) => p.progress,
    eventFilter,
    onComplete,
    onFailed,
  } = config

  const [status, setStatus] = useState<DownloadStatus>(isDownloaded ? 'downloaded' : 'idle')
  const [progress, setProgress] = useState(0)
  const [error, setError] = useState<string | null>(null)

  // Use a ref to prevent double-transitions from both invoke resolution and complete event
  const hasCompletedRef = useRef(false)
  // Track latest callbacks to avoid stale closures
  const onCompleteRef = useRef(onComplete)
  const onFailedRef = useRef(onFailed)
  onCompleteRef.current = onComplete
  onFailedRef.current = onFailed

  // Sync status with parent's isDownloaded (e.g. after model deletion)
  useEffect(() => {
    setStatus(prev => {
      if (isDownloaded && prev === 'idle') return 'downloaded'
      if (!isDownloaded && prev === 'downloaded') return 'idle'
      return prev
    })
  }, [isDownloaded])

  // Set up event listeners (always active to detect in-flight downloads on remount)
  useEffect(() => {
    const cleanups: (() => void)[] = []

    if (progressEvent) {
      const l = listenSafe(progressEvent, (event: any) => {
        if (eventFilter && !eventFilter(event.payload)) return
        const normalized = normalizeProgress(event.payload)
        setProgress(normalized * 100)
        // If we receive progress while idle, an in-flight download exists (remount case)
        setStatus(prev => prev === 'idle' ? 'downloading' : prev)
      })
      cleanups.push(l.cleanup)
    }

    if (completeEvent) {
      const l = listenSafe(completeEvent, (event: any) => {
        if (eventFilter && !eventFilter(event.payload)) return
        if (hasCompletedRef.current) return
        hasCompletedRef.current = true
        setStatus('downloaded')
        setProgress(100)
        onCompleteRef.current?.()
      })
      cleanups.push(l.cleanup)
    }

    if (failedEvent) {
      const l = listenSafe(failedEvent, (event: any) => {
        if (eventFilter && !eventFilter(event.payload)) return
        const errMsg = event.payload?.error || 'Download failed'
        setStatus('failed')
        setError(errMsg)
        onFailedRef.current?.(errMsg)
      })
      cleanups.push(l.cleanup)
    }

    return () => {
      cleanups.forEach(fn => fn())
    }
    // Re-subscribe if event names or filter change
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [progressEvent, completeEvent, failedEvent])

  const startDownload = useCallback(() => {
    hasCompletedRef.current = false
    setStatus('downloading')
    setProgress(0)
    setError(null)

    invoke(downloadCommand, downloadArgs).then(() => {
      // For sync commands without a completeEvent, invoke resolution = success
      if (!completeEvent && !hasCompletedRef.current) {
        hasCompletedRef.current = true
        setStatus('downloaded')
        setProgress(100)
        onCompleteRef.current?.()
      }
    }).catch((err: any) => {
      // For commands without a failedEvent, invoke rejection = failure
      if (!failedEvent) {
        const errMsg = err?.message || String(err)
        setStatus('failed')
        setError(errMsg)
        onFailedRef.current?.(errMsg)
      }
    })
  }, [downloadCommand, downloadArgs, completeEvent, failedEvent])

  const retry = useCallback(() => {
    startDownload()
  }, [startDownload])

  return { status, progress, error, startDownload, retry }
}
