/**
 * Pure state for Try it out's Push panel: the log of received occurrences
 * (deduplicated by event id across delivery modes) and parsers for the
 * fallback tools' text results. No React, so it is unit-tested directly.
 */

/** How the panel received something. */
export type PushDeliveryMode = "push" | "poll" | "tools"

export const PUSH_MODE_LABELS: Record<PushDeliveryMode, string> = {
  push: "Push (events/stream)",
  poll: "Poll (events/poll)",
  tools: "Fallback tools (EventPoll)",
}

/** One occurrence in SEP-3415 wire form. */
export interface PushOccurrence {
  eventId: string
  name: string
  timestamp?: string
  data: unknown
  cursor?: string | null
}

export interface PushLogEvent {
  kind: "event"
  /** Stable React key. */
  key: string
  /** When the panel first received it (ISO). */
  receivedAt: string
  eventId: string
  name: string
  timestamp?: string
  data: unknown
  cursor: string | null
  /** Every mode it arrived through; the first is where it was seen first. */
  modes: PushDeliveryMode[]
}

/** A stream or poll status line: active, heartbeat, gap, error, terminated. */
export interface PushLogNotice {
  kind: "active" | "heartbeat" | "gap" | "error" | "terminated" | "info"
  key: string
  receivedAt: string
  mode: PushDeliveryMode
  name: string | null
  cursor: string | null
  truncated: boolean
  text: string
}

export type PushLogEntry = PushLogEvent | PushLogNotice

export interface PushLogState {
  /** Newest first. */
  entries: PushLogEntry[]
  /** Event id → index-free lookup of the modes it arrived through. */
  seen: Record<string, PushDeliveryMode[]>
  counter: number
}

/** Entries kept; the oldest go first. */
export const PUSH_LOG_MAX = 500

export const emptyPushLog = (): PushLogState => ({ entries: [], seen: {}, counter: 0 })

function trim(entries: PushLogEntry[]): PushLogEntry[] {
  return entries.length > PUSH_LOG_MAX ? entries.slice(0, PUSH_LOG_MAX) : entries
}

/** Add an occurrence. One received again (same event id, any mode) is not a
 *  new row: the mode is added to the row it already has. */
export function addOccurrence(
  state: PushLogState,
  occurrence: PushOccurrence,
  mode: PushDeliveryMode,
  now: Date = new Date(),
): PushLogState {
  const known = state.seen[occurrence.eventId]
  if (known) {
    if (known.includes(mode)) return state
    const modes = [...known, mode]
    return {
      ...state,
      seen: { ...state.seen, [occurrence.eventId]: modes },
      entries: state.entries.map(e =>
        e.kind === "event" && e.eventId === occurrence.eventId ? { ...e, modes } : e,
      ),
    }
  }
  const counter = state.counter + 1
  const entry: PushLogEvent = {
    kind: "event",
    key: `e${counter}`,
    receivedAt: now.toISOString(),
    eventId: occurrence.eventId,
    name: occurrence.name,
    timestamp: occurrence.timestamp,
    data: occurrence.data,
    cursor: occurrence.cursor ?? null,
    modes: [mode],
  }
  return {
    counter,
    seen: { ...state.seen, [occurrence.eventId]: [mode] },
    entries: trim([entry, ...state.entries]),
  }
}

/** Add a status line. Consecutive heartbeats of one stream collapse into one. */
export function addNotice(
  state: PushLogState,
  notice: Omit<PushLogNotice, "key" | "receivedAt">,
  now: Date = new Date(),
): PushLogState {
  const head = state.entries[0]
  if (
    notice.kind === "heartbeat" && head && head.kind === "heartbeat" &&
    head.mode === notice.mode && head.name === notice.name
  ) {
    return {
      ...state,
      entries: [{ ...head, receivedAt: now.toISOString(), cursor: notice.cursor }, ...state.entries.slice(1)],
    }
  }
  const counter = state.counter + 1
  return {
    ...state,
    counter,
    entries: trim([{ ...notice, key: `n${counter}`, receivedAt: now.toISOString() }, ...state.entries]),
  }
}

function str(value: unknown): string | null {
  return typeof value === "string" ? value : null
}

