import { useCallback, useEffect, useMemo, useState } from "react"
import { invoke } from "@tauri-apps/api/core"
import {
  AlertTriangle, BellRing, CheckCircle2, ChevronDown, ChevronRight, Copy, Loader2, Play,
  RefreshCw, Send, Square, Trash2, XCircle,
} from "lucide-react"
import { toast } from "sonner"
import { Badge } from "@/components/ui/Badge"
import { Button } from "@/components/ui/Button"
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/Card"
import { Input } from "@/components/ui/Input"
import { Label } from "@/components/ui/label"
import { ScrollArea } from "@/components/ui/scroll-area"
import { Switch } from "@/components/ui/switch"
import { Tabs, TabsContent, TabsList, TabsTrigger } from "@/components/ui/tabs"
import { Textarea } from "@/components/ui/textarea"
import { cn } from "@/lib/utils"
import { PushPermissionButton } from "@/components/permissions/PushPermissionButton"
import type { McpConnectionState } from "@/lib/mcp-client"
import type {
  ClientPushSettings,
  GetClientPushSettingsParams,
  PreviewPushChannelMessageParams,
  PushChannelPreview,
  PushPermission,
  PushTestEventResult,
  SendPushTestEventParams,
  SetClientPushPermissionParams,
} from "@/types/tauri-commands"
import {
  PUSH_MODE_LABELS,
  TEST_EVENT_NAME,
  TEST_EVENT_QUALIFIED_NAME,
  TEST_SOURCE_SERVER_ID,
  pushAdvertisement,
  type PushDeliveryMode,
  type PushLogEntry,
  type PushLogEvent,
} from "./push-log"
import type { PushTester } from "./use-push-tester"

interface PushPanelProps {
  tester: PushTester
  connectionState: McpConnectionState
  /** The client this session runs as (Against Client mode), else null. */
  clientId: string | null
  clientName: string | null
}

type PermissionChoice = PushPermission | "inherit"

const MODE_BADGE: Record<PushDeliveryMode, string> = {
  push: "Push",
  poll: "Poll",
  tools: "Tools",
}

function Flag({ on, label, detail }: { on: boolean; label: string; detail?: string }) {
  return (
    <div className="flex items-start justify-between gap-3 py-1.5">
      <div className="min-w-0">
        <p className="text-sm">{label}</p>
        {detail && <p className="text-xs text-muted-foreground break-words [overflow-wrap:anywhere]">{detail}</p>}
      </div>
      {on ? (
        <Badge variant="success" className="shrink-0"><CheckCircle2 className="h-3 w-3 mr-1" />Offered</Badge>
      ) : (
        <Badge variant="outline" className="shrink-0 text-muted-foreground"><XCircle className="h-3 w-3 mr-1" />Not offered</Badge>
      )}
    </div>
  )
}

function time(iso: string): string {
  const d = new Date(iso)
  return Number.isNaN(d.getTime()) ? iso : d.toLocaleTimeString()
}

function short(cursor: string | null): string {
  if (!cursor) return "—"
  return cursor.length > 18 ? `${cursor.slice(0, 10)}…${cursor.slice(-6)}` : cursor
}

async function copy(text: string) {
  try {
    await navigator.clipboard.writeText(text)
    toast.success("Copied")
  } catch {
    toast.error("Could not copy to the clipboard")
  }
}

