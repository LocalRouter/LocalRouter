
import { useState, useEffect } from "react"
import { invoke } from "@tauri-apps/api/core"
import { listen } from "@tauri-apps/api/event"
import { ChevronDown, ChevronUp, Pause, Play } from "lucide-react"
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/Card"
import { Button } from "@/components/ui/Button"
import { Badge } from "@/components/ui/Badge"
import { ScrollArea } from "@/components/ui/scroll-area"
import {
  Collapsible,
  CollapsibleContent,
  CollapsibleTrigger,
} from "@/components/ui/collapsible"
import { cn } from "@/lib/utils"

// Backend AccessLogEntry structure from src-tauri/src/monitoring/logger.rs
interface AccessLogEntry {
  timestamp: string
  api_key_name: string
  provider: string
  model: string
  status: string
  status_code: number
  input_tokens: number
  output_tokens: number
  total_tokens: number
  cost_usd: number
  latency_ms: number
  request_id: string
  routellm_win_rate?: number
}

// Mapped entry for UI display
interface LogEntry {
  id: string
  timestamp: string
  client_name?: string
  client_id?: string
  action: string
  status: "success" | "error" | "pending"
  latency_ms?: number
  model?: string
  provider?: string
  tokens?: number
}

// Map backend AccessLogEntry to frontend LogEntry
function mapAccessLogEntry(entry: AccessLogEntry): LogEntry {
  return {
    id: entry.request_id,
    timestamp: entry.timestamp,
    client_name: entry.api_key_name,
    action: `${entry.provider}/${entry.model}`,
    status: entry.status === "success" ? "success" : "error",
    latency_ms: entry.latency_ms,
    model: entry.model,
    provider: entry.provider,
    tokens: entry.total_tokens,
  }
}

interface ActivityLogProps {
  refreshTrigger?: number
  maxEntries?: number
}

export function ActivityLog({
  refreshTrigger = 0,
  maxEntries = 50,
}: ActivityLogProps) {
  const [logs, setLogs] = useState<LogEntry[]>([])
  const [isOpen, setIsOpen] = useState(true)
  const [isPaused, setIsPaused] = useState(false)
  const [loading, setLoading] = useState(true)

  useEffect(() => {
    loadLogs()

    // Subscribe to log updates (backend emits "llm-log-entry")
    const unsubscribe = listen("llm-log-entry", (event: any) => {
      if (!isPaused) {
        const backendEntry = event.payload as AccessLogEntry
        const newEntry = mapAccessLogEntry(backendEntry)
        setLogs((prev) => [newEntry, ...prev].slice(0, maxEntries))
      }
    })

    return () => {
      unsubscribe.then((fn) => fn())
    }
  }, [isPaused, maxEntries])

  useEffect(() => {
    if (!isPaused) {
      loadLogs()
    }
  }, [refreshTrigger, isPaused])

  const loadLogs = async () => {
    try {
      setLoading(true)
      const backendEntries = await invoke<AccessLogEntry[]>("get_llm_logs", {
        limit: maxEntries,
      })
      setLogs(backendEntries.map(mapAccessLogEntry))
    } catch (error) {
      console.error("Failed to load access logs:", error)
      // If the command doesn't exist, just use empty array
      setLogs([])
    } finally {
      setLoading(false)
    }
  }

  const formatTime = (timestamp: string) => {
    const date = new Date(timestamp)
    return date.toLocaleTimeString("en-US", {
      hour: "numeric",
      minute: "2-digit",
      second: "2-digit",
      hour12: true,
    })
  }

  const getStatusBadge = (status: LogEntry["status"]) => {
    switch (status) {
      case "success":
        return <Badge variant="success">OK</Badge>
      case "error":
        return <Badge variant="destructive">ERR</Badge>
      case "pending":
        return <Badge variant="secondary">...</Badge>
    }
  }

  return (
    <Collapsible open={isOpen} onOpenChange={setIsOpen}>
      <Card>
        <CardHeader className="flex flex-row items-center justify-between space-y-0 py-3">
          <CollapsibleTrigger asChild>
            <Button variant="ghost" className="flex items-center gap-2 p-0 h-auto">
              {isOpen ? (
                <ChevronDown className="h-4 w-4" />
              ) : (
                <ChevronUp className="h-4 w-4" />
              )}
              <CardTitle className="text-base font-medium">
                Live Activity
              </CardTitle>
              {!isOpen && (
                <Badge variant="secondary" className="ml-2">
                  {logs.length}
                </Badge>
              )}
            </Button>
          </CollapsibleTrigger>
          <Button
            variant="ghost"
            size="icon"
            onClick={() => setIsPaused(!isPaused)}
            className={cn(isPaused && "text-yellow-600")}
          >
            {isPaused ? (
              <Play className="h-4 w-4" />
            ) : (
              <Pause className="h-4 w-4" />
            )}
            <span className="sr-only">{isPaused ? "Resume" : "Pause"}</span>
          </Button>
        </CardHeader>

        <CollapsibleContent>
          <CardContent className="pt-0">
            {loading ? (
              <div className="flex items-center justify-center py-8 text-muted-foreground">
                Loading activity...
              </div>
            ) : logs.length === 0 ? (
              <div className="flex items-center justify-center py-8 text-muted-foreground">
                No recent activity
              </div>
            ) : (
              <ScrollArea className="h-[300px]">
                <div className="space-y-1">
                  {logs.map((log, index) => (
                    <div
                      key={log.id || index}
                      className="flex items-center gap-4 rounded-md px-2 py-1.5 text-sm hover:bg-muted/50"
                    >
                      <span className="w-20 shrink-0 text-xs text-muted-foreground">
                        {formatTime(log.timestamp)}
                      </span>
                      <span className="w-24 shrink-0 truncate font-medium">
                        {log.client_name || log.client_id?.slice(0, 8) || "-"}
                      </span>
                      <span className="flex-1 truncate text-muted-foreground">
                        {log.action}
                        {log.model && (
                          <span className="ml-2 text-xs">({log.model})</span>
                        )}
                      </span>
                      <span className="w-12 shrink-0">
                        {getStatusBadge(log.status)}
                      </span>
                      {log.latency_ms !== undefined && (
                        <span className="w-16 shrink-0 text-right text-xs text-muted-foreground">
                          {log.latency_ms}ms
                        </span>
                      )}
                    </div>
                  ))}
                </div>
              </ScrollArea>
            )}
          </CardContent>
        </CollapsibleContent>
      </Card>
    </Collapsible>
  )
}
