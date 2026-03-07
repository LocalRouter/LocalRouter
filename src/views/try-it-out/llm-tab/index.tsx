import { useState, useEffect, useMemo, useCallback, useRef } from "react"
// DEPRECATED: Route unused - Strategy mode hidden
import { RefreshCw, Users, /* Route, */ Zap, Settings2, ChevronDown, ChevronRight, MessageSquare, ImageIcon, Hash, Loader2, ChevronsUpDown, Check, Search } from "lucide-react"
import { invoke } from "@tauri-apps/api/core"
import { useIncrementalModels } from "@/hooks/useIncrementalModels"
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
  Popover,
  PopoverContent,
  PopoverTrigger,
} from "@/components/ui/popover"
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
type TestMode = "client" | /* "strategy" | */ "direct" | "all"

/** Searchable model selector with provider grouping and collapsible sections */
function ModelCombobox({
  models,
  selectedModel,
  onModelChange,
  loading,
}: {
  models: Model[]
  selectedModel: string
  onModelChange: (model: string) => void
  loading: boolean
}) {
  const [open, setOpen] = useState(false)
  const [search, setSearch] = useState("")
  const [collapsedProviders, setCollapsedProviders] = useState<Set<string> | "all">("all")
  const inputRef = useRef<HTMLInputElement>(null)

  // Group models by provider
  const grouped = useMemo(() => {
    const groups: Record<string, Model[]> = {}
    for (const model of models) {
      const provider = model.owned_by || "unknown"
      if (!groups[provider]) groups[provider] = []
      groups[provider].push(model)
    }
    return groups
  }, [models])

  const providers = useMemo(() => Object.keys(grouped).sort(), [grouped])

  // Filter by search
  const searchLower = search.toLowerCase()
  const filteredGrouped = useMemo(() => {
    if (!searchLower) return grouped
    const result: Record<string, Model[]> = {}
    for (const [provider, providerModels] of Object.entries(grouped)) {
      const filtered = providerModels.filter(
        (m) =>
          m.id.toLowerCase().includes(searchLower) ||
          provider.toLowerCase().includes(searchLower)
      )
      if (filtered.length > 0) result[provider] = filtered
    }
    return result
  }, [grouped, searchLower])

  const filteredProviders = useMemo(
    () => Object.keys(filteredGrouped).sort(),
    [filteredGrouped]
  )

  const toggleProvider = (provider: string) => {
    setCollapsedProviders((prev) => {
      if (prev === "all") {
        const allProviders = new Set(providers)
        allProviders.delete(provider)
        return allProviders
      }
      const next = new Set(prev)
      if (next.has(provider)) {
        next.delete(provider)
      } else {
        next.add(provider)
      }
      return next
    })
  }

  const isCollapsed = (provider: string) => {
    // When searching, expand all matching groups
    if (searchLower) return false
    if (collapsedProviders === "all") return true
    return collapsedProviders.has(provider)
  }

  // Focus search input when popover opens
  useEffect(() => {
    if (open) {
      setTimeout(() => inputRef.current?.focus(), 0)
    } else {
      setSearch("")
    }
  }, [open])

  const hasResults = filteredProviders.length > 0

  return (
    <Popover open={open} onOpenChange={setOpen}>
      <PopoverTrigger asChild>
        <Button
          variant="outline"
          role="combobox"
          aria-expanded={open}
          className="w-full max-w-[280px] justify-between font-normal"
        >
          {loading ? (
            <span className="flex items-center gap-1.5 text-muted-foreground">
              <Loader2 className="h-3.5 w-3.5 animate-spin" />
              Loading models...
            </span>
          ) : selectedModel ? (
            <span className="truncate">{selectedModel}</span>
          ) : (
            <span className="text-muted-foreground">Select a model</span>
          )}
          <ChevronsUpDown className="ml-2 h-4 w-4 shrink-0 opacity-50" />
        </Button>
      </PopoverTrigger>
      <PopoverContent className="w-[320px] p-0" align="start">
        {/* Search */}
        <div className="flex items-center gap-2 px-3 py-2 border-b">
          <Search className="h-3.5 w-3.5 text-muted-foreground/50 shrink-0" />
          <input
            ref={inputRef}
            type="text"
            placeholder="Search models..."
            value={search}
            onChange={(e) => setSearch(e.target.value)}
            className="flex-1 bg-transparent text-sm outline-none placeholder:text-muted-foreground/50"
          />
        </div>

        {/* Model list */}
        <div className="max-h-[300px] overflow-y-auto">
          {loading && models.length === 0 && (
            <div className="flex items-center justify-center gap-1.5 px-3 py-4 text-xs text-muted-foreground">
              <Loader2 className="h-3.5 w-3.5 animate-spin" />
              Loading models from providers...
            </div>
          )}
          {!loading && !hasResults && (
            <div className="p-4 text-center text-sm text-muted-foreground">
              {search ? `No models match "${search}"` : "No models available"}
            </div>
          )}
          {filteredProviders.map((provider) => {
            const providerModels = filteredGrouped[provider]
            const collapsed = isCollapsed(provider)

            return (
              <div key={provider}>
                <button
                  type="button"
                  onClick={() => toggleProvider(provider)}
                  className="flex items-center gap-2 w-full px-3 py-1.5 bg-muted/30 border-b text-left hover:bg-muted/50 transition-colors sticky top-0 z-10"
                >
                  {collapsed ? (
                    <ChevronRight className="h-3.5 w-3.5 text-muted-foreground/60 shrink-0" />
                  ) : (
                    <ChevronDown className="h-3.5 w-3.5 text-muted-foreground/60 shrink-0" />
                  )}
                  <span className="text-xs font-medium text-muted-foreground">{provider}</span>
                  <span className="text-xs text-muted-foreground/60 ml-auto">{providerModels.length}</span>
                </button>
                {!collapsed &&
                  providerModels.map((model) => (
                    <button
                      key={`${provider}/${model.id}`}
                      type="button"
                      onClick={() => {
                        onModelChange(model.id)
                        setOpen(false)
                      }}
                      className={cn(
                        "flex items-center gap-2 w-full px-3 py-1.5 text-left text-sm hover:bg-accent transition-colors",
                        "border-b border-border/30",
                        selectedModel === model.id && "bg-accent"
                      )}
                      style={{ paddingLeft: "28px" }}
                    >
                      <Check
                        className={cn(
                          "h-3.5 w-3.5 shrink-0",
                          selectedModel === model.id ? "opacity-100" : "opacity-0"
                        )}
                      />
                      <span className="font-mono text-sm truncate">{model.id}</span>
                    </button>
                  ))}
              </div>
            )
          })}
        </div>
      </PopoverContent>
    </Popover>
  )
}