function LogRow({
  entry, expanded, onToggle, selected, onSelect,
}: {
  entry: PushLogEntry
  expanded: boolean
  onToggle: () => void
  selected: boolean
  onSelect: () => void
}) {
  if (entry.kind !== "event") {
    const tone =
      entry.kind === "gap" || entry.kind === "error"
        ? "text-amber-700 dark:text-amber-400"
        : entry.kind === "terminated"
          ? "text-destructive"
          : "text-muted-foreground"
    return (
      <div className={cn("flex items-center gap-2 px-2 py-1 text-xs", tone)}>
        <span className="w-20 shrink-0 tabular-nums">{time(entry.receivedAt)}</span>
        {(entry.kind === "gap" || entry.kind === "error") && <AlertTriangle className="h-3 w-3 shrink-0" />}
        <span className="shrink-0 rounded bg-muted px-1">{MODE_BADGE[entry.mode]}</span>
        <span className="truncate">{entry.text}{entry.name ? ` · ${entry.name}` : ""}</span>
        {entry.cursor && <code className="ml-auto shrink-0 opacity-70">{short(entry.cursor)}</code>}
      </div>
    )
  }
  return (
    <div className={cn("border-b px-2 py-1.5 text-xs", selected && "bg-accent/50")}>
      <div className="flex items-center gap-2">
        <button type="button" onClick={onToggle} className="shrink-0 text-muted-foreground" aria-label={expanded ? "Collapse data" : "Expand data"}>
          {expanded ? <ChevronDown className="h-3.5 w-3.5" /> : <ChevronRight className="h-3.5 w-3.5" />}
        </button>
        <button type="button" onClick={onSelect} className="flex min-w-0 flex-1 items-start gap-2 text-left" title="Preview as a Claude Code channel message">
          <span className="w-20 shrink-0 tabular-nums text-muted-foreground">{time(entry.receivedAt)}</span>
          <span className="min-w-0">
            <span className="block truncate font-medium">{entry.name}</span>
            <code className="block truncate text-muted-foreground">{entry.eventId}</code>
          </span>
        </button>
        <div className="flex shrink-0 gap-1">
          {entry.modes.map((m, i) => (
            <span
              key={m}
              className={cn("rounded px-1", i === 0 ? "bg-primary/10 text-primary" : "bg-muted text-muted-foreground")}
              title={i === 0 ? `First received via ${PUSH_MODE_LABELS[m]}` : `Also received via ${PUSH_MODE_LABELS[m]} (deduplicated by event id)`}
            >
              {MODE_BADGE[m]}
            </span>
          ))}
        </div>
        <code className="hidden shrink-0 text-muted-foreground md:inline" title={entry.cursor ?? undefined}>{short(entry.cursor)}</code>
      </div>
      {expanded && (
        <pre className="mt-1 max-h-60 overflow-auto rounded bg-muted p-2 text-[11px]">
          {JSON.stringify(entry.data, null, 2)}
        </pre>
      )}
    </div>
  )
}

