import { useState, useEffect } from "react"
import { listen } from "@tauri-apps/api/event"
import { Radio, CheckCircle2, XCircle, Clock, RefreshCw, Bot, User, Settings } from "lucide-react"
import { Button } from "@/components/ui/Button"
import { Switch } from "@/components/ui/Toggle"
import { Label } from "@/components/ui/label"
import { ScrollArea } from "@/components/ui/scroll-area"
import { Badge } from "@/components/ui/Badge"
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/Card"
import { cn } from "@/lib/utils"

interface SamplingRequest {
  id: string
  timestamp: Date
  serverId: string
  serverName: string
  messages: Array<{
    role: "user" | "assistant"
    content: { type: string; text?: string }
  }>
  modelPreferences?: {
    hints?: Array<{ name?: string }>
    costPriority?: number
    speedPriority?: number
    intelligencePriority?: number
  }
  systemPrompt?: string
  includeContext?: string
  temperature?: number
  maxTokens?: number
  status: "pending" | "approved" | "rejected" | "completed" | "error"
  response?: {
    model: string
    content: { type: string; text?: string }
  }
  error?: string
}

interface SamplingPanelProps {
  serverPort: number | null
  clientToken: string | null
  isGateway: boolean
  selectedServer: string
  isConnected: boolean
}

