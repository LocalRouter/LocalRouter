import { useState, useEffect } from 'react'
import { invoke } from '@tauri-apps/api/core'
import { listen } from '@tauri-apps/api/event'
import { open } from '@tauri-apps/plugin-shell'
import Button from '../ui/Button'
import Select from '../ui/Select'
import { RouteLLMStatus, RouteLLMState } from '../routellm/types'
import ThresholdTester from './ThresholdTester'

export default function SmartRoutingSubtab() {
  const [routellmStatus, setRouteLLMStatus] = useState<RouteLLMStatus | null>(null)
  const [routellmIdleTimeout, setRouteLLMIdleTimeout] = useState(600)
  const [isDownloadingRouteLLM, setIsDownloadingRouteLLM] = useState(false)
  const [downloadProgress, setDownloadProgress] = useState(0)
  const [feedback, setFeedback] = useState<{ type: 'success' | 'error'; message: string } | null>(null)

  // Auto-dismiss feedback after 5 seconds
  useEffect(() => {
    if (feedback) {
      const timer = setTimeout(() => setFeedback(null), 5000)
      return () => clearTimeout(timer)
    }
  }, [feedback])

  useEffect(() => {
    loadRouteLLMStatus()

    // Listen for download progress events
    const unlistenProgress = listen('routellm-download-progress', (event: any) => {
      const { progress } = event.payload
      setDownloadProgress(progress * 100)
    })

    const unlistenComplete = listen('routellm-download-complete', () => {
      setIsDownloadingRouteLLM(false)
      setDownloadProgress(100)
      loadRouteLLMStatus()
      setFeedback({ type: 'success', message: 'RouteLLM models downloaded successfully!' })
    })

    const unlistenFailed = listen('routellm-download-failed', (event: any) => {
      setIsDownloadingRouteLLM(false)
      setFeedback({ type: 'error', message: `Download failed: ${event.payload.error}` })
    })

    return () => {
      unlistenProgress.then(fn => fn())
      unlistenComplete.then(fn => fn())
      unlistenFailed.then(fn => fn())
    }
  }, [])

  const loadRouteLLMStatus = async () => {
    try {
      const status = await invoke<RouteLLMStatus>('routellm_get_status')
      setRouteLLMStatus(status)
    } catch (error) {
      console.error('Failed to load RouteLLM status:', error)
    }
  }

  const handleRouteLLMDownload = async () => {
    setIsDownloadingRouteLLM(true)
    setDownloadProgress(0)
    setFeedback(null)

    try {
      await invoke('routellm_download_models')
    } catch (error: any) {
      console.error('Failed to start download:', error)
      setFeedback({ type: 'error', message: `Download failed: ${error.message || error}` })
      setIsDownloadingRouteLLM(false)
    }
  }

  const handleRouteLLMUnload = async () => {
    try {
      await invoke('routellm_unload')
      await loadRouteLLMStatus()
      setFeedback({ type: 'success', message: 'RouteLLM models unloaded from memory' })
    } catch (error: any) {
      console.error('Failed to unload:', error)
      setFeedback({ type: 'error', message: `Unload failed: ${error.message || error}` })
    }
  }

  const updateRouteLLMSettings = async () => {
    try {
      await invoke('routellm_update_settings', {
        idleTimeoutSecs: routellmIdleTimeout,
      })
      setFeedback({ type: 'success', message: 'RouteLLM settings updated successfully!' })
    } catch (error: any) {
      console.error('Failed to update RouteLLM settings:', error)
      setFeedback({ type: 'error', message: `Error: ${error.message || error}` })
    }
  }

  const handleOpenRouteLLMFolder = async () => {
    try {
      const homeDir = await invoke<string>('get_home_dir')
      await open(`${homeDir}/.localrouter/routellm`)
    } catch (error) {
      console.error('Failed to open folder:', error)
    }
  }

  const getRouteLLMStatusColor = (state: RouteLLMState): string => {
    switch (state) {
      case 'not_downloaded':
        return 'text-gray-400'
      case 'downloading':
        return 'text-blue-400'
      case 'downloaded_not_running':
        return 'text-yellow-400'
      case 'initializing':
        return 'text-orange-400'
      case 'started':
        return 'text-green-400'
      default:
        return 'text-gray-400'
    }
  }

  const getRouteLLMStatusLabel = (state: RouteLLMState): string => {
    switch (state) {
      case 'not_downloaded':
        return 'Not Downloaded'
      case 'downloading':
        return 'Downloading...'
      case 'downloaded_not_running':
        return 'Ready (Not Loaded)'
      case 'initializing':
        return 'Initializing...'
      case 'started':
        return 'Active in Memory'
      default:
        return 'Unknown'
    }
  }

  return (
    <div className="p-6 max-w-4xl">
      <h2 className="text-xl font-semibold mb-6 text-gray-900 dark:text-gray-100">Smart Routing</h2>

      {/* Feedback Messages */}
      {feedback && (
        <div
          className={`mb-4 p-3 rounded ${
            feedback.type === 'success' ? 'bg-green-500/20 text-green-400' : 'bg-red-500/20 text-red-400'
          }`}
        >
          {feedback.message}
        </div>
      )}

      {/* RouteLLM Intelligent Routing Settings */}
      <div className="bg-gray-100 dark:bg-gray-800 rounded-lg p-6">
        <div className="flex items-center justify-between mb-6">
          <div>
            <h3 className="text-lg font-semibold text-gray-900 dark:text-gray-100">
              RouteLLM Intelligent Routing
            </h3>
            <p className="text-sm text-gray-600 dark:text-gray-400 mt-1">
              ML-based routing to optimize costs while maintaining quality
            </p>
          </div>
          <span className="inline-flex items-center px-3 py-1 rounded-md text-xs font-medium bg-purple-100 dark:bg-purple-900/30 text-purple-700 dark:text-purple-300">
            EXPERIMENTAL
          </span>
        </div>

        <div className="space-y-6">
          {/* Status */}
          {routellmStatus && (
            <div className="p-4 bg-gray-200 dark:bg-gray-900/50 rounded-lg">
              <div className="flex items-center justify-between mb-3">
                <div className="flex items-center gap-3">
                  <div className="text-2xl">
                    {routellmStatus.state === 'not_downloaded' && '⬇️'}
                    {routellmStatus.state === 'downloading' && '⏳'}
                    {routellmStatus.state === 'downloaded_not_running' && '⏸️'}
                    {routellmStatus.state === 'initializing' && '🔄'}
                    {routellmStatus.state === 'started' && '✓'}
                  </div>
                  <div>
                    <div className={`font-semibold ${getRouteLLMStatusColor(routellmStatus.state)}`}>
                      {getRouteLLMStatusLabel(routellmStatus.state)}
                    </div>
                    {routellmStatus.memory_usage_mb && (
                      <div className="text-xs text-gray-600 dark:text-gray-400 mt-1">
                        Memory: {(routellmStatus.memory_usage_mb / 1024).toFixed(2)} GB
                      </div>
                    )}
                  </div>
                </div>
                <div className="flex gap-2">
                  {routellmStatus.state === 'started' && (
                    <Button variant="secondary" onClick={handleRouteLLMUnload}>
                      Unload from Memory
                    </Button>
                  )}
                  {/* Only show re-download when models are actually downloaded */}
                  {(routellmStatus.state === 'downloaded_not_running' ||
                    routellmStatus.state === 'started' ||
                    routellmStatus.state === 'initializing') && (
                    <Button variant="secondary" onClick={handleRouteLLMDownload}>
                      Re-download Models
                    </Button>
                  )}
                </div>
              </div>

              {/* Model Location */}
              <div className="mt-3 pt-3 border-t border-gray-300 dark:border-gray-700">
                <div className="text-xs text-gray-600 dark:text-gray-400">
                  <span>Model location: </span>
                  <button
                    onClick={handleOpenRouteLLMFolder}
                    className="text-blue-600 dark:text-blue-400 hover:underline focus:outline-none font-mono"
                  >
                    ~/.localrouter/routellm/
                  </button>
                </div>
              </div>
            </div>
          )}

          {/* Download Section */}
          {routellmStatus?.state === 'not_downloaded' && (
            <div className="p-4 bg-blue-100 dark:bg-blue-900/20 border border-blue-300 dark:border-blue-700 rounded-lg">
              <h4 className="font-semibold text-blue-900 dark:text-blue-100 mb-2">
                🎯 Intelligent Cost Optimization
              </h4>
              <p className="text-sm text-blue-800 dark:text-blue-200 mb-3">
                RouteLLM uses machine learning to analyze each prompt and automatically route to the
                most cost-effective model while maintaining quality.
              </p>
              <div className="grid grid-cols-3 gap-2 text-xs mb-4">
                <div className="bg-gray-200 dark:bg-gray-900 p-2 rounded">
                  <div className="font-semibold text-green-600 dark:text-green-400">30-60%</div>
                  <div className="text-gray-600 dark:text-gray-400">Cost Savings</div>
                </div>
                <div className="bg-gray-200 dark:bg-gray-900 p-2 rounded">
                  <div className="font-semibold text-blue-600 dark:text-blue-400">85-95%</div>
                  <div className="text-gray-600 dark:text-gray-400">Quality Retained</div>
                </div>
                <div className="bg-gray-200 dark:bg-gray-900 p-2 rounded">
                  <div className="font-semibold text-purple-600 dark:text-purple-400">~10ms</div>
                  <div className="text-gray-600 dark:text-gray-400">Routing Time</div>
                </div>
              </div>

              <div className="p-3 bg-yellow-100 dark:bg-yellow-900/20 border border-yellow-300 dark:border-yellow-700 rounded text-sm mb-4 text-yellow-900 dark:text-yellow-200">
                <strong>Download Required:</strong> Models (~1.08 GB) will be downloaded to{' '}
                <code className="bg-yellow-200 dark:bg-yellow-800/30 px-1 rounded">
                  ~/.localrouter/routellm/
                </code>
              </div>

              <Button onClick={handleRouteLLMDownload} disabled={isDownloadingRouteLLM}>
                {isDownloadingRouteLLM ? `Downloading... ${downloadProgress.toFixed(0)}%` : 'Download Models'}
              </Button>
            </div>
          )}

          {/* Download Progress */}
          {isDownloadingRouteLLM && (
            <div className="p-4 bg-blue-100 dark:bg-blue-900/20 border border-blue-300 dark:border-blue-700 rounded-lg">
              <div className="mb-2 flex justify-between text-sm">
                <span className="font-medium text-blue-900 dark:text-blue-100">
                  Downloading RouteLLM Models...
                </span>
                <span className="text-blue-700 dark:text-blue-300">
                  {downloadProgress.toFixed(0)}%
                </span>
              </div>
              <div className="w-full bg-blue-300 dark:bg-blue-800 rounded-full h-2">
                <div
                  className="bg-blue-600 dark:bg-blue-400 h-2 rounded-full transition-all duration-300"
                  style={{ width: `${downloadProgress}%` }}
                />
              </div>
            </div>
          )}

          {/* Settings (only show when downloaded) */}
          {routellmStatus?.state !== 'not_downloaded' && routellmStatus?.state !== 'downloading' && (
            <>
              <div className="space-y-4">
                <h4 className="font-semibold text-gray-900 dark:text-gray-100">
                  Memory Management
                </h4>

                <Select
                  label="Auto-Unload After Idle"
                  value={routellmIdleTimeout}
                  onChange={(e) => setRouteLLMIdleTimeout(parseInt(e.target.value))}
                  helperText="Automatically unload models from memory after inactivity to save RAM (~2.65 GB)"
                >
                  <option value="300">5 minutes</option>
                  <option value="600">10 minutes (recommended)</option>
                  <option value="1800">30 minutes</option>
                  <option value="3600">1 hour</option>
                  <option value="0">Never</option>
                </Select>

                <Button onClick={updateRouteLLMSettings}>
                  Save Memory Settings
                </Button>
              </div>

              {/* Resource Info */}
              <div className="p-4 bg-orange-100 dark:bg-orange-900/20 border border-orange-300 dark:border-orange-700 rounded-lg">
                <h4 className="font-semibold text-orange-900 dark:text-orange-100 mb-2">
                  Resource Requirements
                </h4>
                <div className="grid grid-cols-2 gap-2 text-xs text-orange-800 dark:text-orange-200">
                  <div>
                    <strong>Cold Start:</strong> ~1.5s
                  </div>
                  <div>
                    <strong>Disk Space:</strong> 1.08 GB
                  </div>
                  <div>
                    <strong>Latency:</strong> ~10ms per request
                  </div>
                  <div>
                    <strong>Memory:</strong> ~2.65 GB (when loaded)
                  </div>
                </div>
              </div>

              {/* Configuration Note */}
              <div className="p-4 bg-gray-200 dark:bg-gray-900/50 border border-gray-300 dark:border-gray-700 rounded-lg">
                <p className="text-sm text-gray-700 dark:text-gray-300">
                  <strong>Note:</strong> RouteLLM routing is configured per-strategy. Go to a
                  strategy's "Intelligent Routing" tab to enable and configure threshold, strong
                  models, and weak models.
                </p>
              </div>
            </>
          )}
        </div>
      </div>

      {/* Threshold Testing - only show when models are downloaded */}
      {routellmStatus?.state !== 'not_downloaded' && routellmStatus?.state !== 'downloading' && (
        <div className="mt-6">
          <ThresholdTester />
        </div>
      )}
    </div>
  )
}
