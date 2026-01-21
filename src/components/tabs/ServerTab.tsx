import { useState, useEffect } from 'react'
import { invoke } from '@tauri-apps/api/core'
import { listen } from '@tauri-apps/api/event'
import { open } from '@tauri-apps/plugin-shell'
import Button from '../ui/Button'
import Select from '../ui/Select'
import Input from '../ui/Input'
import { RouteLLMStatus, RouteLLMState } from '../routellm/types'

interface ServerConfig {
  host: string
  port: number
  actual_port?: number
  enable_cors: boolean
}

interface NetworkInterface {
  name: string
  ip: string
  is_loopback: boolean
}

interface TrayGraphSettings {
  enabled: boolean
  interval_secs: number
}

export default function ServerTab() {
  const [config, setConfig] = useState<ServerConfig>({
    host: '127.0.0.1',
    port: 3625,
    enable_cors: true,
  })
  const [editConfig, setEditConfig] = useState<ServerConfig>(config)
  const [isUpdating, setIsUpdating] = useState(false)
  const [isRestarting, setIsRestarting] = useState(false)
  const [feedback, setFeedback] = useState<{ type: 'success' | 'error'; message: string } | null>(null)
  const [networkInterfaces, setNetworkInterfaces] = useState<NetworkInterface[]>([])
  const [hasUnsavedChanges, setHasUnsavedChanges] = useState(false)
  const [trayGraphSettings, setTrayGraphSettings] = useState<TrayGraphSettings>({
    enabled: false,
    interval_secs: 2,
  })
  const [isUpdatingTrayGraph, setIsUpdatingTrayGraph] = useState(false)
  const [routellmStatus, setRouteLLMStatus] = useState<RouteLLMStatus | null>(null)
  const [routellmIdleTimeout, setRouteLLMIdleTimeout] = useState(600)
  const [isDownloadingRouteLLM, setIsDownloadingRouteLLM] = useState(false)
  const [downloadProgress, setDownloadProgress] = useState(0)

  // Auto-dismiss feedback after 5 seconds
  useEffect(() => {
    if (feedback) {
      const timer = setTimeout(() => setFeedback(null), 5000)
      return () => clearTimeout(timer)
    }
  }, [feedback])

  useEffect(() => {
    loadConfig()
    loadNetworkInterfaces()
    loadTrayGraphSettings()
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

  // Check for unsaved changes
  useEffect(() => {
    const changed =
      editConfig.host !== config.host ||
      editConfig.port !== config.port
    setHasUnsavedChanges(changed)
  }, [editConfig, config])

  const loadConfig = async () => {
    try {
      const serverConfig = await invoke<ServerConfig>('get_server_config')
      setConfig(serverConfig)
      setEditConfig(serverConfig)
    } catch (error) {
      console.error('Failed to load server config:', error)
    }
  }

  const loadNetworkInterfaces = async () => {
    try {
      const interfaces = await invoke<NetworkInterface[]>('get_network_interfaces')
      setNetworkInterfaces(interfaces)
    } catch (error) {
      console.error('Failed to load network interfaces:', error)
    }
  }

  const updateConfig = async (e: React.FormEvent) => {
    e.preventDefault()
    setIsUpdating(true)
    setFeedback(null)

    try {
      await invoke('update_server_config', {
        host: editConfig.host,
        port: editConfig.port,
        enableCors: true, // Always enable CORS
      })

      await invoke('restart_server')

      // Wait for server to restart, then reload config
      await new Promise(resolve => setTimeout(resolve, 1500))
      await loadConfig()

      setFeedback({ type: 'success', message: 'Server configuration updated and restarted successfully!' })
    } catch (error: any) {
      console.error('Failed to update server config:', error)
      setFeedback({ type: 'error', message: `Error: ${error.message || error}` })
    } finally {
      setIsUpdating(false)
    }
  }

  const restartServer = async () => {
    setIsRestarting(true)
    setFeedback(null)

    try {
      await invoke('restart_server')

      // Wait for server to restart, then reload config
      await new Promise(resolve => setTimeout(resolve, 1500))
      await loadConfig()

      setFeedback({ type: 'success', message: 'Server restarted successfully!' })
    } catch (error: any) {
      console.error('Failed to restart server:', error)
      setFeedback({ type: 'error', message: `Error: ${error.message || error}` })
    } finally {
      setIsRestarting(false)
    }
  }

  const copyToClipboard = (text: string) => {
    navigator.clipboard.writeText(text)
    setFeedback({ type: 'success', message: 'Copied to clipboard!' })
  }

  const loadTrayGraphSettings = async () => {
    try {
      const settings = await invoke<TrayGraphSettings>('get_tray_graph_settings')
      setTrayGraphSettings(settings)
    } catch (error) {
      console.error('Failed to load tray graph settings:', error)
    }
  }

  const updateTrayGraphSettings = async (e: React.FormEvent) => {
    e.preventDefault()
    setIsUpdatingTrayGraph(true)
    setFeedback(null)

    try {
      await invoke('update_tray_graph_settings', {
        enabled: trayGraphSettings.enabled,
        intervalSecs: trayGraphSettings.interval_secs,
      })

      setFeedback({ type: 'success', message: 'Tray graph settings updated successfully!' })
    } catch (error: any) {
      console.error('Failed to update tray graph settings:', error)
      setFeedback({ type: 'error', message: `Error: ${error.message || error}` })
    } finally {
      setIsUpdatingTrayGraph(false)
    }
  }

  // Calculate time window for display
  const calculateTimeWindow = (intervalSecs: number): string => {
    const totalSecs = 30 * intervalSecs
    if (totalSecs < 60) {
      return `${totalSecs} seconds`
    } else {
      const mins = Math.floor(totalSecs / 60)
      const secs = totalSecs % 60
      return secs > 0 ? `${mins}m ${secs}s` : `${mins} minute${mins > 1 ? 's' : ''}`
    }
  }

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
        return 'text-gray-600 dark:text-gray-400'
      case 'downloading':
        return 'text-blue-600 dark:text-blue-400'
      case 'downloaded_not_running':
        return 'text-yellow-600 dark:text-yellow-400'
      case 'initializing':
        return 'text-orange-600 dark:text-orange-400'
      case 'started':
        return 'text-green-600 dark:text-green-400'
      default:
        return 'text-gray-600 dark:text-gray-400'
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
    <div className="space-y-6 relative">
      {/* Toast Notification - Fixed position at bottom-right */}
      {feedback && (
        <div
          className={`fixed bottom-4 right-4 z-50 p-4 rounded-lg shadow-lg border min-w-[300px] max-w-[500px] animate-slide-in ${
            feedback.type === 'success'
              ? 'bg-green-50 dark:bg-green-900/20 border-green-300 dark:border-green-700 text-green-900 dark:text-green-200'
              : 'bg-red-50 dark:bg-red-900/20 border-red-300 dark:border-red-700 text-red-900 dark:text-red-200'
          }`}
        >
          <div className="flex justify-between items-start gap-3">
            <div className="flex-1">
              <p className="text-sm font-semibold mb-1">
                {feedback.type === 'success' ? '✓ Success' : '✕ Error'}
              </p>
              <p className="text-sm">{feedback.message}</p>
            </div>
            <button
              onClick={() => setFeedback(null)}
              className="text-lg font-bold hover:opacity-70 flex-shrink-0"
            >
              ✕
            </button>
          </div>
        </div>
      )}

      {/* Server Status */}
      <div className="bg-white dark:bg-gray-800 rounded-xl shadow-sm border border-gray-200 dark:border-gray-700 p-6">
        <div className="flex items-center justify-between">
          <div className="flex items-center gap-3">
            <div className="flex flex-col">
              <span className="text-xs text-gray-500 dark:text-gray-400 mb-1">Server URL</span>
              <code className="text-sm font-mono bg-gray-100 dark:bg-gray-900 px-3 py-2 rounded border border-gray-200 dark:border-gray-700 text-gray-900 dark:text-gray-100">
                http://{config.host}:{config.actual_port ?? config.port}/v1
              </code>
              {config.actual_port && config.actual_port !== config.port && (
                <div className="mt-2 p-2 bg-amber-50 dark:bg-amber-900/20 border border-amber-200 dark:border-amber-700 rounded text-xs text-amber-800 dark:text-amber-200">
                  Port {config.port} was already in use. Server is running on port {config.actual_port} instead.
                </div>
              )}
            </div>
            <div className="flex gap-2 self-end mb-2">
              <Button
                variant="secondary"
                onClick={() => copyToClipboard(`http://${config.host}:${config.actual_port ?? config.port}/v1`)}
                title="Copy URL"
              >
                ⎘
              </Button>
            </div>
          </div>

          <div>
            <Button
              onClick={restartServer}
              disabled={isRestarting}
              variant="secondary"
              title="Restart the server"
            >
              {isRestarting ? 'Restarting...' : '↻ Restart Server'}
            </Button>
          </div>
        </div>
      </div>

      {/* Server Configuration */}
      <div className="bg-white dark:bg-gray-800 rounded-xl shadow-sm border border-gray-200 dark:border-gray-700 p-6">
        <h2 className="text-xl font-bold text-gray-900 dark:text-gray-100 mb-6">Server Configuration</h2>

        <form onSubmit={updateConfig} className="space-y-6">
          <Select
            label="Network Interface"
            value={editConfig.host}
            onChange={(e) => setEditConfig({ ...editConfig, host: e.target.value })}
          >
            {networkInterfaces.map((iface) => (
              <option key={iface.ip} value={iface.ip}>
                {iface.name} ({iface.ip})
              </option>
            ))}
          </Select>

          <Input
            label="Port"
            type="number"
            value={editConfig.port}
            onChange={(e) => setEditConfig({ ...editConfig, port: parseInt(e.target.value) })}
            placeholder="3625"
            helperText="The port number to listen on"
          />

          <div className="p-4 bg-blue-50 dark:bg-blue-900/20 border border-blue-200 dark:border-blue-700 rounded-lg">
            <p className="text-sm text-blue-800 dark:text-blue-200">
              <strong>CORS:</strong> Cross-Origin Resource Sharing is enabled by default to allow all origins.
              This is recommended for web apps and browser tools.
            </p>
          </div>

          {hasUnsavedChanges && (
            <div className="p-4 bg-amber-50 dark:bg-amber-900/20 border border-amber-200 dark:border-amber-700 rounded-lg">
              <p className="text-sm text-amber-800 dark:text-amber-200">
                You have unsaved changes. Click "Update & Restart Server" to apply them.
              </p>
            </div>
          )}

          <div className="flex gap-2 pt-4">
            <Button type="submit" disabled={isUpdating || !hasUnsavedChanges}>
              {isUpdating ? 'Updating...' : 'Update & Restart Server'}
            </Button>
            <Button
              type="button"
              variant="secondary"
              onClick={() => setEditConfig(config)}
              disabled={!hasUnsavedChanges}
            >
              Reset
            </Button>
          </div>
        </form>
      </div>

      {/* UI Preferences */}
      <div className="bg-white dark:bg-gray-800 rounded-xl shadow-sm border border-gray-200 dark:border-gray-700 p-6">
        <h2 className="text-xl font-bold text-gray-900 dark:text-gray-100 mb-6">UI Preferences</h2>

        <form onSubmit={updateTrayGraphSettings} className="space-y-6">
          {/* Tray Graph Enable Toggle */}
          <div className="flex items-center justify-between p-4 bg-gray-50 dark:bg-gray-900/50 rounded-lg">
            <div className="flex-1">
              <label className="text-sm font-semibold text-gray-900 dark:text-gray-100 block mb-1">
                Dynamic Tray Icon Graph
              </label>
              <p className="text-xs text-gray-600 dark:text-gray-400">
                Show live token usage sparkline graph in system tray icon
              </p>
            </div>
            <label className="relative inline-flex items-center cursor-pointer ml-4">
              <input
                type="checkbox"
                checked={trayGraphSettings.enabled}
                onChange={(e) =>
                  setTrayGraphSettings({ ...trayGraphSettings, enabled: e.target.checked })
                }
                className="sr-only peer"
              />
              <div className="w-11 h-6 bg-gray-300 peer-focus:outline-none peer-focus:ring-4 peer-focus:ring-blue-300 dark:peer-focus:ring-blue-800 rounded-full peer dark:bg-gray-700 peer-checked:after:translate-x-full peer-checked:after:border-white after:content-[''] after:absolute after:top-[2px] after:left-[2px] after:bg-white after:border-gray-300 after:border after:rounded-full after:h-5 after:w-5 after:transition-all dark:border-gray-600 peer-checked:bg-blue-600"></div>
            </label>
          </div>

          {/* Refresh Rate Selector */}
          {trayGraphSettings.enabled && (
            <>
              <Select
                label="Update Interval (Time per Pixel)"
                value={trayGraphSettings.interval_secs}
                onChange={(e) =>
                  setTrayGraphSettings({
                    ...trayGraphSettings,
                    interval_secs: parseInt(e.target.value),
                  })
                }
                helperText={`Each pixel = ${trayGraphSettings.interval_secs}s | Total window = ${calculateTimeWindow(trayGraphSettings.interval_secs)}`}
              >
                <option value="1">1 second/pixel (30 second window)</option>
                <option value="2">2 seconds/pixel (1 minute window)</option>
                <option value="5">5 seconds/pixel (2.5 minute window)</option>
                <option value="10">10 seconds/pixel (5 minute window)</option>
                <option value="30">30 seconds/pixel (15 minute window)</option>
                <option value="60">60 seconds/pixel (30 minute window)</option>
              </Select>

              {/* Info Box */}
              <div className="p-4 bg-blue-50 dark:bg-blue-900/20 border border-blue-200 dark:border-blue-700 rounded-lg">
                <p className="text-sm text-blue-800 dark:text-blue-200 mb-2">
                  <strong>How it works:</strong> The graph shows 30 bars (pixels) of token usage history.
                  Each bar represents one interval period. The graph shifts left every time it updates.
                </p>
                <p className="text-sm text-blue-800 dark:text-blue-200">
                  <strong>Scaling:</strong> 1 pixel height = 5 tokens (up to 145 tokens).
                  Values above 145 tokens auto-scale to fit.
                </p>
              </div>
            </>
          )}

          {/* Save Button */}
          <div className="flex gap-2 pt-4">
            <Button type="submit" disabled={isUpdatingTrayGraph}>
              {isUpdatingTrayGraph ? 'Updating...' : 'Save UI Preferences'}
            </Button>
          </div>
        </form>
      </div>

      {/* RouteLLM Intelligent Routing Settings */}
      <div className="bg-white dark:bg-gray-800 rounded-xl shadow-sm border border-gray-200 dark:border-gray-700 p-6">
        <div className="flex items-center justify-between mb-6">
          <div>
            <h2 className="text-xl font-bold text-gray-900 dark:text-gray-100">
              RouteLLM Intelligent Routing
            </h2>
            <p className="text-sm text-gray-600 dark:text-gray-400 mt-1">
              ML-based routing to optimize costs while maintaining quality
            </p>
          </div>
          <span className="inline-flex items-center px-3 py-1 rounded-md text-xs font-medium bg-purple-100 text-purple-800 dark:bg-purple-900/30 dark:text-purple-300">
            EXPERIMENTAL
          </span>
        </div>

        <div className="space-y-6">
          {/* Status */}
          {routellmStatus && (
            <div className="p-4 bg-gray-50 dark:bg-gray-900/50 rounded-lg">
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
                      <div className="text-xs text-gray-500 dark:text-gray-400 mt-1">
                        Memory: {(routellmStatus.memory_usage_mb / 1024).toFixed(2)} GB
                      </div>
                    )}
                  </div>
                </div>
                {routellmStatus.state === 'started' && (
                  <Button variant="secondary" onClick={handleRouteLLMUnload}>
                    Unload from Memory
                  </Button>
                )}
              </div>

              {/* Model Location */}
              <div className="mt-3 pt-3 border-t border-gray-200 dark:border-gray-700">
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
            <div className="p-4 bg-blue-50 dark:bg-blue-900/20 border border-blue-200 dark:border-blue-700 rounded-lg">
              <h4 className="font-semibold text-blue-900 dark:text-blue-100 mb-2">
                🎯 Intelligent Cost Optimization
              </h4>
              <p className="text-sm text-blue-800 dark:text-blue-200 mb-3">
                RouteLLM uses machine learning to analyze each prompt and automatically route to the
                most cost-effective model while maintaining quality.
              </p>
              <div className="grid grid-cols-3 gap-2 text-xs mb-4">
                <div className="bg-white dark:bg-gray-800 p-2 rounded">
                  <div className="font-semibold text-green-600 dark:text-green-400">30-60%</div>
                  <div className="text-gray-600 dark:text-gray-400">Cost Savings</div>
                </div>
                <div className="bg-white dark:bg-gray-800 p-2 rounded">
                  <div className="font-semibold text-blue-600 dark:text-blue-400">85-95%</div>
                  <div className="text-gray-600 dark:text-gray-400">Quality Retained</div>
                </div>
                <div className="bg-white dark:bg-gray-800 p-2 rounded">
                  <div className="font-semibold text-purple-600 dark:text-purple-400">~10ms</div>
                  <div className="text-gray-600 dark:text-gray-400">Routing Time</div>
                </div>
              </div>

              <div className="p-3 bg-yellow-50 dark:bg-yellow-900/20 border border-yellow-200 dark:border-yellow-700 rounded text-sm mb-4">
                <strong>Download Required:</strong> Models (~1.08 GB) will be downloaded to{' '}
                <code className="bg-yellow-100 dark:bg-yellow-800/30 px-1 rounded">
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
            <div className="p-4 bg-blue-50 dark:bg-blue-900/20 border border-blue-200 dark:border-blue-700 rounded-lg">
              <div className="mb-2 flex justify-between text-sm">
                <span className="font-medium text-blue-900 dark:text-blue-100">
                  Downloading RouteLLM Models...
                </span>
                <span className="text-blue-700 dark:text-blue-300">
                  {downloadProgress.toFixed(0)}%
                </span>
              </div>
              <div className="w-full bg-blue-200 dark:bg-blue-800 rounded-full h-2">
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
                <h3 className="font-semibold text-gray-900 dark:text-gray-100">
                  Memory Management
                </h3>

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
              <div className="p-4 bg-orange-50 dark:bg-orange-900/20 border border-orange-200 dark:border-orange-700 rounded-lg">
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
              <div className="p-4 bg-gray-50 dark:bg-gray-900/50 border border-gray-200 dark:border-gray-700 rounded-lg">
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
    </div>
  )
}
