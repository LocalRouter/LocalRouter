import { useState, useEffect, useRef } from 'react'
import { invoke } from '@tauri-apps/api/core'
import 'rapidoc'
import Button from '../ui/Button'
import Select from '../ui/Select'

// Declare RapiDoc web component for TypeScript
declare global {
  namespace JSX {
    interface IntrinsicElements {
      'rapi-doc': any
    }
  }
}

interface ServerConfig {
  host: string
  port: number
  actual_port?: number
  enable_cors: boolean
}

interface ApiKey {
  id: string
  name: string
  key: string
  enabled: boolean
}

export default function DocumentationTab() {
  const [spec, setSpec] = useState<string>('')
  const [config, setConfig] = useState<ServerConfig | null>(null)
  const [isLoading, setIsLoading] = useState(true)
  const [error, setError] = useState<string | null>(null)
  const [apiKeys, setApiKeys] = useState<ApiKey[]>([])
  const [selectedKeyId, setSelectedKeyId] = useState<string>('')
  const [selectedKey, setSelectedKey] = useState<string>('')
  const [feedback, setFeedback] = useState<{ type: 'success' | 'error'; message: string } | null>(null)
  const rapiDocRef = useRef<any>(null)

  // Auto-dismiss feedback after 5 seconds
  useEffect(() => {
    if (feedback) {
      const timer = setTimeout(() => setFeedback(null), 5000)
      return () => clearTimeout(timer)
    }
  }, [feedback])

  useEffect(() => {
    loadServerConfig()
    loadOpenAPISpec()
    loadApiKeys()
  }, [])

  // Auto-select first API key
  useEffect(() => {
    if (apiKeys.length > 0 && !selectedKeyId) {
      const firstKey = apiKeys[0]
      setSelectedKeyId(firstKey.id)
      setSelectedKey(firstKey.key)
    }
  }, [apiKeys, selectedKeyId])

  // Update RapiDoc spec when it changes
  useEffect(() => {
    if (rapiDocRef.current && spec) {
      rapiDocRef.current.loadSpec(JSON.parse(spec))
    }
  }, [spec])

  const loadServerConfig = async () => {
    try {
      const serverConfig = await invoke<ServerConfig>('get_server_config')
      setConfig(serverConfig)
    } catch (err: any) {
      console.error('Failed to load server config:', err)
      setError(`Failed to load server config: ${err.message || err}`)
    }
  }

  const loadApiKeys = async () => {
    try {
      const keys = await invoke<ApiKey[]>('list_api_keys')
      setApiKeys(keys.filter((k) => k.enabled))
    } catch (err: any) {
      console.error('Failed to load API keys:', err)
    }
  }

  const loadOpenAPISpec = async () => {
    try {
      setIsLoading(true)
      setError(null)
      const openApiSpec = await invoke<string>('get_openapi_spec')
      setSpec(openApiSpec)
      setFeedback({ type: 'success', message: 'OpenAPI specification loaded successfully!' })
    } catch (err: any) {
      console.error('Failed to load OpenAPI spec:', err)
      setError(`Failed to load OpenAPI spec: ${err.message || err}`)
      setFeedback({ type: 'error', message: `Failed to load spec: ${err.message || err}` })
    } finally {
      setIsLoading(false)
    }
  }

  const handleKeyChange = (event: React.ChangeEvent<HTMLSelectElement>) => {
    const keyId = event.target.value
    const key = apiKeys.find((k) => k.id === keyId)
    if (key) {
      setSelectedKeyId(keyId)
      setSelectedKey(key.key)
    }
  }

  const downloadSpec = (format: 'json' | 'yaml') => {
    try {
      const blob = new Blob([spec], {
        type: format === 'json' ? 'application/json' : 'application/yaml',
      })
      const url = URL.createObjectURL(blob)
      const a = document.createElement('a')
      a.href = url
      a.download = `localrouter-openapi.${format}`
      document.body.appendChild(a)
      a.click()
      document.body.removeChild(a)
      URL.revokeObjectURL(url)
      setFeedback({ type: 'success', message: `Downloaded OpenAPI spec as ${format.toUpperCase()}!` })
    } catch (err: any) {
      console.error('Failed to download spec:', err)
      setFeedback({ type: 'error', message: `Failed to download: ${err.message || err}` })
    }
  }

  const copyToClipboard = (text: string, label: string) => {
    navigator.clipboard.writeText(text)
    setFeedback({ type: 'success', message: `${label} copied to clipboard!` })
  }

  const exportPostman = () => {
    try {
      const specObj = JSON.parse(spec)
      const port = config?.actual_port ?? config?.port ?? 3625
      const baseUrl = `http://${config?.host ?? '127.0.0.1'}:${port}`

      // Create basic Postman collection from OpenAPI spec
      const postmanCollection = {
        info: {
          name: specObj.info.title || 'LocalRouter AI API',
          description: specObj.info.description || '',
          schema: 'https://schema.getpostman.com/json/collection/v2.1.0/collection.json',
        },
        auth: {
          type: 'bearer',
          bearer: [
            {
              key: 'token',
              value: selectedKey || '{{api_key}}',
              type: 'string',
            },
          ],
        },
        item: Object.entries(specObj.paths || {}).flatMap(([path, methods]: [string, any]) => {
          return Object.entries(methods)
            .filter(([method]) => ['get', 'post', 'put', 'delete', 'patch'].includes(method))
            .map(([method, operation]: [string, any]) => ({
              name: operation.summary || `${method.toUpperCase()} ${path}`,
              request: {
                method: method.toUpperCase(),
                header: [
                  {
                    key: 'Content-Type',
                    value: 'application/json',
                  },
                ],
                url: {
                  raw: `${baseUrl}${path}`,
                  host: [config?.host || '127.0.0.1'],
                  port: `${port}`,
                  path: path.split('/').filter(Boolean),
                },
                description: operation.description || '',
                body: method !== 'get' && operation.requestBody ? {
                  mode: 'raw',
                  raw: JSON.stringify(
                    operation.requestBody.content?.['application/json']?.schema?.example || {},
                    null,
                    2
                  ),
                } : undefined,
              },
            }))
        }),
      }

      const blob = new Blob([JSON.stringify(postmanCollection, null, 2)], {
        type: 'application/json',
      })
      const url = URL.createObjectURL(blob)
      const a = document.createElement('a')
      a.href = url
      a.download = 'localrouter-postman-collection.json'
      document.body.appendChild(a)
      a.click()
      document.body.removeChild(a)
      URL.revokeObjectURL(url)
      setFeedback({
        type: 'success',
        message: 'Postman collection exported successfully!',
      })
    } catch (err: any) {
      console.error('Failed to export Postman collection:', err)
      setFeedback({
        type: 'error',
        message: `Failed to export: ${err.message || err}`,
      })
    }
  }

  const copyCurlExample = () => {
    const port = config?.actual_port ?? config?.port ?? 3625
    const baseUrl = `http://${config?.host ?? '127.0.0.1'}:${port}`
    const curlCommand = `curl -X POST ${baseUrl}/v1/chat/completions \\
  -H "Authorization: Bearer ${selectedKey || 'YOUR_API_KEY'}" \\
  -H "Content-Type: application/json" \\
  -d '{
    "model": "gpt-4",
    "messages": [
      {"role": "user", "content": "Hello!"}
    ]
  }'`

    copyToClipboard(curlCommand, 'cURL example')
  }

  if (isLoading) {
    return (
      <div className="flex items-center justify-center h-full">
        <div className="text-center space-y-4">
          <div className="text-6xl animate-spin">⚙️</div>
          <p className="text-gray-600">Loading OpenAPI specification...</p>
        </div>
      </div>
    )
  }

  if (error || !spec) {
    return (
      <div className="flex items-center justify-center h-full">
        <div className="text-center space-y-4 max-w-md">
          <div className="text-6xl">⚠️</div>
          <p className="text-red-600 font-semibold">{error || 'Failed to load OpenAPI specification'}</p>
          <Button onClick={loadOpenAPISpec}>Retry</Button>
        </div>
      </div>
    )
  }

  const port = config?.actual_port ?? config?.port ?? 3625
  const baseUrl = `http://${config?.host ?? '127.0.0.1'}:${port}`

  return (
    <div className="h-full flex flex-col relative">
      {/* Toast Notification */}
      {feedback && (
        <div
          className={`fixed bottom-4 right-4 z-50 p-4 rounded-lg shadow-lg border min-w-[300px] max-w-[500px] animate-slide-in ${
            feedback.type === 'success'
              ? 'bg-green-50 border-green-300 text-green-900'
              : 'bg-red-50 border-red-300 text-red-900'
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

      {/* Header */}
      <div className="p-4 border-b bg-white flex-shrink-0">
        <div className="flex justify-between items-center gap-4">
          {/* Left: Title and Info */}
          <div className="flex items-center gap-4">
            <h2 className="text-xl font-bold text-gray-900">API Documentation</h2>
            <code className="text-sm font-mono bg-gray-100 px-3 py-2 rounded border border-gray-200">
              {baseUrl}
            </code>
            <Button
              variant="secondary"
              onClick={() => copyToClipboard(baseUrl, 'Server URL')}
              title="Copy server URL"
            >
              ⎘
            </Button>
          </div>

          {/* Right: Actions */}
          <div className="flex items-center gap-2">
            {/* API Key Selector */}
            {apiKeys.length > 0 && (
              <div className="min-w-[200px]">
                <Select
                  label="Test with API Key"
                  value={selectedKeyId}
                  onChange={handleKeyChange}
                >
                  {apiKeys.length === 0 ? (
                    <option value="">No API keys available</option>
                  ) : (
                    apiKeys.map((key) => (
                      <option key={key.id} value={key.id}>
                        {key.name} ({key.key.slice(0, 8)}...)
                      </option>
                    ))
                  )}
                </Select>
              </div>
            )}

            <Button variant="secondary" onClick={() => downloadSpec('json')} title="Download OpenAPI spec as JSON">
              JSON ⬇
            </Button>
            <Button variant="secondary" onClick={() => downloadSpec('yaml')} title="Download OpenAPI spec as YAML">
              YAML ⬇
            </Button>
            <Button variant="secondary" onClick={exportPostman} title="Export as Postman collection">
              Postman ⬇
            </Button>
            <Button variant="secondary" onClick={copyCurlExample} title="Copy example cURL command">
              cURL ⎘
            </Button>
            <Button variant="secondary" onClick={loadOpenAPISpec} title="Refresh specification">
              ↻
            </Button>
          </div>
        </div>

        {/* API Key Info */}
        {selectedKey && (
          <div className="mt-3 p-3 bg-blue-50 border border-blue-200 rounded-lg">
            <p className="text-sm text-blue-800">
              <span className="font-semibold">Try It Out:</span> Use the "Try It Out" feature below to test endpoints.
              Your selected API key will be used for authentication.
            </p>
          </div>
        )}
      </div>

      {/* RapiDoc API Reference */}
      <div className="flex-1 overflow-auto">
        <rapi-doc
          ref={rapiDocRef}
          theme="light"
          bg-color="#ffffff"
          text-color="#1f2937"
          primary-color="#3b82f6"
          render-style="view"
          layout="row"
          show-header="false"
          show-info="true"
          allow-authentication="true"
          allow-server-selection="false"
          allow-api-list-style-selection="false"
          api-key-name="Authorization"
          api-key-value={selectedKey ? `Bearer ${selectedKey}` : ''}
          api-key-location="header"
          server-url={baseUrl}
          style={{ width: '100%', height: '100%' }}
        />
      </div>
    </div>
  )
}
