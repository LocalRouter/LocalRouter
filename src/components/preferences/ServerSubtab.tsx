import { useState, useEffect } from 'react'
import { invoke } from '@tauri-apps/api/core'
import Button from '../ui/Button'
import Select from '../ui/Select'
import Input from '../ui/Input'

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

export default function ServerSubtab() {
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

  return (
    <div className="p-6 max-w-4xl">
      <h2 className="text-xl font-semibold mb-6 text-gray-900 dark:text-gray-100">Server Configuration</h2>

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

      {/* Server Status */}
      <div className="bg-gray-50 dark:bg-gray-800 rounded-lg p-6 mb-6">
        <h3 className="text-lg font-semibold mb-4 text-gray-900 dark:text-gray-100">Server Status</h3>

        <div className="flex items-center justify-between">
          <div className="flex items-center gap-3">
            <div className="flex flex-col">
              <span className="text-xs text-gray-500 dark:text-gray-400 mb-1">Server URL</span>
              <code className="text-sm font-mono bg-white dark:bg-gray-900 px-3 py-2 rounded border border-gray-300 dark:border-gray-700 text-gray-900 dark:text-gray-100">
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

      {/* Server Configuration Form */}
      <div className="bg-gray-50 dark:bg-gray-800 rounded-lg p-6">
        <h3 className="text-lg font-semibold mb-4 text-gray-900 dark:text-gray-100">Network Settings</h3>

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
    </div>
  )
}
