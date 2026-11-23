import { useEffect, useState } from "react"
import { invoke } from "@tauri-apps/api/core"
import { Copy } from "lucide-react"
import { toast } from "sonner"
import { Button } from "@/components/ui/Button"
import { Card, CardContent, CardDescription, CardHeader, CardTitle } from "@/components/ui/Card"
import { LOCALROUTER_MCP_NAME, LOCALROUTER_MCP_PATH } from "@/constants/localrouter-mcp"
import type { ServerConfigInfo } from "@/types/tauri-commands"

export function LocalRouterMcpDetails({ onClients }: { onClients: () => void }) {
  const [endpoint, setEndpoint] = useState<string>()
  const [failed, setFailed] = useState(false)
  useEffect(() => {
    let active = true
    invoke<ServerConfigInfo>("get_server_config")
      .then(config => {
        if (active) setEndpoint(`http://localhost:${config.actual_port ?? config.port}${LOCALROUTER_MCP_PATH}`)
      })
      .catch(() => { if (active) setFailed(true) })
    return () => { active = false }
  }, [])

  async function copy() {
    if (!endpoint) return
    try {
      await navigator.clipboard.writeText(endpoint)
      toast.success("MCP endpoint copied")
    } catch {
      toast.error("Could not copy the endpoint")
    }
  }

  return (
    <Card>
      <CardHeader>
        <CardTitle>{LOCALROUTER_MCP_NAME}</CardTitle>
        <CardDescription>BUILT-IN</CardDescription>
      </CardHeader>
      <CardContent className="space-y-4">
        <div className="space-y-2">
          <div className="text-sm font-medium">Streamable HTTP endpoint</div>
          {endpoint ? (
            <div className="flex items-center gap-2">
              <code className="min-w-0 break-all rounded-md bg-muted px-3 py-2 text-sm">{endpoint}</code>
              <Button size="icon" variant="outline" aria-label="Copy MCP endpoint" onClick={() => void copy()}>
                <Copy className="h-4 w-4" />
              </Button>
            </div>
          ) : (
            <p className="text-sm text-muted-foreground">{failed ? "Could not load the server endpoint." : "Loading endpoint…"}</p>
          )}
        </div>
        <p className="text-sm text-muted-foreground">
          Enable LocalRouter under a client’s MCP Server Permissions. Add this endpoint to your MCP client using its LocalRouter credential.
          Call <code>lr_skills_get</code> to load a workflow and unlock its tools.
        </p>
        <Button variant="outline" onClick={onClients}>Open Clients</Button>
      </CardContent>
    </Card>
  )
}
