import * as React from "react"
import { useState, useRef, useEffect, useCallback } from "react"
import { Send, Bot, User, Square, ImagePlus, X } from "lucide-react"
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/Card"
import { Button } from "@/components/ui/Button"
import { Input } from "@/components/ui/Input"
import { ScrollArea } from "@/components/ui/scroll-area"
import { cn } from "@/lib/utils"
import type OpenAI from "openai"

interface MessageMetadata {
  model?: string
  promptTokens?: number
  completionTokens?: number
  totalTokens?: number
  latencyMs?: number
}

interface ImageAttachment {
  id: string
  dataUrl: string
  name: string
}

interface Message {
  id: string
  role: "user" | "assistant"
  content: string
  images?: ImageAttachment[]
  timestamp: Date
  metadata?: MessageMetadata
}

interface ModelParameters {
  temperature: number
  maxTokens: number
  topP: number
}

interface ChatPanelProps {
  openaiClient: OpenAI | null
  isReady: boolean
  selectedModel: string
  parameters: ModelParameters
}

export function ChatPanel({
  openaiClient,
  isReady,
  selectedModel,
  parameters,
}: ChatPanelProps) {
  const [messages, setMessages] = useState<Message[]>([])
  const [input, setInput] = useState("")
  const [isLoading, setIsLoading] = useState(false)
  const [attachedImages, setAttachedImages] = useState<ImageAttachment[]>([])
  const messagesEndRef = useRef<HTMLDivElement>(null)
  const fileInputRef = useRef<HTMLInputElement>(null)
  const abortControllerRef = useRef<AbortController | null>(null)

  // Cleanup abort controller on unmount
  useEffect(() => {
    return () => {
      if (abortControllerRef.current) {
        abortControllerRef.current.abort()
      }
    }
  }, [])

  // Detect demo mode (running at /demo route, typically in iframe)
  const isDemo = typeof window !== "undefined" && window.location.pathname === "/demo"

  // Get the last message content for scroll dependency (to scroll during streaming)
  const lastMessageContent = messages[messages.length - 1]?.content

  // Auto-scroll to bottom when messages change or content streams in
  // Disabled in demo mode to prevent scrolling issues in iframe
  useEffect(() => {
    if (messages.length > 0 && !isDemo) {
      messagesEndRef.current?.scrollIntoView({ behavior: "smooth" })
    }
  }, [messages.length, lastMessageContent, isDemo])

  const handleStop = useCallback(() => {
    if (abortControllerRef.current) {
      abortControllerRef.current.abort()
      abortControllerRef.current = null
      setIsLoading(false)
    }
  }, [])

  const handleImageUpload = (event: React.ChangeEvent<HTMLInputElement>) => {
    const files = event.target.files
    if (!files) return

    Array.from(files).forEach((file) => {
      if (!file.type.startsWith("image/")) return

      const reader = new FileReader()
      reader.onload = (e) => {
        const dataUrl = e.target?.result as string
        setAttachedImages((prev) => [
          ...prev,
          { id: crypto.randomUUID(), dataUrl, name: file.name },
        ])
      }
      reader.readAsDataURL(file)
    })

    // Reset input
    if (fileInputRef.current) {
      fileInputRef.current.value = ""
    }
  }

  const removeImage = (id: string) => {
    setAttachedImages((prev) => prev.filter((img) => img.id !== id))
  }

  const handleSend = async () => {
    if (!input.trim() && attachedImages.length === 0) return
    if (isLoading || !openaiClient || !selectedModel) return

    // Build message content
    const userMessageContent: OpenAI.ChatCompletionContentPart[] = []

    // Add images first
    for (const img of attachedImages) {
      userMessageContent.push({
        type: "image_url",
        image_url: { url: img.dataUrl },
      })
    }

    // Add text
    if (input.trim()) {
      userMessageContent.push({ type: "text", text: input.trim() })
    }

    const userMessage: Message = {
      id: crypto.randomUUID(),
      role: "user",
      content: input.trim(),
      images: attachedImages.length > 0 ? [...attachedImages] : undefined,
      timestamp: new Date(),
    }

    // Add user message immediately
    setMessages((prev) => [...prev, userMessage])
    setInput("")
    setAttachedImages([])
    setIsLoading(true)

    // Create abort controller for this request
    abortControllerRef.current = new AbortController()

    // Create assistant message placeholder for streaming
    const assistantId = crypto.randomUUID()
    const startTime = performance.now()

    setMessages((prev) => [
      ...prev,
      { id: assistantId, role: "assistant", content: "", timestamp: new Date() },
    ])

    try {
      // Build messages array for API
      const apiMessages: OpenAI.ChatCompletionMessageParam[] = []

      for (const msg of [...messages, userMessage]) {
        if (msg.role === "user") {
          if (msg.images && msg.images.length > 0) {
            // Message with images
            const content: OpenAI.ChatCompletionContentPart[] = []
            for (const img of msg.images) {
              content.push({
                type: "image_url",
                image_url: { url: img.dataUrl },
              })
            }
            if (msg.content) {
              content.push({ type: "text", text: msg.content })
            }
            apiMessages.push({ role: "user", content })
          } else {
            // Text-only message
            apiMessages.push({ role: "user", content: msg.content })
          }
        } else {
          apiMessages.push({ role: "assistant", content: msg.content })
        }
      }

      // Use OpenAI SDK for streaming chat completion
      const stream = await openaiClient.chat.completions.create(
        {
          model: selectedModel,
          messages: apiMessages,
          stream: true,
          temperature: parameters.temperature,
          max_tokens: parameters.maxTokens,
          top_p: parameters.topP,
          stream_options: { include_usage: true },
        },
        {
          signal: abortControllerRef.current.signal,
        }
      )

      let completionTokens = 0
      let promptTokens = 0
      let modelUsed = selectedModel

      // Stream response token by token
      for await (const chunk of stream) {
        const content = chunk.choices[0]?.delta?.content || ""
        if (content) {
          setMessages((prev) =>
            prev.map((m) =>
              m.id === assistantId ? { ...m, content: m.content + content } : m
            )
          )
        }

        // Capture usage from final chunk
        if (chunk.usage) {
          promptTokens = chunk.usage.prompt_tokens
          completionTokens = chunk.usage.completion_tokens
        }
        if (chunk.model) {
          modelUsed = chunk.model
        }
      }

      // Update with metadata
      const latencyMs = Math.round(performance.now() - startTime)
      setMessages((prev) =>
        prev.map((m) =>
          m.id === assistantId
            ? {
                ...m,
                metadata: {
                  model: modelUsed,
                  promptTokens,
                  completionTokens,
                  totalTokens: promptTokens + completionTokens,
                  latencyMs,
                },
              }
            : m
        )
      )
    } catch (error) {
      if (error instanceof Error && error.name === "AbortError") {
        setMessages((prev) =>
          prev.map((m) =>
            m.id === assistantId && m.content === ""
              ? { ...m, content: "[Generation stopped]" }
              : m
          )
        )
      } else {
        console.error("Failed to send message:", error)
        setMessages((prev) =>
          prev.map((m) =>
            m.id === assistantId
              ? {
                  ...m,
                  content: `Error: ${error instanceof Error ? error.message : "Failed to get response"}`,
                }
              : m
          )
        )
      }
    } finally {
      abortControllerRef.current = null
      setIsLoading(false)
    }
  }

  const handleKeyDown = (e: React.KeyboardEvent) => {
    if (e.key === "Enter" && !e.shiftKey) {
      e.preventDefault()
      handleSend()
    }
  }

  const clearChat = () => {
    if (isLoading) {
      handleStop()
    }
    setMessages([])
    setAttachedImages([])
  }

  return (
    <Card className="flex flex-col h-full min-h-[400px]">
      <CardHeader className="pb-3 flex-shrink-0">
        <div className="flex items-center justify-between">
          <CardTitle className="text-base">Chat</CardTitle>
          <Button variant="outline" size="sm" onClick={clearChat}>
            Clear
          </Button>
        </div>
      </CardHeader>
      <CardContent className="flex-1 flex flex-col min-h-0">
        {/* Messages */}
        <ScrollArea className="flex-1 pr-4">
          <div className="space-y-4">
            {messages.length === 0 ? (
              <div className="flex items-center justify-center h-64 text-muted-foreground">
                <p className="text-sm">
                  {!isReady
                    ? "Select a model to start chatting"
                    : "Send a message to start chatting"}
                </p>
              </div>
            ) : (
              messages.map((message) => (
                <div key={message.id} className="space-y-1">
                  <div
                    className={cn(
                      "flex gap-3",
                      message.role === "user" ? "justify-end" : "justify-start"
                    )}
                  >
                    {message.role === "assistant" && (
                      <div className="flex h-8 w-8 shrink-0 items-center justify-center rounded-full bg-muted">
                        <Bot className="h-4 w-4" />
                      </div>
                    )}
                    <div
                      className={cn(
                        "rounded-lg px-4 py-2 max-w-[80%]",
                        message.role === "user"
                          ? "bg-primary text-primary-foreground"
                          : "bg-muted"
                      )}
                    >
                      {/* Show attached images */}
                      {message.images && message.images.length > 0 && (
                        <div className="flex flex-wrap gap-2 mb-2">
                          {message.images.map((img) => (
                            <img
                              key={img.id}
                              src={img.dataUrl}
                              alt={img.name}
                              className="h-20 w-20 object-cover rounded"
                            />
                          ))}
                        </div>
                      )}
                      <p className="text-sm whitespace-pre-wrap">
                        {message.content}
                        {message.role === "assistant" &&
                          isLoading &&
                          message.content === "" && (
                            <span className="inline-block w-2 h-4 bg-foreground/50 animate-pulse" />
                          )}
                      </p>
                    </div>
                    {message.role === "user" && (
                      <div className="flex h-8 w-8 shrink-0 items-center justify-center rounded-full bg-primary">
                        <User className="h-4 w-4 text-primary-foreground" />
                      </div>
                    )}
                  </div>
                  {/* Response metadata */}
                  {message.role === "assistant" && message.metadata && (
                    <div
                      className={cn(
                        "flex gap-2 text-xs text-muted-foreground",
                        "ml-11"
                      )}
                    >
                      {message.metadata.model && (
                        <span className="text-[10px] opacity-60">
                          {message.metadata.model}
                        </span>
                      )}
                      {message.metadata.totalTokens !== undefined &&
                        message.metadata.totalTokens > 0 && (
                          <span>
                            {message.metadata.promptTokens} +{" "}
                            {message.metadata.completionTokens} ={" "}
                            {message.metadata.totalTokens} tokens
                          </span>
                        )}
                      {message.metadata.latencyMs !== undefined && (
                        <span>
                          {(message.metadata.latencyMs / 1000).toFixed(2)}s
                        </span>
                      )}
                    </div>
                  )}
                </div>
              ))
            )}
            <div ref={messagesEndRef} />
          </div>
        </ScrollArea>

        {/* Attached images preview */}
        {attachedImages.length > 0 && (
          <div className="flex flex-wrap gap-2 pt-2 border-t mt-2">
            {attachedImages.map((img) => (
              <div key={img.id} className="relative group">
                <img
                  src={img.dataUrl}
                  alt={img.name}
                  className="h-16 w-16 object-cover rounded"
                />
                <button
                  onClick={() => removeImage(img.id)}
                  className="absolute -top-1 -right-1 bg-destructive text-destructive-foreground rounded-full p-0.5 opacity-0 group-hover:opacity-100 transition-opacity"
                >
                  <X className="h-3 w-3" />
                </button>
              </div>
            ))}
          </div>
        )}

        {/* Input */}
        <div className="flex gap-2 pt-4 border-t mt-4 flex-shrink-0">
          <input
            ref={fileInputRef}
            type="file"
            accept="image/*"
            multiple
            onChange={handleImageUpload}
            className="hidden"
          />
          <Button
            variant="outline"
            size="icon"
            onClick={() => fileInputRef.current?.click()}
            disabled={isLoading}
            title="Attach image (for vision models)"
          >
            <ImagePlus className="h-4 w-4" />
          </Button>
          <Input
            placeholder="Type a message..."
            value={input}
            onChange={(e) => setInput(e.target.value)}
            onKeyDown={handleKeyDown}
            disabled={isLoading || !isReady}
          />
          {isLoading ? (
            <Button
              variant="destructive"
              onClick={handleStop}
              title="Stop generation"
            >
              <Square className="h-4 w-4" />
            </Button>
          ) : (
            <Button
              onClick={handleSend}
              disabled={
                (!input.trim() && attachedImages.length === 0) || !isReady
              }
            >
              <Send className="h-4 w-4" />
            </Button>
          )}
        </div>
      </CardContent>
    </Card>
  )
}
