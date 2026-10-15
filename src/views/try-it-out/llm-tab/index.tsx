import { useState, useEffect, useMemo, useCallback, useRef } from "react"
// DEPRECATED: Route unused - Strategy mode hidden
import { RefreshCw, Users, /* Route, */ Zap, Settings2, ChevronDown, ChevronRight, MessageSquare, ImageIcon, Hash, Volume2, Mic, Scale, Loader2, ChevronsUpDown, Check, Search, AlertTriangle } from "lucide-react"
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
import { ReplayPanel } from "./replay-panel"
import type { ReplayInput } from "@/types/tauri-commands"
import { ChatPanel } from "./chat-panel"
import { ImagesPanel } from "./images-panel"
import { SearchPanel } from "./search-panel"
import { DocumentsPanel } from "./documents-panel"
import { OcrPanel } from "./ocr-panel"
import { RerankPanel } from "./rerank-panel"
import { VideosPanel } from "./videos-panel"
import { RealtimePanel } from "./realtime-panel"
import { EmbeddingsPanel } from "./embeddings-panel"
import { SpeechPanel } from "./speech-panel"
import { TranscribePanel } from "./transcribe-panel"
import { SystemOnePanel } from "./systemone-panel"
import { DecisionsPanel } from "./decisions-panel"
import { MessagesPanel } from "./messages-panel"
import { FilesPanel } from "./files-panel"
import { BatchesPanel } from "./batches-panel"
import type {
  ApiPathSupport,
  GetApiPathSupportParams,
  GetProviderFeatureSupportParams,
  ProviderFeatureSupport,
  NativeOperation,
  UpstreamToolsPolicy,
  GetClientNativeOperationsParams,
  GetClientUpstreamToolsParams,
} from "@/types/tauri-commands"

import { DropdownMenu, DropdownMenuContent, DropdownMenuTrigger, DropdownMenuRadioGroup, DropdownMenuRadioItem } from "@/components/ui/dropdown-menu"
import { modelProviders, visibleTryItOutSections, sectionNavigation, type ProviderSupport, type TryItOutSection } from "@/utils/try-it-out-sections"

const SECTION_ICONS: Partial<Record<TryItOutSection, typeof MessageSquare>> = {
  chat: MessageSquare, messages: MessageSquare, decisions: Scale, systemone: Scale,
  images: ImageIcon, embeddings: Hash, speech: Volume2, transcribe: Mic, search: Search,
}

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
  frequencyPenalty: number
  presencePenalty: number
  seed: number | null
  topK: number | null
  repetitionPenalty: number | null
  reasoningEffort: string | null
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
  const singleProvider = providers.length <= 1

  // Render a model row (shared between grouped and flat views)
  const renderModelRow = (model: Model, provider: string) => (
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
      style={singleProvider ? undefined : { paddingLeft: "28px" }}
    >
      <Check
        className={cn(
          "h-3.5 w-3.5 shrink-0",
          selectedModel === model.id ? "opacity-100" : "opacity-0"
        )}
      />
      <span className="font-mono text-sm truncate">{model.id}</span>
    </button>
  )

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
          {singleProvider ? (
            /* Flat list when single provider — no grouping needed */
            filteredProviders.flatMap((provider) =>
              filteredGrouped[provider].map((model) => renderModelRow(model, provider))
            )
          ) : (
            filteredProviders.map((provider) => {
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
                    providerModels.map((model) => renderModelRow(model, provider))}
                </div>
              )
            })
          )}
        </div>
      </PopoverContent>
    </Popover>
  )
}

interface LlmTabProps {
  initialReplay?: ReplayInput
  initialMode?: TestMode
  initialProvider?: string
  initialClientId?: string
  hideModeSwitcher?: boolean
  hideProviderSelector?: boolean
  /**
   * Send requests to this base URL instead of LocalRouter's own `/v1`.
   *
   * Used by reverse-proxy clients, whose whole point is that apps talk to the
   * *wrapped provider's* address. Testing against the native gateway would
   * exercise a path those clients are explicitly denied, and would prove
   * nothing about the wrap.
   */
  baseUrlOverride?: string | null
}

