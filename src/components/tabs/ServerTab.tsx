import { useState, useEffect, useRef } from 'react'
import { invoke } from '@tauri-apps/api/core'
import Button from '../ui/Button'
import Select from '../ui/Select'
import Input from '../ui/Input'
import Modal from '../ui/Modal'
import OpenAI from 'openai'
import ReactMarkdown from 'react-markdown'
import remarkGfm from 'remark-gfm'

interface ServerConfig {
  host: string
  port: number
  actual_port?: number
  enable_cors: boolean
}

interface ApiKey {
  id: string
  name: string
  enabled: boolean
}

interface Model {
  id: string
  name: string
  provider: string
}

interface ChatMessage {
  role: 'user' | 'assistant'
  content: string
}

interface NetworkInterface {
  name: string
  ip: string
  is_loopback: boolean
}

export default function ServerTab() {
  const [config, setConfig] = useState<ServerConfig>({
    host: '127.0.0.1',
    port: 3625,
    enable_cors: true,
  })
  const [isEditModalOpen, setIsEditModalOpen] = useState(false)
  const [editConfig, setEditConfig] = useState<ServerConfig>(config)
  const [isUpdating, setIsUpdating] = useState(false)
  const [apiKeys, setApiKeys] = useState<ApiKey[]>([])
  const [models, setModels] = useState<Model[]>([])
  const [selectedKeyId, setSelectedKeyId] = useState('')
  const [selectedModel, setSelectedModel] = useState('')
  const [apiKey, setApiKey] = useState('')
  const [isLoadingModels, setIsLoadingModels] = useState(false)
  const [messages, setMessages] = useState<ChatMessage[]>([])
  const [input, setInput] = useState('')
  const [isLoading, setIsLoading] = useState(false)
  const [error, setError] = useState<string | null>(null)
  const [feedback, setFeedback] = useState<{ type: 'success' | 'error'; message: string } | null>(null)
  const [networkInterfaces, setNetworkInterfaces] = useState<NetworkInterface[]>([])
  const messagesEndRef = useRef<HTMLDivElement>(null)
  const [client, setClient] = useState<OpenAI | null>(null)
  const streamingContentRef = useRef<string>('')

  // Auto-dismiss feedback after 5 seconds
  useEffect(() => {
    if (feedback) {
      const timer = setTimeout(() => setFeedback(null), 5000)
      return () => clearTimeout(timer)
    }
  }, [feedback])

  useEffect(() => {
    loadConfig()
    loadApiKeys()
    loadNetworkInterfaces()
  }, [])

  useEffect(() => {
    messagesEndRef.current?.scrollIntoView({ behavior: 'smooth' })
  }, [messages])

  useEffect(() => {
    if (apiKey && config.host && config.port) {
      // Use actual_port if available (server might be running on a different port if configured port was taken)
      const port = config.actual_port ?? config.port
      const newClient = new OpenAI({
        apiKey: apiKey,
        baseURL: `http://${config.host}:${port}/v1`,
        dangerouslyAllowBrowser: true,
      })
      setClient(newClient)
    }
  }, [apiKey, config.host, config.port, config.actual_port])

  // Auto-select first API key and auto-fetch models
  useEffect(() => {
    if (apiKeys.length > 0 && !selectedKeyId) {
      const firstKey = apiKeys[0]
      setSelectedKeyId(firstKey.id)
      // Auto-fetch models for the first key
      loadModelsForKey(firstKey.id)
    }
  }, [apiKeys])

  // Auto-select first model when models are loaded
  useEffect(() => {
    if (models.length > 0 && !selectedModel) {
      setSelectedModel(models[0].id)
    }
  }, [models])

  const loadConfig = async () => {
    try {
      const serverConfig = await invoke<ServerConfig>('get_server_config')
      setConfig(serverConfig)
      setEditConfig(serverConfig)
    } catch (error) {
      console.error('Failed to load server config:', error)
    }
  }

  const loadApiKeys = async () => {
    try {
      const keys = await invoke<ApiKey[]>('list_api_keys')
      setApiKeys(keys.filter((k) => k.enabled))
    } catch (error) {
      console.error('Failed to load API keys:', error)
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

  const loadModelsForKey = async (keyId: string) => {
    if (!keyId) {
      return
    }

    setIsLoadingModels(true)

    try {
      // Get the actual API key value
      const keyValue = await invoke<string>('get_api_key_value', {
        id: keyId,
      })
      setApiKey(keyValue)

      // Fetch models from the server
      const url = `http://${config.host}:${config.port}/v1/models`

      const response = await fetch(url, {
        headers: {
          Authorization: `Bearer ${keyValue}`,
        },
      })

      if (!response.ok) {
        const errorText = await response.text()
        throw new Error(`Failed to fetch models: ${response.status} ${response.statusText}\n${errorText}`)
      }

      const data = await response.json()

      const modelList = data.data.map((m: any) => ({
        id: m.id,
        name: m.id,
        provider: m.id.split('/')[0] || 'unknown',
      }))
      setModels(modelList)

      setFeedback({ type: 'success', message: `Successfully loaded ${modelList.length} models!` })
    } catch (error: any) {
      console.error('Failed to load models:', error)
      setFeedback({ type: 'error', message: `Error loading models: ${error.message || error}` })
    } finally {
      setIsLoadingModels(false)
    }
  }

  const handleApiKeyChange = (newKeyId: string) => {
    setSelectedKeyId(newKeyId)
    setSelectedModel('') // Reset model selection
    setModels([]) // Clear models
    if (newKeyId) {
      loadModelsForKey(newKeyId)
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
        enableCors: editConfig.enable_cors,
      })

      await invoke('restart_server')
      setConfig(editConfig)
      setIsEditModalOpen(false)
      setFeedback({ type: 'success', message: 'Server configuration updated and restarted successfully!' })
    } catch (error: any) {
      console.error('Failed to update server config:', error)
      setFeedback({ type: 'error', message: `Error: ${error.message || error}` })
    } finally {
      setIsUpdating(false)
    }
  }

  const sendMessage = async (e: React.FormEvent) => {
    e.preventDefault()

    if (!input.trim() || !client || !selectedModel) {
      return
    }

    const userMessage: ChatMessage = {
      role: 'user',
      content: input.trim(),
    }

    setMessages((prev) => [...prev, userMessage])
    setInput('')
    setIsLoading(true)
    setError(null)

    try {
      const stream = await client.chat.completions.create({
        model: selectedModel,
        messages: [...messages, userMessage].map((m) => ({
          role: m.role,
          content: m.content,
        })),
        stream: true,
      })

      // Reset the streaming content ref for this new message
      streamingContentRef.current = ''

      // Add empty assistant message that will be filled as stream arrives
      setMessages((prev) => [...prev, { role: 'assistant', content: '' }])

      // Process streaming chunks
      for await (const chunk of stream) {
        const content = chunk.choices[0]?.delta?.content || ''
        if (content) {
          // Accumulate in ref (single source of truth)
          streamingContentRef.current += content

          // Update state with the accumulated content from ref
          // React may call this function multiple times, but it will always
          // set to the same value from the ref, preventing duplication
          setMessages((prev) => {
            const newMessages = [...prev]
            newMessages[newMessages.length - 1].content = streamingContentRef.current
            return newMessages
          })
        }
      }
    } catch (err: any) {
      console.error('Chat error:', err)
      setError(err.message || 'Failed to send message')
      // Reset ref and remove the empty assistant message if there was an error
      streamingContentRef.current = ''
      setMessages((prev) => {
        const newMessages = [...prev]
        if (newMessages[newMessages.length - 1]?.role === 'assistant' && !newMessages[newMessages.length - 1]?.content) {
          newMessages.pop()
        }
        return newMessages
      })
    } finally {
      setIsLoading(false)
    }
  }

  const clearChat = () => {
    setMessages([])
    setError(null)
    streamingContentRef.current = ''
  }

  const copyToClipboard = (text: string) => {
    navigator.clipboard.writeText(text)
    setFeedback({ type: 'success', message: 'Copied to clipboard!' })
  }

  const canChat = selectedKeyId && selectedModel

  return (
    <div className="space-y-6 relative">
      {/* Toast Notification - Fixed position at bottom-right */}
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

      {/* Top Bar */}
      <div className="bg-white rounded-xl shadow-sm border border-gray-200 p-6">
        <div className="flex items-center justify-between gap-6">
          {/* Left: Server Info */}
          <div className="flex items-center gap-3">
            <div className="flex flex-col">
              <span className="text-xs text-gray-500 mb-1">Server URL</span>
              <code className="text-sm font-mono bg-gray-100 px-3 py-2 rounded border border-gray-200">
                http://{config.host}:{config.actual_port ?? config.port}/v1
              </code>
              {config.actual_port && config.actual_port !== config.port && (
                <span className="text-xs text-amber-600 mt-1">
                  (configured: {config.port}, using: {config.actual_port})
                </span>
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
              <Button
                variant="secondary"
                onClick={() => setIsEditModalOpen(true)}
                title="Edit configuration"
              >
                ⚙
              </Button>
            </div>
          </div>

          {/* Right: API Key and Model Selectors */}
          <div className="flex items-end gap-3">
            <div className="min-w-[200px]">
              <Select
                label="API Key"
                value={selectedKeyId}
                onChange={(e) => handleApiKeyChange(e.target.value)}
              >
                <option value="">Select an API key...</option>
                {apiKeys.map((key) => (
                  <option key={key.id} value={key.id}>
                    {key.name}
                  </option>
                ))}
              </Select>
            </div>
            <div className="min-w-[250px]">
              <Select
                label="Model"
                value={selectedModel}
                onChange={(e) => setSelectedModel(e.target.value)}
                disabled={!selectedKeyId || isLoadingModels}
              >
                <option value="">Select a model...</option>
                {models.map((model) => (
                  <option key={model.id} value={model.id}>
                    {model.id}
                  </option>
                ))}
              </Select>
            </div>
            <Button
              onClick={() => loadModelsForKey(selectedKeyId)}
              disabled={!selectedKeyId || isLoadingModels}
              variant="secondary"
              title="Refresh models"
            >
              {isLoadingModels ? '...' : '↻'}
            </Button>
          </div>
        </div>
      </div>

      {/* Chat Interface - Always Visible */}
      <div className="bg-white rounded-xl shadow-sm border border-gray-200">
        <div className="p-6">
          <div className="flex justify-between items-center mb-4">
            <h2 className="text-xl font-bold text-gray-900">Chat Testing</h2>
            <Button onClick={clearChat} variant="secondary" disabled={messages.length === 0 || !canChat} title="Clear chat history">
              🗑️
            </Button>
          </div>

          {!canChat && (
            <div className="mb-4 p-4 bg-blue-50 border border-blue-200 rounded-lg">
              <p className="text-sm text-blue-800">
                Please select an API key and model above to start chatting.
              </p>
            </div>
          )}

          <div className="border border-gray-200 rounded-lg bg-white">
            {/* Messages */}
            <div className="h-96 overflow-y-auto p-4 space-y-4">
              {messages.length === 0 && (
                <div className="text-center text-gray-400 mt-20">
                  {canChat ? 'Start a conversation by typing a message below' : 'Select an API key and model to begin'}
                </div>
              )}
              {messages.map((message, index) => (
                <div
                  key={index}
                  className={`flex ${message.role === 'user' ? 'justify-end' : 'justify-start'}`}
                >
                  <div
                    className={`max-w-[80%] rounded-lg px-4 py-2 ${
                      message.role === 'user'
                        ? 'bg-blue-600 text-white'
                        : 'bg-gray-100 text-gray-900'
                    }`}
                  >
                    <div className="text-xs font-semibold mb-1 opacity-75">
                      {message.role === 'user' ? 'You' : 'Assistant'}
                    </div>
                    <div className="text-sm prose prose-sm max-w-none">
                      <ReactMarkdown remarkPlugins={[remarkGfm]}>
                        {message.content}
                      </ReactMarkdown>
                    </div>
                  </div>
                </div>
              ))}
              {isLoading && (
                <div className="flex justify-start">
                  <div className="bg-gray-100 text-gray-900 rounded-lg px-4 py-2">
                    <div className="text-xs font-semibold mb-1 opacity-75">Assistant</div>
                    <div className="text-sm">Thinking...</div>
                  </div>
                </div>
              )}
              <div ref={messagesEndRef} />
            </div>

            {/* Error Display */}
            {error && (
              <div className="px-4 py-2 bg-red-50 border-t border-red-200">
                <p className="text-sm text-red-600">Error: {error}</p>
              </div>
            )}

            {/* Input */}
            <form onSubmit={sendMessage} className="border-t border-gray-200 p-4">
              <div className="flex gap-2">
                <input
                  type="text"
                  value={input}
                  onChange={(e) => setInput(e.target.value)}
                  placeholder={canChat ? 'Type your message...' : 'Select API key and model first...'}
                  disabled={isLoading || !canChat}
                  className="flex-1 px-4 py-2 border border-gray-300 rounded-lg focus:outline-none focus:ring-2 focus:ring-blue-500 disabled:bg-gray-100 disabled:cursor-not-allowed"
                />
                <Button type="submit" disabled={isLoading || !input.trim() || !canChat}>
                  {isLoading ? 'Sending...' : 'Send'}
                </Button>
              </div>
            </form>
          </div>
        </div>
      </div>

      {/* Edit Server Config Modal */}
      <Modal
        isOpen={isEditModalOpen}
        onClose={() => {
          setIsEditModalOpen(false)
          setEditConfig(config) // Reset to current config
        }}
        title="Edit Server Configuration"
      >
        <form onSubmit={updateConfig} className="space-y-4">
          <Select
            label="Interface"
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

          <div className="space-y-2">
            <label className="text-sm font-medium text-gray-700">Cross-Origin Requests (CORS)</label>
            <div className="space-y-2">
              <div className="flex items-center gap-2">
                <input
                  type="radio"
                  id="cors-allow-all"
                  name="cors-mode"
                  checked={editConfig.enable_cors}
                  onChange={() => setEditConfig({ ...editConfig, enable_cors: true })}
                  className="w-4 h-4 text-blue-600 border-gray-300 focus:ring-blue-500"
                />
                <label htmlFor="cors-allow-all" className="text-sm text-gray-700 cursor-pointer">
                  <span className="font-medium">Allow All Origins</span>
                  <span className="text-gray-500 ml-1">(recommended for web apps and browser tools)</span>
                </label>
              </div>
              <div className="flex items-center gap-2">
                <input
                  type="radio"
                  id="cors-strict"
                  name="cors-mode"
                  checked={!editConfig.enable_cors}
                  onChange={() => setEditConfig({ ...editConfig, enable_cors: false })}
                  className="w-4 h-4 text-blue-600 border-gray-300 focus:ring-blue-500"
                />
                <label htmlFor="cors-strict" className="text-sm text-gray-700 cursor-pointer">
                  <span className="font-medium">Strict - Same Origin Only</span>
                  <span className="text-gray-500 ml-1">(browsers block requests from other domains)</span>
                </label>
              </div>
            </div>
          </div>

          <div className="flex gap-2 pt-4">
            <Button type="submit" disabled={isUpdating}>
              {isUpdating ? 'Updating...' : 'Update & Restart Server'}
            </Button>
            <Button
              type="button"
              variant="secondary"
              onClick={() => {
                setIsEditModalOpen(false)
                setEditConfig(config)
              }}
            >
              Cancel
            </Button>
          </div>
        </form>
      </Modal>
    </div>
  )
}
