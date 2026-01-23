import { useState, useEffect, useMemo } from "react"
import { invoke } from "@tauri-apps/api/core"
import { listen } from "@tauri-apps/api/event"
import { Search, X } from "lucide-react"
import { Tabs, TabsContent, TabsList, TabsTrigger } from "@/components/ui/tabs"
import { Card, CardContent } from "@/components/ui/Card"
import { Button } from "@/components/ui/Button"
import { Badge } from "@/components/ui/Badge"
import { Input } from "@/components/ui/Input"
import { Label } from "@/components/ui/label"
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/Select"
import { ScrollArea } from "@/components/ui/scroll-area"

interface LLMLogEntry {
  timestamp: string
  api_key_name: string
  provider: string
  model: string
  status: string
  status_code: number
  input_tokens: number
  output_tokens: number
  total_tokens: number
  cost_usd: number
  latency_ms: number
  request_id: string
}

interface MCPLogEntry {
  timestamp: string
  client_id: string
  server_id: string
  method: string
  status: string
  status_code: number
  error_code?: number
  latency_ms: number
  transport: string
  request_id: string
}

interface LogsViewProps {
  activeSubTab: string | null
  onTabChange: (view: string, subTab?: string | null) => void
}

export function LogsView({ activeSubTab, onTabChange }: LogsViewProps) {
  const [llmLogs, setLlmLogs] = useState<LLMLogEntry[]>([])
  const [mcpLogs, setMcpLogs] = useState<MCPLogEntry[]>([])
  const [loading, setLoading] = useState(true)
  const [limit, setLimit] = useState(100)

  // Filter states
  const [searchQuery, setSearchQuery] = useState("")
  const [selectedClient, setSelectedClient] = useState<string>("all")
  const [selectedProvider, setSelectedProvider] = useState<string>("all")
  const [selectedModel, setSelectedModel] = useState<string>("all")
  const [selectedStatus, setSelectedStatus] = useState<string>("all")
  const [selectedServer, setSelectedServer] = useState<string>("all")
  const [selectedMethod, setSelectedMethod] = useState<string>("all")
  const [selectedTransport, setSelectedTransport] = useState<string>("all")

  // Parse active tab from subTab
  const activeTab = activeSubTab === "mcp" ? "mcp" : "llm"

  const handleTabChange = (tab: string) => {
    onTabChange("logs", tab)
  }

  // Extract unique values for filter dropdowns
  const uniqueClients = useMemo(() => {
    const clients = new Set(llmLogs.map((l) => l.api_key_name))
    return Array.from(clients).sort()
  }, [llmLogs])

  const uniqueProviders = useMemo(() => {
    const providers = new Set(llmLogs.map((l) => l.provider))
    return Array.from(providers).sort()
  }, [llmLogs])

  const uniqueModels = useMemo(() => {
    const models = new Set(llmLogs.map((l) => l.model))
    return Array.from(models).sort()
  }, [llmLogs])

  const uniqueMcpClients = useMemo(() => {
    const clients = new Set(mcpLogs.map((l) => l.client_id))
    return Array.from(clients).sort()
  }, [mcpLogs])

  const uniqueServers = useMemo(() => {
    const servers = new Set(mcpLogs.map((l) => l.server_id))
    return Array.from(servers).sort()
  }, [mcpLogs])

  const uniqueMethods = useMemo(() => {
    const methods = new Set(mcpLogs.map((l) => l.method))
    return Array.from(methods).sort()
  }, [mcpLogs])

  const uniqueTransports = useMemo(() => {
    const transports = new Set(mcpLogs.map((l) => l.transport))
    return Array.from(transports).sort()
  }, [mcpLogs])

  // Filter logs
  const filteredLlmLogs = useMemo(() => {
    return llmLogs.filter((log) => {
      // Search query filter
      if (searchQuery) {
        const query = searchQuery.toLowerCase()
        const matchesSearch =
          log.api_key_name.toLowerCase().includes(query) ||
          log.provider.toLowerCase().includes(query) ||
          log.model.toLowerCase().includes(query) ||
          log.request_id.toLowerCase().includes(query)
        if (!matchesSearch) return false
      }

      // Dropdown filters
      if (selectedClient !== "all" && log.api_key_name !== selectedClient)
        return false
      if (selectedProvider !== "all" && log.provider !== selectedProvider)
        return false
      if (selectedModel !== "all" && log.model !== selectedModel) return false
      if (selectedStatus !== "all" && log.status !== selectedStatus)
        return false

      return true
    })
  }, [
    llmLogs,
    searchQuery,
    selectedClient,
    selectedProvider,
    selectedModel,
    selectedStatus,
  ])

  const filteredMcpLogs = useMemo(() => {
    return mcpLogs.filter((log) => {
      // Search query filter
      if (searchQuery) {
        const query = searchQuery.toLowerCase()
        const matchesSearch =
          log.client_id.toLowerCase().includes(query) ||
          log.server_id.toLowerCase().includes(query) ||
          log.method.toLowerCase().includes(query) ||
          log.request_id.toLowerCase().includes(query)
        if (!matchesSearch) return false
      }

      // Dropdown filters
      if (selectedClient !== "all" && log.client_id !== selectedClient)
        return false
      if (selectedServer !== "all" && log.server_id !== selectedServer)
        return false
      if (selectedMethod !== "all" && log.method !== selectedMethod)
        return false
      if (selectedStatus !== "all" && log.status !== selectedStatus)
        return false
      if (selectedTransport !== "all" && log.transport !== selectedTransport)
        return false

      return true
    })
  }, [
    mcpLogs,
    searchQuery,
    selectedClient,
    selectedServer,
    selectedMethod,
    selectedStatus,
    selectedTransport,
  ])

  useEffect(() => {
    loadLogs()
  }, [limit])

  // Subscribe to real-time log events
  useEffect(() => {
    let llmUnlisten: (() => void) | undefined
    let mcpUnlisten: (() => void) | undefined

    listen<LLMLogEntry>("llm-log-entry", (event) => {
      setLlmLogs((prev) => {
        const newLogs = [event.payload, ...prev].slice(0, limit)
        return newLogs
      })
    }).then((unlisten) => {
      llmUnlisten = unlisten
    })

    listen<MCPLogEntry>("mcp-log-entry", (event) => {
      setMcpLogs((prev) => {
        const newLogs = [event.payload, ...prev].slice(0, limit)
        return newLogs
      })
    }).then((unlisten) => {
      mcpUnlisten = unlisten
    })

    return () => {
      if (llmUnlisten) llmUnlisten()
      if (mcpUnlisten) mcpUnlisten()
    }
  }, [limit])

  const loadLogs = async () => {
    setLoading(true)
    try {
      const [llm, mcp] = await Promise.all([
        invoke<LLMLogEntry[]>("get_llm_logs", { limit, offset: 0 }),
        invoke<MCPLogEntry[]>("get_mcp_logs", { limit, offset: 0 }),
      ])
      setLlmLogs(llm)
      setMcpLogs(mcp)
    } catch (error) {
      console.error("Failed to load logs:", error)
    } finally {
      setLoading(false)
    }
  }

  const clearFilters = () => {
    setSearchQuery("")
    setSelectedClient("all")
    setSelectedProvider("all")
    setSelectedModel("all")
    setSelectedStatus("all")
    setSelectedServer("all")
    setSelectedMethod("all")
    setSelectedTransport("all")
  }

  const hasActiveFilters =
    searchQuery ||
    selectedClient !== "all" ||
    selectedProvider !== "all" ||
    selectedModel !== "all" ||
    selectedStatus !== "all" ||
    selectedServer !== "all" ||
    selectedMethod !== "all" ||
    selectedTransport !== "all"

  const formatTimestamp = (timestamp: string) => {
    const date = new Date(timestamp)
    return date.toLocaleString("en-US", {
      month: "short",
      day: "numeric",
      hour: "numeric",
      minute: "2-digit",
      second: "2-digit",
      hour12: true,
    })
  }

  const formatCost = (cost: number) => {
    return `$${cost.toFixed(6)}`
  }

  const formatLatency = (latencyMs: number) => {
    if (latencyMs < 1000) {
      return `${latencyMs}ms`
    }
    return `${(latencyMs / 1000).toFixed(2)}s`
  }

  const getStatusBadge = (status: string) => {
    return (
      <Badge variant={status === "success" ? "success" : "destructive"}>
        {status}
      </Badge>
    )
  }

  return (
    <div className="space-y-4">
      <div className="flex items-center justify-between">
        <div>
          <h1 className="text-2xl font-bold tracking-tight">Logs</h1>
          <p className="text-sm text-muted-foreground">
            View and search LLM and MCP request logs
          </p>
        </div>
        <Select value={String(limit)} onValueChange={(v) => setLimit(Number(v))}>
          <SelectTrigger className="w-[120px]">
            <SelectValue />
          </SelectTrigger>
          <SelectContent>
            <SelectItem value="50">Last 50</SelectItem>
            <SelectItem value="100">Last 100</SelectItem>
            <SelectItem value="500">Last 500</SelectItem>
            <SelectItem value="1000">Last 1000</SelectItem>
          </SelectContent>
        </Select>
      </div>

      <Tabs value={activeTab} onValueChange={handleTabChange} className="space-y-4">
        <TabsList>
          <TabsTrigger value="llm">
            LLM Requests
            <Badge variant="secondary" className="ml-2">
              {filteredLlmLogs.length}
            </Badge>
          </TabsTrigger>
          <TabsTrigger value="mcp">
            MCP Requests
            <Badge variant="secondary" className="ml-2">
              {filteredMcpLogs.length}
            </Badge>
          </TabsTrigger>
        </TabsList>

        {/* Search and Filters */}
        <div className="space-y-3">
          <div className="flex gap-2">
            <div className="relative flex-1">
              <Search className="absolute left-3 top-1/2 h-4 w-4 -translate-y-1/2 text-muted-foreground" />
              <Input
                placeholder="Search logs..."
                value={searchQuery}
                onChange={(e) => setSearchQuery(e.target.value)}
                className="pl-9"
              />
              {searchQuery && (
                <button
                  onClick={() => setSearchQuery("")}
                  className="absolute right-3 top-1/2 -translate-y-1/2 text-muted-foreground hover:text-foreground"
                >
                  <X className="h-4 w-4" />
                </button>
              )}
            </div>
            {hasActiveFilters && (
              <Button variant="ghost" onClick={clearFilters}>
                Clear
              </Button>
            )}
          </div>

          <Card>
            <CardContent className="pt-4">
              {activeTab === "llm" ? (
                <div className="grid grid-cols-4 gap-4">
                  <div className="space-y-1.5">
                    <Label>Client</Label>
                    <Select
                      value={selectedClient}
                      onValueChange={setSelectedClient}
                    >
                      <SelectTrigger>
                        <SelectValue placeholder="All clients" />
                      </SelectTrigger>
                      <SelectContent>
                        <SelectItem value="all">All clients</SelectItem>
                        {uniqueClients.map((client) => (
                          <SelectItem key={client} value={client}>
                            {client}
                          </SelectItem>
                        ))}
                      </SelectContent>
                    </Select>
                  </div>
                  <div className="space-y-1.5">
                    <Label>Provider</Label>
                    <Select
                      value={selectedProvider}
                      onValueChange={setSelectedProvider}
                    >
                      <SelectTrigger>
                        <SelectValue placeholder="All providers" />
                      </SelectTrigger>
                      <SelectContent>
                        <SelectItem value="all">All providers</SelectItem>
                        {uniqueProviders.map((provider) => (
                          <SelectItem key={provider} value={provider}>
                            {provider}
                          </SelectItem>
                        ))}
                      </SelectContent>
                    </Select>
                  </div>
                  <div className="space-y-1.5">
                    <Label>Model</Label>
                    <Select
                      value={selectedModel}
                      onValueChange={setSelectedModel}
                    >
                      <SelectTrigger>
                        <SelectValue placeholder="All models" />
                      </SelectTrigger>
                      <SelectContent>
                        <SelectItem value="all">All models</SelectItem>
                        {uniqueModels.map((model) => (
                          <SelectItem key={model} value={model}>
                            {model}
                          </SelectItem>
                        ))}
                      </SelectContent>
                    </Select>
                  </div>
                  <div className="space-y-1.5">
                    <Label>Status</Label>
                    <Select
                      value={selectedStatus}
                      onValueChange={setSelectedStatus}
                    >
                      <SelectTrigger>
                        <SelectValue placeholder="All statuses" />
                      </SelectTrigger>
                      <SelectContent>
                        <SelectItem value="all">All statuses</SelectItem>
                        <SelectItem value="success">Success</SelectItem>
                        <SelectItem value="error">Error</SelectItem>
                      </SelectContent>
                    </Select>
                  </div>
                </div>
              ) : (
                <div className="grid grid-cols-5 gap-4">
                  <div className="space-y-1.5">
                    <Label>Client</Label>
                    <Select
                      value={selectedClient}
                      onValueChange={setSelectedClient}
                    >
                      <SelectTrigger>
                        <SelectValue placeholder="All clients" />
                      </SelectTrigger>
                      <SelectContent>
                        <SelectItem value="all">All clients</SelectItem>
                        {uniqueMcpClients.map((client) => (
                          <SelectItem key={client} value={client}>
                            {client}
                          </SelectItem>
                        ))}
                      </SelectContent>
                    </Select>
                  </div>
                  <div className="space-y-1.5">
                    <Label>Server</Label>
                    <Select
                      value={selectedServer}
                      onValueChange={setSelectedServer}
                    >
                      <SelectTrigger>
                        <SelectValue placeholder="All servers" />
                      </SelectTrigger>
                      <SelectContent>
                        <SelectItem value="all">All servers</SelectItem>
                        {uniqueServers.map((server) => (
                          <SelectItem key={server} value={server}>
                            {server}
                          </SelectItem>
                        ))}
                      </SelectContent>
                    </Select>
                  </div>
                  <div className="space-y-1.5">
                    <Label>Method</Label>
                    <Select
                      value={selectedMethod}
                      onValueChange={setSelectedMethod}
                    >
                      <SelectTrigger>
                        <SelectValue placeholder="All methods" />
                      </SelectTrigger>
                      <SelectContent>
                        <SelectItem value="all">All methods</SelectItem>
                        {uniqueMethods.map((method) => (
                          <SelectItem key={method} value={method}>
                            {method}
                          </SelectItem>
                        ))}
                      </SelectContent>
                    </Select>
                  </div>
                  <div className="space-y-1.5">
                    <Label>Transport</Label>
                    <Select
                      value={selectedTransport}
                      onValueChange={setSelectedTransport}
                    >
                      <SelectTrigger>
                        <SelectValue placeholder="All transports" />
                      </SelectTrigger>
                      <SelectContent>
                        <SelectItem value="all">All transports</SelectItem>
                        {uniqueTransports.map((transport) => (
                          <SelectItem key={transport} value={transport}>
                            {transport}
                          </SelectItem>
                        ))}
                      </SelectContent>
                    </Select>
                  </div>
                  <div className="space-y-1.5">
                    <Label>Status</Label>
                    <Select
                      value={selectedStatus}
                      onValueChange={setSelectedStatus}
                    >
                      <SelectTrigger>
                        <SelectValue placeholder="All statuses" />
                      </SelectTrigger>
                      <SelectContent>
                        <SelectItem value="all">All statuses</SelectItem>
                        <SelectItem value="success">Success</SelectItem>
                        <SelectItem value="error">Error</SelectItem>
                      </SelectContent>
                    </Select>
                  </div>
                </div>
              )}
            </CardContent>
          </Card>
        </div>

        <TabsContent value="llm">
          <Card>
            <ScrollArea className="h-[calc(100vh-320px)]">
              <div className="overflow-x-auto">
                <table className="min-w-full divide-y divide-border">
                  <thead className="bg-muted/50 sticky top-0">
                    <tr>
                      <th className="px-4 py-3 text-left text-xs font-medium text-muted-foreground uppercase tracking-wider">
                        Timestamp
                      </th>
                      <th className="px-4 py-3 text-left text-xs font-medium text-muted-foreground uppercase tracking-wider">
                        Client
                      </th>
                      <th className="px-4 py-3 text-left text-xs font-medium text-muted-foreground uppercase tracking-wider">
                        Provider
                      </th>
                      <th className="px-4 py-3 text-left text-xs font-medium text-muted-foreground uppercase tracking-wider">
                        Model
                      </th>
                      <th className="px-4 py-3 text-left text-xs font-medium text-muted-foreground uppercase tracking-wider">
                        Status
                      </th>
                      <th className="px-4 py-3 text-left text-xs font-medium text-muted-foreground uppercase tracking-wider">
                        Tokens
                      </th>
                      <th className="px-4 py-3 text-left text-xs font-medium text-muted-foreground uppercase tracking-wider">
                        Cost
                      </th>
                      <th className="px-4 py-3 text-left text-xs font-medium text-muted-foreground uppercase tracking-wider">
                        Latency
                      </th>
                    </tr>
                  </thead>
                  <tbody className="divide-y divide-border">
                    {filteredLlmLogs.length === 0 && !loading && (
                      <tr>
                        <td
                          colSpan={8}
                          className="px-4 py-12 text-center text-muted-foreground"
                        >
                          {hasActiveFilters
                            ? "No logs match your filters"
                            : "No LLM request logs found"}
                        </td>
                      </tr>
                    )}
                    {filteredLlmLogs.map((log, index) => (
                      <tr
                        key={`${log.request_id}-${index}`}
                        className="hover:bg-muted/50"
                      >
                        <td className="px-4 py-3 whitespace-nowrap text-sm">
                          {formatTimestamp(log.timestamp)}
                        </td>
                        <td className="px-4 py-3 whitespace-nowrap text-sm">
                          {log.api_key_name}
                        </td>
                        <td className="px-4 py-3 whitespace-nowrap text-sm">
                          {log.provider}
                        </td>
                        <td className="px-4 py-3 whitespace-nowrap text-sm text-muted-foreground">
                          {log.model}
                        </td>
                        <td className="px-4 py-3 whitespace-nowrap">
                          {getStatusBadge(log.status)}
                        </td>
                        <td className="px-4 py-3 whitespace-nowrap text-sm">
                          <div className="flex flex-col">
                            <span className="text-xs text-muted-foreground">
                              In: {log.input_tokens}
                            </span>
                            <span className="text-xs text-muted-foreground">
                              Out: {log.output_tokens}
                            </span>
                          </div>
                        </td>
                        <td className="px-4 py-3 whitespace-nowrap text-sm">
                          {formatCost(log.cost_usd)}
                        </td>
                        <td className="px-4 py-3 whitespace-nowrap text-sm">
                          {formatLatency(log.latency_ms)}
                        </td>
                      </tr>
                    ))}
                  </tbody>
                </table>
              </div>
            </ScrollArea>
          </Card>
        </TabsContent>

        <TabsContent value="mcp">
          <Card>
            <ScrollArea className="h-[calc(100vh-320px)]">
              <div className="overflow-x-auto">
                <table className="min-w-full divide-y divide-border">
                  <thead className="bg-muted/50 sticky top-0">
                    <tr>
                      <th className="px-4 py-3 text-left text-xs font-medium text-muted-foreground uppercase tracking-wider">
                        Timestamp
                      </th>
                      <th className="px-4 py-3 text-left text-xs font-medium text-muted-foreground uppercase tracking-wider">
                        Client
                      </th>
                      <th className="px-4 py-3 text-left text-xs font-medium text-muted-foreground uppercase tracking-wider">
                        Server
                      </th>
                      <th className="px-4 py-3 text-left text-xs font-medium text-muted-foreground uppercase tracking-wider">
                        Method
                      </th>
                      <th className="px-4 py-3 text-left text-xs font-medium text-muted-foreground uppercase tracking-wider">
                        Transport
                      </th>
                      <th className="px-4 py-3 text-left text-xs font-medium text-muted-foreground uppercase tracking-wider">
                        Status
                      </th>
                      <th className="px-4 py-3 text-left text-xs font-medium text-muted-foreground uppercase tracking-wider">
                        Latency
                      </th>
                    </tr>
                  </thead>
                  <tbody className="divide-y divide-border">
                    {filteredMcpLogs.length === 0 && !loading && (
                      <tr>
                        <td
                          colSpan={7}
                          className="px-4 py-12 text-center text-muted-foreground"
                        >
                          {hasActiveFilters
                            ? "No logs match your filters"
                            : "No MCP request logs found"}
                        </td>
                      </tr>
                    )}
                    {filteredMcpLogs.map((log, index) => (
                      <tr
                        key={`${log.request_id}-${index}`}
                        className="hover:bg-muted/50"
                      >
                        <td className="px-4 py-3 whitespace-nowrap text-sm">
                          {formatTimestamp(log.timestamp)}
                        </td>
                        <td className="px-4 py-3 whitespace-nowrap text-sm">
                          {log.client_id}
                        </td>
                        <td className="px-4 py-3 whitespace-nowrap text-sm">
                          {log.server_id}
                        </td>
                        <td className="px-4 py-3 whitespace-nowrap text-sm text-muted-foreground">
                          {log.method}
                        </td>
                        <td className="px-4 py-3 whitespace-nowrap">
                          <Badge variant="secondary">{log.transport}</Badge>
                        </td>
                        <td className="px-4 py-3 whitespace-nowrap">
                          <Badge
                            variant={
                              log.status === "success" ? "success" : "destructive"
                            }
                          >
                            {log.status}
                            {log.error_code && ` (${log.error_code})`}
                          </Badge>
                        </td>
                        <td className="px-4 py-3 whitespace-nowrap text-sm">
                          {formatLatency(log.latency_ms)}
                        </td>
                      </tr>
                    ))}
                  </tbody>
                </table>
              </div>
            </ScrollArea>
          </Card>
        </TabsContent>
      </Tabs>
    </div>
  )
}