export function SamplingPanel({
  serverPort: _serverPort,
  clientToken: _clientToken,
  isGateway: _isGateway,
  selectedServer: _selectedServer,
  isConnected,
}: SamplingPanelProps) {
  // Note: serverPort, clientToken, isGateway, selectedServer are reserved for future use
  void _serverPort
  void _clientToken
  void _isGateway
  void _selectedServer
  const [requests, setRequests] = useState<SamplingRequest[]>([])
  const [selectedRequest, setSelectedRequest] = useState<SamplingRequest | null>(null)
  const [autoApprove, setAutoApprove] = useState(false)

  // Listen for sampling requests from MCP servers
  useEffect(() => {
    if (!isConnected) {
      setRequests([])
      setSelectedRequest(null)
      return
    }

    const unsubscribe = listen<SamplingRequest>("mcp-sampling-request", (event) => {
      const request = {
        ...event.payload,
        timestamp: new Date(),
        status: autoApprove ? "completed" : "pending",
      } as SamplingRequest

      setRequests((prev) => [request, ...prev])

      // Auto-select the first pending request
      if (!autoApprove && !selectedRequest) {
        setSelectedRequest(request)
      }
    })

    return () => {
      unsubscribe.then((fn) => fn())
    }
  }, [isConnected, autoApprove, selectedRequest])

  const handleApprove = async (request: SamplingRequest) => {
    // In a real implementation, this would send approval to the backend
    setRequests((prev) =>
      prev.map((r) =>
        r.id === request.id ? { ...r, status: "approved" as const } : r
      )
    )
  }

  const handleReject = async (request: SamplingRequest) => {
    setRequests((prev) =>
      prev.map((r) =>
        r.id === request.id ? { ...r, status: "rejected" as const } : r
      )
    )
  }

  const clearHistory = () => {
    setRequests([])
    setSelectedRequest(null)
  }

  const getStatusBadge = (status: SamplingRequest["status"]) => {
    switch (status) {
      case "pending":
        return (
          <Badge variant="outline" className="bg-yellow-50 text-yellow-700 border-yellow-200">
            <Clock className="h-3 w-3 mr-1" />
            Pending
          </Badge>
        )
      case "approved":
        return (
          <Badge variant="outline" className="bg-blue-50 text-blue-700 border-blue-200">
            <RefreshCw className="h-3 w-3 mr-1 animate-spin" />
            Processing
          </Badge>
        )
      case "completed":
        return (
          <Badge variant="outline" className="bg-green-50 text-green-700 border-green-200">
            <CheckCircle2 className="h-3 w-3 mr-1" />
            Completed
          </Badge>
        )
      case "rejected":
        return (
          <Badge variant="outline" className="bg-red-50 text-red-700 border-red-200">
            <XCircle className="h-3 w-3 mr-1" />
            Rejected
          </Badge>
        )
      case "error":
        return (
          <Badge variant="destructive">
            <XCircle className="h-3 w-3 mr-1" />
            Error
          </Badge>
        )
    }
  }

  if (!isConnected) {
    return (
      <div className="flex items-center justify-center h-full text-muted-foreground">
        <p>Connect to an MCP server to monitor sampling requests</p>
      </div>
    )
  }

  return (
    <div className="flex flex-col h-full gap-4">
      {/* Controls */}
      <div className="flex items-center justify-between">
        <div className="flex items-center gap-4">
          <div className="flex items-center space-x-2">
            <Switch
              id="auto-approve"
              checked={autoApprove}
              onCheckedChange={setAutoApprove}
            />
            <Label htmlFor="auto-approve">Auto-approve sampling requests</Label>
          </div>
        </div>
        <Button variant="outline" size="sm" onClick={clearHistory}>
          Clear History
        </Button>
      </div>

      <div className="flex h-full gap-4 min-h-0">
        {/* Left: Request List */}
        <div className="w-80 flex flex-col border rounded-lg">
          <div className="p-3 border-b flex items-center justify-between">
            <div className="flex items-center gap-2">
              <Radio className="h-4 w-4" />
              <span className="font-medium text-sm">Sampling Requests</span>
            </div>
            <Badge variant="secondary">{requests.length}</Badge>
          </div>

          <ScrollArea className="flex-1">
            {requests.length === 0 ? (
              <div className="p-4 text-sm text-muted-foreground text-center">
                <p>No sampling requests yet</p>
                <p className="text-xs mt-1">
                  Requests from MCP servers will appear here
                </p>
              </div>
            ) : (
              <div className="p-2 space-y-1">
                {requests.map((request) => (
                  <button
                    key={request.id}
                    onClick={() => setSelectedRequest(request)}
                    className={cn(
                      "w-full text-left p-2 rounded-md transition-colors",
                      "hover:bg-accent",
                      selectedRequest?.id === request.id && "bg-accent"
                    )}
                  >
                    <div className="flex items-center justify-between">
                      <span className="text-sm font-medium truncate">
                        {request.serverName}
                      </span>
                      {getStatusBadge(request.status)}
                    </div>
                    <p className="text-xs text-muted-foreground mt-1">
                      {request.timestamp.toLocaleTimeString()}
                    </p>
                    {request.modelPreferences?.hints?.[0]?.name && (
                      <p className="text-xs text-muted-foreground truncate">
                        Model hint: {request.modelPreferences.hints[0].name}
                      </p>
                    )}
                  </button>
                ))}
              </div>
            )}
          </ScrollArea>
        </div>

        {/* Right: Request Details */}
        <div className="flex-1 flex flex-col border rounded-lg">
          {selectedRequest ? (
            <>
              <div className="p-4 border-b">
                <div className="flex items-center justify-between">
                  <div>
                    <h3 className="font-semibold">{selectedRequest.serverName}</h3>
                    <p className="text-xs text-muted-foreground">
                      {selectedRequest.timestamp.toLocaleString()}
                    </p>
                  </div>
                  <div className="flex items-center gap-2">
                    {getStatusBadge(selectedRequest.status)}
                    {selectedRequest.status === "pending" && (
                      <>
                        <Button
                          size="sm"
                          onClick={() => handleApprove(selectedRequest)}
                        >
                          <CheckCircle2 className="h-4 w-4 mr-1" />
                          Approve
                        </Button>
                        <Button
                          size="sm"
                          variant="destructive"
                          onClick={() => handleReject(selectedRequest)}
                        >
                          <XCircle className="h-4 w-4 mr-1" />
                          Reject
                        </Button>
                      </>
                    )}
                  </div>
                </div>
              </div>

              <ScrollArea className="flex-1 p-4">
                <div className="space-y-6">
                  {/* Request Messages */}
                  <div className="space-y-2">
                    <h4 className="text-sm font-medium">Request Messages</h4>
                    <div className="space-y-3">
                      {selectedRequest.messages.map((msg, idx) => (
                        <div
                          key={idx}
                          className={cn(
                            "flex gap-3",
                            msg.role === "user" ? "justify-end" : "justify-start"
                          )}
                        >
                          {msg.role === "assistant" && (
                            <div className="flex h-8 w-8 shrink-0 items-center justify-center rounded-full bg-muted">
                              <Bot className="h-4 w-4" />
                            </div>
                          )}
                          <div
                            className={cn(
                              "rounded-lg px-4 py-2 max-w-[80%]",
                              msg.role === "user"
                                ? "bg-primary text-primary-foreground"
                                : "bg-muted"
                            )}
                          >
                            <p className="text-sm whitespace-pre-wrap">
                              {msg.content.text || JSON.stringify(msg.content)}
                            </p>
                          </div>
                          {msg.role === "user" && (
                            <div className="flex h-8 w-8 shrink-0 items-center justify-center rounded-full bg-primary">
                              <User className="h-4 w-4 text-primary-foreground" />
                            </div>
                          )}
                        </div>
                      ))}
                    </div>
                  </div>

                  {/* Model Preferences */}
                  {selectedRequest.modelPreferences && (
                    <Card>
                      <CardHeader className="py-2">
                        <CardTitle className="text-sm flex items-center gap-2">
                          <Settings className="h-4 w-4" />
                          Model Preferences
                        </CardTitle>
                      </CardHeader>
                      <CardContent className="py-2 space-y-2">
                        {selectedRequest.modelPreferences.hints && selectedRequest.modelPreferences.hints.length > 0 && (
                          <div className="flex items-center gap-2">
                            <span className="text-xs text-muted-foreground">Hints:</span>
                            {selectedRequest.modelPreferences.hints.map((hint, idx) => (
                              <Badge key={idx} variant="secondary" className="text-xs">
                                {hint.name}
                              </Badge>
                            ))}
                          </div>
                        )}
                        <div className="flex gap-4 text-xs">
                          {selectedRequest.modelPreferences.costPriority !== undefined && (
                            <span>Cost: {selectedRequest.modelPreferences.costPriority}</span>
                          )}
                          {selectedRequest.modelPreferences.speedPriority !== undefined && (
                            <span>Speed: {selectedRequest.modelPreferences.speedPriority}</span>
                          )}
                          {selectedRequest.modelPreferences.intelligencePriority !== undefined && (
                            <span>Intelligence: {selectedRequest.modelPreferences.intelligencePriority}</span>
                          )}
                        </div>
                      </CardContent>
                    </Card>
                  )}

                  {/* Parameters */}
                  <div className="flex gap-4">
                    {selectedRequest.temperature !== undefined && (
                      <Badge variant="outline">Temperature: {selectedRequest.temperature}</Badge>
                    )}
                    {selectedRequest.maxTokens !== undefined && (
                      <Badge variant="outline">Max Tokens: {selectedRequest.maxTokens}</Badge>
                    )}
                  </div>

                  {/* System Prompt */}
                  {selectedRequest.systemPrompt && (
                    <div className="space-y-2">
                      <h4 className="text-sm font-medium">System Prompt</h4>
                      <pre className="p-3 bg-muted rounded-md text-xs whitespace-pre-wrap">
                        {selectedRequest.systemPrompt}
                      </pre>
                    </div>
                  )}

                  {/* Response */}
                  {selectedRequest.response && (
                    <div className="space-y-2">
                      <h4 className="text-sm font-medium flex items-center gap-2">
                        Response
                        <Badge variant="secondary" className="text-xs">
                          {selectedRequest.response.model}
                        </Badge>
                      </h4>
                      <div className="flex gap-3">
                        <div className="flex h-8 w-8 shrink-0 items-center justify-center rounded-full bg-muted">
                          <Bot className="h-4 w-4" />
                        </div>
                        <div className="rounded-lg px-4 py-2 bg-muted max-w-[80%]">
                          <p className="text-sm whitespace-pre-wrap">
                            {selectedRequest.response.content.text ||
                              JSON.stringify(selectedRequest.response.content)}
                          </p>
                        </div>
                      </div>
                    </div>
                  )}

                  {/* Error */}
                  {selectedRequest.error && (
                    <div className="p-3 bg-destructive/10 text-destructive rounded-md text-sm">
                      {selectedRequest.error}
                    </div>
                  )}
                </div>
              </ScrollArea>
            </>
          ) : (
            <div className="flex items-center justify-center h-full text-muted-foreground">
              <p>Select a request to view details</p>
            </div>
          )}
        </div>
      </div>
    </div>
  )
}