/** Fold one `notifications/events/*` of a stream into the log. */
export function addStreamNotification(
  state: PushLogState,
  method: string,
  params: Record<string, unknown>,
  name: string,
  now: Date = new Date(),
): PushLogState {
  const cursor = str(params.cursor)
  switch (method) {
    case "notifications/events/event":
      return addOccurrence(state, {
        eventId: str(params.eventId) ?? "(no id)",
        name: str(params.name) ?? name,
        timestamp: str(params.timestamp) ?? undefined,
        data: params.data,
        cursor,
      }, "push", now)
    case "notifications/events/active": {
      const truncated = params.truncated === true
      return addNotice(state, {
        kind: truncated ? "gap" : "active", mode: "push", name, cursor, truncated,
        text: truncated ? "Stream active — earlier events were dropped (truncated)" : "Stream active",
      }, now)
    }
    case "notifications/events/heartbeat":
      return addNotice(state, { kind: "heartbeat", mode: "push", name, cursor, truncated: false, text: "Heartbeat" }, now)
    case "notifications/events/terminated":
      return addNotice(state, {
        kind: "terminated", mode: "push", name, cursor, truncated: false,
        text: `Stream terminated${str(params.reason) ? ` (${str(params.reason)})` : ""}`,
      }, now)
    case "notifications/events/error":
      return addNotice(state, {
        kind: "error", mode: "push", name, cursor, truncated: false,
        text: `Stream error${str(params.message) ? `: ${str(params.message)}` : ""}`,
      }, now)
    default:
      return addNotice(state, { kind: "info", mode: "push", name, cursor, truncated: false, text: method }, now)
  }
}

function unescapeAttr(value: string): string {
  return value
    .replace(/&lt;/g, "<")
    .replace(/&gt;/g, ">")
    .replace(/&quot;/g, '"')
    .replace(/&amp;/g, "&")
}

/** Event data as text: JSON where it parses, else the text itself. */
function parseData(text: string): unknown {
  try {
    return JSON.parse(text)
  } catch {
    return text
  }
}

export interface EventPollResult {
  events: PushOccurrence[]
  nextCursor: string | null
  /** Earlier events were dropped before they were read. */
  truncated: boolean
  hasMore: boolean
}

/** Parse the text of an `EventPoll` tool result. */
export function parseEventPollResult(text: string): EventPollResult {
  const events: PushOccurrence[] = []
  const block = /<event id="([^"]*)" name="([^"]*)" server="[^"]*" ts="([^"]*)">\n<untrusted-data source="[^"]*">\n([\s\S]*?)\n<\/untrusted-data>\n<\/event>/g
  for (const match of text.matchAll(block)) {
    events.push({
      eventId: unescapeAttr(match[1]),
      name: unescapeAttr(match[2]),
      timestamp: match[3],
      data: parseData(match[4].replace(/<\\\/untrusted-data>/g, "</untrusted-data>")),
    })
  }
  const cursor = /^next_cursor: (\S+)$/m.exec(text)
  return {
    events,
    nextCursor: cursor ? cursor[1] : null,
    truncated: text.includes("earlier events were dropped"),
    hasMore: !text.includes("(no more events)"),
  }
}

export interface WatcherCommand {
  /** "Multi-shot" or "One-shot". */
  label: string
  command: string
}

/** The shell commands in an `EventWait` tool result, labelled. */
export function parseEventWaitCommands(text: string): WatcherCommand[] {
  const commands: WatcherCommand[] = []
  const fence = /```(?:bash|sh)?\n([\s\S]*?)\n```/g
  for (const match of text.matchAll(fence)) {
    const before = text.slice(0, match.index ?? 0)
    const label = before.lastIndexOf("ONE-SHOT") > before.lastIndexOf("MULTI-SHOT") ? "One-shot" : "Multi-shot"
    commands.push({ label, command: match[1].trim() })
  }
  return commands
}

/** Push capabilities the gateway advertised to this session. */
export interface PushAdvertisementView {
  eventsExtension: boolean
  eventsTopLevel: boolean
  claudeChannel: boolean
}

export const EVENTS_EXTENSION_ID = "io.modelcontextprotocol/events"

export function pushAdvertisement(raw: Record<string, unknown> | undefined): PushAdvertisementView {
  const extensions = (raw?.extensions ?? {}) as Record<string, unknown>
  const experimental = (raw?.experimental ?? {}) as Record<string, unknown>
  return {
    eventsExtension: EVENTS_EXTENSION_ID in extensions,
    eventsTopLevel: !!raw && "events" in raw,
    claudeChannel: "claude/channel" in experimental,
  }
}

/** The built-in test source's event type. */
export const TEST_EVENT_QUALIFIED_NAME = "localrouter__test.event"
export const TEST_SOURCE_SERVER_ID = "localrouter"
export const TEST_EVENT_NAME = "test.event"

/** Clamp a server's `nextPollMs` for auto-poll: at least 1 s, at most 5 min. */
export function autoPollDelay(nextPollMs: unknown): number {
  const ms = typeof nextPollMs === "number" && Number.isFinite(nextPollMs) ? nextPollMs : 1000
  return Math.min(300_000, Math.max(1000, ms))
}
