import * as React from "react"
import { useState, useRef, useEffect, useCallback } from "react"
import { invoke } from "@tauri-apps/api/core"
import { Send, Bot, User, Square, ImagePlus, X, AlertCircle } from "lucide-react"
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/Card"
import { Button } from "@/components/ui/Button"
import { Input } from "@/components/ui/Input"
import { ScrollArea } from "@/components/ui/scroll-area"
import { cn } from "@/lib/utils"
import type OpenAI from "openai"
import type { ApiPathSupport, GetApiPathSupportParams, SupportLevel } from "@/types/tauri-commands"

interface MessageMetadata {
  model?: string
  promptTokens?: number
  completionTokens?: number
  reasoningTokens?: number
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
  isError?: boolean
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

interface ChatPanelProps {
  openaiClient: OpenAI | null
  isReady: boolean
  selectedModel: string
  parameters: ModelParameters
  noModelsAvailable?: boolean
  /**
   * Provider instance name that currently serves `selectedModel`. Used
   * to annotate the endpoint dropdown with "(Translated)" when
   * LocalRouter has to emulate a path on top of a non-native upstream.
   */
  providerInstance?: string | null
}

export function ChatPanel({
  openaiClient,
  isReady,
  selectedModel,
  parameters,
  noModelsAvailable,
  providerInstance,
}: ChatPanelProps) {
  const [messages, setMessages] = useState<Message[]>([])
  const [input, setInput] = useState("")
  const [isLoading, setIsLoading] = useState(false)
  const [attachedImages, setAttachedImages] = useState<ImageAttachment[]>([])
  // Which server endpoint the Chat panel talks to. Defaults to the
  // familiar chat-completions path; switching to `responses` or
  // `completions` lets the user try the Responses API end-to-end (via
  // LocalRouter's new /v1/responses route) or the legacy completions
  // endpoint without leaving the panel.
  const [endpoint, setEndpoint] = useState<"chat" | "responses" | "completions">("chat")
  // Per-path support for the provider instance that serves the current
  // model. `null` until the lookup lands (or no provider known).
  const [pathSupport, setPathSupport] = useState<ApiPathSupport | null>(null)
  // Last response_id returned by the `/v1/responses` upstream. Sent
  // back as `previous_response_id` on subsequent turns so conversation
  // threading works the same way ChatGPT does.
  const lastResponseIdRef = useRef<string | null>(null)
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

  // Resolve per-path support for the provider serving this model so the
  // dropdown can mark translated endpoints. Skip when no provider is
  // known (e.g. "all providers" client mode with no match yet).
  useEffect(() => {
    let cancelled = false
    if (!providerInstance) {
      setPathSupport(null)
      return
    }
    invoke<ApiPathSupport>('get_api_path_support', {
      instanceName: providerInstance,
    } satisfies GetApiPathSupportParams)
      .then(s => { if (!cancelled) setPathSupport(s) })
      .catch(() => { if (!cancelled) setPathSupport(null) })
    return () => { cancelled = true }
  }, [providerInstance])

  const endpointLabel = (base: string, level: SupportLevel | undefined): string => {
    if (level === 'translated') return `${base} (Translated)`
    if (level === 'not_supported' || level === 'not_implemented') return `${base} (Unavailable)`
    return base
  }

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
    if (isLoading || !openaiClient || !selectedModel?.trim()) return

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
      // Build optional parameters - only include when set
      const optionalParams: Record<string, unknown> = {}
      if (parameters.seed !== null) optionalParams.seed = parameters.seed
      if (parameters.topK !== null) optionalParams.top_k = parameters.topK
      if (parameters.repetitionPenalty !== null) optionalParams.repetition_penalty = parameters.repetitionPenalty
      if (parameters.reasoningEffort !== null) optionalParams.reasoning_effort = parameters.reasoningEffort

      let completionTokens = 0
      let promptTokens = 0
      let reasoningTokens: number | undefined
      let modelUsed = selectedModel

      if (endpoint === "chat") {
        const stream = await openaiClient.chat.completions.create(
          {
            model: selectedModel,
            messages: apiMessages,
            stream: true,
            temperature: parameters.temperature,
            max_tokens: parameters.maxTokens,
            top_p: parameters.topP,
            frequency_penalty: parameters.frequencyPenalty,
            presence_penalty: parameters.presencePenalty,
            stream_options: { include_usage: true },
            ...optionalParams,
          },
          {
            signal: abortControllerRef.current.signal,
          }
        )

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
            const details = (chunk.usage as unknown as Record<string, unknown>).completion_tokens_details as
              | { reasoning_tokens?: number; thinking_tokens?: number }
              | undefined
            if (details) {
              reasoningTokens = details.reasoning_tokens ?? details.thinking_tokens
            }
          }
          if (chunk.model) {
            modelUsed = chunk.model
          }
        }
      } else if (endpoint === "responses") {
        // POST /v1/responses — LocalRouter's inbound Responses API.
        // The OpenAI SDK's `responses` namespace expects Azure-style
        // resource URLs, so we bypass it and call fetch() directly
        // with the same base URL + bearer token the SDK is configured
        // with.
        const baseURL =
          (openaiClient as unknown as { baseURL: string }).baseURL.replace(/\/$/, "")
        const apiKey =
          (openaiClient as unknown as { apiKey: string }).apiKey

        // Flatten the last user message into Responses-API `input[]`.
        // Prior turns are carried by `previous_response_id` — no need
        // to resend them.
        const input: Record<string, unknown>[] = []
        if (userMessage.images && userMessage.images.length > 0) {
          const content: Record<string, unknown>[] = []
          for (const img of userMessage.images) {
            content.push({ type: "input_image", image_url: img.dataUrl })
          }
          if (userMessage.content) {
            content.push({ type: "input_text", text: userMessage.content })
          }
          input.push({ type: "message", role: "user", content })
        } else {
          input.push({
            type: "message",
            role: "user",
            content: [{ type: "input_text", text: userMessage.content }],
          })
        }

        const body = JSON.stringify({
          model: selectedModel,
          input,
          stream: true,
          store: true,
          previous_response_id: lastResponseIdRef.current,
          temperature: parameters.temperature,
          top_p: parameters.topP,
          max_output_tokens: parameters.maxTokens,
          ...(parameters.reasoningEffort !== null && {
            reasoning: { effort: parameters.reasoningEffort },
          }),
        })

        const resp = await fetch(`${baseURL}/responses`, {
          method: "POST",
          headers: {
            Authorization: `Bearer ${apiKey}`,
            "Content-Type": "application/json",
            Accept: "text/event-stream",
          },
          body,
          signal: abortControllerRef.current.signal,
        })
        if (!resp.ok) {
          const errText = await resp.text()
          throw new Error(`Responses API error ${resp.status}: ${errText}`)
        }
        if (!resp.body) {
          throw new Error("Responses API returned empty body")
        }

        // Parse SSE frames. Each frame is `event: <name>\ndata: <json>\n\n`.
        const reader = resp.body.getReader()
        const decoder = new TextDecoder()
        let buf = ""
        while (true) {
          const { done, value } = await reader.read()
          if (done) break
          buf += decoder.decode(value, { stream: true })
          let boundary = buf.indexOf("\n\n")
          while (boundary !== -1) {
            const frame = buf.slice(0, boundary)
            buf = buf.slice(boundary + 2)
            boundary = buf.indexOf("\n\n")
            const dataLines: string[] = []
            for (const line of frame.split("\n")) {
              if (line.startsWith("data:")) dataLines.push(line.slice(5).trim())
            }
            if (dataLines.length === 0) continue
            let payload: Record<string, unknown>
            try {
              payload = JSON.parse(dataLines.join("\n"))
            } catch {
              continue
            }
            switch (payload.type) {
              case "response.created": {
                const r = payload.response as Record<string, unknown> | undefined
                if (r && typeof r.id === "string") {
                  lastResponseIdRef.current = r.id
                }
                if (r && typeof r.model === "string") {
                  modelUsed = r.model
                }
                break
              }
              case "response.output_text.delta": {
                const delta = (payload.delta as string | undefined) ?? ""
                if (delta) {
                  setMessages((prev) =>
                    prev.map((m) =>
                      m.id === assistantId
                        ? { ...m, content: m.content + delta }
                        : m
                    )
                  )
                }
                break
              }
              case "response.completed": {
                const r = payload.response as Record<string, unknown> | undefined
                const usage = r?.usage as Record<string, number> | undefined
                if (usage) {
                  promptTokens = usage.input_tokens ?? 0
                  completionTokens = usage.output_tokens ?? 0
                }
                break
              }
              case "response.failed":
              case "response.incomplete": {
                const err = payload.response as Record<string, unknown> | undefined
                const msg = (err?.error as Record<string, unknown> | undefined)?.message
                throw new Error(
                  typeof msg === "string" ? msg : "Responses API stream failed"
                )
              }
            }
          }
        }
      } else {
        // Legacy /v1/completions — text-only, no chat framing.
        const prompt = apiMessages
          .map((m) =>
            typeof m.content === "string"
              ? `${m.role}: ${m.content}`
              : `${m.role}: [multimodal omitted]`
          )
          .join("\n")
        const stream = await openaiClient.completions.create(
          {
            model: selectedModel,
            prompt,
            stream: true,
            temperature: parameters.temperature,
            max_tokens: parameters.maxTokens,
            top_p: parameters.topP,
            frequency_penalty: parameters.frequencyPenalty,
            presence_penalty: parameters.presencePenalty,
            ...optionalParams,
          },
          {
            signal: abortControllerRef.current.signal,
          }
        )
        for await (const chunk of stream) {
          const text = chunk.choices[0]?.text || ""
          if (text) {
            setMessages((prev) =>
              prev.map((m) =>
                m.id === assistantId ? { ...m, content: m.content + text } : m
              )
            )
          }
          if (chunk.model) modelUsed = chunk.model
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
                  reasoningTokens,
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
                  content: error instanceof Error ? error.message : "Failed to get response",
                  isError: true,
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
      if (!isLoading) {
        handleSend()
      }
    }
  }

  const clearChat = () => {
    if (isLoading) {
      handleStop()
    }
    setMessages([])
    setAttachedImages([])
    // Clearing the chat also resets the Responses conversation chain
    // so the next turn doesn't replay prior context server-side.
    lastResponseIdRef.current = null
  }

  return (
    <Card className="flex flex-col h-full min-h-[400px]">
      <CardHeader className="pb-3 flex-shrink-0">
        <div className="flex items-center justify-between">
          <div className="flex items-center gap-2">
            <CardTitle className="text-base">Chat</CardTitle>
            <select
              className="h-7 rounded border border-input bg-background px-2 text-xs"
              value={endpoint}
              onChange={(e) =>
                setEndpoint(e.target.value as "chat" | "responses" | "completions")
              }
              title={
                pathSupport
                  ? "(Translated) = LocalRouter emulates this path on top of a different upstream API."
                  : "Which server endpoint to hit"
              }
            >
              <option value="chat">
                {endpointLabel('Chat Completions', pathSupport?.chat_completions)}
              </option>
              <option value="responses">
                {endpointLabel('Responses', pathSupport?.responses)}
              </option>
              <option value="completions">
                {endpointLabel('Completions (legacy)', pathSupport?.completions)}
              </option>
            </select>
          </div>
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
                <p className="text-sm text-center">
                  {!isReady
                    ? noModelsAvailable
                      ? "No models available — check that your provider is running and has models downloaded"
                      : "Select a model to start chatting"
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
                      <div className={cn(
                        "flex h-8 w-8 shrink-0 items-center justify-center rounded-full",
                        message.isError ? "bg-destructive/10" : "bg-muted"
                      )}>
                        {message.isError
                          ? <AlertCircle className="h-4 w-4 text-destructive" />
                          : <Bot className="h-4 w-4" />
                        }
                      </div>
                    )}
                    <div
                      className={cn(
                        "rounded-lg px-4 py-2 max-w-[80%]",
                        message.role === "user"
                          ? "bg-primary text-primary-foreground"
                          : message.isError
                            ? "bg-destructive/10 border border-destructive/20 text-destructive"
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
                            {message.metadata.completionTokens}
                            {message.metadata.reasoningTokens != null &&
                              message.metadata.reasoningTokens > 0 &&
                              ` (${message.metadata.reasoningTokens} reasoning)`}
                            {" "}= {message.metadata.totalTokens} tokens
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
            disabled={!isReady}
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
