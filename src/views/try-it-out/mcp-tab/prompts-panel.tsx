import { useState, useEffect, useCallback } from "react"
import { Search, Play, RefreshCw, ChevronRight, MessageSquare, User, Bot } from "lucide-react"
import { Button } from "@/components/ui/Button"
import { Input } from "@/components/ui/Input"
import { ScrollArea } from "@/components/ui/scroll-area"
import { Badge } from "@/components/ui/Badge"
import { Label } from "@/components/ui/label"
import { Textarea } from "@/components/ui/textarea"
import { cn } from "@/lib/utils"
import type { McpClientWrapper, Prompt, GetPromptResult } from "@/lib/mcp-client"

interface PromptsPanelProps {
  mcpClient: McpClientWrapper | null
  isConnected: boolean
}

export function PromptsPanel({
  mcpClient,
  isConnected,
}: PromptsPanelProps) {
  const [prompts, setPrompts] = useState<Prompt[]>([])
  const [filteredPrompts, setFilteredPrompts] = useState<Prompt[]>([])
  const [searchQuery, setSearchQuery] = useState("")
  const [selectedPrompt, setSelectedPrompt] = useState<Prompt | null>(null)
  const [isLoading, setIsLoading] = useState(false)
  const [isGetting, setIsGetting] = useState(false)
  const [argValues, setArgValues] = useState<Record<string, string>>({})
  const [result, setResult] = useState<GetPromptResult | null>(null)
  const [error, setError] = useState<string | null>(null)

  // Fetch prompts list using MCP SDK
  const fetchPrompts = useCallback(async () => {
    if (!mcpClient || !isConnected) return

    setIsLoading(true)
    setError(null)

    try {
      const promptsList = await mcpClient.listPrompts()
      setPrompts(promptsList)
      setFilteredPrompts(promptsList)
    } catch (err) {
      setError(err instanceof Error ? err.message : "Failed to fetch prompts")
    } finally {
      setIsLoading(false)
    }
  }, [mcpClient, isConnected])

  useEffect(() => {
    if (isConnected && mcpClient) {
      fetchPrompts()
    } else {
      setPrompts([])
      setFilteredPrompts([])
      setSelectedPrompt(null)
      setResult(null)
    }
  }, [isConnected, mcpClient, fetchPrompts])

  // Filter prompts by search query
  useEffect(() => {
    if (!searchQuery) {
      setFilteredPrompts(prompts)
    } else {
      const query = searchQuery.toLowerCase()
      setFilteredPrompts(
        prompts.filter(
          (p) =>
            p.name.toLowerCase().includes(query) ||
            p.description?.toLowerCase().includes(query)
        )
      )
    }
  }, [searchQuery, prompts])

  // Get prompt with arguments
  const getPrompt = async () => {
    if (!mcpClient || !selectedPrompt) return

    setIsGetting(true)
    setResult(null)
    setError(null)

    try {
      const response = await mcpClient.getPrompt(selectedPrompt.name, argValues)
      setResult(response)
    } catch (err) {
      setError(err instanceof Error ? err.message : "Failed to get prompt")
    } finally {
      setIsGetting(false)
    }
  }

  const handlePromptSelect = (prompt: Prompt) => {
    setSelectedPrompt(prompt)
    setArgValues({})
    setResult(null)
    setError(null)
  }

  const renderMessage = (msg: { role: string; content: unknown }, idx: number) => {
    const role = msg.role as "user" | "assistant"
    const content = msg.content

    // Extract text content
    let textContent = ""
    if (typeof content === "string") {
      textContent = content
    } else if (Array.isArray(content)) {
      textContent = content
        .filter((c: unknown) => typeof c === "object" && c !== null && "type" in c && (c as { type: string }).type === "text")
        .map((c: unknown) => (c as { text?: string }).text || "")
        .join("\n")
    } else if (typeof content === "object" && content !== null && "text" in content) {
      textContent = (content as { text: string }).text
    }

    return (
      <div
        key={idx}
        className={cn(
          "flex gap-3",
          role === "user" ? "justify-end" : "justify-start"
        )}
      >
        {role === "assistant" && (
          <div className="flex h-8 w-8 shrink-0 items-center justify-center rounded-full bg-muted">
            <Bot className="h-4 w-4" />
          </div>
        )}
        <div
          className={cn(
            "rounded-lg px-4 py-2 max-w-[80%]",
            role === "user"
              ? "bg-primary text-primary-foreground"
              : "bg-muted"
          )}
        >
          <p className="text-sm whitespace-pre-wrap">{textContent}</p>
        </div>
        {role === "user" && (
          <div className="flex h-8 w-8 shrink-0 items-center justify-center rounded-full bg-primary">
            <User className="h-4 w-4 text-primary-foreground" />
          </div>
        )}
      </div>
    )
  }

  if (!isConnected) {
    return (
      <div className="flex items-center justify-center h-full text-muted-foreground">
        <p>Connect to an MCP server to browse prompts</p>
      </div>
    )
  }

  return (
    <div className="flex h-full gap-4">
      {/* Left: Prompts list */}
      <div className="w-72 flex flex-col border rounded-lg">
        <div className="p-3 border-b">
          <div className="flex items-center gap-2 mb-2">
            <span className="font-medium text-sm">Prompts</span>
            <Badge variant="secondary">{prompts.length}</Badge>
            <Button
              variant="ghost"
              size="icon"
              className="h-6 w-6 ml-auto"
              onClick={fetchPrompts}
              disabled={isLoading}
            >
              <RefreshCw className={cn("h-3 w-3", isLoading && "animate-spin")} />
            </Button>
          </div>
          <div className="relative">
            <Search className="absolute left-2 top-2.5 h-4 w-4 text-muted-foreground" />
            <Input
              placeholder="Search prompts..."
              className="pl-8 h-9"
              value={searchQuery}
              onChange={(e) => setSearchQuery(e.target.value)}
            />
          </div>
        </div>

        <ScrollArea className="flex-1">
          {error && !selectedPrompt && (
            <div className="p-4 text-sm text-destructive">{error}</div>
          )}
          <div className="p-2">
            {filteredPrompts.map((prompt) => (
              <button
                key={prompt.name}
                onClick={() => handlePromptSelect(prompt)}
                className={cn(
                  "w-full text-left px-3 py-2 rounded-md text-sm transition-colors",
                  "hover:bg-accent",
                  selectedPrompt?.name === prompt.name && "bg-accent"
                )}
              >
                <div className="flex items-center gap-2">
                  <ChevronRight className="h-3 w-3 text-muted-foreground" />
                  <MessageSquare className="h-3 w-3 text-muted-foreground" />
                  <span className="font-medium truncate">{prompt.name}</span>
                </div>
                {prompt.description && (
                  <p className="text-xs text-muted-foreground truncate ml-8 mt-0.5">
                    {prompt.description}
                  </p>
                )}
                {prompt.arguments && prompt.arguments.length > 0 && (
                  <p className="text-xs text-muted-foreground ml-8 mt-0.5">
                    {prompt.arguments.length} argument(s)
                  </p>
                )}
              </button>
            ))}
          </div>
        </ScrollArea>
      </div>

      {/* Right: Prompt details and execution */}
      <div className="flex-1 flex flex-col border rounded-lg">
        {selectedPrompt ? (
          <>
            <div className="p-4 border-b">
              <h3 className="font-semibold">{selectedPrompt.name}</h3>
              {selectedPrompt.description && (
                <p className="text-sm text-muted-foreground mt-1">
                  {selectedPrompt.description}
                </p>
              )}
            </div>

            <ScrollArea className="flex-1 p-4">
              <div className="space-y-4">
                {/* Arguments form */}
                {selectedPrompt.arguments && selectedPrompt.arguments.length > 0 && (
                  <div className="space-y-4">
                    <h4 className="text-sm font-medium">Arguments</h4>
                    {selectedPrompt.arguments.map((arg) => (
                      <div key={arg.name} className="space-y-2">
                        <Label className="flex items-center gap-2">
                          {arg.name}
                          {arg.required && (
                            <span className="text-destructive">*</span>
                          )}
                        </Label>
                        {arg.description && (
                          <p className="text-xs text-muted-foreground">
                            {arg.description}
                          </p>
                        )}
                        <Textarea
                          placeholder={`Enter ${arg.name}...`}
                          value={argValues[arg.name] || ""}
                          onChange={(e) =>
                            setArgValues((prev) => ({
                              ...prev,
                              [arg.name]: e.target.value,
                            }))
                          }
                          rows={2}
                        />
                      </div>
                    ))}
                  </div>
                )}

                {/* Get prompt button */}
                <Button onClick={getPrompt} disabled={isGetting} className="w-full">
                  {isGetting ? (
                    <>
                      <RefreshCw className="h-4 w-4 mr-2 animate-spin" />
                      Getting...
                    </>
                  ) : (
                    <>
                      <Play className="h-4 w-4 mr-2" />
                      Get Prompt
                    </>
                  )}
                </Button>

                {/* Error */}
                {error && (
                  <div className="p-3 bg-destructive/10 text-destructive rounded-md text-sm">
                    {error}
                  </div>
                )}

                {/* Result messages */}
                {result && result.messages.length > 0 && (
                  <div className="space-y-4">
                    <h4 className="text-sm font-medium">Generated Messages</h4>
                    <div className="space-y-3">
                      {result.messages.map((msg, idx) => renderMessage(msg, idx))}
                    </div>
                  </div>
                )}
              </div>
            </ScrollArea>
          </>
        ) : (
          <div className="flex items-center justify-center h-full text-muted-foreground">
            <p>Select a prompt to view details and execute</p>
          </div>
        )}
      </div>
    </div>
  )
}
