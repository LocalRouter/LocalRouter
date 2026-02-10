import { useState, useEffect, useMemo, useCallback } from "react"
// DEPRECATED: Route unused - Strategy mode hidden
import { RefreshCw, Users, /* Route, */ Zap, Settings2, ChevronDown, MessageSquare, ImageIcon, Hash } from "lucide-react"
import { invoke } from "@tauri-apps/api/core"
import { Tabs, TabsContent, TabsList, TabsTrigger } from "@/components/ui/tabs"
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/Card"
import { Button } from "@/components/ui/Button"
import { Label } from "@/components/ui/label"
import { RadioGroup, RadioGroupItem } from "@/components/ui/radio-group"
import { Slider } from "@/components/ui/Slider"
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/Select"
import {
  Collapsible,
  CollapsibleContent,
  CollapsibleTrigger,
} from "@/components/ui/collapsible"
import { cn } from "@/lib/utils"
import { createOpenAIClient } from "@/lib/openai-client"
import { ChatPanel } from "./chat-panel"
import { ImagesPanel } from "./images-panel"
import { EmbeddingsPanel } from "./embeddings-panel"

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

// DEPRECATED: Strategy UI hidden - 1:1 client-to-strategy relationship
// interface Strategy {
//   id: string
//   name: string
// }

interface Provider {
  instance_name: string
  provider_type: string
  enabled: boolean
}

interface ProviderModel {
  id: string
  provider: string
}

interface Model {
  id: string
  object: string
  owned_by: string
}

interface ModelParameters {
  temperature: number
  maxTokens: number
  topP: number
}

// DEPRECATED: "strategy" mode hidden - 1:1 client-to-strategy relationship
type TestMode = "client" | /* "strategy" | */ "direct"

interface LlmTabProps {
  initialMode?: TestMode
  initialProvider?: string
  initialClientId?: string
}

