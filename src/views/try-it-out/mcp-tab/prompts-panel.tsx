import { useState, useEffect, useCallback } from "react"
import { Search, Play, RefreshCw, ChevronRight, AlertCircle, MessageSquare, User, Bot } from "lucide-react"
import { Button } from "@/components/ui/Button"
import { Input } from "@/components/ui/Input"
import { ScrollArea } from "@/components/ui/scroll-area"
import { Badge } from "@/components/ui/Badge"
import { Label } from "@/components/ui/label"
import { Textarea } from "@/components/ui/textarea"
import { cn } from "@/lib/utils"
import type { McpClientWrapper, Prompt } from "@/lib/mcp-client"

interface PromptMessage {
  role: "user" | "assistant"
  content: {
    type: string
    text?: string
  }
}

interface PromptsPanelProps {
  mcpClient: McpClientWrapper | null
  isConnected: boolean
}

export function PromptsPanel({ mcpClient, isConnected }: PromptsPanelProps) {
  const [prompts, setPrompts] = useState<Prompt[]>([])
  const [filteredPrompts, setFilteredPrompts] = useState<Prompt[]>([])
  const [searchQuery, setSearchQuery] = useState("")
  const [selectedPrompt, setSelectedPrompt] = useState<Prompt | null>(null)
  const [isLoading, setIsLoading] = useState(false)
  const [isExpanding, setIsExpanding] = useState(false)
  const [argValues, setArgValues] = useState<Record<string, string>>({})
  const [expandedMessages, setExpandedMessages] = useState<PromptMessage[]>([])
  const [error, setError] = useState<string | null>(null)

  // Fetch prompts list using MCP client
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
    if (isConnected) {
      fetchPrompts()
    } else {
      setPrompts([])
      setFilteredPrompts([])
      setSelectedPrompt(null)
      setExpandedMessages([])
    }
  }, [isConnected, fetchPrompts])

  // Filter prompts based on search
  useEffect(() => {
    if (!searchQuery.trim()) {
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

  // Reset form when prompt changes
  useEffect(() => {
    if (selectedPrompt) {
      const defaults: Record<string, string> = {}
      for (const arg of selectedPrompt.arguments || []) {
        defaults[arg.name] = ""
      }
      setArgValues(defaults)
      setExpandedMessages([])
    }
  }, [selectedPrompt])

  const handleExpand = async () => {
    if (!selectedPrompt || !mcpClient) return

    setIsExpanding(true)
    setExpandedMessages([])
    setError(null)

    try {
      const result = await mcpClient.getPrompt(selectedPrompt.name, argValues)
      const messages = (result.messages || []).map(msg => ({
        role: msg.role as "user" | "assistant",
        content: msg.content as { type: string; text?: string },
      }))
      setExpandedMessages(messages)
    } catch (err) {
      setError(err instanceof Error ? err.message : "Failed to get prompt")
    } finally {
      setIsExpanding(false)
    }
  }

  if (!isConnected) {
    return (
      <div className="flex items-center justify-center h-full text-muted-foreground">
        <p>Connect to an MCP server to view prompts</p>
      </div>
    )
  }

  return (
    <div className="flex h-full gap-4">
      {/* Left: Prompts List */}
      <div className="w-80 flex flex-col border rounded-lg">
        <div className="p-3 border-b flex items-center gap-2">
          <Search className="h-4 w-4 text-muted-foreground" />
          <Input
            placeholder="Search prompts..."
            value={searchQuery}
            onChange={(e) => setSearchQuery(e.target.value)}
            className="h-8 border-0 p-0 focus-visible:ring-0"
          />
          <Button
            variant="ghost"
            size="icon"
            className="h-8 w-8"
            onClick={fetchPrompts}
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
          ) : filteredPrompts.length === 0 ? (
            <div className="p-4 text-sm text-muted-foreground text-center">
              {isLoading ? "Loading prompts..." : "No prompts available"}
            </div>
          ) : (
            <div className="p-2 space-y-1">
              {filteredPrompts.map((prompt) => (
                <button
                  key={prompt.name}
                  onClick={() => setSelectedPrompt(prompt)}
                  className={cn(
                    "w-full text-left p-2 rounded-md transition-colors",
                    "hover:bg-accent",
                    selectedPrompt?.name === prompt.name && "bg-accent"
                  )}
                >
                  <div className="flex items-center gap-2">
                    <MessageSquare className="h-4 w-4 text-muted-foreground" />
                    <span className="font-mono text-sm truncate">{prompt.name}</span>
                    <ChevronRight className="h-3 w-3 ml-auto text-muted-foreground" />
                  </div>
                  {prompt.description && (
                    <p className="text-xs text-muted-foreground truncate mt-1">
                      {prompt.description}
                    </p>
                  )}
                  {prompt.arguments && prompt.arguments.length > 0 && (
                    <div className="flex gap-1 mt-1 flex-wrap">
                      {prompt.arguments.map((arg) => (
                        <Badge
                          key={arg.name}
                          variant={arg.required ? "default" : "secondary"}
                          className="text-xs"
                        >
                          {arg.name}
                        </Badge>
                      ))}
                    </div>
                  )}
                </button>
              ))}
            </div>
          )}
        </ScrollArea>

        <div className="p-2 border-t text-xs text-muted-foreground text-center">
          {filteredPrompts.length} prompt{filteredPrompts.length !== 1 ? "s" : ""}
        </div>
      </div>

      {/* Right: Prompt Details & Expansion */}
      <div className="flex-1 flex flex-col border rounded-lg">
        {selectedPrompt ? (
          <>
            <div className="p-4 border-b">
              <div className="flex items-center justify-between">
                <div className="flex items-center gap-2">
                  <MessageSquare className="h-4 w-4" />
                  <h3 className="font-mono font-semibold">{selectedPrompt.name}</h3>
                </div>
                <Button onClick={handleExpand} disabled={isExpanding}>
                  {isExpanding ? (
                    <RefreshCw className="h-4 w-4 mr-2 animate-spin" />
                  ) : (
                    <Play className="h-4 w-4 mr-2" />
                  )}
                  Get Prompt
                </Button>
              </div>
              {selectedPrompt.description && (
                <p className="text-sm text-muted-foreground mt-1">
                  {selectedPrompt.description}
                </p>
              )}
            </div>

            <ScrollArea className="flex-1 p-4">
              <div className="space-y-6">
                {/* Arguments Form */}
                {selectedPrompt.arguments && selectedPrompt.arguments.length > 0 && (
                  <div className="space-y-4">
                    <h4 className="text-sm font-medium">Arguments</h4>
                    {selectedPrompt.arguments.map((arg) => (
                      <div key={arg.name} className="space-y-2">
                        <div className="flex items-center gap-2">
                          <Label className="font-mono text-sm">{arg.name}</Label>
                          {arg.required && (
                            <Badge variant="outline" className="text-xs">
                              required
                            </Badge>
                          )}
                        </div>
                        {arg.description && (
                          <p className="text-xs text-muted-foreground">
                            {arg.description}
                          </p>
                        )}
                        <Textarea
                          value={argValues[arg.name] || ""}
                          onChange={(e) =>
                            setArgValues({ ...argValues, [arg.name]: e.target.value })
                          }
                          placeholder={`Enter ${arg.name}...`}
                          rows={2}
                        />
                      </div>
                    ))}
                  </div>
                )}

                {/* Expanded Messages */}
                {expandedMessages.length > 0 && (
                  <div className="space-y-4">
                    <h4 className="text-sm font-medium">Expanded Messages</h4>
                    <div className="space-y-3">
                      {expandedMessages.map((msg, idx) => (
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
                )}
              </div>
            </ScrollArea>
          </>
        ) : (
          <div className="flex items-center justify-center h-full text-muted-foreground">
            <p>Select a prompt to view details and expand</p>
          </div>
        )}
      </div>
    </div>
  )
}