export function PushPanel({ tester, connectionState, clientId, clientName }: PushPanelProps) {
  const ad = pushAdvertisement(connectionState.rawServerCapabilities)
  const eventsAdvertised = ad.eventsExtension || ad.eventsTopLevel
  const [mode, setMode] = useState<PushDeliveryMode>("push")
  const [expanded, setExpanded] = useState<Set<string>>(new Set())
  const [previewKey, setPreviewKey] = useState<string | null>(null)
  const [preview, setPreview] = useState<PushChannelPreview | null>(null)
  const [previewError, setPreviewError] = useState<string | null>(null)

  // Test source
  const [settings, setSettings] = useState<ClientPushSettings | null>(null)
  const [message, setMessage] = useState("")
  const [dataText, setDataText] = useState("")
  const [sending, setSending] = useState(false)
  const [lastSent, setLastSent] = useState<string | null>(null)

  const testKey = `${TEST_SOURCE_SERVER_ID}__${TEST_EVENT_NAME}`
  const testPermission: PermissionChoice = settings?.permissions.events?.[testKey] ?? "inherit"
  const inherited: PushPermission | null = settings
    ? settings.permissions.servers?.[TEST_SOURCE_SERVER_ID] ?? settings.effective_client
    : null

  const loadSettings = useCallback(async () => {
    if (!clientId) {
      setSettings(null)
      return
    }
    try {
      setSettings(await invoke<ClientPushSettings>("get_client_push_settings", { clientId } satisfies GetClientPushSettingsParams))
    } catch {
      setSettings(null)
    }
  }, [clientId])

  useEffect(() => { void loadSettings() }, [loadSettings])

  const setTestPermission = async (choice: PermissionChoice) => {
    if (!clientId) return
    try {
      await invoke("set_client_push_permission", {
        clientId,
        level: "event",
        serverId: TEST_SOURCE_SERVER_ID,
        eventName: TEST_EVENT_NAME,
        state: choice === "inherit" ? null : choice,
      } satisfies SetClientPushPermissionParams)
      await loadSettings()
      tester.refreshEvents()
    } catch (error) {
      toast.error(`Failed to update the test event permission: ${error}`)
    }
  }

  const sendTestEvent = async () => {
    let data: unknown = undefined
    if (dataText.trim()) {
      try {
        data = JSON.parse(dataText)
      } catch {
        toast.error("Data must be valid JSON")
        return
      }
    }
    setSending(true)
    try {
      const result = await invoke<PushTestEventResult>("send_push_test_event", {
        clientId,
        message: message.trim() || null,
        data: data ?? null,
      } satisfies SendPushTestEventParams)
      const effective = testPermission === "inherit" ? inherited : testPermission
      setLastSent(
        effective === "ask"
          ? `Sent ${result.event_id} — held for approval in the popup`
          : effective === "off"
            ? `Sent ${result.event_id} — off for this client (see Monitor)`
            : `Sent ${result.event_id}`,
      )
    } catch (error) {
      toast.error(String(error))
    } finally {
      setSending(false)
    }
  }

  const events = useMemo(
    () => tester.log.entries.filter((e): e is PushLogEvent => e.kind === "event"),
    [tester.log.entries],
  )
  // Preview the clicked row, else the newest event of the selected type, else a sample
  const previewEvent = useMemo(() => {
    const clicked = events.find(e => e.key === previewKey)
    if (clicked) return { eventId: clicked.eventId, name: clicked.name, timestamp: clicked.timestamp, data: clicked.data }
    const latest = events.find(e => e.name === tester.selected)
    if (latest) return { eventId: latest.eventId, name: latest.name, timestamp: latest.timestamp, data: latest.data }
    if (!tester.selected) return null
    const slug = tester.selected.split("__")[0]
    return {
      eventId: `${slug}__example`,
      name: tester.selected,
      data: { message: "Example data — receive an event to preview it exactly" },
    }
  }, [events, previewKey, tester.selected])

  // Keyed by content: heartbeats re-render the log without changing the event
  const previewJson = previewEvent ? JSON.stringify(previewEvent) : null
  useEffect(() => {
    if (!previewJson) {
      setPreview(null)
      return
    }
    let cancelled = false
    const event = JSON.parse(previewJson) as PreviewPushChannelMessageParams["event"]
    invoke<PushChannelPreview>("preview_push_channel_message", { event } satisfies PreviewPushChannelMessageParams)
      .then(p => { if (!cancelled) { setPreview(p); setPreviewError(null) } })
      .catch(e => { if (!cancelled) { setPreview(null); setPreviewError(String(e)) } })
    return () => { cancelled = true }
  }, [previewJson])

  const streamsForSelected = tester.streams.filter(s => s.name === tester.selected)
  const activeStream = streamsForSelected.find(s => s.status === "active")
  const hasPoll = tester.fallbackTools.includes("EventPoll")
  const hasWait = tester.fallbackTools.includes("EventWait")

  return (
    <ScrollArea className="h-full">
      <div className="grid gap-4 pr-4 lg:grid-cols-[minmax(280px,360px)_1fr]">
        {/* Left column: what was advertised, the test source, the catalogue */}
        <div className="space-y-4">
          <Card>
            <CardHeader className="pb-2">
              <CardTitle className="text-sm flex items-center gap-2"><BellRing className="h-4 w-4" />Advertised to this session</CardTitle>
            </CardHeader>
            <CardContent className="divide-y pt-0">
              <Flag on={ad.eventsExtension} label="MCP Events extension" detail="capabilities.extensions[&quot;io.modelcontextprotocol/events&quot;]" />
              <Flag on={ad.eventsTopLevel} label="Top-level events" detail="capabilities.events (read by ChatGPT and OpenClaw)" />
              <Flag
                on={ad.claudeChannel}
                label="Claude Code Channels"
                detail="experimental[&quot;claude/channel&quot;] is offered only to STDIO bridge sessions; this HTTP test session never gets it. The preview below shows what a bridge session receives."
              />
              <Flag
                on={tester.fallbackTools.length > 0}
                label="Fallback tools"
                detail={tester.fallbackTools.length > 0 ? tester.fallbackTools.join(", ") : "EventList, EventPoll, EventWait — shown unless the session declares MCP Events"}
              />
              {!eventsAdvertised && (
                <p className="pt-2 text-xs text-muted-foreground">
                  Nothing was advertised: no upstream push source this client may use was known when the session started.
                  The test source is offered to Try it out sessions against a client — reconnect after switching to Against Client.
                </p>
              )}
            </CardContent>
          </Card>

          <Card>
            <CardHeader className="pb-2">
              <CardTitle className="text-sm">Test event source</CardTitle>
              <p className="text-xs text-muted-foreground">
                <code>{TEST_EVENT_QUALIFIED_NAME}</code> runs through the real pipeline — permission, approval popup, Monitor, journal and
                every delivery mode. Only sessions that opted into push testing see it: this one, or a STDIO bridge started with{" "}
                <code>LOCALROUTER_PUSH_TEST=1</code>.
              </p>
            </CardHeader>
            <CardContent className="space-y-3 pt-0">
              {clientId ? (
                <>
                  <div className="space-y-1.5">
                    <Label className="text-xs">Permission for {clientName ?? "this client"}</Label>
                    <div>
                      <PushPermissionButton
                        value={testPermission === "inherit" ? (inherited ?? "allow") : testPermission}
                        inherited={testPermission === "inherit"}
                        onChange={choice => void setTestPermission(choice)}
                        size="sm"
                      />
                    </div>
                    <p className="text-[11px] text-muted-foreground">Ask holds each event for approval in the popup.</p>
                  </div>
                  <div className="space-y-1.5">
                    <Label className="text-xs" htmlFor="push-test-message">Message</Label>
                    <Input id="push-test-message" className="h-8" placeholder="Test event from LocalRouter" value={message} onChange={e => setMessage(e.target.value)} />
                  </div>
                  <div className="space-y-1.5">
                    <Label className="text-xs" htmlFor="push-test-data">Data (JSON, optional — replaces the message)</Label>
                    <Textarea id="push-test-data" className="min-h-[60px] font-mono text-xs" placeholder='{"text": "hello"}' value={dataText} onChange={e => setDataText(e.target.value)} />
                  </div>
                  <Button size="sm" onClick={() => void sendTestEvent()} disabled={sending}>
                    {sending ? <Loader2 className="h-3.5 w-3.5 mr-1.5 animate-spin" /> : <Send className="h-3.5 w-3.5 mr-1.5" />}
                    Send test event
                  </Button>
                  {lastSent && <p className="text-xs text-muted-foreground break-all">{lastSent}</p>}
                </>
              ) : (
                <p className="text-xs text-muted-foreground">
                  Push permissions belong to a client. Connect in <span className="font-medium">Against Client</span> mode to send test events.
                </p>
              )}
            </CardContent>
          </Card>

          <Card>
            <CardHeader className="pb-2">
              <div className="flex items-center justify-between">
                <CardTitle className="text-sm">Event types (events/list)</CardTitle>
                <Button variant="ghost" size="sm" className="h-7" onClick={tester.refreshEvents} disabled={tester.loadingEvents}>
                  <RefreshCw className={cn("h-3.5 w-3.5", tester.loadingEvents && "animate-spin")} />
                </Button>
              </div>
            </CardHeader>
            <CardContent className="space-y-1 pt-0">
              {tester.eventsError && <p className="text-xs text-destructive">{tester.eventsError}</p>}
              {!tester.eventsError && tester.eventTypes.length === 0 && (
                <p className="text-xs text-muted-foreground">No event types for this session.</p>
              )}
              {tester.eventTypes.map(t => (
                <button
                  key={t.name}
                  type="button"
                  onClick={() => tester.setSelected(t.name)}
                  className={cn(
                    "w-full rounded-md border px-2 py-1.5 text-left",
                    tester.selected === t.name ? "border-primary bg-primary/5" : "hover:bg-muted",
                  )}
                >
                  <div className="flex items-center gap-2">
                    <span className="truncate text-sm font-medium">{t.name}</span>
                    {t.name === TEST_EVENT_QUALIFIED_NAME && <Badge variant="info" className="text-[10px] px-1.5 py-0">test source</Badge>}
                  </div>
                  {t.description && <p className="text-xs text-muted-foreground line-clamp-2">{t.description}</p>}
                  <div className="mt-1 flex gap-1">
                    {t.delivery.map(d => (
                      <span key={d} className="rounded bg-muted px-1 text-[10px] text-muted-foreground">{d}</span>
                    ))}
                  </div>
                </button>
              ))}
            </CardContent>
          </Card>
        </div>

        {/* Right column: receive, log, channel preview */}
        <div className="min-w-0 space-y-4">
          <Card>
            <CardHeader className="pb-2">
              <CardTitle className="text-sm">
                Receive {tester.selected ? <code className="ml-1 font-normal">{tester.selected}</code> : <span className="font-normal text-muted-foreground">— pick an event type</span>}
              </CardTitle>
            </CardHeader>
            <CardContent className="pt-0">
              <Tabs value={mode} onValueChange={v => setMode(v as PushDeliveryMode)}>
                <TabsList className="h-8">
                  <TabsTrigger value="push" className="text-xs">Push</TabsTrigger>
                  <TabsTrigger value="poll" className="text-xs">Poll</TabsTrigger>
                  <TabsTrigger value="tools" className="text-xs">Fallback tools</TabsTrigger>
                </TabsList>

                <TabsContent value="push" className="space-y-2 pt-2">
                  <p className="text-xs text-muted-foreground">
                    <code>events/stream</code> stays open; its active, heartbeat and event notifications arrive on this session's SSE connection.
                    Stop sends <code>notifications/cancelled</code>.
                  </p>
                  <div className="flex items-center gap-2">
                    {activeStream ? (
                      <Button size="sm" variant="outline" onClick={() => tester.stopStream(activeStream.id)}>
                        <Square className="h-3.5 w-3.5 mr-1.5" />Stop stream
                      </Button>
                    ) : (
                      <Button size="sm" onClick={() => tester.selected && tester.startStream(tester.selected)} disabled={!tester.selected}>
                        <Play className="h-3.5 w-3.5 mr-1.5" />Start stream
                      </Button>
                    )}
                  </div>
                  {tester.streams.length > 0 && (
                    <div className="space-y-1">
                      {tester.streams.map(s => (
                        <div key={s.id} className="flex items-center gap-2 text-xs">
                          <span className={cn("h-2 w-2 rounded-full", s.status === "active" ? "bg-green-500" : s.status === "error" ? "bg-destructive" : "bg-muted-foreground")} />
                          <code className="truncate">{s.name}</code>
                          <span className="text-muted-foreground">#{s.id}</span>
                          <span className="text-muted-foreground">
                            {s.status === "active"
                              ? s.lastHeartbeat ? `heartbeat ${time(s.lastHeartbeat)}` : `open since ${time(s.openedAt)}`
                              : s.detail ?? s.status}
                          </span>
                          <Button variant="ghost" size="sm" className="ml-auto h-6 px-2" onClick={() => tester.stopStream(s.id)}>
                            {s.status === "active" ? "Stop" : "Remove"}
                          </Button>
                        </div>
                      ))}
                    </div>
                  )}
                </TabsContent>

                <TabsContent value="poll" className="space-y-2 pt-2">
                  <p className="text-xs text-muted-foreground">
                    <code>events/poll</code> with a cursor. The first poll starts at now (no history); auto-poll waits the server's <code>nextPollMs</code>.
                  </p>
                  <div className="flex flex-wrap items-center gap-3">
                    <Button size="sm" onClick={tester.pollNow} disabled={!tester.selected || tester.pollBusy}>
                      {tester.pollBusy ? <Loader2 className="h-3.5 w-3.5 mr-1.5 animate-spin" /> : <RefreshCw className="h-3.5 w-3.5 mr-1.5" />}
                      Poll now
                    </Button>
                    <label className="flex items-center gap-2 text-xs">
                      <Switch checked={tester.autoPoll} onCheckedChange={tester.setAutoPoll} disabled={!tester.selected} />
                      Auto-poll
                    </label>
                    <Button size="sm" variant="ghost" onClick={tester.resetPollCursor} disabled={!tester.pollCursor}>Reset cursor</Button>
                  </div>
                  <div className="text-xs text-muted-foreground">
                    Cursor <code title={tester.pollCursor ?? undefined}>{short(tester.pollCursor)}</code>
                    {tester.nextPollMs !== null && <> · nextPollMs {tester.nextPollMs}</>}
                  </div>
                  {tester.pollError && <p className="text-xs text-destructive">{tester.pollError}</p>}
                </TabsContent>

                <TabsContent value="tools" className="space-y-2 pt-2">
                  {tester.fallbackTools.length === 0 ? (
                    <p className="text-xs text-muted-foreground">This session was not given the fallback tools.</p>
                  ) : (
                    <>
                      <div className="flex flex-wrap items-end gap-3">
                        <div className="space-y-1">
                          <Label className="text-xs" htmlFor="push-wait-seconds">wait_seconds</Label>
                          <Input
                            id="push-wait-seconds"
                            type="number"
                            min={0}
                            max={50}
                            className="h-8 w-24"
                            value={tester.waitSeconds}
                            onChange={e => tester.setWaitSeconds(Math.max(0, Math.min(50, Number(e.target.value) || 0)))}
                          />
                        </div>
                        <Button size="sm" onClick={tester.callEventPoll} disabled={!tester.selected || !hasPoll || tester.toolBusy}>
                          {tester.toolBusy ? <Loader2 className="h-3.5 w-3.5 mr-1.5 animate-spin" /> : <Play className="h-3.5 w-3.5 mr-1.5" />}
                          EventPoll
                        </Button>
                        <Button size="sm" variant="outline" onClick={tester.callEventWait} disabled={!tester.selected || !hasWait || tester.toolBusy}>
                          EventWait
                        </Button>
                      </div>
                      <div className="text-xs text-muted-foreground">
                        Cursor <code title={tester.toolCursor ?? undefined}>{short(tester.toolCursor)}</code>
                      </div>
                      {tester.toolError && <p className="text-xs text-destructive">{tester.toolError}</p>}
                      {tester.waitCommands.map(c => (
                        <div key={c.label} className="space-y-1">
                          <div className="flex items-center justify-between">
                            <span className="text-xs font-medium">{c.label}</span>
                            <Button variant="ghost" size="sm" className="h-6 px-2" onClick={() => void copy(c.command)}>
                              <Copy className="h-3 w-3 mr-1" />Copy
                            </Button>
                          </div>
                          <pre className="whitespace-pre-wrap break-all rounded bg-muted p-2 text-[11px]">{c.command}</pre>
                        </div>
                      ))}
                      {tester.waitText && tester.waitCommands.length === 0 && (
                        <pre className="whitespace-pre-wrap rounded bg-muted p-2 text-[11px]">{tester.waitText}</pre>
                      )}
                    </>
                  )}
                </TabsContent>
              </Tabs>
            </CardContent>
          </Card>

          <Card>
            <CardHeader className="pb-2">
              <div className="flex items-center justify-between">
                <CardTitle className="text-sm">
                  Received <span className="font-normal text-muted-foreground">({events.length} event{events.length === 1 ? "" : "s"}, deduplicated by event id)</span>
                </CardTitle>
                <Button variant="ghost" size="sm" className="h-7" onClick={tester.clearLog} disabled={tester.log.entries.length === 0}>
                  <Trash2 className="h-3.5 w-3.5 mr-1" />Clear
                </Button>
              </div>
            </CardHeader>
            <CardContent className="pt-0">
              {tester.log.entries.length === 0 ? (
                <p className="py-6 text-center text-xs text-muted-foreground">Start a stream, poll, or call EventPoll, then send a test event.</p>
              ) : (
                <div className="max-h-[360px] overflow-auto rounded-md border">
                  {tester.log.entries.map(entry => (
                    <LogRow
                      key={entry.key}
                      entry={entry}
                      expanded={expanded.has(entry.key)}
                      onToggle={() => setExpanded(prev => {
                        const next = new Set(prev)
                        if (next.has(entry.key)) next.delete(entry.key)
                        else next.add(entry.key)
                        return next
                      })}
                      selected={previewKey === entry.key}
                      onSelect={() => setPreviewKey(entry.key)}
                    />
                  ))}
                </div>
              )}
            </CardContent>
          </Card>

          <Card>
            <CardHeader className="pb-2">
              <CardTitle className="text-sm">Claude Code channel preview</CardTitle>
              <p className="text-xs text-muted-foreground">
                The <code>notifications/claude/channel</code> a STDIO bridge session receives for{" "}
                {previewEvent ? <code>{previewEvent.eventId}</code> : "the selected event"}, rendered by the relay itself. Click a received event to preview it.
              </p>
            </CardHeader>
            <CardContent className="space-y-2 pt-0">
              {previewError && <p className="text-xs text-destructive">{previewError}</p>}
              {preview ? (
                <>
                  <div>
                    <p className="mb-1 text-xs font-medium">content</p>
                    <pre className="max-h-48 overflow-auto whitespace-pre-wrap rounded bg-muted p-2 text-[11px]">{preview.content}</pre>
                  </div>
                  <div>
                    <p className="mb-1 text-xs font-medium">meta</p>
                    <div className="grid grid-cols-[auto_1fr] gap-x-3 gap-y-0.5 rounded bg-muted p-2 text-[11px]">
                      {Object.entries(preview.meta).map(([k, v]) => (
                        <div key={k} className="contents">
                          <code className="text-muted-foreground">{k}</code>
                          <code className="break-all">{v}</code>
                        </div>
                      ))}
                    </div>
                  </div>
                </>
              ) : (
                !previewError && <p className="text-xs text-muted-foreground">Pick an event type to preview.</p>
              )}
            </CardContent>
          </Card>
        </div>
      </div>
    </ScrollArea>
  )
}
