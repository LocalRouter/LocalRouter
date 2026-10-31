import { useCallback, useEffect, useRef, useState } from "react"
import type { EventStreamHandle, McpClientWrapper } from "@/lib/mcp-client"
import {
  addNotice,
  addOccurrence,
  addStreamNotification,
  autoPollDelay,
  emptyPushLog,
  parseEventPollResult,
  parseEventWaitCommands,
  type PushLogState,
  type PushOccurrence,
  type WatcherCommand,
} from "./push-log"

/** One entry of `events/list`. */
export interface PushEventType {
  name: string
  description: string
  delivery: string[]
}

export interface PushStreamInfo {
  id: number
  name: string
  status: "active" | "ended" | "error"
  openedAt: string
  lastHeartbeat: string | null
  detail: string | null
}

export const FALLBACK_TOOL_NAMES = ["EventList", "EventPoll", "EventWait"] as const

/** The Push panel's session state. Lives in the MCP tab (always mounted) so
 *  streams, auto-poll and the log survive switching subtabs. */
export interface PushTester {
  log: PushLogState
  clearLog: () => void
  eventTypes: PushEventType[]
  eventsError: string | null
  loadingEvents: boolean
  refreshEvents: () => void
  /** Fallback tool names this session was given. */
  fallbackTools: string[]
  selected: string | null
  setSelected: (name: string | null) => void

  streams: PushStreamInfo[]
  startStream: (name: string) => void
  stopStream: (id: number) => void

  pollCursor: string | null
  pollBusy: boolean
  pollError: string | null
  nextPollMs: number | null
  autoPoll: boolean
  setAutoPoll: (on: boolean) => void
  pollNow: () => void
  resetPollCursor: () => void

  toolCursor: string | null
  toolBusy: boolean
  toolError: string | null
  waitSeconds: number
  setWaitSeconds: (seconds: number) => void
  callEventPoll: () => void
  callEventWait: () => void
  waitCommands: WatcherCommand[]
  waitText: string | null
}

interface Options {
  mcpClient: McpClientWrapper | null
  isConnected: boolean
  /** Bumped on `notifications/events/list_changed`. */
  eventsRefreshTrigger: number
  /** Bumped on `notifications/tools/list_changed`. */
  toolsRefreshTrigger: number
}

function message(error: unknown): string {
  return error instanceof Error ? error.message : String(error)
}

function textOf(content: unknown[]): string {
  return content
    .map(c => (c && typeof c === "object" && "text" in c ? String((c as { text: unknown }).text) : ""))
    .join("\n")
}

