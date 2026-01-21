import { useState, useEffect } from 'react'
import { invoke } from '@tauri-apps/api/core'
import { listen } from '@tauri-apps/api/event'
import { check, Update } from '@tauri-apps/plugin-updater'
import { relaunch } from '@tauri-apps/plugin-process'
import Button from '../ui/Button'
import Select from '../ui/Select'
import ReactMarkdown from 'react-markdown'

interface UpdateConfig {
  mode: 'manual' | 'automatic'
  check_interval_days: number
  last_check?: string
  skipped_version?: string
}

export default function UpdatesSubtab() {
  const [currentVersion, setCurrentVersion] = useState<string>('')
  const [updateConfig, setUpdateConfig] = useState<UpdateConfig>({
    mode: 'automatic',
    check_interval_days: 7,
  })
  const [isChecking, setIsChecking] = useState(false)
  const [checkError, setCheckError] = useState<string | null>(null)
  const [updateAvailable, setUpdateAvailable] = useState<Update | null>(null)
  const [isDownloading, setIsDownloading] = useState(false)
  const [downloadProgress, setDownloadProgress] = useState(0)
  const [feedback, setFeedback] = useState<{ type: 'success' | 'error'; message: string } | null>(null)

  useEffect(() => {
    loadCurrentVersion()
    loadUpdateConfig()

    // Auto-check on mount if automatic mode is enabled
    checkForUpdatesOnMount()

    // Listen for background update check events
    const unlistenUpdateCheck = listen('check-for-updates', () => {
      handleCheckForUpdates()
    })

    return () => {
      unlistenUpdateCheck.then(fn => fn())
    }
  }, [])

  // Auto-dismiss feedback after 5 seconds
  useEffect(() => {
    if (feedback) {
      const timer = setTimeout(() => setFeedback(null), 5000)
      return () => clearTimeout(timer)
    }
  }, [feedback])

  const loadCurrentVersion = async () => {
    try {
      const version = await invoke<string>('get_app_version')
      setCurrentVersion(version)
    } catch (err) {
      console.error('Failed to get app version:', err)
    }
  }

  const loadUpdateConfig = async () => {
    try {
      const config = await invoke<UpdateConfig>('get_update_config')
      setUpdateConfig(config)
    } catch (err) {
      console.error('Failed to load update config:', err)
    }
  }

  const checkForUpdatesOnMount = async () => {
    try {
      const config = await invoke<UpdateConfig>('get_update_config')
      if (config.mode === 'automatic') {
        // Trigger check after a short delay to avoid blocking UI
        setTimeout(() => {
          handleCheckForUpdates()
        }, 500)
      }
    } catch (err) {
      console.error('Failed to check update config:', err)
    }
  }

  const handleCheckForUpdates = async () => {
    setIsChecking(true)
    setCheckError(null)
    setFeedback(null)

    try {
      const update = await check()

      // Mark check as performed (save timestamp)
      await invoke('mark_update_check_performed')

      if (update?.available) {
        // Check if this version was skipped
        if (updateConfig.skipped_version === update.version) {
          setFeedback({ type: 'success', message: 'Already up to date (skipped version ignored)' })
          setUpdateAvailable(null)
          // Clear tray notification
          await invoke('set_update_notification', { available: false })
        } else {
          setUpdateAvailable(update)
          setFeedback({ type: 'success', message: `New version ${update.version} available!` })
          // Show tray notification
          await invoke('set_update_notification', { available: true })
        }
      } else {
        setUpdateAvailable(null)
        setFeedback({ type: 'success', message: 'Already up to date' })
        // Clear tray notification
        await invoke('set_update_notification', { available: false })
      }
    } catch (err: any) {
      setCheckError(err.message || 'Failed to check for updates')
      setFeedback({ type: 'error', message: `Check failed: ${err.message}` })
    } finally {
      setIsChecking(false)
    }
  }

  const handleUpdateNow = async () => {
    if (!updateAvailable) return

    setIsDownloading(true)
    setFeedback(null)

    try {
      await updateAvailable.downloadAndInstall((event) => {
        switch (event.event) {
          case 'Started':
            setDownloadProgress(0)
            break
          case 'Progress':
            // Estimate progress based on chunk downloads
            setDownloadProgress((prev) => Math.min(prev + 5, 95))
            break
          case 'Finished':
            setDownloadProgress(100)
            break
        }
      })

      setFeedback({ type: 'success', message: 'Update installed! Restarting...' })

      // Clear tray notification
      await invoke('set_update_notification', { available: false })

      // Restart after a brief delay
      setTimeout(async () => {
        await relaunch()
      }, 2000)
    } catch (err: any) {
      setFeedback({ type: 'error', message: `Update failed: ${err.message}` })
      setIsDownloading(false)
    }
  }

  const handleSkipVersion = async () => {
    if (!updateAvailable) return

    try {
      await invoke('skip_update_version', { version: updateAvailable.version })
      setUpdateAvailable(null)
      setFeedback({ type: 'success', message: `Skipped version ${updateAvailable.version}` })

      // Reload config to reflect the skip
      loadUpdateConfig()
    } catch (err: any) {
      setFeedback({ type: 'error', message: `Failed to skip version: ${err.message}` })
    }
  }

  const handleUpdateConfig = async (updates: Partial<UpdateConfig>) => {
    const newConfig = { ...updateConfig, ...updates }

    try {
      await invoke('update_update_config', {
        mode: newConfig.mode,
        checkIntervalDays: newConfig.check_interval_days,
      })
      setUpdateConfig(newConfig)
      setFeedback({ type: 'success', message: 'Settings saved' })
    } catch (err: any) {
      setFeedback({ type: 'error', message: `Failed to save settings: ${err.message}` })
    }
  }

  const formatLastCheck = (lastCheck?: string | null) => {
    if (!lastCheck) return 'Never'

    try {
      const date = new Date(lastCheck)
      const now = new Date()
      const diffMs = now.getTime() - date.getTime()
      const diffDays = Math.floor(diffMs / (1000 * 60 * 60 * 24))

      if (diffDays === 0) return 'Today'
      if (diffDays === 1) return '1 day ago'
      if (diffDays < 7) return `${diffDays} days ago`
      if (diffDays < 30) {
        const weeks = Math.floor(diffDays / 7)
        return weeks === 1 ? '1 week ago' : `${weeks} weeks ago`
      }
      const months = Math.floor(diffDays / 30)
      return months === 1 ? '1 month ago' : `${months} months ago`
    } catch {
      return 'Never'
    }
  }

  return (
    <div className="p-6 max-w-4xl">
      <h2 className="text-xl font-semibold mb-6 text-gray-900 dark:text-gray-100">App Updates</h2>

      {/* Feedback Messages */}
      {feedback && (
        <div
          className={`mb-4 p-3 rounded ${
            feedback.type === 'success'
              ? 'bg-green-500/20 text-green-600 dark:text-green-400'
              : 'bg-red-500/20 text-red-600 dark:text-red-400'
          }`}
        >
          {feedback.message}
        </div>
      )}

      {/* Version Info */}
      <div className="bg-gray-100 dark:bg-gray-800 rounded-lg p-4 mb-6">
        <div className="grid grid-cols-2 gap-4">
          <div>
            <div className="text-sm text-gray-600 dark:text-gray-400 mb-1">Current Version</div>
            <div className="text-lg font-semibold text-gray-900 dark:text-gray-100">{currentVersion || 'Loading...'}</div>
          </div>
          {updateAvailable && (
            <div>
              <div className="text-sm text-gray-600 dark:text-gray-400 mb-1">Latest Version</div>
              <div className="text-lg font-semibold text-blue-600 dark:text-blue-400">{updateAvailable.version}</div>
            </div>
          )}
        </div>
      </div>

      {/* Update Settings */}
      <div className="bg-gray-100 dark:bg-gray-800 rounded-lg p-4 mb-6">
        <h3 className="font-semibold mb-4 text-gray-900 dark:text-gray-100">Update Settings</h3>

        <label className="flex items-center mb-4 cursor-pointer">
          <input
            type="checkbox"
            checked={updateConfig.mode === 'automatic'}
            onChange={(e) => handleUpdateConfig({ mode: e.target.checked ? 'automatic' : 'manual' })}
            className="mr-3 w-4 h-4 rounded"
          />
          <span className="text-sm text-gray-900 dark:text-gray-100">Automatically check for updates</span>
        </label>

        {updateConfig.mode === 'automatic' && (
          <div className="ml-7 mb-4">
            <label className="text-sm text-gray-600 dark:text-gray-400 mb-2 block">Check every:</label>
            <Select
              value={updateConfig.check_interval_days.toString()}
              onChange={(e) => handleUpdateConfig({ check_interval_days: parseInt(e.target.value, 10) })}
              className="w-48"
            >
              <option value="1">1 day</option>
              <option value="7">7 days (recommended)</option>
              <option value="14">14 days</option>
              <option value="30">30 days</option>
            </Select>
          </div>
        )}

        <div className="text-sm text-gray-600 dark:text-gray-400">
          Last checked: {formatLastCheck(updateConfig.last_check)}
        </div>
      </div>

      {/* Check Now Button */}
      <div className="mb-6">
        <Button
          onClick={handleCheckForUpdates}
          disabled={isChecking || isDownloading}
          className="bg-blue-600 hover:bg-blue-700 dark:bg-blue-600 dark:hover:bg-blue-700"
        >
          {isChecking ? 'Checking...' : 'Check Now'}
        </Button>
      </div>

      {/* Update Available Section */}
      {updateAvailable && !isDownloading && (
        <div className="bg-blue-500/10 dark:bg-blue-500/20 border border-blue-500/30 dark:border-blue-500/40 rounded-lg p-4 mb-6">
          <h3 className="font-semibold text-blue-600 dark:text-blue-400 mb-2">Update Available: {updateAvailable.version}</h3>
          {updateAvailable.date && (
            <div className="text-sm text-gray-700 dark:text-gray-300 mb-4">Published: {new Date(updateAvailable.date).toLocaleDateString()}</div>
          )}

          {updateAvailable.body && (
            <div className="bg-gray-200 dark:bg-gray-900 rounded p-4 mb-4 max-h-64 overflow-y-auto">
              <div className="text-sm prose dark:prose-invert max-w-none">
                <ReactMarkdown>{updateAvailable.body}</ReactMarkdown>
              </div>
            </div>
          )}

          <div className="flex gap-3">
            <Button
              onClick={handleUpdateNow}
              className="bg-blue-600 hover:bg-blue-700 dark:bg-blue-600 dark:hover:bg-blue-700"
            >
              Update Now
            </Button>
            <Button
              onClick={handleSkipVersion}
              className="bg-gray-600 hover:bg-gray-700 dark:bg-gray-700 dark:hover:bg-gray-600"
            >
              Skip This Version
            </Button>
          </div>
        </div>
      )}

      {/* Download Progress */}
      {isDownloading && (
        <div className="bg-gray-100 dark:bg-gray-800 rounded-lg p-4">
          <h3 className="font-semibold mb-4 text-gray-900 dark:text-gray-100">Installing Update...</h3>
          <div className="w-full bg-gray-300 dark:bg-gray-700 rounded-full h-2 mb-2">
            <div
              className="bg-blue-600 dark:bg-blue-500 h-2 rounded-full transition-all duration-300"
              style={{ width: `${downloadProgress}%` }}
            />
          </div>
          <div className="text-sm text-gray-600 dark:text-gray-400">{Math.round(downloadProgress)}%</div>
        </div>
      )}

      {/* Error Display */}
      {checkError && !feedback && (
        <div className="bg-red-500/20 border border-red-500/30 rounded-lg p-4 text-red-600 dark:text-red-400">
          <strong>Error:</strong> {checkError}
        </div>
      )}
    </div>
  )
}