interface LlmTabProps {
  initialMode?: TestMode
  initialProvider?: string
  initialClientId?: string
  hideModeSwitcher?: boolean
  hideProviderSelector?: boolean
}

export function LlmTab({ initialMode, initialProvider, initialClientId, hideModeSwitcher, hideProviderSelector }: LlmTabProps) {
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
  const { models: incrementalModels } = useIncrementalModels({ refreshOnMount: false })
  const providerModels = useMemo<ProviderModel[]>(() => incrementalModels.map(m => ({ id: m.id, provider: m.provider })), [incrementalModels])
  const [internalTestToken, setInternalTestToken] = useState<string | null>(null)

  // Shared model state
  const [models, setModels] = useState<Model[]>([])
  const [selectedModel, setSelectedModel] = useState<string>("")
  const [loadingModels, setLoadingModels] = useState(true)

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
        const [clientsList, /* strategiesList, */ providersList] = await Promise.all([
          invoke<Client[]>("list_clients"),
          // invoke<Strategy[]>("list_strategies"),
          invoke<Provider[]>("list_provider_instances"),
        ])

        setClients(clientsList.filter(c => c.enabled))
        // DEPRECATED: setStrategies(strategiesList)
        setProviders(providersList.filter(p => p.enabled))

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
      } finally {
        setLoadingModels(false)
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
      const match = clients.find(c => c.client_id === initialClientId)
      if (match) {
        setSelectedClientId(match.id)
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

  // Fetch internal test token for direct and all modes
  useEffect(() => {
    const fetchInternalToken = async () => {
      if (mode === "direct" || mode === "all") {
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
      case "all":
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

      // Auto-select first model if none selected or current selection not in new list
      // Use functional update to avoid selectedModel dependency and race conditions
      setSelectedModel(prev => {
        if (modelsList.length === 0) return ""
        if (!prev || !modelsList.some(m => m.id === prev)) return modelsList[0].id
        return prev
      })
    } catch (error) {
      console.error("Failed to fetch models:", error)
    } finally {
      setLoadingModels(false)
    }
  }, [openaiClient])

  // Fetch models when auth changes
  useEffect(() => {
    if (mode === "direct" && selectedProvider && serverPort) {
      // For direct mode, filter to provider's models
      const filtered = providerModels.filter(m => m.provider === selectedProvider)
      setModels(filtered.map(m => ({ id: m.id, object: "model", owned_by: m.provider })))
      setSelectedModel(prev => {
        if (filtered.length === 0) return ""
        if (!prev || !filtered.some(m => m.id === prev)) return filtered[0].id
        return prev
      })
    } else if (mode === "all" && serverPort) {
      // For "all" mode, show all provider models in a single combined dropdown
      setModels(providerModels.map(m => ({ id: m.id, object: "model", owned_by: m.provider })))
      setSelectedModel(prev => {
        if (providerModels.length === 0) return ""
        if (!prev || !providerModels.some(m => m.id === prev)) return providerModels[0].id
        return prev
      })
    } else if (openaiClient) {
      fetchModels()
    }
  // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [mode, selectedProvider, serverPort, providerModels, openaiClient, fetchModels])

  const getModeDescription = () => {
    switch (mode) {
      case "client":
        return "Test requests using a client's credentials through the full routing pipeline"
      // DEPRECATED: Strategy mode hidden - 1:1 client-to-strategy relationship
      // case "strategy":
      //   return "Test requests with a specific strategy applied"
      case "direct":
        return "Send requests directly to a provider, bypassing routing"
      case "all":
        return "Test requests against all available models from all providers"
    }
  }

  const isReady = () => {
    if (!openaiClient || !selectedModel) return false
    if (mode === "direct" && !selectedProvider) return false
    return true
  }

  // Get the effective model string for API calls
  // In direct/all mode, internal test token requires provider/model format
  const getEffectiveModel = () => {
    if (mode === "direct" && selectedProvider && selectedModel) {
      return `${selectedProvider}/${selectedModel}`
    }
    if (mode === "all" && selectedModel) {
      const modelInfo = providerModels.find(m => m.id === selectedModel)
      if (modelInfo) return `${modelInfo.provider}/${selectedModel}`
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
            {!hideModeSwitcher && (
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
            )}

            {/* Right side: mode-specific options, model selector */}
            <div className="flex flex-col gap-3 flex-1 min-w-0">
              {mode === "client" && !hideModeSwitcher && (
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

              {mode === "direct" && !hideProviderSelector && (
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
                  <ModelCombobox
                    models={models}
                    selectedModel={selectedModel}
                    onModelChange={setSelectedModel}
                    loading={loadingModels}
                  />

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
