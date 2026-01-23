
import { useState, useEffect } from "react"
import { invoke } from "@tauri-apps/api/core"
import { RefreshCw } from "lucide-react"
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/Card"
import { Button } from "@/components/ui/Button"
import { Badge } from "@/components/ui/Badge"
import { ScrollArea } from "@/components/ui/scroll-area"

interface Client {
  id: string
  name: string
  client_id: string
}

interface LogEntry {
  id: string
  timestamp: string
  method: string
  path: string
  status_code: number
  latency_ms: number
  model?: string
  provider?: string
  tokens_used?: number
  error?: string
}

interface LogsTabProps {
  client: Client
  refreshTrigger?: number
}

export function ClientLogsTab({ client, refreshTrigger = 0 }: LogsTabProps) {
  const [logs, setLogs] = useState<LogEntry[]>([])
  const [loading, setLoading] = useState(true)

  useEffect(() => {
    loadLogs()
  }, [client.client_id, refreshTrigger])

  const loadLogs = async () => {
    try {
      setLoading(true)
      const logEntries = await invoke<LogEntry[]>("get_client_access_logs", {
        clientId: client.client_id,
        limit: 100,
      })
      setLogs(logEntries)
    } catch (error) {
      console.error("Failed to load logs:", error)
      // If command doesn't exist, use empty array
      setLogs([])
    } finally {
      setLoading(false)
    }
  }

  const formatTime = (timestamp: string) => {
    const date = new Date(timestamp)
    return date.toLocaleString("en-US", {
      month: "short",
      day: "numeric",
      hour: "numeric",
      minute: "2-digit",
      second: "2-digit",
      hour12: true,
    })
  }

  const getStatusBadge = (status: number) => {
    if (status >= 200 && status < 300) {
      return <Badge variant="success">{status}</Badge>
    } else if (status >= 400 && status < 500) {
      return <Badge variant="warning">{status}</Badge>
    } else if (status >= 500) {
      return <Badge variant="destructive">{status}</Badge>
    }
    return <Badge variant="secondary">{status}</Badge>
  }

  return (
    <Card>
      <CardHeader className="flex flex-row items-center justify-between">
        <CardTitle className="text-base">Request Logs</CardTitle>
        <Button variant="ghost" size="icon" onClick={loadLogs} disabled={loading}>
          <RefreshCw className={`h-4 w-4 ${loading ? "animate-spin" : ""}`} />
        </Button>
      </CardHeader>
      <CardContent>
        {loading ? (
          <div className="flex items-center justify-center py-8 text-muted-foreground">
            Loading logs...
          </div>
        ) : logs.length === 0 ? (
          <div className="flex items-center justify-center py-8 text-muted-foreground">
            No logs found for this client
          </div>
        ) : (
          <ScrollArea className="h-[400px]">
            <div className="space-y-2">
              {logs.map((log, index) => (
                <div
                  key={log.id || index}
                  className="flex items-center gap-4 rounded-md border p-3 text-sm"
                >
                  <span className="w-32 shrink-0 text-xs text-muted-foreground">
                    {formatTime(log.timestamp)}
                  </span>
                  <span className="w-16 shrink-0 font-mono text-xs">
                    {log.method}
                  </span>
                  <span className="flex-1 truncate font-mono text-xs text-muted-foreground">
                    {log.path}
                  </span>
                  {log.model && (
                    <span className="text-xs text-muted-foreground">
                      {log.model}
                    </span>
                  )}
                  <span className="w-12 shrink-0">
                    {getStatusBadge(log.status_code)}
                  </span>
                  <span className="w-16 shrink-0 text-right text-xs text-muted-foreground">
                    {log.latency_ms}ms
                  </span>
                </div>
              ))}
            </div>
          </ScrollArea>
        )}
      </CardContent>
    </Card>
  )
}
