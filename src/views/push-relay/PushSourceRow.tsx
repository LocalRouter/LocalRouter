import { cn } from "@/lib/utils"
import type { McpServerPushStatus, PushPermission, PushSourceInfo } from "@/types/tauri-commands"
import {
  capabilitiesFooter,
  eventIssue,
  eventModesLabel,
  extrasLine,
  offersLine,
  pushLiveLine,
  type PushLiveTone,
} from "./push-relay-utils"

const TONE_DOT: Record<PushLiveTone, string> = {
  listening: "bg-emerald-500",
  idle: "bg-muted-foreground/40",
  warning: "bg-amber-500",
  error: "bg-red-500",
}

const TONE_TEXT: Record<PushLiveTone, string> = {
  listening: "text-foreground",
  idle: "text-muted-foreground",
  warning: "text-amber-600 dark:text-amber-400",
  error: "text-red-600 dark:text-red-400",
}

interface PushSourceDetailsProps {
  source: PushSourceInfo
  /** Live status from the running relay; null while it loads */
  status: McpServerPushStatus | null
  defaultPermission: PushPermission
  /** Include the "Capabilities checked … · MCP …" footer */
  showFooter?: boolean
}

/**
 * What one MCP server offers for push (one line per event type, how it can be
 * received, never `webhook` for a type LocalRouter can consume) and what
 * LocalRouter is doing with it right now, from the running relay.
 */
export function PushSourceDetails({ source, status, defaultPermission, showFooter = true }: PushSourceDetailsProps) {
  const offers = offersLine(source)
  const extras = source.snapshot_status === "current" ? extrasLine(source) : null
  const live = pushLiveLine(source, status, defaultPermission)
  const footer = showFooter ? capabilitiesFooter(source) : null
  return (
    <div className="space-y-2">
      {offers && <p className="text-sm text-muted-foreground">{offers}</p>}
      {source.snapshot_status === "current" && source.events.length > 0 && (
        <ul className="space-y-1">
          {source.events.map((event) => {
            const issue = eventIssue(status, event.name)
            return (
              <li key={event.name} className={cn("min-w-0", !event.supported && "opacity-70")}>
                <div className="text-sm">
                  <code className="text-xs">{event.name}</code>
                  <span className="text-muted-foreground"> — {eventModesLabel(event)}</span>
                </div>
                {event.description && (
                  <p className="text-xs text-muted-foreground truncate" title={event.description}>
                    {event.description}
                  </p>
                )}
                {issue && <p className="text-xs text-amber-600 dark:text-amber-400 truncate" title={issue}>{issue}</p>}
              </li>
            )
          })}
        </ul>
      )}
      {extras && <p className="text-sm text-muted-foreground">{extras}</p>}
      {live && (
        <div className={cn("flex items-start gap-2 text-sm", TONE_TEXT[live.tone])}>
          <span className={cn("mt-1.5 h-2 w-2 shrink-0 rounded-full", TONE_DOT[live.tone])} aria-hidden />
          <span className="min-w-0 break-words">{live.text}</span>
        </div>
      )}
      {footer && <p className="text-xs text-muted-foreground">{footer}</p>}
    </div>
  )
}

interface PushSourceRowProps extends PushSourceDetailsProps {
  /** Show the server name (off where the page already names the server) */
  showName?: boolean
}

/** One MCP server in the Push Relay page's Upstream sources list. */
export function PushSourceRow({ showName = true, ...props }: PushSourceRowProps) {
  return (
    <div className="rounded-md border p-3 space-y-2">
      {showName && <div className="text-sm font-medium">{props.source.server_name}</div>}
      <PushSourceDetails {...props} />
    </div>
  )
}