export function usePushTester({ mcpClient, isConnected, eventsRefreshTrigger, toolsRefreshTrigger }: Options): PushTester {
  const [log, setLog] = useState<PushLogState>(emptyPushLog)
  const [eventTypes, setEventTypes] = useState<PushEventType[]>([])
  const [eventsError, setEventsError] = useState<string | null>(null)
  const [loadingEvents, setLoadingEvents] = useState(false)
  const [fallbackTools, setFallbackTools] = useState<string[]>([])
  const [selected, setSelected] = useState<string | null>(null)
  const [streams, setStreams] = useState<PushStreamInfo[]>([])
  const handles = useRef(new Map<number, EventStreamHandle>())

  const [pollCursors, setPollCursors] = useState<Record<string, string>>({})
  const pollCursorsRef = useRef(pollCursors)
  pollCursorsRef.current = pollCursors
  const [pollBusy, setPollBusy] = useState(false)
  const [pollError, setPollError] = useState<string | null>(null)
  const [nextPollMs, setNextPollMs] = useState<number | null>(null)
  const [autoPoll, setAutoPoll] = useState(false)

  const [toolCursors, setToolCursors] = useState<Record<string, string>>({})
  const toolCursorsRef = useRef(toolCursors)
  toolCursorsRef.current = toolCursors
  const [toolBusy, setToolBusy] = useState(false)
  const [toolError, setToolError] = useState<string | null>(null)
  const [waitSeconds, setWaitSeconds] = useState(10)
  const [waitCommands, setWaitCommands] = useState<WatcherCommand[]>([])
  const [waitText, setWaitText] = useState<string | null>(null)

  const refreshEvents = useCallback(async () => {
    if (!mcpClient || !isConnected) return
    setLoadingEvents(true)
    try {
      const result = await mcpClient.listEvents()
      const list = Array.isArray(result.events) ? result.events as Record<string, unknown>[] : []
      const types = list.map(e => ({
        name: String(e.name ?? ""),
        description: typeof e.description === "string" ? e.description : "",
        delivery: Array.isArray(e.delivery) ? e.delivery.map(String) : [],
      }))
      setEventTypes(types)
      setEventsError(null)
      setSelected(prev => (prev && types.some(t => t.name === prev) ? prev : types[0]?.name ?? null))
    } catch (error) {
      setEventTypes([])
      setEventsError(message(error))
    } finally {
      setLoadingEvents(false)
    }
  }, [mcpClient, isConnected])

  const refreshTools = useCallback(async () => {
    if (!mcpClient || !isConnected) return
    try {
      const tools = await mcpClient.listTools()
      setFallbackTools(tools.map(t => t.name).filter(n => (FALLBACK_TOOL_NAMES as readonly string[]).includes(n)))
    } catch {
      setFallbackTools([])
    }
  }, [mcpClient, isConnected])

  useEffect(() => { void refreshEvents() }, [refreshEvents, eventsRefreshTrigger])
  useEffect(() => { void refreshTools() }, [refreshTools, toolsRefreshTrigger])

  // A new connection starts clean: streams died with the old one and cursors
  // belong to its session.
  useEffect(() => {
    if (isConnected && mcpClient) return
    for (const handle of handles.current.values()) handle.cancel("Disconnected")
    handles.current.clear()
    setStreams([])
    setAutoPoll(false)
    setPollCursors({})
    setToolCursors({})
    setNextPollMs(null)
    setWaitCommands([])
    setWaitText(null)
    setEventTypes([])
    setFallbackTools([])
  }, [isConnected, mcpClient])

  const updateStream = useCallback((id: number, patch: Partial<PushStreamInfo>) => {
    setStreams(prev => prev.map(s => (s.id === id ? { ...s, ...patch } : s)))
  }, [])

  const startStream = useCallback((name: string) => {
    if (!mcpClient || !isConnected) return
    let handle: EventStreamHandle
    try {
      handle = mcpClient.streamEvents(name, null, notification => {
        setLog(prev => addStreamNotification(prev, notification.method, notification.params, name))
        const id = handle?.requestId
        if (id === undefined) return
        if (notification.method === "notifications/events/heartbeat") {
          updateStream(id, { lastHeartbeat: new Date().toISOString() })
        } else if (notification.method === "notifications/events/terminated") {
          updateStream(id, { status: "ended", detail: String(notification.params.reason ?? "terminated") })
        }
      })
    } catch (error) {
      setLog(prev => addNotice(prev, { kind: "error", mode: "push", name, cursor: null, truncated: false, text: message(error) }))
      return
    }
    const id = handle.requestId
    handles.current.set(id, handle)
    setStreams(prev => [
      ...prev,
      { id, name, status: "active", openedAt: new Date().toISOString(), lastHeartbeat: null, detail: null },
    ])
    handle.done
      .then(() => {
        updateStream(id, { status: "ended", detail: "Server ended the stream" })
      })
      .catch(error => {
        const cancelled = /cancel|stopped|disconnect/i.test(message(error))
        updateStream(id, { status: cancelled ? "ended" : "error", detail: cancelled ? "Stopped" : message(error) })
        if (!cancelled) {
          setLog(prev => addNotice(prev, { kind: "error", mode: "push", name, cursor: null, truncated: false, text: message(error) }))
        }
      })
      .finally(() => handles.current.delete(id))
  }, [mcpClient, isConnected, updateStream])

  const stopStream = useCallback((id: number) => {
    const handle = handles.current.get(id)
    if (handle) {
      // Sends notifications/cancelled; the entry is marked ended when it settles
      handle.cancel()
    } else {
      setStreams(prev => prev.filter(s => s.id !== id))
    }
  }, [])

  /** One `events/poll` for the selected type; returns the delay before the next. */
  const doPoll = useCallback(async (name: string): Promise<number> => {
    if (!mcpClient || !isConnected) return 1000
    setPollBusy(true)
    try {
      const cursor = pollCursorsRef.current[name] ?? null
      const result = await mcpClient.pollEvents(name, cursor)
      const events = Array.isArray(result.events) ? result.events as PushOccurrence[] : []
      setLog(prev => {
        let next = prev
        if (cursor === null) {
          next = addNotice(next, { kind: "info", mode: "poll", name, cursor: null, truncated: false, text: "Poll cursor started at now (no history)" })
        }
        if (result.truncated === true) {
          next = addNotice(next, { kind: "gap", mode: "poll", name, cursor: null, truncated: true, text: "Earlier events were dropped (truncated)" })
        }
        for (const event of events) next = addOccurrence(next, event, "poll")
        return next
      })
      if (typeof result.cursor === "string") {
        const nextCursor = result.cursor
        setPollCursors(prev => ({ ...prev, [name]: nextCursor }))
      }
      const ms = typeof result.nextPollMs === "number" ? result.nextPollMs : null
      setNextPollMs(ms)
      setPollError(null)
      return result.hasMore === true ? 0 : autoPollDelay(ms)
    } catch (error) {
      setPollError(message(error))
      return 5000
    } finally {
      setPollBusy(false)
    }
  }, [mcpClient, isConnected])

  const pollNow = useCallback(() => {
    if (selected) void doPoll(selected)
  }, [selected, doPoll])

  const resetPollCursor = useCallback(() => {
    if (!selected) return
    setPollCursors(prev => {
      const next = { ...prev }
      delete next[selected]
      return next
    })
  }, [selected])

  useEffect(() => {
    if (!autoPoll || !selected || !mcpClient || !isConnected) return
    let cancelled = false
    let timer: ReturnType<typeof setTimeout> | undefined
    const run = async () => {
      const delay = await doPoll(selected)
      if (!cancelled) timer = setTimeout(run, delay)
    }
    void run()
    return () => {
      cancelled = true
      if (timer) clearTimeout(timer)
    }
  }, [autoPoll, selected, mcpClient, isConnected, doPoll])

  const callEventPoll = useCallback(async () => {
    if (!mcpClient || !isConnected || !selected) return
    const name = selected
    setToolBusy(true)
    try {
      const cursor = toolCursorsRef.current[name]
      const args: Record<string, unknown> = { events: [name] }
      if (cursor) args.cursor = cursor
      if (waitSeconds > 0) args.wait_seconds = waitSeconds
      const result = await mcpClient.callTool("EventPoll", args)
      const text = textOf(result.content)
      if (result.isError) throw new Error(text || "EventPoll failed")
      const parsed = parseEventPollResult(text)
      setLog(prev => {
        let next = prev
        if (!cursor) {
          next = addNotice(next, { kind: "info", mode: "tools", name, cursor: null, truncated: false, text: "EventPoll cursor started at now (no history)" })
        }
        if (parsed.truncated) {
          next = addNotice(next, { kind: "gap", mode: "tools", name, cursor: null, truncated: true, text: "Earlier events were dropped before they were read" })
        }
        for (const event of parsed.events) next = addOccurrence(next, { ...event, cursor: parsed.nextCursor }, "tools")
        return next
      })
      if (parsed.nextCursor) {
        const nextCursor = parsed.nextCursor
        setToolCursors(prev => ({ ...prev, [name]: nextCursor }))
      }
      setToolError(null)
    } catch (error) {
      setToolError(message(error))
    } finally {
      setToolBusy(false)
    }
  }, [mcpClient, isConnected, selected, waitSeconds])

  const callEventWait = useCallback(async () => {
    if (!mcpClient || !isConnected || !selected) return
    setToolBusy(true)
    try {
      const args: Record<string, unknown> = { events: [selected] }
      const cursor = toolCursorsRef.current[selected]
      if (cursor) args.cursor = cursor
      const result = await mcpClient.callTool("EventWait", args)
      const text = textOf(result.content)
      if (result.isError) throw new Error(text || "EventWait failed")
      setWaitText(text)
      setWaitCommands(parseEventWaitCommands(text))
      setToolError(null)
    } catch (error) {
      setToolError(message(error))
    } finally {
      setToolBusy(false)
    }
  }, [mcpClient, isConnected, selected])

  return {
    log,
    clearLog: () => setLog(emptyPushLog()),
    eventTypes,
    eventsError,
    loadingEvents,
    refreshEvents: () => { void refreshEvents() },
    fallbackTools,
    selected,
    setSelected,
    streams,
    startStream,
    stopStream,
    pollCursor: selected ? pollCursors[selected] ?? null : null,
    pollBusy,
    pollError,
    nextPollMs,
    autoPoll,
    setAutoPoll,
    pollNow,
    resetPollCursor,
    toolCursor: selected ? toolCursors[selected] ?? null : null,
    toolBusy,
    toolError,
    waitSeconds,
    setWaitSeconds,
    callEventPoll: () => { void callEventPoll() },
    callEventWait: () => { void callEventWait() },
    waitCommands,
    waitText,
  }
}
