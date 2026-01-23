import * as React from "react"
import { useState, useRef, useEffect, useMemo } from "react"
import { Send, Bot, User, RefreshCw, Users, Route, Zap } from "lucide-react"
import { invoke } from "@tauri-apps/api/core"
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/Card"
import { Button } from "@/components/ui/Button"
import { Input } from "@/components/ui/Input"
import { ScrollArea } from "@/components/ui/scroll-area"
import { RadioGroup, RadioGroupItem } from "@/components/ui/radio-group"
import { Label } from "@/components/ui/label"
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/Select"
import { cn } from "@/lib/utils"
import { createOpenAIClient } from "@/lib/openai-client"

interface ServerConfig {
  host: string
  port: number
  actual_port: number | null
  enable_cors: boolean
}

interface Client {
  id: string
  name: string
  client_id: string
  enabled: boolean
}

interface Strategy {
  id: string
  name: string
}

interface Provider {
  instance_name: string
  provider_type: string
  enabled: boolean
}

interface ProviderModel {
  id: string
  provider: string
}

interface Message {
  id: string
  role: "user" | "assistant"
  content: string
  timestamp: Date
}

interface Model {
  id: string
  object: string
  owned_by: string
}

interface ModelsResponse {
  object: string
  data: Model[]
}

interface LlmTabProps {
  innerPath: string | null
  onPathChange: (path: string | null) => void
}

type TestMode = "client" | "strategy" | "direct"

