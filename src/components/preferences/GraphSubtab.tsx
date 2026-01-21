import { useState, useEffect } from 'react'
import { invoke } from '@tauri-apps/api/core'
import Button from '../ui/Button'
import Select from '../ui/Select'

interface TrayGraphSettings {
  enabled: boolean
  refresh_rate_secs: number
}

export default function GraphSubtab() {
  const [trayGraphSettings, setTrayGraphSettings] = useState<TrayGraphSettings>({
    enabled: false,
    refresh_rate_secs: 60,
  })
  const [isUpdatingTrayGraph, setIsUpdatingTrayGraph] = useState(false)
  const [feedback, setFeedback] = useState<{ type: 'success' | 'error'; message: string } | null>(null)

  // Auto-dismiss feedback after 5 seconds
  useEffect(() => {
    if (feedback) {
      const timer = setTimeout(() => setFeedback(null), 5000)
      return () => clearTimeout(timer)
    }
  }, [feedback])

  useEffect(() => {
    loadTrayGraphSettings()
  }, [])

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
        refreshRateSecs: trayGraphSettings.refresh_rate_secs,
      })

      setFeedback({ type: 'success', message: 'Tray graph settings updated successfully!' })
    } catch (error: any) {
      console.error('Failed to update tray graph settings:', error)
      setFeedback({ type: 'error', message: `Error: ${error.message || error}` })
    } finally {
      setIsUpdatingTrayGraph(false)
    }
  }

  return (
    <div className="p-6 max-w-4xl">
      <h2 className="text-xl font-semibold mb-6 text-gray-900 dark:text-gray-100">Tray Icon Graph</h2>

      {/* Feedback Messages */}
      {feedback && (
        <div
          className={`mb-4 p-3 rounded ${
            feedback.type === 'success'
              ? 'bg-green-50 dark:bg-green-500/20 text-green-700 dark:text-green-400'
              : 'bg-red-50 dark:bg-red-500/20 text-red-700 dark:text-red-400'
          }`}
        >
          {feedback.message}
        </div>
      )}

      {/* Tray Icon Settings */}
      <div className="bg-white dark:bg-gray-800 rounded-lg p-6 mb-6 border border-gray-200 dark:border-gray-700">
        <h3 className="text-lg font-semibold mb-4 text-gray-900 dark:text-gray-100">System Tray Icon</h3>

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
                label="Refresh Rate"
                value={trayGraphSettings.refresh_rate_secs}
                onChange={(e) =>
                  setTrayGraphSettings({
                    ...trayGraphSettings,
                    refresh_rate_secs: parseInt(e.target.value),
                  })
                }
                helperText={
                  trayGraphSettings.refresh_rate_secs === 1
                    ? '1 sec/bar × 26 bars = 26 second history (starts fresh)'
                    : trayGraphSettings.refresh_rate_secs === 10
                    ? '10 sec/bar × 26 bars = ~4.3 minute history (interpolated on load)'
                    : '1 min/bar × 26 bars = 26 minute history'
                }
              >
                <option value="1">Fast (1 second per bar)</option>
                <option value="10">Medium (10 seconds per bar)</option>
                <option value="60">Slow (1 minute per bar) - Default</option>
              </Select>

              {/* Info Box */}
              <div className="p-4 bg-blue-50 dark:bg-blue-900/20 border border-blue-200 dark:border-blue-700 rounded-lg">
                <p className="text-sm text-blue-900 dark:text-blue-200 mb-2">
                  <strong>Fast (1s):</strong> Shows 26 seconds of real-time activity. Graph starts empty and builds up.
                </p>
                <p className="text-sm text-blue-900 dark:text-blue-200 mb-2">
                  <strong>Medium (10s):</strong> Shows ~4.3 minutes of history. Interpolates minute data on initial load, then tracks in real-time.
                </p>
                <p className="text-sm text-blue-900 dark:text-blue-200">
                  <strong>Slow (1min):</strong> Shows 26 minutes of history. Each bar represents one minute of activity.
                </p>
              </div>
            </>
          )}

          {/* Save Button */}
          <div className="flex gap-2 pt-4">
            <Button type="submit" disabled={isUpdatingTrayGraph}>
              {isUpdatingTrayGraph ? 'Saving...' : 'Save Changes'}
            </Button>
          </div>
        </form>
      </div>
    </div>
  )
}
