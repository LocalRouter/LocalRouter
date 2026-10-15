import { useEffect, useRef, useState } from 'react'
import type OpenAI from 'openai'
import { Loader2, Play, Square } from 'lucide-react'
import { Button } from '@/components/ui/Button'
import { Label } from '@/components/ui/label'
import { Textarea } from '@/components/ui/textarea'
import { JsonTree } from '@/components/shared/JsonTree'
import { readSseData } from '@/lib/sse'
import type { ApiPathSupport } from '@/types/tauri-commands'

export function MessagesPanel({
  openaiClient,
  isReady,
  selectedModel,
  pathSupport,
}: {
  openaiClient: OpenAI | null
  isReady: boolean
  selectedModel: string
  pathSupport: ApiPathSupport | null
}) {
  const [body, setBody] = useState(() =>
    JSON.stringify(
      {
        model: selectedModel,
        max_tokens: 512,
        messages: [{ role: 'user', content: 'Hello!' }],
      },
      null,
      2,
    ),
  )
  const [running, setRunning] = useState(false)
  const [result, setResult] = useState<unknown>(null)
  const [error, setError] = useState<string | null>(null)
  const [duration, setDuration] = useState<number | null>(null)
  const abort = useRef<AbortController | null>(null)
  useEffect(() => () => abort.current?.abort(), [])
  useEffect(() => {
    setBody((previous) => {
      try {
        return JSON.stringify(
          { ...JSON.parse(previous), model: selectedModel },
          null,
          2,
        )
      } catch {
        return previous
      }
    })
  }, [selectedModel])

  const run = async (count: boolean) => {
    if (!openaiClient || running) return
    setError(null)
    setResult(null)
    setDuration(null)
    let request: Record<string, unknown>
    try {
      request = JSON.parse(body)
      if (
        !request ||
        Array.isArray(request) ||
        typeof request.model !== 'string' ||
        !Array.isArray(request.messages)
      )
        throw new Error('Provide a model and messages array.')
      if (count)
        request = Object.fromEntries(
          ['model', 'messages', 'system', 'tools', 'tool_choice', 'thinking']
            .filter((key) => key in request)
            .map((key) => [key, request[key]]),
        )
    } catch (e) {
      setError(e instanceof Error ? e.message : 'Invalid JSON.')
      return
    }
    const controller = new AbortController()
    abort.current = controller
    setRunning(true)
    const started = performance.now()
    try {
      const response = await openaiClient
        .post(count ? '/messages/count_tokens' : '/messages', {
          body: request,
          maxRetries: 0,
          signal: controller.signal,
          headers: { 'anthropic-version': '2023-06-01' },
        })
        .asResponse()
      if (response.headers.get('content-type')?.includes('text/event-stream')) {
        if (!response.body) throw new Error('No stream body returned.')
        const events: unknown[] = []
        for await (const data of readSseData(response.body)) {
          const event: unknown = JSON.parse(data)
          events.push(event)
          // Bound visible history; the connection still delivers every event.
          if (events.length > 1000) events.shift()
          setResult([...events])
          if (
            event &&
            typeof event === 'object' &&
            'type' in event &&
            event.type === 'error'
          ) {
            setError(
              'The provider returned a terminal stream error. See the event below.',
            )
            break
          }
        }
      } else setResult(await response.json())
    } catch (e) {
      setError(
        controller.signal.aborted
          ? 'Request stopped; partial upstream usage may still be billed.'
          : e instanceof Error
            ? e.message
            : 'Messages request failed.',
      )
    } finally {
      abort.current = null
      setDuration(Math.round(performance.now() - started))
      setRunning(false)
    }
  }

  return (
    <div className="space-y-4">
      <p className="text-sm text-muted-foreground">
        Anthropic Messages format. Native transports preserve thinking
        signatures, cache markers and client tool history. Chat translation
        accepts a validated subset and rejects unsupported native fields. Bound
        histories require an explicit provider/model.
      </p>
      {pathSupport?.messages === 'translated' && (
        <p className="text-xs text-muted-foreground">
          This provider uses chat translation.
        </p>
      )}
      <div className="space-y-2">
        <Label htmlFor="messages-request">Request</Label>
        <Textarea
          id="messages-request"
          className="min-h-72 font-mono text-xs"
          value={body}
          onChange={(e) => setBody(e.target.value)}
          spellCheck={false}
          disabled={running}
        />
      </div>
      <div className="flex flex-wrap gap-2">
        <Button onClick={() => void run(false)} disabled={!isReady || running}>
          {running ? (
            <Loader2 className="mr-2 h-4 w-4 animate-spin" />
          ) : (
            <Play className="mr-2 h-4 w-4" />
          )}
          Send Messages
        </Button>
        <Button
          variant="outline"
          onClick={() => void run(true)}
          disabled={
            !isReady ||
            running ||
            pathSupport?.messages_count_tokens !== 'supported'
          }
        >
          Count input tokens
        </Button>
        {running && (
          <Button variant="outline" onClick={() => abort.current?.abort()}>
            <Square className="mr-2 h-4 w-4" />
            Stop
          </Button>
        )}
      </div>
      <p className="text-xs text-muted-foreground">
        Counting sends the current model, history, system, tools and thinking
        configuration to a verified native token-count endpoint. It returns the
        provider’s estimate without generation. Set stream to true in the
        request to inspect SSE events.
      </p>
      {error && (
        <p role="alert" className="text-sm text-destructive">
          {error}
        </p>
      )}
      {result !== null && (
        <div className="space-y-2">
          <p className="text-xs text-muted-foreground">
            {duration === null ? 'Streaming…' : `${duration} ms`}
          </p>
          <JsonTree data={result} />
        </div>
      )}
    </div>
  )
}
