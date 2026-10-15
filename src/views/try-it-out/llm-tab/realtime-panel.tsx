import { useEffect, useRef, useState } from 'react'
import type OpenAI from 'openai'
import { Radio, Send, Square } from 'lucide-react'
import { Button } from '@/components/ui/Button'
import { Input } from '@/components/ui/Input'
import { Label } from '@/components/ui/label'
import { Textarea } from '@/components/ui/textarea'
interface LocalTicket {
  value: string
  expires_at: number
  extensions?: { 'localrouter.realtime'?: { demo?: boolean } }
}
interface EventRow {
  id: number
  type: string
  preview: string
}
export function RealtimePanel({
  openaiClient,
  isReady,
}: {
  openaiClient: OpenAI | null
  isReady: boolean
}) {
  const [model, setModel] = useState('')
  const [text, setText] = useState('')
  const [status, setStatus] = useState<
    'disconnected' | 'connecting' | 'connected'
  >('disconnected')
  const [demo, setDemo] = useState(false)
  const [events, setEvents] = useState<EventRow[]>([])
  const [turns, setTurns] = useState(0)
  const [error, setError] = useState<string | null>(null)
  const socket = useRef<WebSocket | null>(null)
  const abort = useRef<AbortController | null>(null)
  const live = useRef(true)
  const sequence = useRef(0)
  useEffect(() => {
    live.current = true
    return () => {
      live.current = false
      abort.current?.abort()
      socket.current?.close()
    }
  }, [])
  function event(type: string, preview = '') {
    if (!live.current) return
    const id = ++sequence.current
    setEvents((rows) =>
      [
        ...rows,
        { id, type: type.slice(0, 120), preview: preview.slice(0, 1024) },
      ].slice(-40),
    )
  }
  function disconnect() {
    abort.current?.abort()
    socket.current?.close()
    socket.current = null
    setStatus('disconnected')
    setDemo(false)
  }
  async function connect() {
    if (!openaiClient || !isReady || status !== 'disconnected') return
    const controller = new AbortController()
    abort.current = controller
    setStatus('connecting')
    setEvents([])
    setError(null)
    setTurns(0)
    try {
      const ticket = await openaiClient.post<LocalTicket>(
        '/realtime/client_secrets',
        {
          body: { session: { type: 'realtime', model: model.trim() } },
          maxRetries: 0,
          signal: controller.signal,
        },
      )
      if (controller.signal.aborted || !live.current) return
      if (ticket.extensions?.['localrouter.realtime']?.demo) {
        setDemo(true)
        setStatus('connected')
        event(
          'demo.session.ready',
          'Local fixture only; no realtime provider is connected.',
        )
        return
      }
      const url = new URL(openaiClient.baseURL)
      url.protocol = url.protocol === 'https:' ? 'wss:' : 'ws:'
      url.pathname = `${url.pathname.replace(/\/+$/, '')}/realtime`
      url.search = ''
      url.hash = ''
      const ws = new WebSocket(url, [
        'realtime',
        `openai-insecure-api-key.${ticket.value}`,
      ])
      socket.current = ws
      ws.onmessage = (message) => {
        if (!live.current || socket.current !== ws) return
        try {
          const value = JSON.parse(String(message.data)) as {
            type?: string
            delta?: string
            error?: { message?: string }
          }
          if (typeof value.type !== 'string') return
          if (value.type === 'session.updated') setStatus('connected')
          if (value.type === 'error')
            setError(value.error?.message ?? 'Native realtime error')
          event(
            value.type,
            typeof value.delta === 'string' && value.type.includes('text')
              ? value.delta
              : value.type === 'error'
                ? (value.error?.message ?? '')
                : '',
          )
        } catch {
          setError('Realtime returned a malformed JSON event')
          ws.close()
        }
      }
      ws.onclose = () => {
        if (live.current && socket.current === ws) {
          socket.current = null
          setStatus('disconnected')
          event('local.connection.closed')
        }
      }
      ws.onerror = () => {
        if (live.current && socket.current === ws) {
          setError(
            'Realtime connection failed. No automatic reconnect was attempted.',
          )
          ws.close()
        }
      }
    } catch (error) {
      if (live.current && !controller.signal.aborted) {
        setError(String(error))
        setStatus('disconnected')
      }
    }
  }
  function send() {
    if (status !== 'connected' || !text.trim() || turns >= 20) return
    if (demo) {
      event(
        'demo.response.fixture',
        'Demo event fixture. No model answered this message.',
      )
      setTurns((n) => n + 1)
      return
    }
    const ws = socket.current
    if (!ws || ws.readyState !== WebSocket.OPEN) return
    ws.send(
      JSON.stringify({
        type: 'conversation.item.create',
        item: {
          type: 'message',
          role: 'user',
          content: [{ type: 'input_text', text }],
        },
      }),
    )
    ws.send(
      JSON.stringify({
        type: 'response.create',
        response: { output_modalities: ['text'] },
      }),
    )
    setTurns((n) => n + 1)
  }
  return (
    <div className="space-y-4 overflow-y-auto h-full pr-2">
      <p className="text-sm text-muted-foreground">
        Enable Allow realtime sessions in client Settings. Connect to an
        explicit provider/model. Native audio uses base64 JSON events through
        the same API; this panel sends text.
      </p>
      <div className="space-y-2">
        <Label htmlFor="realtime-model">Realtime model</Label>
        <Input
          id="realtime-model"
          value={model}
          disabled={status !== 'disconnected'}
          onChange={(e) => setModel(e.target.value)}
          placeholder="openai/gpt-realtime or gateway/openai/gpt-realtime"
        />
      </div>
      <p className="text-xs text-muted-foreground">
        Sessions last up to 120 seconds, with up to 20 manual responses and
        1,024 output tokens per response. Provider fees may apply and may remain
        unknown. Disconnect closes the relay; already generated output may still
        be billed.
      </p>
      <div className="flex gap-2">
        <Button
          onClick={() => void connect()}
          disabled={!isReady || status !== 'disconnected' || !model.trim()}
        >
          <Radio className="h-4 w-4 mr-2" />
          Connect realtime
        </Button>
        <Button
          variant="outline"
          disabled={status === 'disconnected'}
          onClick={disconnect}
        >
          <Square className="h-4 w-4 mr-2" />
          Disconnect
        </Button>
      </div>
      <p role="status" className="text-sm">
        Session: {status} · Responses requested: {turns}/20
      </p>
      {demo && (
        <p className="text-sm text-muted-foreground">
          Demo session. No realtime model is connected or billed.
        </p>
      )}
      <div className="space-y-2">
        <Label htmlFor="realtime-text">Realtime message</Label>
        <Textarea
          id="realtime-text"
          maxLength={16384}
          value={text}
          onChange={(e) => setText(e.target.value)}
        />
      </div>
      <div className="flex gap-2">
        <Button
          disabled={status !== 'connected' || !text.trim() || turns >= 20}
          onClick={send}
        >
          <Send className="h-4 w-4 mr-2" />
          Send text
        </Button>
        <Button
          variant="outline"
          disabled={status !== 'connected' || demo}
          onClick={() =>
            socket.current?.send(JSON.stringify({ type: 'response.cancel' }))
          }
        >
          Cancel response
        </Button>
      </div>
      {error && (
        <p role="alert" className="text-sm text-destructive">
          {error}
        </p>
      )}
      <p className="text-xs text-muted-foreground">
        Last 40 events; text previews are limited to 1,024 characters. Audio
        payloads and credentials are not displayed. No automatic reconnect.
      </p>
      <div className="space-y-2">
        {events.map((row) => (
          <article key={row.id} className="rounded-md border p-2 text-sm">
            <p className="font-mono break-all">{row.type}</p>
            {row.preview && (
              <p className="whitespace-pre-wrap break-words">{row.preview}</p>
            )}
          </article>
        ))}
      </div>
    </div>
  )
}
