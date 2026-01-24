import { useEffect, useCallback, type SetStateAction } from "react"
import { HelpCircle, CheckCircle2, XCircle, Clock, Send, Bot } from "lucide-react"
import { Button } from "@/components/ui/Button"
import { Input } from "@/components/ui/Input"
import { Label } from "@/components/ui/label"
import { ScrollArea } from "@/components/ui/scroll-area"
import { Badge } from "@/components/ui/Badge"
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/Card"
import { Textarea } from "@/components/ui/textarea"
import { cn } from "@/lib/utils"
import type { PendingElicitationRequest, ElicitationState, CompletedElicitationRequest } from "./index"

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

interface ElicitationPanelProps {
  isConnected: boolean
  pendingRequests: PendingElicitationRequest[]
  onResolve: (id: string, result: { action: "accept" | "decline"; content?: Record<string, unknown> }) => void
  elicitationState: ElicitationState
  onElicitationStateChange: (state: SetStateAction<ElicitationState>) => void
}

export function ElicitationPanel({
  isConnected,
  pendingRequests,
  onResolve,
  elicitationState,
  onElicitationStateChange,
}: ElicitationPanelProps) {
  // Destructure lifted state
  const { completedRequests, selectedRequestId, formValues } = elicitationState

  // Helper to update partial state (using functional update to avoid infinite loops)
  const updateState = useCallback(
    (updates: Partial<ElicitationState>) => {
      onElicitationStateChange(prev => ({ ...prev, ...updates }))
    },
    [onElicitationStateChange]
  )

  // Find the selected request from either pending or completed
  const selectedPending = pendingRequests.find((r) => r.id === selectedRequestId)
  const selectedCompleted = completedRequests.find((r) => r.id === selectedRequestId)
  const selectedRequest = selectedPending || selectedCompleted

  const initializeFormValues = useCallback((params: PendingElicitationRequest["params"]) => {
    const defaults: Record<string, unknown> = {}
    // Handle form mode elicitation
    if ("requestedSchema" in params && params.requestedSchema) {
      const props = params.requestedSchema.properties || {}
      for (const [key, prop] of Object.entries(props)) {
        const typedProp = prop as SchemaProperty
        if (typedProp.default !== undefined) {
          defaults[key] = typedProp.default
        } else if (typedProp.type === "boolean") {
          defaults[key] = false
        } else if (typedProp.type === "number" || typedProp.type === "integer") {
          defaults[key] = typedProp.minimum ?? 0
        } else {
          defaults[key] = ""
        }
      }
    }
    updateState({ formValues: defaults })
  }, [updateState])

  // Auto-select first pending request if none selected
  useEffect(() => {
    if (!selectedRequest && pendingRequests.length > 0) {
      const firstPending = pendingRequests[0]
      updateState({ selectedRequestId: firstPending.id })
      initializeFormValues(firstPending.params)
    }
  }, [pendingRequests, selectedRequest, updateState, initializeFormValues])

  const handleSelectRequest = (request: PendingElicitationRequest | CompletedElicitationRequest) => {
    updateState({ selectedRequestId: request.id })
    if ("resolve" in request) {
      // It's a pending request
      initializeFormValues(request.params)
    } else if (request.response) {
      updateState({ formValues: request.response })
    }
  }

  const handleSubmit = async () => {
    if (!selectedPending) return

    onResolve(selectedPending.id, { action: "accept", content: formValues })

    // Move to completed
    const newCompleted: CompletedElicitationRequest[] = [
      {
        id: selectedPending.id,
        params: selectedPending.params,
        timestamp: selectedPending.timestamp,
        status: "submitted",
        response: formValues,
      },
      ...completedRequests,
    ]

    // Select next pending or clear selection
    const remainingPending = pendingRequests.filter((r) => r.id !== selectedPending.id)
    if (remainingPending.length > 0) {
      updateState({
        completedRequests: newCompleted,
        selectedRequestId: remainingPending[0].id,
      })
      initializeFormValues(remainingPending[0].params)
    } else {
      updateState({
        completedRequests: newCompleted,
        selectedRequestId: null,
      })
    }
  }

  const handleCancel = async () => {
    if (!selectedPending) return

    onResolve(selectedPending.id, { action: "decline" })

    // Move to completed
    const newCompleted: CompletedElicitationRequest[] = [
      {
        id: selectedPending.id,
        params: selectedPending.params,
        timestamp: selectedPending.timestamp,
        status: "cancelled",
      },
      ...completedRequests,
    ]

    // Select next pending or clear selection
    const remainingPending = pendingRequests.filter((r) => r.id !== selectedPending.id)
    if (remainingPending.length > 0) {
      updateState({
        completedRequests: newCompleted,
        selectedRequestId: remainingPending[0].id,
      })
      initializeFormValues(remainingPending[0].params)
    } else {
      updateState({
        completedRequests: newCompleted,
        selectedRequestId: null,
      })
    }
  }

  const clearHistory = () => {
    if (pendingRequests.length > 0) {
      updateState({
        completedRequests: [],
        selectedRequestId: pendingRequests[0].id,
      })
      initializeFormValues(pendingRequests[0].params)
    } else {
      updateState({
        completedRequests: [],
        selectedRequestId: null,
      })
    }
  }

  const getStatusBadge = (status: "pending" | "submitted" | "cancelled") => {
    switch (status) {
      case "pending":
        return (
          <Badge variant="outline" className="bg-yellow-50 text-yellow-700 border-yellow-200 dark:bg-yellow-950 dark:text-yellow-300 dark:border-yellow-800">
            <Clock className="h-3 w-3 mr-1" />
            Pending
          </Badge>
        )
      case "submitted":
        return (
          <Badge variant="outline" className="bg-green-50 text-green-700 border-green-200 dark:bg-green-950 dark:text-green-300 dark:border-green-800">
            <CheckCircle2 className="h-3 w-3 mr-1" />
            Submitted
          </Badge>
        )
      case "cancelled":
        return (
          <Badge variant="outline" className="bg-red-50 text-red-700 border-red-200 dark:bg-red-950 dark:text-red-300 dark:border-red-800">
            <XCircle className="h-3 w-3 mr-1" />
            Cancelled
          </Badge>
        )
    }
  }

  const setFormValuesLocal = (newValues: Record<string, unknown>) => {
    updateState({ formValues: newValues })
  }

  const renderFormField = (name: string, prop: SchemaProperty, _isRequired: boolean) => {
    const value = formValues[name]
    const disabled = !selectedPending

    if (prop.enum) {
      return (
        <select
          className="w-full p-2 border rounded-md bg-background disabled:opacity-50"
          value={value as string || ""}
          onChange={(e) => setFormValuesLocal({ ...formValues, [name]: e.target.value })}
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
            onChange={(e) => setFormValuesLocal({ ...formValues, [name]: e.target.checked })}
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
              setFormValuesLocal({
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
              onChange={(e) => setFormValuesLocal({ ...formValues, [name]: e.target.value })}
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
            onChange={(e) => setFormValuesLocal({ ...formValues, [name]: e.target.value })}
            placeholder={prop.description}
            minLength={prop.minLength}
            maxLength={prop.maxLength}
            disabled={disabled}
          />
        )
    }
  }

  // Get the schema from the selected request
  const getRequestSchema = (params: PendingElicitationRequest["params"]) => {
    if ("requestedSchema" in params && params.requestedSchema) {
      return params.requestedSchema
    }
    return null
  }

  // Get the message from the selected request
  const getRequestMessage = (params: PendingElicitationRequest["params"]) => {
    if ("message" in params) {
      return params.message
    }
    return null
  }

  // Combine pending and completed for display
  const allRequests = [
    ...pendingRequests.map((r) => ({ ...r, status: "pending" as const })),
    ...completedRequests,
  ].sort((a, b) => b.timestamp.getTime() - a.timestamp.getTime())

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
        <div className="w-80 flex-shrink-0 flex flex-col border rounded-lg">
          <div className="p-3 border-b flex items-center justify-between">
            <div className="flex items-center gap-2">
              <HelpCircle className="h-4 w-4" />
              <span className="font-medium text-sm">Elicitation Requests</span>
            </div>
            <Badge variant="secondary">{allRequests.length}</Badge>
          </div>

          <ScrollArea className="flex-1">
            {allRequests.length === 0 ? (
              <div className="p-4 text-sm text-muted-foreground text-center">
                <p>No elicitation requests yet</p>
                <p className="text-xs mt-1">
                  Requests from MCP servers will appear here
                </p>
              </div>
            ) : (
              <div className="p-2 space-y-1">
                {allRequests.map((request) => (
                  <button
                    key={request.id}
                    onClick={() => handleSelectRequest(request as PendingElicitationRequest | CompletedElicitationRequest)}
                    className={cn(
                      "w-full text-left p-2 rounded-md transition-colors",
                      "hover:bg-accent",
                      selectedRequestId === request.id && "bg-accent"
                    )}
                  >
                    <div className="flex items-center justify-between">
                      <span className="text-sm font-medium truncate">
                        Elicitation Request
                      </span>
                      {getStatusBadge(request.status)}
                    </div>
                    <p className="text-xs text-muted-foreground mt-1">
                      {request.timestamp.toLocaleTimeString()}
                    </p>
                    {getRequestMessage(request.params) && (
                      <p className="text-xs text-muted-foreground truncate mt-1">
                        {getRequestMessage(request.params)}
                      </p>
                    )}
                  </button>
                ))}
              </div>
            )}
          </ScrollArea>
        </div>

        {/* Right: Request Details & Form */}
        <div className="flex-1 min-w-0 flex flex-col border rounded-lg">
          {selectedRequest ? (
            <>
              <div className="p-4 border-b">
                <div className="flex items-center justify-between">
                  <div>
                    <h3 className="font-semibold">Elicitation Request</h3>
                    <p className="text-xs text-muted-foreground">
                      {selectedRequest.timestamp.toLocaleString()}
                    </p>
                  </div>
                  <div className="flex items-center gap-2">
                    {getStatusBadge(selectedPending ? "pending" : selectedCompleted?.status || "pending")}
                    {selectedPending && (
                      <>
                        <Button size="sm" onClick={handleSubmit}>
                          <Send className="h-4 w-4 mr-1" />
                          Submit
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
                  {getRequestMessage(selectedRequest.params) && (
                    <Card>
                      <CardHeader className="py-2">
                        <CardTitle className="text-sm flex items-center gap-2">
                          <Bot className="h-4 w-4" />
                          Server Request
                        </CardTitle>
                      </CardHeader>
                      <CardContent className="py-2">
                        <p className="text-sm">{getRequestMessage(selectedRequest.params)}</p>
                      </CardContent>
                    </Card>
                  )}

                  {/* Form Fields */}
                  {(() => {
                    const schema = getRequestSchema(selectedRequest.params)
                    if (!schema?.properties) return null
                    return (
                      <div className="space-y-4">
                        <h4 className="text-sm font-medium">Response Form</h4>
                        {Object.entries(schema.properties).map(
                          ([name, prop]) => {
                            const typedProp = prop as SchemaProperty
                            const isRequired = schema.required?.includes(name) ?? false
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
                                    {typedProp.type}
                                  </Badge>
                                </div>
                                {typedProp.description && (
                                  <p className="text-xs text-muted-foreground">
                                    {typedProp.description}
                                  </p>
                                )}
                                {renderFormField(name, typedProp, isRequired)}
                              </div>
                            )
                          }
                        )}
                      </div>
                    )
                  })()}

                  {/* Submitted Response */}
                  {selectedCompleted?.status === "submitted" && selectedCompleted.response && (
                    <div className="space-y-2">
                      <h4 className="text-sm font-medium">Submitted Response</h4>
                      <pre className="p-3 bg-muted rounded-md text-xs overflow-auto whitespace-pre-wrap break-all">
                        {JSON.stringify(selectedCompleted.response, null, 2)}
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