export function LlmTab({ initialReplay, initialMode, initialProvider, initialClientId, hideModeSwitcher, hideProviderSelector, baseUrlOverride }: LlmTabProps) {
  const [activeSubtab, setActiveSubtab] = useState(initialReplay ? "replay" : "chat")
  const [mode, setMode] = useState<TestMode>(initialMode ?? "client")
  const [serverPort, setServerPort] = useState<number | null>(null)

  // Client mode state
  const [clients, setClients] = useState<Client[]>([])
  const [selectedClientId, setSelectedClientId] = useState<string>("")
  const [clientCredential, setClientCredential] = useState<{ clientId: string; secret: string } | null>(null)
  // Never reuse the previous client's credential while the next lookup is pending.
  const clientApiKey = clientCredential?.clientId === selectedClientId ? clientCredential.secret : null

  // DEPRECATED: Strategy mode hidden - 1:1 client-to-strategy relationship
  // const [strategies, setStrategies] = useState<Strategy[]>([])
  // const [selectedStrategy, setSelectedStrategy] = useState<string>("")
  // const [strategyToken, setStrategyToken] = useState<string | null>(null)

  // Direct mode state
  const [providers, setProviders] = useState<Provider[]>([])
  const [selectedProvider, setSelectedProvider] = useState<string>(initialProvider ?? "")
  const { models: incrementalModels, refresh: refreshIncrementalModels } = useIncrementalModels({ refreshOnMount: false })
  const providerModels = useMemo<ProviderModel[]>(() => incrementalModels.map(m => ({ id: m.id, provider: m.provider })), [incrementalModels])
  const [internalTestToken, setInternalTestToken] = useState<string | null>(null)

  // Shared model state
  const [models, setModels] = useState<Model[]>([])
  const [selectedModel, setSelectedModel] = useState<string>("")
  const [loadingModels, setLoadingModels] = useState(true)
  const [modelLoadError, setModelLoadError] = useState<string | null>(null)

  // Model parameters
  const [showParameters, setShowParameters] = useState(false)
  const [parameters, setParameters] = useState<ModelParameters>({
    temperature: 1.0,
    maxTokens: 2048,
    topP: 1.0,
    frequencyPenalty: 0,
    presencePenalty: 0,
    seed: null,
    topK: null,
    repetitionPenalty: null,
    reasoningEffort: null,
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

        // Set default selections only if not provided via props
        const enabledClients = clientsList.filter(client => client.enabled)
        if (!initialClientId && enabledClients.length > 0) {
          setSelectedClientId(enabledClients[0].id)
        }
        // DEPRECATED: Strategy default selection removed
        // if (strategiesList.length > 0) {
        //   setSelectedStrategy(strategiesList[0].name)
        // }
        if (!initialProvider && providersList.filter(p => p.enabled).length > 0) {
          setSelectedProvider(providersList.filter(p => p.enabled)[0].instance_name)
        }
      } catch (error) {
        console.error("Failed to initialize:", error)
        setLoadingModels(false)
      }
    }
    init()
  }, [])

  // Apply initial client ID once clients are loaded (needs id lookup from client_id)
  useEffect(() => {
    if (initialClientId && clients.length > 0) {
      const match = clients.find(c => c.client_id === initialClientId)
      if (match) {
        setSelectedClientId(match.id)
      }
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [initialClientId, clients.length])

  // Fetch client API key when client changes
  useEffect(() => {
    let cancelled = false
    setClientCredential(null)
    const fetchClientKey = async () => {
      if (mode === "client" && selectedClientId) {
        try {
          const secret = await invoke<string>("get_client_value", { id: selectedClientId })
          if (!cancelled) setClientCredential({ clientId: selectedClientId, secret })
        } catch (error) {
          console.error("Failed to get client API key:", error)
          if (!cancelled) setClientCredential(null)
          setLoadingModels(false)
        }
      }
    }
    fetchClientKey()
    return () => { cancelled = true }
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
    // A wrapped local provider is reached at its own address and needs no
    // credentials — the listener is the client's identity. The SDK still
    // requires a non-empty key, so pass a placeholder.
    if (baseUrlOverride) {
      return createOpenAIClient({ apiKey: "local", baseURL: baseUrlOverride })
    }

    const token = getAuthToken()
    if (!token || !serverPort) return null

    return createOpenAIClient({
      apiKey: token,
      baseURL: `http://localhost:${serverPort}/v1`,
    })
  }, [getAuthToken, serverPort, baseUrlOverride])

  const modelScope = JSON.stringify([mode, selectedClientId, selectedProvider, baseUrlOverride])
  const [loadedModelScope, setLoadedModelScope] = useState("")
  const modelRequest = useRef(0)

  // Fetch models using OpenAI SDK
  const fetchModels = useCallback(async () => {
    if (!openaiClient) return

    const request = ++modelRequest.current
    setLoadingModels(true)
    setModelLoadError(null)
    try {
      const response = await openaiClient.models.list()
      if (request !== modelRequest.current) return
      const modelsList = response.data || []
      setModels(modelsList.map(m => ({ id: m.id, object: m.object, owned_by: m.owned_by })))
      setLoadedModelScope(modelScope)

      // Auto-select first model if none selected or current selection not in new list
      // Use functional update to avoid selectedModel dependency and race conditions
      setSelectedModel(prev => {
        if (modelsList.length === 0) return ""
        if (!prev || !modelsList.some(m => m.id === prev)) return modelsList[0].id
        return prev
      })
    } catch (error) {
      if (request !== modelRequest.current) return
      console.error("Failed to fetch models:", error)
      setLoadedModelScope("")
      const msg = error instanceof Error ? error.message : String(error)
      setModelLoadError(msg)
    } finally {
      if (request === modelRequest.current) setLoadingModels(false)
    }
  }, [openaiClient, modelScope])

  // Fetch models when auth changes
  useEffect(() => {
    if (mode === "direct" && selectedProvider && serverPort) {
      // For direct mode, filter to provider's models
      const filtered = providerModels.filter(m => m.provider === selectedProvider)
      setModels(filtered.map(m => ({ id: m.id, object: "model", owned_by: m.provider })))
      setLoadedModelScope(modelScope)
      setSelectedModel(prev => {
        if (filtered.length === 0) return ""
        if (!prev || !filtered.some(m => m.id === prev)) return filtered[0].id
        return prev
      })
      setLoadingModels(false)
    } else if (mode === "all" && serverPort) {
      // For "all" mode, show all provider models in a single combined dropdown
      setModels(providerModels.map(m => ({ id: m.id, object: "model", owned_by: m.provider })))
      setLoadedModelScope(modelScope)
      setSelectedModel(prev => {
        if (providerModels.length === 0) return ""
        if (!prev || !providerModels.some(m => m.id === prev)) return providerModels[0].id
        return prev
      })
      setLoadingModels(false)
    } else if (openaiClient) {
      fetchModels()
    }
    return () => { modelRequest.current += 1 }
  // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [mode, selectedProvider, serverPort, providerModels, openaiClient, fetchModels])

  // Refresh models: use API for client mode, incremental refresh for direct/all
  const handleRefresh = useCallback(() => {
    if (mode === "direct" || mode === "all") {
      setLoadingModels(true)
      refreshIncrementalModels(true)
    } else {
      fetchModels()
    }
  }, [mode, refreshIncrementalModels, fetchModels])

  const providerNames = useMemo(() => providers.map(provider => provider.instance_name), [providers])
  const selectedProviders = loadedModelScope === modelScope ? modelProviders(selectedModel, providerNames, providerModels) : []
  const availabilityProvider = mode === "direct" ? selectedProvider : selectedProviders.length === 1 ? selectedProviders[0] : undefined
  const localClient = mode === "client" && !!selectedClientId && !baseUrlOverride
  // Native panels select their own model. Check every allowed model's provider,
  // rather than hiding a native operation because the chat selection cannot do it.
  const candidateNames = (loadedModelScope === modelScope ? models : []).flatMap(model => {
    const names = modelProviders(model.id, providerNames, providerModels)
    return names.length ? names : [null]
  })
  const supportNames = [...new Set([...candidateNames, availabilityProvider].filter((name): name is string => !!name))].sort()
  const supportKey = JSON.stringify(supportNames)
  const [supportSnapshot, setSupportSnapshot] = useState<{ key: string; reports: Record<string, ProviderSupport> } | null>(null)
  const reports = supportSnapshot?.key === supportKey ? supportSnapshot.reports : {}
  const selectedSupport = availabilityProvider ? reports[availabilityProvider] : undefined
  const messagesPathSupport = selectedSupport?.paths ?? null
  const imageEditsLevel = selectedSupport?.features?.endpoints.find(endpoint => endpoint.endpoint === "/v1/images/edits")?.support
  const imageEditsAvailable = imageEditsLevel === undefined ? null : imageEditsLevel !== "not_supported" && imageEditsLevel !== "not_implemented"

  useEffect(() => {
    let cancelled = false
    const names: string[] = JSON.parse(supportKey)
    void Promise.all(names.map(async instanceName => {
      const [paths, features] = await Promise.allSettled([
        invoke<ApiPathSupport>("get_api_path_support", { instanceName } satisfies GetApiPathSupportParams),
        invoke<ProviderFeatureSupport>("get_provider_feature_support", { instanceName } satisfies GetProviderFeatureSupportParams),
      ])
      return [instanceName, {
        paths: paths.status === "fulfilled" ? paths.value : undefined,
        features: features.status === "fulfilled" ? features.value : undefined,
      }] as const
    })).then(entries => {
      if (!cancelled) setSupportSnapshot({ key: supportKey, reports: Object.fromEntries(entries) })
    })
    return () => { cancelled = true }
  }, [supportKey])

  const [permissionSnapshot, setPermissionSnapshot] = useState<{ clientId: string; nativeOperations?: NativeOperation[]; upstreamTools?: UpstreamToolsPolicy } | null>(null)
  useEffect(() => {
    if (!localClient) return
    let cancelled = false
    void Promise.allSettled([
      invoke<NativeOperation[]>("get_client_native_operations", { clientId: selectedClientId } satisfies GetClientNativeOperationsParams),
      invoke<UpstreamToolsPolicy>("get_client_upstream_tools", { clientId: selectedClientId } satisfies GetClientUpstreamToolsParams),
    ]).then(([operations, tools]) => {
      if (!cancelled) setPermissionSnapshot({
        clientId: selectedClientId,
        nativeOperations: operations.status === "fulfilled" ? operations.value : undefined,
        upstreamTools: tools.status === "fulfilled" ? tools.value : undefined,
      })
    })
    return () => { cancelled = true }
  }, [localClient, selectedClientId])
  const permissions = localClient && permissionSnapshot?.clientId === selectedClientId ? permissionSnapshot : undefined
  const visibleSections = visibleTryItOutSections({
    localClient, selected: selectedSupport,
    candidates: candidateNames.map(name => name ? reports[name] : undefined),
    nativeOperations: permissions?.nativeOperations, upstreamTools: permissions?.upstreamTools,
  })
  const navigation = sectionNavigation(visibleSections, activeSubtab)
  const availableSection = navigation.selected?.id ?? ""
  useEffect(() => {
    if (activeSubtab !== availableSection) setActiveSubtab(availableSection)
  }, [activeSubtab, availableSection])

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
    <div className="flex flex-col h-full min-w-0 max-w-full gap-4">
      {/* Mode Selection */}
      <Card>
        <CardHeader className="pb-3">
          <div className="space-y-1.5">
            <CardTitle className="text-base">{["search", "documents", "ocr", "rerank", "videos", "realtime"].includes(activeSubtab) ? "Client connection" : "Connect to LLM"}</CardTitle>
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
              {activeSubtab !== "search" && activeSubtab !== "documents" && activeSubtab !== "ocr" && activeSubtab !== "rerank" && activeSubtab !== "videos" && activeSubtab !== "realtime" && (<div className="space-y-1.5">
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
                    onClick={handleRefresh}
                    disabled={loadingModels || (mode === "client" && !openaiClient)}
                    title="Refresh models"
                  >
                    <RefreshCw className={cn("h-4 w-4", loadingModels && "animate-spin")} />
                  </Button>
                </div>
                {!loadingModels && models.length === 0 && (
                  <div className="flex items-start gap-2 text-xs text-muted-foreground bg-muted/50 rounded-md px-3 py-2">
                    <AlertTriangle className="h-3.5 w-3.5 shrink-0 mt-0.5 text-amber-500" />
                    <div>
                      {modelLoadError ? (
                        <span>Failed to load models: {modelLoadError}</span>
                      ) : (
                        <span>
                          No models found. This can happen if the provider is not running,
                          models haven't been downloaded yet, or the API key is invalid.
                          {mode === "direct" && " Try checking that the provider is online and has models available."}
                        </span>
                      )}
                    </div>
                  </div>
                )}
              </div>)}
            </div>
          </div>

          {/* Model Parameters - outside two-column layout */}
          {activeSubtab !== "search" && activeSubtab !== "documents" && activeSubtab !== "ocr" && activeSubtab !== "rerank" && activeSubtab !== "videos" && activeSubtab !== "realtime" && (
          <Collapsible open={showParameters} onOpenChange={setShowParameters}>
            <CollapsibleTrigger asChild>
              <Button variant="ghost" size="sm" className="gap-2">
                <Settings2 className="h-4 w-4" />
                Model Parameters
                <ChevronDown className={cn("h-4 w-4 transition-transform", showParameters && "rotate-180")} />
              </Button>
            </CollapsibleTrigger>
            <CollapsibleContent className="pt-4">
              <div className="grid grid-cols-3 gap-x-6 gap-y-4">
                {/* Row 1: Temperature, Max Tokens, Top P */}
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

                {/* Row 2: Frequency Penalty, Presence Penalty, Reasoning Effort */}
                <div className="space-y-2">
                  <div className="flex items-center justify-between">
                    <Label className="text-sm">Frequency Penalty</Label>
                    <span className="text-sm text-muted-foreground">{parameters.frequencyPenalty.toFixed(2)}</span>
                  </div>
                  <Slider
                    value={[parameters.frequencyPenalty]}
                    onValueChange={(values: number[]) => setParameters(p => ({ ...p, frequencyPenalty: values[0] }))}
                    min={-2}
                    max={2}
                    step={0.01}
                  />
                </div>
                <div className="space-y-2">
                  <div className="flex items-center justify-between">
                    <Label className="text-sm">Presence Penalty</Label>
                    <span className="text-sm text-muted-foreground">{parameters.presencePenalty.toFixed(2)}</span>
                  </div>
                  <Slider
                    value={[parameters.presencePenalty]}
                    onValueChange={(values: number[]) => setParameters(p => ({ ...p, presencePenalty: values[0] }))}
                    min={-2}
                    max={2}
                    step={0.01}
                  />
                </div>
                <div className="space-y-2">
                  <Label className="text-sm">Reasoning Effort</Label>
                  <Select
                    value={parameters.reasoningEffort ?? "none"}
                    onValueChange={(v) => setParameters(p => ({ ...p, reasoningEffort: v === "none" ? null : v }))}
                  >
                    <SelectTrigger className="w-full h-8">
                      <SelectValue />
                    </SelectTrigger>
                    <SelectContent>
                      <SelectItem value="none">Not set</SelectItem>
                      <SelectItem value="low">Low</SelectItem>
                      <SelectItem value="medium">Medium</SelectItem>
                      <SelectItem value="high">High</SelectItem>
                    </SelectContent>
                  </Select>
                </div>

                {/* Row 3: Seed, Top K, Repetition Penalty */}
                <div className="space-y-2">
                  <Label className="text-sm">Seed</Label>
                  <input
                    type="number"
                    placeholder="Not set"
                    value={parameters.seed ?? ""}
                    onChange={(e) => setParameters(p => ({ ...p, seed: e.target.value === "" ? null : parseInt(e.target.value, 10) }))}
                    className="flex h-8 w-full rounded-md border border-input bg-background px-3 py-1 text-sm shadow-sm transition-colors placeholder:text-muted-foreground focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-ring"
                  />
                </div>
                <div className="space-y-2">
                  <Label className="text-sm">Top K</Label>
                  <input
                    type="number"
                    placeholder="Not set"
                    min={1}
                    value={parameters.topK ?? ""}
                    onChange={(e) => setParameters(p => ({ ...p, topK: e.target.value === "" ? null : parseInt(e.target.value, 10) }))}
                    className="flex h-8 w-full rounded-md border border-input bg-background px-3 py-1 text-sm shadow-sm transition-colors placeholder:text-muted-foreground focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-ring"
                  />
                </div>
                <div className="space-y-2">
                  <Label className="text-sm">Repetition Penalty</Label>
                  <input
                    type="number"
                    placeholder="Not set"
                    min={0}
                    max={2}
                    step={0.01}
                    value={parameters.repetitionPenalty ?? ""}
                    onChange={(e) => setParameters(p => ({ ...p, repetitionPenalty: e.target.value === "" ? null : parseFloat(e.target.value) }))}
                    className="flex h-8 w-full rounded-md border border-input bg-background px-3 py-1 text-sm shadow-sm transition-colors placeholder:text-muted-foreground focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-ring"
                  />
                </div>
              </div>
            </CollapsibleContent>
          </Collapsible>)}
        </CardContent>
      </Card>

      <Tabs value={availableSection} onValueChange={setActiveSubtab} className="flex flex-col flex-1 min-h-0 min-w-0 max-w-full">
        <div className="flex flex-wrap items-center gap-1 rounded-lg bg-muted p-1 shrink-0" aria-label="Available API panels">
          <TabsList aria-label="Try It Out panels" className="h-auto max-w-full flex-wrap justify-start gap-1 p-0">
            {navigation.tabs.map(section => {
              const Icon = SECTION_ICONS[section.id]
              return <TabsTrigger key={section.id} value={section.id} className="flex items-center gap-1">
                {Icon && <Icon className="h-3 w-3" />}{section.label}
              </TabsTrigger>
            })}
          </TabsList>
          {navigation.more.length > 0 && <DropdownMenu>
            <DropdownMenuTrigger asChild>
              <Button variant="ghost" size="sm" className="h-7 px-3">More<ChevronDown className="ml-1 h-3 w-3" /></Button>
            </DropdownMenuTrigger>
            <DropdownMenuContent align="start" className="max-h-[min(24rem,var(--radix-dropdown-menu-content-available-height))] overflow-y-auto">
              <DropdownMenuRadioGroup value={availableSection} onValueChange={setActiveSubtab}>
                {navigation.more.map(section => <DropdownMenuRadioItem key={section.id} value={section.id}>{section.label}</DropdownMenuRadioItem>)}
              </DropdownMenuRadioGroup>
            </DropdownMenuContent>
          </DropdownMenu>}
          {visibleSections.length === 0 && <p className="px-2 text-sm text-muted-foreground">No supported panels are available for this selection.</p>}
        </div>

        <TabsContent value="batches" className="mt-4">{mode === "client" && !baseUrlOverride ? <BatchesPanel key={selectedClientId} openaiClient={openaiClient} isReady={!!openaiClient} /> : <p>Select client mode to manage native batches.</p>}</TabsContent>
        <TabsContent value="files" className="mt-4">{mode === "client" && !baseUrlOverride ? <FilesPanel key={selectedClientId} openaiClient={openaiClient} isReady={!!openaiClient} /> : <p>Select client mode to manage local files.</p>}</TabsContent>
        <TabsContent value="replay" className="mt-4">{mode === "client" && !baseUrlOverride ? <ReplayPanel key={selectedClientId} clientId={selectedClientId} models={models} initialInput={initialReplay} /> : <p>Select client mode to run a comparison.</p>}</TabsContent>
        <TabsContent value="chat" className="flex-1 min-h-0 mt-4">
          <ChatPanel
            key={`${mode}-${selectedClientId}-${selectedProvider}`}
            openaiClient={openaiClient}
            isReady={isReady()}
            selectedModel={getEffectiveModel()}
            parameters={parameters}
            noModelsAvailable={!loadingModels && models.length === 0}
            providerInstance={
              mode === "direct"
                ? selectedProvider
                : providerModels.find(m => m.id === selectedModel)?.provider ?? null
            }
          />
        </TabsContent>

        <TabsContent value="images" className="flex-1 min-h-0 mt-4">
          <ImagesPanel
            key={`${mode}-${selectedClientId}-${selectedProvider}`}
            openaiClient={openaiClient}
            isReady={isReady()}
            selectedModel={getModelWithProvider()}
            supportsEdits={imageEditsAvailable}
          />
        </TabsContent>

        <TabsContent value="realtime" className="flex-1 min-h-0 mt-4"><RealtimePanel key={`${mode}-${selectedClientId}`} openaiClient={openaiClient} isReady={mode === "client" && !baseUrlOverride && !!openaiClient} /></TabsContent>
        <TabsContent value="videos" className="flex-1 min-h-0 mt-4"><VideosPanel key={`${mode}-${selectedClientId}`} openaiClient={openaiClient} isReady={mode === "client" && !baseUrlOverride && !!openaiClient} /></TabsContent>
        <TabsContent value="rerank" className="flex-1 min-h-0 mt-4"><RerankPanel key={`${mode}-${selectedClientId}`} openaiClient={openaiClient} isReady={mode === "client" && !baseUrlOverride && !!openaiClient} /></TabsContent>
        <TabsContent value="ocr" className="flex-1 min-h-0 mt-4"><OcrPanel key={`${mode}-${selectedClientId}`} openaiClient={openaiClient} isReady={mode === "client" && !baseUrlOverride && !!openaiClient} /></TabsContent>
        <TabsContent value="documents" className="flex-1 min-h-0 mt-4"><DocumentsPanel key={`${mode}-${selectedClientId}`} openaiClient={openaiClient} isReady={mode === "client" && !baseUrlOverride && !!openaiClient} /></TabsContent>
        <TabsContent value="search" className="flex-1 min-h-0 mt-4"><SearchPanel key={`${mode}-${selectedClientId}`} openaiClient={openaiClient} isReady={mode === "client" && !baseUrlOverride && !!openaiClient} /></TabsContent>
        <TabsContent value="embeddings" className="flex-1 min-h-0 mt-4">
          <EmbeddingsPanel
            key={`${mode}-${selectedClientId}-${selectedProvider}`}
            openaiClient={openaiClient}
            isReady={isReady()}
            selectedModel={getModelWithProvider()}
          />
        </TabsContent>

        <TabsContent value="speech" className="flex-1 min-h-0 mt-4">
          <SpeechPanel
            key={`${mode}-${selectedClientId}-${selectedProvider}`}
            openaiClient={openaiClient}
            isReady={isReady()}
            selectedModel={getModelWithProvider()}
          />
        </TabsContent>

        <TabsContent value="transcribe" className="flex-1 min-h-0 mt-4">
          <TranscribePanel
            key={`${mode}-${selectedClientId}-${selectedProvider}`}
            openaiClient={openaiClient}
            isReady={isReady()}
            selectedModel={getModelWithProvider()}
          />
        </TabsContent>

        <TabsContent value="messages" className="flex-1 min-h-0 mt-4"><MessagesPanel key={`${mode}-${selectedClientId}-${selectedProvider}`} openaiClient={openaiClient} isReady={isReady()} selectedModel={getModelWithProvider()} pathSupport={messagesPathSupport} /></TabsContent>
        <TabsContent value="decisions" className="flex-1 min-h-0 mt-4"><DecisionsPanel key={`${mode}-${selectedClientId}-${selectedProvider}`} openaiClient={openaiClient} isReady={isReady()} selectedModel={getModelWithProvider()} /></TabsContent>
        <TabsContent value="systemone" className="flex-1 min-h-0 mt-4">
          <SystemOnePanel
            key={`${mode}-${selectedClientId}-${selectedProvider}`}
            openaiClient={openaiClient}
            isReady={isReady()}
            selectedModel={getModelWithProvider()}
          />
        </TabsContent>
      </Tabs>
    </div>
  )
}