export function LlmTab({ }: LlmTabProps) {
  const [mode, setMode] = useState<TestMode>("client")
  const [messages, setMessages] = useState<Message[]>([])
  const [input, setInput] = useState("")
  const [isLoading, setIsLoading] = useState(false)
  const [serverPort, setServerPort] = useState<number | null>(null)
  const scrollRef = useRef<HTMLDivElement>(null)

  // Client mode state
  const [clients, setClients] = useState<Client[]>([])
  const [selectedClientId, setSelectedClientId] = useState<string>("")
  const [clientApiKey, setClientApiKey] = useState<string | null>(null)

  // Strategy mode state
  const [strategies, setStrategies] = useState<Strategy[]>([])
  const [selectedStrategy, setSelectedStrategy] = useState<string>("")
  const [strategyToken, setStrategyToken] = useState<string | null>(null)

  // Direct mode state
  const [providers, setProviders] = useState<Provider[]>([])
  const [selectedProvider, setSelectedProvider] = useState<string>("")
  const [providerModels, setProviderModels] = useState<ProviderModel[]>([])
  const [internalTestToken, setInternalTestToken] = useState<string | null>(null)

  // Shared model state
  const [models, setModels] = useState<Model[]>([])
  const [selectedModel, setSelectedModel] = useState<string>("")
  const [loadingModels, setLoadingModels] = useState(false)

  // Initialize: load server config and data
  useEffect(() => {
    const init = async () => {
      try {
        const serverConfig = await invoke<ServerConfig>("get_server_config")
        const port = serverConfig.actual_port ?? serverConfig.port
        setServerPort(port)

        // Load all needed data
        const [clientsList, strategiesList, providersList, allModels] = await Promise.all([
          invoke<Client[]>("list_clients"),
          invoke<Strategy[]>("list_strategies"),
          invoke<Provider[]>("list_provider_instances"),
          invoke<{ id: string; provider: string }[]>("list_all_models"),
        ])

        // Convert to ProviderModel format
        const providerModelsList: ProviderModel[] = allModels.map(m => ({
          id: m.id,
          provider: m.provider,
        }))

        setClients(clientsList.filter(c => c.enabled))
        setStrategies(strategiesList)
        setProviders(providersList.filter(p => p.enabled))
        setProviderModels(providerModelsList)

        // Set default selections
        if (clientsList.length > 0) {
          setSelectedClientId(clientsList[0].id)
        }
        if (strategiesList.length > 0) {
          setSelectedStrategy(strategiesList[0].name)
        }
        if (providersList.filter(p => p.enabled).length > 0) {
          setSelectedProvider(providersList.filter(p => p.enabled)[0].instance_name)
        }
      } catch (error) {
        console.error("Failed to initialize:", error)
      }
    }
    init()
  }, [])

  // Fetch client API key when client changes
  useEffect(() => {
    const fetchClientKey = async () => {
      if (mode === "client" && selectedClientId) {
        try {
          const secret = await invoke<string>("get_client_value", { id: selectedClientId })
          setClientApiKey(secret)
        } catch (error) {
          console.error("Failed to get client API key:", error)
          setClientApiKey(null)
        }
      }
    }
    fetchClientKey()
  }, [mode, selectedClientId])

  // Create test client for strategy when strategy changes
  useEffect(() => {
    const createStrategyClient = async () => {
      if (mode === "strategy" && selectedStrategy) {
        try {
          const token = await invoke<string>("create_test_client_for_strategy", {
            strategyId: selectedStrategy,
          })
          setStrategyToken(token)
        } catch (error) {
          console.error("Failed to create test client:", error)
          setStrategyToken(null)
        }
      }
    }
    createStrategyClient()
  }, [mode, selectedStrategy])

  // Fetch internal test token for direct mode
  useEffect(() => {
    const fetchInternalToken = async () => {
      if (mode === "direct") {
        try {
          const token = await invoke<string>("get_internal_test_token")
          setInternalTestToken(token)
        } catch (error) {
          console.error("Failed to get internal test token:", error)
          setInternalTestToken(null)
        }
      }
    }
    fetchInternalToken()
  }, [mode])

  // Get the current auth token based on mode
  const getAuthToken = (): string | null => {
    switch (mode) {
      case "client":
        return clientApiKey
      case "strategy":
        return strategyToken
      case "direct":
        return internalTestToken
      default:
        return null
    }
  }

  // Create OpenAI client when token/port changes
  const openaiClient = useMemo(() => {
    const token = getAuthToken()
    if (!token || !serverPort) return null

    return createOpenAIClient({
      apiKey: token,
      baseURL: `http://localhost:${serverPort}/v1`,
    })
  }, [clientApiKey, strategyToken, internalTestToken, serverPort, mode])

  // Fetch models when auth changes
  useEffect(() => {
    const token = getAuthToken()
    if (mode === "direct" && selectedProvider && serverPort) {
      // For direct mode, filter to provider's models
      const filtered = providerModels.filter(m => m.provider === selectedProvider)
      setModels(filtered.map(m => ({ id: m.id, object: "model", owned_by: m.provider })))
      if (filtered.length > 0 && !selectedModel) {
        setSelectedModel(filtered[0].id)
      }
    } else if (token && serverPort) {
      fetchModels(token)
    }
  }, [mode, clientApiKey, strategyToken, selectedProvider, serverPort, providerModels])

  const fetchModels = async (token: string) => {
    if (!serverPort) return

    setLoadingModels(true)
    try {
      const response = await fetch(`http://localhost:${serverPort}/v1/models`, {
        headers: {
          Authorization: `Bearer ${token}`,
        },
      })

      if (!response.ok) {
        throw new Error(`Failed to fetch models: ${response.status}`)
      }

      const data: ModelsResponse = await response.json()
      setModels(data.data || [])

      if (!selectedModel && data.data?.length > 0) {
        setSelectedModel(data.data[0].id)
      }
    } catch (error) {
      console.error("Failed to fetch models:", error)
    } finally {
      setLoadingModels(false)
    }
  }

  useEffect(() => {
    if (scrollRef.current) {
      scrollRef.current.scrollTop = scrollRef.current.scrollHeight
    }
  }, [messages])

  const handleSend = async () => {
    if (!input.trim() || isLoading || !serverPort) return

    if (!openaiClient) {
      const errorMessage: Message = {
        id: crypto.randomUUID(),
        role: "assistant",
        content: "Error: No authentication token available",
        timestamp: new Date(),
      }
      setMessages((prev) => [...prev, errorMessage])
      return
    }

    if (!selectedModel) {
      const errorMessage: Message = {
        id: crypto.randomUUID(),
        role: "assistant",
        content: "Error: Please select a model first",
        timestamp: new Date(),
      }
      setMessages((prev) => [...prev, errorMessage])
      return
    }

    const userMessage: Message = {
      id: crypto.randomUUID(),
      role: "user",
      content: input.trim(),
      timestamp: new Date(),
    }

    // Add user message immediately
    setMessages((prev) => [...prev, userMessage])
    setInput("")
    setIsLoading(true)

    // Create assistant message placeholder for streaming
    const assistantId = crypto.randomUUID()
    setMessages((prev) => [
      ...prev,
      { id: assistantId, role: "assistant", content: "", timestamp: new Date() },
    ])

    try {
      // Use OpenAI SDK for streaming chat completion
      const stream = await openaiClient.chat.completions.create({
        model: selectedModel,
        messages: [...messages, userMessage].map((m) => ({
          role: m.role,
          content: m.content,
        })),
        stream: true,
      })

      // Stream response token by token
      for await (const chunk of stream) {
        const content = chunk.choices[0]?.delta?.content || ""
        if (content) {
          setMessages((prev) =>
            prev.map((m) =>
              m.id === assistantId ? { ...m, content: m.content + content } : m
            )
          )
        }
      }
    } catch (error) {
      console.error("Failed to send message:", error)
      // Update the assistant message with error
      setMessages((prev) =>
        prev.map((m) =>
          m.id === assistantId
            ? {
                ...m,
                content: `Error: ${error instanceof Error ? error.message : "Failed to get response"}`,
              }
            : m
        )
      )
    } finally {
      setIsLoading(false)
    }
  }

  const handleKeyDown = (e: React.KeyboardEvent) => {
    if (e.key === "Enter" && !e.shiftKey) {
      e.preventDefault()
      handleSend()
    }
  }

  const clearChat = () => {
    setMessages([])
  }

  const getModeDescription = () => {
    switch (mode) {
      case "client":
        return "Test requests using a client's credentials through the full routing pipeline"
      case "strategy":
        return "Test requests with a specific routing strategy applied"
      case "direct":
        return "Send requests directly to a provider, bypassing routing"
    }
  }

  const isReady = () => {
    if (!openaiClient || !selectedModel) return false
    if (mode === "direct" && !selectedProvider) return false
    return true
  }

  return (
    <div className="flex flex-col h-full gap-4">
      {/* Mode Selection */}
      <Card>
        <CardHeader className="pb-3">
          <CardTitle className="text-base">Test Mode</CardTitle>
          <p className="text-sm text-muted-foreground">{getModeDescription()}</p>
        </CardHeader>
        <CardContent>
          <div className="space-y-4">
            <RadioGroup
              value={mode}
              onValueChange={(v) => {
                setMode(v as TestMode)
                setMessages([])
                setSelectedModel("")
              }}
              className="flex gap-4"
            >
              <div className="flex items-center space-x-2">
                <RadioGroupItem value="client" id="mode-client" />
                <Label htmlFor="mode-client" className="flex items-center gap-2 cursor-pointer">
                  <Users className="h-4 w-4" />
                  Against Client
                </Label>
              </div>
              <div className="flex items-center space-x-2">
                <RadioGroupItem value="strategy" id="mode-strategy" />
                <Label htmlFor="mode-strategy" className="flex items-center gap-2 cursor-pointer">
                  <Route className="h-4 w-4" />
                  Against Strategy
                </Label>
              </div>
              <div className="flex items-center space-x-2">
                <RadioGroupItem value="direct" id="mode-direct" />
                <Label htmlFor="mode-direct" className="flex items-center gap-2 cursor-pointer">
                  <Zap className="h-4 w-4" />
                  Direct Model
                </Label>
              </div>
            </RadioGroup>

            {/* Mode-specific selectors */}
            <div className="flex items-center gap-2">
              {mode === "client" && (
                <Select value={selectedClientId} onValueChange={setSelectedClientId}>
                  <SelectTrigger className="w-[250px]">
                    <SelectValue placeholder="Select a client" />
                  </SelectTrigger>
                  <SelectContent>
                    {clients.map((client) => (
                      <SelectItem key={client.id} value={client.id}>
                        {client.name}
                      </SelectItem>
                    ))}
                  </SelectContent>
                </Select>
              )}

              {mode === "strategy" && (
                <Select value={selectedStrategy} onValueChange={setSelectedStrategy}>
                  <SelectTrigger className="w-[250px]">
                    <SelectValue placeholder="Select a strategy" />
                  </SelectTrigger>
                  <SelectContent>
                    {strategies.map((strategy) => (
                      <SelectItem key={strategy.name} value={strategy.name}>
                        {strategy.name}
                      </SelectItem>
                    ))}
                  </SelectContent>
                </Select>
              )}

              {mode === "direct" && (
                <Select value={selectedProvider} onValueChange={(v) => {
                  setSelectedProvider(v)
                  setSelectedModel("")
                }}>
                  <SelectTrigger className="w-[250px]">
                    <SelectValue placeholder="Select a provider" />
                  </SelectTrigger>
                  <SelectContent>
                    {providers.map((provider) => (
                      <SelectItem key={provider.instance_name} value={provider.instance_name}>
                        {provider.instance_name} ({provider.provider_type})
                      </SelectItem>
                    ))}
                  </SelectContent>
                </Select>
              )}

              {/* Model selector */}
              <Select value={selectedModel} onValueChange={setSelectedModel}>
                <SelectTrigger className="w-[300px]">
                  <SelectValue placeholder={loadingModels ? "Loading models..." : "Select a model"} />
                </SelectTrigger>
                <SelectContent>
                  {models.map((model) => (
                    <SelectItem key={model.id} value={model.id}>
                      {model.id}
                    </SelectItem>
                  ))}
                </SelectContent>
              </Select>

              <Button
                variant="outline"
                size="icon"
                onClick={() => {
                  const token = getAuthToken()
                  if (token) fetchModels(token)
                }}
                disabled={loadingModels || (!clientApiKey && mode === "client") || (!strategyToken && mode === "strategy")}
                title="Refresh models"
              >
                <RefreshCw className={cn("h-4 w-4", loadingModels && "animate-spin")} />
              </Button>
            </div>
          </div>
        </CardContent>
      </Card>

      {/* Chat Interface */}
      <Card className="flex flex-col flex-1 min-h-0">
        <CardHeader className="pb-3 flex-shrink-0">
          <div className="flex items-center justify-between">
            <div>
              <CardTitle className="text-base">Chat Interface</CardTitle>
              <p className="text-sm text-muted-foreground">
                {mode === "client" && selectedClientId && `Using client: ${clients.find(c => c.id === selectedClientId)?.name}`}
                {mode === "strategy" && selectedStrategy && `Using strategy: ${selectedStrategy}`}
                {mode === "direct" && selectedProvider && `Direct to: ${selectedProvider}`}
              </p>
            </div>
            <Button variant="outline" size="sm" onClick={clearChat}>
              Clear
            </Button>
          </div>
        </CardHeader>
        <CardContent className="flex-1 flex flex-col min-h-0">
          {/* Messages */}
          <ScrollArea className="flex-1 pr-4" ref={scrollRef}>
            <div className="space-y-4">
              {messages.length === 0 ? (
                <div className="flex items-center justify-center h-64 text-muted-foreground">
                  <p className="text-sm">
                    {!isReady() ? "Select a model to start chatting" : "Send a message to start chatting"}
                  </p>
                </div>
              ) : (
                messages.map((message) => (
                  <div
                    key={message.id}
                    className={cn(
                      "flex gap-3",
                      message.role === "user" ? "justify-end" : "justify-start"
                    )}
                  >
                    {message.role === "assistant" && (
                      <div className="flex h-8 w-8 shrink-0 items-center justify-center rounded-full bg-muted">
                        <Bot className="h-4 w-4" />
                      </div>
                    )}
                    <div
                      className={cn(
                        "rounded-lg px-4 py-2 max-w-[80%]",
                        message.role === "user"
                          ? "bg-primary text-primary-foreground"
                          : "bg-muted"
                      )}
                    >
                      <p className="text-sm whitespace-pre-wrap">
                        {message.content}
                        {message.role === "assistant" && isLoading && message.content === "" && (
                          <span className="inline-block w-2 h-4 bg-foreground/50 animate-pulse" />
                        )}
                      </p>
                    </div>
                    {message.role === "user" && (
                      <div className="flex h-8 w-8 shrink-0 items-center justify-center rounded-full bg-primary">
                        <User className="h-4 w-4 text-primary-foreground" />
                      </div>
                    )}
                  </div>
                ))
              )}
            </div>
          </ScrollArea>

          {/* Input */}
          <div className="flex gap-2 pt-4 border-t mt-4 flex-shrink-0">
            <Input
              placeholder="Type a message..."
              value={input}
              onChange={(e) => setInput(e.target.value)}
              onKeyDown={handleKeyDown}
              disabled={isLoading || !isReady()}
            />
            <Button onClick={handleSend} disabled={!input.trim() || isLoading || !isReady()}>
              <Send className="h-4 w-4" />
            </Button>
          </div>
        </CardContent>
      </Card>
    </div>
  )
}
