import { useState, useEffect, useCallback } from "react"
import { listen, emit } from "@tauri-apps/api/event"
import { HelpCircle, CheckCircle2, XCircle, Clock, Send, Bot } from "lucide-react"
import { Button } from "@/components/ui/Button"
import { Input } from "@/components/ui/Input"
import { Label } from "@/components/ui/label"
import { ScrollArea } from "@/components/ui/scroll-area"
import { Badge } from "@/components/ui/Badge"
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/Card"
import { Textarea } from "@/components/ui/textarea"
import { cn } from "@/lib/utils"
import type { McpClientWrapper } from "@/lib/mcp-client"

interface SchemaProperty {
  type: string
  description?: string
  enum?: string[]
  default?: unknown
  minLength?: number
  maxLength?: number
  minimum?: number
  maximum?: number
}

interface ElicitationRequest {
  id: string
  timestamp: Date
  serverId: string
  serverName: string
  message: string
  requestedSchema: {
    type: string
    properties?: Record<string, SchemaProperty>
    required?: string[]
  }
  status: "pending" | "submitted" | "cancelled"
  response?: Record<string, unknown>
}

interface ElicitationPanelProps {
  mcpClient: McpClientWrapper | null
  isConnected: boolean
}

export function ElicitationPanel({ mcpClient: _mcpClient, isConnected }: ElicitationPanelProps) {
  // Note: mcpClient is reserved for future use when elicitation is fully implemented
  void _mcpClient

  const [requests, setRequests] = useState<ElicitationRequest[]>([])
  const [selectedRequest, setSelectedRequest] = useState<ElicitationRequest | null>(null)
  const [formValues, setFormValues] = useState<Record<string, unknown>>({})
  const [isSubmitting, setIsSubmitting] = useState(false)

  // Listen for elicitation requests from backend
  useEffect(() => {
    if (!isConnected) {
      setRequests([])
      setSelectedRequest(null)
      return
    }

    const unsubscribe = listen<Omit<ElicitationRequest, "timestamp" | "status">>("mcp-elicitation-request", (event) => {
      const request: ElicitationRequest = {
        ...event.payload,
        timestamp: new Date(),
        status: "pending",
      }

      setRequests((prev) => [request, ...prev])

      // Auto-select if no request selected
      if (!selectedRequest) {
        setSelectedRequest(request)
        initializeFormValues(request)
      }
    })

    return () => {
      unsubscribe.then((fn) => fn())
    }
  }, [isConnected, selectedRequest])

  const initializeFormValues = useCallback((request: ElicitationRequest) => {
    const defaults: Record<string, unknown> = {}
    const props = request.requestedSchema?.properties || {}
    for (const [key, prop] of Object.entries(props)) {
      if (prop.default !== undefined) {
        defaults[key] = prop.default
      } else if (prop.type === "boolean") {
        defaults[key] = false
      } else if (prop.type === "number" || prop.type === "integer") {
        defaults[key] = prop.minimum ?? 0
      } else {
        defaults[key] = ""
      }
    }
    setFormValues(defaults)
  }, [])

  const handleSelectRequest = useCallback((request: ElicitationRequest) => {
    setSelectedRequest(request)
    if (request.status === "pending") {
      initializeFormValues(request)
    } else if (request.response) {
      setFormValues(request.response)
    }
  }, [initializeFormValues])

  const handleSubmit = async () => {
    if (!selectedRequest) return

    setIsSubmitting(true)

    try {
      // Send response back to backend
      await emit("mcp-elicitation-response", {
        requestId: selectedRequest.id,
        serverId: selectedRequest.serverId,
        response: formValues,
      })

      // Update local state
      const updatedRequest = {
        ...selectedRequest,
        status: "submitted" as const,
        response: formValues,
      }

      setRequests((prev) =>
        prev.map((r) => (r.id === selectedRequest.id ? updatedRequest : r))
      )
      setSelectedRequest(updatedRequest)
    } catch (error) {
      console.error("Failed to submit elicitation response:", error)
    } finally {
      setIsSubmitting(false)
    }
  }

  const handleCancel = async () => {
    if (!selectedRequest) return

    try {
      // Notify backend of cancellation
      await emit("mcp-elicitation-cancelled", {
        requestId: selectedRequest.id,
        serverId: selectedRequest.serverId,
      })

      // Update local state
      const updatedRequest = {
        ...selectedRequest,
        status: "cancelled" as const,
      }

      setRequests((prev) =>
        prev.map((r) => (r.id === selectedRequest.id ? updatedRequest : r))
      )
      setSelectedRequest(updatedRequest)
    } catch (error) {
      console.error("Failed to cancel elicitation:", error)
    }
  }

  const clearHistory = () => {
    setRequests([])
    setSelectedRequest(null)
  }

  const getStatusBadge = (status: ElicitationRequest["status"]) => {
    switch (status) {
      case "pending":
        return (
          <Badge variant="outline" className="bg-yellow-50 text-yellow-700 border-yellow-200">
            <Clock className="h-3 w-3 mr-1" />
            Pending
          </Badge>
        )
      case "submitted":
        return (
          <Badge variant="outline" className="bg-green-50 text-green-700 border-green-200">
            <CheckCircle2 className="h-3 w-3 mr-1" />
            Submitted
          </Badge>
        )
      case "cancelled":
        return (
          <Badge variant="outline" className="bg-red-50 text-red-700 border-red-200">
            <XCircle className="h-3 w-3 mr-1" />
            Cancelled
          </Badge>
        )
    }
  }

  const renderFormField = (name: string, prop: SchemaProperty, _isRequired: boolean) => {
    const value = formValues[name]
    const disabled = selectedRequest?.status !== "pending"

    if (prop.enum) {
      return (
        <select
          className="w-full p-2 border rounded-md bg-background disabled:opacity-50"
          value={value as string || ""}
          onChange={(e) => setFormValues({ ...formValues, [name]: e.target.value })}
          disabled={disabled}
        >
          <option value="">Select...</option>
          {prop.enum.map((opt) => (
            <option key={opt} value={opt}>
              {opt}
            </option>
          ))}
        </select>
      )
    }

    switch (prop.type) {
      case "boolean":
        return (
          <input
            type="checkbox"
            checked={value as boolean || false}
            onChange={(e) => setFormValues({ ...formValues, [name]: e.target.checked })}
            disabled={disabled}
            className="h-4 w-4"
          />
        )
      case "number":
      case "integer":
        return (
          <Input
            type="number"
            value={value as number ?? ""}
            onChange={(e) =>
              setFormValues({
                ...formValues,
                [name]: e.target.value ? Number(e.target.value) : undefined,
              })
            }
            min={prop.minimum}
            max={prop.maximum}
            disabled={disabled}
          />
        )
      default:
        if (prop.maxLength && prop.maxLength > 100) {
          return (
            <Textarea
              value={value as string || ""}
              onChange={(e) => setFormValues({ ...formValues, [name]: e.target.value })}
              placeholder={prop.description}
              minLength={prop.minLength}
              maxLength={prop.maxLength}
              disabled={disabled}
              rows={3}
            />
          )
        }
        return (
          <Input
            value={value as string || ""}
            onChange={(e) => setFormValues({ ...formValues, [name]: e.target.value })}
            placeholder={prop.description}
            minLength={prop.minLength}
            maxLength={prop.maxLength}
            disabled={disabled}
          />
        )
    }
  }

  if (!isConnected) {
    return (
      <div className="flex items-center justify-center h-full text-muted-foreground">
        <p>Connect to an MCP server to handle elicitation requests</p>
      </div>
    )
  }

  return (
    <div className="flex flex-col h-full gap-4">
      {/* Controls */}
      <div className="flex items-center justify-between">
        <p className="text-sm text-muted-foreground">
          Elicitation requests allow MCP servers to ask for user input
        </p>
        <Button variant="outline" size="sm" onClick={clearHistory}>
          Clear History
        </Button>
      </div>

      <div className="flex h-full gap-4 min-h-0">
        {/* Left: Request List */}
        <div className="w-80 flex flex-col border rounded-lg">
          <div className="p-3 border-b flex items-center justify-between">
            <div className="flex items-center gap-2">
              <HelpCircle className="h-4 w-4" />
              <span className="font-medium text-sm">Elicitation Requests</span>
            </div>
            <Badge variant="secondary">{requests.length}</Badge>
          </div>

          <ScrollArea className="flex-1">
            {requests.length === 0 ? (
              <div className="p-4 text-sm text-muted-foreground text-center">
                <p>No elicitation requests yet</p>
                <p className="text-xs mt-1">
                  Requests from MCP servers will appear here
                </p>
              </div>
            ) : (
              <div className="p-2 space-y-1">
                {requests.map((request) => (
                  <button
                    key={request.id}
                    onClick={() => handleSelectRequest(request)}
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
                    <p className="text-xs text-muted-foreground truncate mt-1">
                      {request.message}
                    </p>
                  </button>
                ))}
              </div>
            )}
          </ScrollArea>
        </div>

        {/* Right: Request Details & Form */}
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
                        <Button size="sm" onClick={handleSubmit} disabled={isSubmitting}>
                          <Send className="h-4 w-4 mr-1" />
                          {isSubmitting ? "Submitting..." : "Submit"}
                        </Button>
                        <Button
                          size="sm"
                          variant="outline"
                          onClick={handleCancel}
                        >
                          <XCircle className="h-4 w-4 mr-1" />
                          Cancel
                        </Button>
                      </>
                    )}
                  </div>
                </div>
              </div>

              <ScrollArea className="flex-1 p-4">
                <div className="space-y-6">
                  {/* Server Message */}
                  <Card>
                    <CardHeader className="py-2">
                      <CardTitle className="text-sm flex items-center gap-2">
                        <Bot className="h-4 w-4" />
                        Server Request
                      </CardTitle>
                    </CardHeader>
                    <CardContent className="py-2">
                      <p className="text-sm">{selectedRequest.message}</p>
                    </CardContent>
                  </Card>

                  {/* Form Fields */}
                  {selectedRequest.requestedSchema?.properties && (
                    <div className="space-y-4">
                      <h4 className="text-sm font-medium">Response Form</h4>
                      {Object.entries(selectedRequest.requestedSchema.properties).map(
                        ([name, prop]) => {
                          const isRequired =
                            selectedRequest.requestedSchema?.required?.includes(name) ?? false
                          return (
                            <div key={name} className="space-y-2">
                              <div className="flex items-center gap-2">
                                <Label className="font-mono text-sm">{name}</Label>
                                {isRequired && (
                                  <Badge variant="outline" className="text-xs">
                                    required
                                  </Badge>
                                )}
                                <Badge variant="secondary" className="text-xs">
                                  {prop.type}
                                </Badge>
                              </div>
                              {prop.description && (
                                <p className="text-xs text-muted-foreground">
                                  {prop.description}
                                </p>
                              )}
                              {renderFormField(name, prop, isRequired)}
                            </div>
                          )
                        }
                      )}
                    </div>
                  )}

                  {/* Submitted Response */}
                  {selectedRequest.status === "submitted" && selectedRequest.response && (
                    <div className="space-y-2">
                      <h4 className="text-sm font-medium">Submitted Response</h4>
                      <pre className="p-3 bg-muted rounded-md text-xs overflow-auto">
                        {JSON.stringify(selectedRequest.response, null, 2)}
                      </pre>
                    </div>
                  )}
                </div>
              </ScrollArea>
            </>
          ) : (
            <div className="flex items-center justify-center h-full text-muted-foreground">
              <p>Select a request to view details and respond</p>
            </div>
          )}
        </div>
      </div>
    </div>
  )
}
