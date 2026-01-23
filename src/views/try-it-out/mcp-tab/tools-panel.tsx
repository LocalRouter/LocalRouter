import { useState, useEffect, useCallback } from "react"
import { Search, Play, RefreshCw, ChevronRight, AlertCircle, CheckCircle2 } from "lucide-react"
import { Button } from "@/components/ui/Button"
import { Input } from "@/components/ui/Input"
import { ScrollArea } from "@/components/ui/scroll-area"
import { Badge } from "@/components/ui/Badge"
import { Textarea } from "@/components/ui/textarea"
import { Label } from "@/components/ui/label"
import { cn } from "@/lib/utils"
import type { McpClientWrapper, Tool } from "@/lib/mcp-client"

interface SchemaProperty {
  type: string
  description?: string
  enum?: string[]
  default?: unknown
  items?: SchemaProperty
}

interface ToolsPanelProps {
  mcpClient: McpClientWrapper | null
  isConnected: boolean
}

export function ToolsPanel({ mcpClient, isConnected }: ToolsPanelProps) {
  const [tools, setTools] = useState<Tool[]>([])
  const [filteredTools, setFilteredTools] = useState<Tool[]>([])
  const [searchQuery, setSearchQuery] = useState("")
  const [selectedTool, setSelectedTool] = useState<Tool | null>(null)
  const [isLoading, setIsLoading] = useState(false)
  const [isExecuting, setIsExecuting] = useState(false)
  const [formValues, setFormValues] = useState<Record<string, unknown>>({})
  const [result, setResult] = useState<{ success: boolean; data: unknown } | null>(null)
  const [error, setError] = useState<string | null>(null)

  // Fetch tools list using MCP client
  const fetchTools = useCallback(async () => {
    if (!mcpClient || !isConnected) return

    setIsLoading(true)
    setError(null)

    try {
      const toolsList = await mcpClient.listTools()
      setTools(toolsList)
      setFilteredTools(toolsList)
    } catch (err) {
      setError(err instanceof Error ? err.message : "Failed to fetch tools")
    } finally {
      setIsLoading(false)
    }
  }, [mcpClient, isConnected])

  useEffect(() => {
    if (isConnected) {
      fetchTools()
    } else {
      setTools([])
      setFilteredTools([])
      setSelectedTool(null)
    }
  }, [isConnected, fetchTools])

  // Filter tools based on search
  useEffect(() => {
    if (!searchQuery.trim()) {
      setFilteredTools(tools)
    } else {
      const query = searchQuery.toLowerCase()
      setFilteredTools(
        tools.filter(
          (t) =>
            t.name.toLowerCase().includes(query) ||
            t.description?.toLowerCase().includes(query)
        )
      )
    }
  }, [searchQuery, tools])

  // Reset form when tool changes
  useEffect(() => {
    if (selectedTool) {
      const defaults: Record<string, unknown> = {}
      const schema = selectedTool.inputSchema as { properties?: Record<string, SchemaProperty> } | undefined
      const props = schema?.properties || {}
      for (const [key, prop] of Object.entries(props)) {
        if (prop.default !== undefined) {
          defaults[key] = prop.default
        }
      }
      setFormValues(defaults)
      setResult(null)
    }
  }, [selectedTool])

  const handleExecute = async () => {
    if (!selectedTool || !mcpClient) return

    setIsExecuting(true)
    setResult(null)
    setError(null)

    try {
      const callResult = await mcpClient.callTool(selectedTool.name, formValues)
      setResult({
        success: !callResult.isError,
        data: callResult.content,
      })
    } catch (err) {
      setResult({
        success: false,
        data: err instanceof Error ? err.message : "Execution failed",
      })
    } finally {
      setIsExecuting(false)
    }
  }

  const renderFormField = (name: string, prop: SchemaProperty) => {
    const value = formValues[name]

    if (prop.enum) {
      return (
        <select
          className="w-full p-2 border rounded-md bg-background"
          value={value as string || ""}
          onChange={(e) => setFormValues({ ...formValues, [name]: e.target.value })}
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
            className="h-4 w-4"
          />
        )
      case "number":
      case "integer":
        return (
          <Input
            type="number"
            value={value as number || ""}
            onChange={(e) =>
              setFormValues({ ...formValues, [name]: e.target.value ? Number(e.target.value) : undefined })
            }
          />
        )
      case "array":
      case "object":
        return (
          <Textarea
            value={typeof value === "string" ? value : JSON.stringify(value || "", null, 2)}
            onChange={(e) => {
              try {
                const parsed = JSON.parse(e.target.value)
                setFormValues({ ...formValues, [name]: parsed })
              } catch {
                setFormValues({ ...formValues, [name]: e.target.value })
              }
            }}
            placeholder={`Enter ${prop.type} as JSON`}
            rows={3}
          />
        )
      default:
        return (
          <Input
            value={value as string || ""}
            onChange={(e) => setFormValues({ ...formValues, [name]: e.target.value })}
            placeholder={prop.description}
          />
        )
    }
  }

  const renderResultContent = (data: unknown): string => {
    if (Array.isArray(data)) {
      return data.map(item => {
        if (typeof item === "object" && item !== null && "text" in item) {
          return (item as { text: string }).text
        }
        return JSON.stringify(item, null, 2)
      }).join("\n")
    }
    if (typeof data === "string") {
      return data
    }
    return JSON.stringify(data, null, 2)
  }

  if (!isConnected) {
    return (
      <div className="flex items-center justify-center h-full text-muted-foreground">
        <p>Connect to an MCP server to view tools</p>
      </div>
    )
  }

  const toolSchema = selectedTool?.inputSchema as { properties?: Record<string, SchemaProperty>; required?: string[] } | undefined

  return (
    <div className="flex h-full gap-4">
      {/* Left: Tools List */}
      <div className="w-80 flex flex-col border rounded-lg">
        <div className="p-3 border-b flex items-center gap-2">
          <Search className="h-4 w-4 text-muted-foreground" />
          <Input
            placeholder="Search tools..."
            value={searchQuery}
            onChange={(e) => setSearchQuery(e.target.value)}
            className="h-8 border-0 p-0 focus-visible:ring-0"
          />
          <Button
            variant="ghost"
            size="icon"
            className="h-8 w-8"
            onClick={fetchTools}
            disabled={isLoading}
          >
            <RefreshCw className={cn("h-4 w-4", isLoading && "animate-spin")} />
          </Button>
        </div>

        <ScrollArea className="flex-1">
          {error ? (
            <div className="p-4 text-sm text-destructive flex items-center gap-2">
              <AlertCircle className="h-4 w-4" />
              {error}
            </div>
          ) : filteredTools.length === 0 ? (
            <div className="p-4 text-sm text-muted-foreground text-center">
              {isLoading ? "Loading tools..." : "No tools available"}
            </div>
          ) : (
            <div className="p-2 space-y-1">
              {filteredTools.map((tool) => (
                <button
                  key={tool.name}
                  onClick={() => setSelectedTool(tool)}
                  className={cn(
                    "w-full text-left p-2 rounded-md transition-colors",
                    "hover:bg-accent",
                    selectedTool?.name === tool.name && "bg-accent"
                  )}
                >
                  <div className="flex items-center gap-2">
                    <span className="font-mono text-sm truncate">{tool.name}</span>
                    <ChevronRight className="h-3 w-3 ml-auto text-muted-foreground" />
                  </div>
                  {tool.description && (
                    <p className="text-xs text-muted-foreground truncate mt-1">
                      {tool.description}
                    </p>
                  )}
                </button>
              ))}
            </div>
          )}
        </ScrollArea>

        <div className="p-2 border-t text-xs text-muted-foreground text-center">
          {filteredTools.length} tool{filteredTools.length !== 1 ? "s" : ""}
        </div>
      </div>

      {/* Right: Tool Details & Execution */}
      <div className="flex-1 flex flex-col border rounded-lg">
        {selectedTool ? (
          <>
            <div className="p-4 border-b">
              <div className="flex items-center justify-between">
                <h3 className="font-mono font-semibold">{selectedTool.name}</h3>
                <Button onClick={handleExecute} disabled={isExecuting}>
                  {isExecuting ? (
                    <RefreshCw className="h-4 w-4 mr-2 animate-spin" />
                  ) : (
                    <Play className="h-4 w-4 mr-2" />
                  )}
                  Execute
                </Button>
              </div>
              {selectedTool.description && (
                <p className="text-sm text-muted-foreground mt-1">
                  {selectedTool.description}
                </p>
              )}
            </div>

            <ScrollArea className="flex-1 p-4">
              <div className="space-y-6">
                {/* Input Form */}
                {toolSchema?.properties && (
                  <div className="space-y-4">
                    <h4 className="text-sm font-medium">Arguments</h4>
                    {Object.entries(toolSchema.properties).map(
                      ([name, prop]) => (
                        <div key={name} className="space-y-2">
                          <div className="flex items-center gap-2">
                            <Label className="font-mono text-sm">{name}</Label>
                            {toolSchema.required?.includes(name) && (
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
                          {renderFormField(name, prop)}
                        </div>
                      )
                    )}
                  </div>
                )}

                {/* Result */}
                {result && (
                  <div className="space-y-2">
                    <div className="flex items-center gap-2">
                      <h4 className="text-sm font-medium">Result</h4>
                      {result.success ? (
                        <CheckCircle2 className="h-4 w-4 text-green-500" />
                      ) : (
                        <AlertCircle className="h-4 w-4 text-destructive" />
                      )}
                    </div>
                    <pre className="p-3 bg-muted rounded-md text-sm overflow-auto max-h-64">
                      {renderResultContent(result.data)}
                    </pre>
                  </div>
                )}
              </div>
            </ScrollArea>
          </>
        ) : (
          <div className="flex items-center justify-center h-full text-muted-foreground">
            <p>Select a tool to view details and execute</p>
          </div>
        )}
      </div>
    </div>
  )
}
