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

export function ToolsPanel({
  mcpClient,
  isConnected,
}: ToolsPanelProps) {
  const [tools, setTools] = useState<Tool[]>([])
  const [filteredTools, setFilteredTools] = useState<Tool[]>([])
  const [searchQuery, setSearchQuery] = useState("")
  const [selectedTool, setSelectedTool] = useState<Tool | null>(null)
  const [isLoading, setIsLoading] = useState(false)
  const [isExecuting, setIsExecuting] = useState(false)
  const [formValues, setFormValues] = useState<Record<string, unknown>>({})
  const [result, setResult] = useState<{ success: boolean; data: unknown } | null>(null)
  const [error, setError] = useState<string | null>(null)

  // Fetch tools list using MCP SDK
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
    if (isConnected && mcpClient) {
      fetchTools()
    } else {
      setTools([])
      setFilteredTools([])
      setSelectedTool(null)
    }
  }, [isConnected, mcpClient, fetchTools])

  // Filter tools by search query
  useEffect(() => {
    if (!searchQuery) {
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

  // Execute tool using MCP SDK
  const executeTool = async () => {
    if (!mcpClient || !selectedTool) return

    setIsExecuting(true)
    setResult(null)
    setError(null)

    try {
      const response = await mcpClient.callTool(selectedTool.name, formValues)
      setResult({
        success: !response.isError,
        data: response.content,
      })
    } catch (err) {
      setError(err instanceof Error ? err.message : "Failed to execute tool")
    } finally {
      setIsExecuting(false)
    }
  }

  const handleToolSelect = (tool: Tool) => {
    setSelectedTool(tool)
    setFormValues({})
    setResult(null)
    setError(null)
  }

  const renderFormField = (name: string, schema: SchemaProperty) => {
    const value = formValues[name] ?? schema.default ?? ""

    if (schema.enum) {
      return (
        <select
          className="flex h-9 w-full rounded-md border border-input bg-transparent px-3 py-1 text-sm"
          value={String(value)}
          onChange={(e) =>
            setFormValues((prev) => ({ ...prev, [name]: e.target.value }))
          }
        >
          <option value="">Select...</option>
          {schema.enum.map((opt) => (
            <option key={opt} value={opt}>
              {opt}
            </option>
          ))}
        </select>
      )
    }

    if (schema.type === "boolean") {
      return (
        <input
          type="checkbox"
          checked={Boolean(value)}
          onChange={(e) =>
            setFormValues((prev) => ({ ...prev, [name]: e.target.checked }))
          }
          className="h-4 w-4"
        />
      )
    }

    if (schema.type === "number" || schema.type === "integer") {
      return (
        <Input
          type="number"
          value={String(value)}
          onChange={(e) =>
            setFormValues((prev) => ({
              ...prev,
              [name]: e.target.value ? Number(e.target.value) : undefined,
            }))
          }
        />
      )
    }

    if (schema.type === "array" || schema.type === "object") {
      return (
        <Textarea
          placeholder="Enter JSON..."
          value={typeof value === "string" ? value : JSON.stringify(value, null, 2)}
          onChange={(e) => {
            try {
              const parsed = JSON.parse(e.target.value)
              setFormValues((prev) => ({ ...prev, [name]: parsed }))
            } catch {
              setFormValues((prev) => ({ ...prev, [name]: e.target.value }))
            }
          }}
          rows={3}
        />
      )
    }

    // Default: string input
    return (
      <Input
        value={String(value)}
        onChange={(e) =>
          setFormValues((prev) => ({ ...prev, [name]: e.target.value }))
        }
      />
    )
  }

  if (!isConnected) {
    return (
      <div className="flex items-center justify-center h-full text-muted-foreground">
        <p>Connect to an MCP server to browse tools</p>
      </div>
    )
  }

  return (
    <div className="flex h-full gap-4">
      {/* Left: Tools list */}
      <div className="w-72 flex flex-col border rounded-lg">
        <div className="p-3 border-b">
          <div className="flex items-center gap-2 mb-2">
            <span className="font-medium text-sm">Tools</span>
            <Badge variant="secondary">{tools.length}</Badge>
            <Button
              variant="ghost"
              size="icon"
              className="h-6 w-6 ml-auto"
              onClick={fetchTools}
              disabled={isLoading}
            >
              <RefreshCw className={cn("h-3 w-3", isLoading && "animate-spin")} />
            </Button>
          </div>
          <div className="relative">
            <Search className="absolute left-2 top-2.5 h-4 w-4 text-muted-foreground" />
            <Input
              placeholder="Search tools..."
              className="pl-8 h-9"
              value={searchQuery}
              onChange={(e) => setSearchQuery(e.target.value)}
            />
          </div>
        </div>

        <ScrollArea className="flex-1">
          {error && !selectedTool && (
            <div className="p-4 text-sm text-destructive">{error}</div>
          )}
          <div className="p-2">
            {filteredTools.map((tool) => (
              <button
                key={tool.name}
                onClick={() => handleToolSelect(tool)}
                className={cn(
                  "w-full text-left px-3 py-2 rounded-md text-sm transition-colors",
                  "hover:bg-accent",
                  selectedTool?.name === tool.name && "bg-accent"
                )}
              >
                <div className="flex items-center gap-2">
                  <ChevronRight className="h-3 w-3 text-muted-foreground flex-shrink-0" />
                  <span className="font-medium truncate">
                    {tool.description || tool.name}
                  </span>
                </div>
                {tool.description && (
                  <p className="text-xs text-muted-foreground truncate ml-5 mt-0.5 font-mono">
                    {tool.name}
                  </p>
                )}
              </button>
            ))}
          </div>
        </ScrollArea>
      </div>

      {/* Right: Tool details and execution */}
      <div className="flex-1 flex flex-col border rounded-lg">
        {selectedTool ? (
          <>
            <div className="p-4 border-b">
              <h3 className="font-semibold">
                {selectedTool.description || selectedTool.name}
              </h3>
              {selectedTool.description && (
                <p className="text-xs text-muted-foreground mt-1 font-mono">
                  {selectedTool.name}
                </p>
              )}
            </div>

            <ScrollArea className="flex-1 p-4">
              <div className="space-y-4">
                {/* Input form */}
                {selectedTool.inputSchema?.properties && (
                  <div className="space-y-4">
                    <h4 className="text-sm font-medium">Parameters</h4>
                    {Object.entries(selectedTool.inputSchema.properties).map(
                      ([name, schema]) => (
                        <div key={name} className="space-y-2">
                          <Label className="flex items-center gap-2">
                            {name}
                            {selectedTool.inputSchema?.required?.includes(name) && (
                              <span className="text-destructive">*</span>
                            )}
                          </Label>
                          {(schema as SchemaProperty).description && (
                            <p className="text-xs text-muted-foreground">
                              {(schema as SchemaProperty).description}
                            </p>
                          )}
                          {renderFormField(name, schema as SchemaProperty)}
                        </div>
                      )
                    )}
                  </div>
                )}

                {/* Execute button */}
                <Button onClick={executeTool} disabled={isExecuting} className="w-full">
                  {isExecuting ? (
                    <>
                      <RefreshCw className="h-4 w-4 mr-2 animate-spin" />
                      Executing...
                    </>
                  ) : (
                    <>
                      <Play className="h-4 w-4 mr-2" />
                      Execute Tool
                    </>
                  )}
                </Button>

                {/* Results */}
                {(result || error) && (
                  <div className="space-y-2">
                    <h4 className="text-sm font-medium flex items-center gap-2">
                      Result
                      {result?.success && (
                        <CheckCircle2 className="h-4 w-4 text-green-500" />
                      )}
                      {(error || !result?.success) && (
                        <AlertCircle className="h-4 w-4 text-destructive" />
                      )}
                    </h4>
                    {error ? (
                      <pre className="p-3 bg-destructive/10 text-destructive rounded-md text-xs overflow-auto">
                        {error}
                      </pre>
                    ) : (
                      <pre className="p-3 bg-muted rounded-md text-xs overflow-auto max-h-64">
                        {JSON.stringify(result?.data, null, 2)}
                      </pre>
                    )}
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