export function LlmTab({ initialMode, initialProvider, initialClientId }: LlmTabProps) {
  const [activeSubtab, setActiveSubtab] = useState("chat")
  const [mode, setMode] = useState<TestMode>("client")
  const [serverPort, setServerPort] = useState<number | null>(null)

  // Client mode state
  const [clients, setClients] = useState<Client[]>([])
  const [selectedClientId, setSelectedClientId] = useState<string>("")
  const [clientApiKey, setClientApiKey] = useState<string | null>(null)

  // DEPRECATED: Strategy mode hidden - 1:1 client-to-strategy relationship
  // const [strategies, setStrategies] = useState<Strategy[]>([])
  // const [selectedStrategy, setSelectedStrategy] = useState<string>("")
  // const [strategyToken, setStrategyToken] = useState<string | null>(null)

  // Direct mode state
  const [providers, setProviders] = useState<Provider[]>([])
  const [selectedProvider, setSelectedProvider] = useState<string>("")
  const [providerModels, setProviderModels] = useState<ProviderModel[]>([])
  const [internalTestToken, setInternalTestToken] = useState<string | null>(null)

  // Shared model state
  const [models, setModels] = useState<Model[]>([])
  const [selectedModel, setSelectedModel] = useState<string>("")
  const [loadingModels, setLoadingModels] = useState(false)

  // Model parameters
  const [showParameters, setShowParameters] = useState(false)
  const [parameters, setParameters] = useState<ModelParameters>({
    temperature: 1.0,
    maxTokens: 2048,
    topP: 1.0,
  })

  // Initialize: load server config and data
  useEffect(() => {
    const init = async () => {
      try {
        const serverConfig = await invoke<ServerConfig>("get_server_config")
        const port = serverConfig.actual_port ?? serverConfig.port
        setServerPort(port)

        // Load all needed data
        // DEPRECATED: Strategy loading removed - 1:1 client-to-strategy relationship
        const [clientsList, /* strategiesList, */ providersList, allModels] = await Promise.all([
          invoke<Client[]>("list_clients"),
          // invoke<Strategy[]>("list_strategies"),
          invoke<Provider[]>("list_provider_instances"),
          invoke<{ id: string; provider: string }[]>("list_all_models"),
        ])

        // Convert to ProviderModel format
        const providerModelsList: ProviderModel[] = allModels.map(m => ({
          id: m.id,
          provider: m.provider,
        }))

        setClients(clientsList.filter(c => c.enabled))
        // DEPRECATED: setStrategies(strategiesList)
        setProviders(providersList.filter(p => p.enabled))
        setProviderModels(providerModelsList)

        // Set default selections
        if (clientsList.length > 0) {
          setSelectedClientId(clientsList[0].id)
        }
        // DEPRECATED: Strategy default selection removed
        // if (strategiesList.length > 0) {
        //   setSelectedStrategy(strategiesList[0].name)
        // }
        if (providersList.filter(p => p.enabled).length > 0) {
          setSelectedProvider(providersList.filter(p => p.enabled)[0].instance_name)
        }
      } catch (error) {
        console.error("Failed to initialize:", error)
      }
    }
    init()
  }, [])

  // Apply initial props once data is loaded
  useEffect(() => {
    if (initialMode) {
      setMode(initialMode)
    }
    if (initialMode === "direct" && initialProvider && providers.length > 0) {
      const match = providers.find(p => p.instance_name === initialProvider)
      if (match) {
        setSelectedProvider(initialProvider)
        setSelectedModel("")
      }
    }
    if (initialMode === "client" && initialClientId && clients.length > 0) {
      const match = clients.find(c => c.id === initialClientId)
      if (match) {
        setSelectedClientId(initialClientId)
      }
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [initialMode, initialProvider, initialClientId, providers.length, clients.length])

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

  // DEPRECATED: Strategy test client creation hidden - 1:1 client-to-strategy relationship
  // useEffect(() => {
  //   const createStrategyClient = async () => {
  //     if (mode === "strategy" && selectedStrategy) {
  //       try {
  //         const token = await invoke<string>("create_test_client_for_strategy", {
  //           strategyId: selectedStrategy,
  //         })
  //         setStrategyToken(token)
  //       } catch (error) {
  //         console.error("Failed to create test client:", error)
  //         setStrategyToken(null)
  //       }
  //     }
  //   }
  //   createStrategyClient()
  // }, [mode, selectedStrategy])

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
  const getAuthToken = useCallback((): string | null => {
    switch (mode) {
      case "client":
        return clientApiKey
      // DEPRECATED: Strategy mode hidden - 1:1 client-to-strategy relationship
      // case "strategy":
      //   return strategyToken
      case "direct":
        return internalTestToken
      default:
        return null
    }
  }, [mode, clientApiKey, internalTestToken])

  // Create OpenAI client when token/port changes
  const openaiClient = useMemo(() => {
    const token = getAuthToken()
    if (!token || !serverPort) return null

    return createOpenAIClient({
      apiKey: token,
      baseURL: `http://localhost:${serverPort}/v1`,
    })
  }, [getAuthToken, serverPort])

  // Fetch models using OpenAI SDK
  const fetchModels = useCallback(async () => {
    if (!openaiClient) return

    setLoadingModels(true)
    try {
      const response = await openaiClient.models.list()
      const modelsList = response.data || []
      setModels(modelsList.map(m => ({ id: m.id, object: m.object, owned_by: m.owned_by })))

      if (!selectedModel && modelsList.length > 0) {
        setSelectedModel(modelsList[0].id)
      }
    } catch (error) {
      console.error("Failed to fetch models:", error)
    } finally {
      setLoadingModels(false)
    }
  }, [openaiClient, selectedModel])

  // Fetch models when auth changes
  useEffect(() => {
    if (mode === "direct" && selectedProvider && serverPort) {
      // For direct mode, filter to provider's models
      const filtered = providerModels.filter(m => m.provider === selectedProvider)
      setModels(filtered.map(m => ({ id: m.id, object: "model", owned_by: m.provider })))
      if (filtered.length > 0 && !selectedModel) {
        setSelectedModel(filtered[0].id)
      }
    } else if (openaiClient) {
      fetchModels()
    }
  }, [mode, selectedProvider, serverPort, providerModels, openaiClient, fetchModels, selectedModel])

  const getModeDescription = () => {
    switch (mode) {
      case "client":
        return "Test requests using a client's credentials through the full routing pipeline"
      // DEPRECATED: Strategy mode hidden - 1:1 client-to-strategy relationship
      // case "strategy":
      //   return "Test requests with a specific strategy applied"
      case "direct":
        return "Send requests directly to a provider, bypassing routing"
    }
  }

  const isReady = () => {
    if (!openaiClient || !selectedModel) return false
    if (mode === "direct" && !selectedProvider) return false
    return true
  }

  // Get the effective model string for API calls
  // In direct mode, internal test token requires provider/model format
  const getEffectiveModel = () => {
    if (mode === "direct" && selectedProvider && selectedModel) {
      return `${selectedProvider}/${selectedModel}`
    }
    return selectedModel
  }

  // Get model string with provider prefix for endpoints that don't use routing
  // (images, embeddings) - these need provider/model format to know which provider to call
  const getModelWithProvider = () => {
    // If already in provider/model format (direct mode), use as-is
    if (mode === "direct" && selectedProvider && selectedModel) {
      return `${selectedProvider}/${selectedModel}`
    }
    // For client/strategy modes, look up the provider from providerModels
    const modelInfo = providerModels.find(m => m.id === selectedModel)
    if (modelInfo) {
      return `${modelInfo.provider}/${modelInfo.id}`
    }
    // Fallback to just the model (will error if not dall-e-* or similar)
    return selectedModel
  }

  return (
    <div className="flex flex-col h-full gap-4">
      {/* Mode Selection */}
      <Card>
        <CardHeader className="pb-3">
          <div className="space-y-1.5">
            <CardTitle className="text-base">Connect to LLM</CardTitle>
            <p className="text-sm text-muted-foreground">{getModeDescription()}</p>
          </div>
        </CardHeader>
        <CardContent className="flex flex-col gap-4">
          {/* Two-column layout: radio buttons left, options right */}
          <div className="flex gap-6">
            <div className="flex flex-col gap-2 flex-shrink-0">
            <Label className="text-sm font-medium">Mode</Label>
            <RadioGroup
              value={mode}
              onValueChange={(v: string) => {
                setMode(v as TestMode)
                setSelectedModel("")
              }}
              className="flex flex-col gap-3"
            >
              <div className="flex items-center space-x-2">
                <RadioGroupItem value="client" id="mode-client" />
                <Label htmlFor="mode-client" className="flex items-center gap-2 cursor-pointer">
                  <Users className="h-4 w-4" />
                  Against Client
                </Label>
              </div>
              {/* DEPRECATED: Strategy mode hidden - 1:1 client-to-strategy relationship */}
              {/* <div className="flex items-center space-x-2">
                <RadioGroupItem value="strategy" id="mode-strategy" />
                <Label htmlFor="mode-strategy" className="flex items-center gap-2 cursor-pointer">
                  <Route className="h-4 w-4" />
                  Against Strategy
                </Label>
              </div> */}
              <div className="flex items-center space-x-2">
                <RadioGroupItem value="direct" id="mode-direct" />
                <Label htmlFor="mode-direct" className="flex items-center gap-2 cursor-pointer">
                  <Zap className="h-4 w-4" />
                  Direct Model
                </Label>
              </div>
            </RadioGroup>
            </div>

            {/* Right side: mode-specific options, model selector */}
            <div className="flex flex-col gap-3 flex-1 min-w-0">
              {mode === "client" && (
                <div className="space-y-1.5">
                  <Label className="text-sm">Client</Label>
                  <Select value={selectedClientId} onValueChange={setSelectedClientId}>
                    <SelectTrigger className="w-full max-w-[280px]">
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
                </div>
              )}

              {/* DEPRECATED: Strategy mode hidden - 1:1 client-to-strategy relationship */}
              {/* {mode === "strategy" && (
                <div className="space-y-1.5">
                  <Label className="text-sm">Strategy</Label>
                  <Select value={selectedStrategy} onValueChange={setSelectedStrategy}>
                    <SelectTrigger className="w-full max-w-[280px]">
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
                </div>
              )} */}

              {mode === "direct" && (
                <div className="space-y-1.5">
                  <Label className="text-sm">Provider</Label>
                  <Select value={selectedProvider} onValueChange={(v) => {
                    setSelectedProvider(v)
                    setSelectedModel("")
                  }}>
                    <SelectTrigger className="w-full max-w-[280px]">
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
                </div>
              )}

              {/* Model selector */}
              <div className="space-y-1.5">
                <Label className="text-sm">Model</Label>
                <div className="flex items-center gap-2">
                  <Select value={selectedModel} onValueChange={setSelectedModel}>
                    <SelectTrigger className="w-full max-w-[280px]">
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
                    onClick={fetchModels}
                    disabled={loadingModels || !openaiClient}
                    title="Refresh models"
                  >
                    <RefreshCw className={cn("h-4 w-4", loadingModels && "animate-spin")} />
                  </Button>
                </div>
              </div>
            </div>
          </div>

          {/* Model Parameters - outside two-column layout */}
          <Collapsible open={showParameters} onOpenChange={setShowParameters}>
            <CollapsibleTrigger asChild>
              <Button variant="ghost" size="sm" className="gap-2">
                <Settings2 className="h-4 w-4" />
                Model Parameters
                <ChevronDown className={cn("h-4 w-4 transition-transform", showParameters && "rotate-180")} />
              </Button>
            </CollapsibleTrigger>
            <CollapsibleContent className="pt-4">
              <div className="grid grid-cols-3 gap-6">
                <div className="space-y-2">
                  <div className="flex items-center justify-between">
                    <Label className="text-sm">Temperature</Label>
                    <span className="text-sm text-muted-foreground">{parameters.temperature.toFixed(2)}</span>
                  </div>
                  <Slider
                    value={[parameters.temperature]}
                    onValueChange={(values: number[]) => setParameters(p => ({ ...p, temperature: values[0] }))}
                    min={0}
                    max={2}
                    step={0.01}
                  />
                </div>
                <div className="space-y-2">
                  <div className="flex items-center justify-between">
                    <Label className="text-sm">Max Tokens</Label>
                    <span className="text-sm text-muted-foreground">{parameters.maxTokens}</span>
                  </div>
                  <Slider
                    value={[parameters.maxTokens]}
                    onValueChange={(values: number[]) => setParameters(p => ({ ...p, maxTokens: values[0] }))}
                    min={1}
                    max={8192}
                    step={1}
                  />
                </div>
                <div className="space-y-2">
                  <div className="flex items-center justify-between">
                    <Label className="text-sm">Top P</Label>
                    <span className="text-sm text-muted-foreground">{parameters.topP.toFixed(2)}</span>
                  </div>
                  <Slider
                    value={[parameters.topP]}
                    onValueChange={(values: number[]) => setParameters(p => ({ ...p, topP: values[0] }))}
                    min={0}
                    max={1}
                    step={0.01}
                  />
                </div>
              </div>
            </CollapsibleContent>
          </Collapsible>
        </CardContent>
      </Card>

      {/* Subtabs for Chat, Images, Embeddings */}
      <Tabs value={activeSubtab} onValueChange={setActiveSubtab} className="flex flex-col flex-1 min-h-0">
        <TabsList className="w-fit">
          <TabsTrigger value="chat" className="flex items-center gap-1">
            <MessageSquare className="h-3 w-3" />
            Chat
          </TabsTrigger>
          <TabsTrigger value="images" className="flex items-center gap-1">
            <ImageIcon className="h-3 w-3" />
            Images
          </TabsTrigger>
          <TabsTrigger value="embeddings" className="flex items-center gap-1">
            <Hash className="h-3 w-3" />
            Embeddings
          </TabsTrigger>
        </TabsList>

        <TabsContent value="chat" className="flex-1 min-h-0 mt-4">
          <ChatPanel
            openaiClient={openaiClient}
            isReady={isReady()}
            selectedModel={getEffectiveModel()}
            parameters={parameters}
          />
        </TabsContent>

        <TabsContent value="images" className="flex-1 min-h-0 mt-4">
          <ImagesPanel
            openaiClient={openaiClient}
            isReady={isReady()}
            selectedModel={getModelWithProvider()}
          />
        </TabsContent>

        <TabsContent value="embeddings" className="flex-1 min-h-0 mt-4">
          <EmbeddingsPanel
            openaiClient={openaiClient}
            isReady={isReady()}
            selectedModel={getModelWithProvider()}
          />
        </TabsContent>
      </Tabs>
    </div>
  )
}
