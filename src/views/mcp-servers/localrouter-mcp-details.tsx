import { useEffect, useState } from "react"
import { invoke } from "@tauri-apps/api/core"
import { Copy, FlaskConical } from "lucide-react"
import { toast } from "sonner"
import { Logo } from "@/components/Logo"
import { Button } from "@/components/ui/Button"
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/Card"
import { ScrollArea } from "@/components/ui/scroll-area"
import { Tabs, TabsContent, TabsList, TabsTrigger } from "@/components/ui/tabs"
import { LOCALROUTER_MCP_NAME, localrouterMcpUrl } from "@/constants/localrouter-mcp"
import { TAB_ICONS, TAB_ICON_CLASS } from "@/constants/tab-icons"
import type { ServerConfigInfo } from "@/types/tauri-commands"
import { McpTab } from "@/views/try-it-out/mcp-tab"

export function LocalRouterMcpDetails({ onClients }: { onClients: () => void }) {
  const [tab, setTab] = useState("info")
  const [endpoint, setEndpoint] = useState<string>()
  const [failed, setFailed] = useState(false)

  useEffect(() => {
    let active = true
    invoke<ServerConfigInfo>("get_server_config")
      .then(config => {
        if (active) setEndpoint(localrouterMcpUrl(config.actual_port ?? config.port))
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
    <ScrollArea className="h-full">
      <div className="p-6 space-y-6">
        <header className="flex items-start justify-between gap-4">
          <div className="flex items-center gap-3">
            <Logo className="h-8 w-8 shrink-0" />
            <div>
              <h2 className="text-xl font-bold">{LOCALROUTER_MCP_NAME}</h2>
              <p className="text-sm text-muted-foreground">Built-in · Streamable HTTP</p>
            </div>
          </div>
          <Button variant="outline" size="sm" onClick={() => setTab("try-it-out")}>
            <FlaskConical className="h-4 w-4 mr-1" />
            Try It Out
          </Button>
        </header>

        <Tabs value={tab} onValueChange={setTab}>
          <TabsList>
            <TabsTrigger value="info"><TAB_ICONS.info className={TAB_ICON_CLASS} />Info</TabsTrigger>
            <TabsTrigger value="try-it-out"><TAB_ICONS.tryItOut className={TAB_ICON_CLASS} />Try It Out</TabsTrigger>
          </TabsList>
          <TabsContent value="info">
            <Card>
              <CardHeader className="pb-3">
                <CardTitle className="text-sm">Connection Details</CardTitle>
              </CardHeader>
              <CardContent className="space-y-4">
                <div>
                  <p className="text-sm text-muted-foreground">Local endpoint</p>
                  {endpoint ? (
                    <div className="flex items-center gap-2">
                      <code className="min-w-0 break-all text-sm">{endpoint}</code>
                      <Button size="icon" variant="ghost" aria-label="Copy local MCP endpoint" onClick={() => void copy()}>
                        <Copy className="h-4 w-4" />
                      </Button>
                    </div>
                  ) : (
                    <p className="text-sm text-muted-foreground">{failed ? "Could not load the endpoint." : "Loading endpoint…"}</p>
                  )}
                </div>
                <div>
                  <p className="text-sm text-muted-foreground">Authentication</p>
                  <p className="text-sm">Loopback connections require a bearer credential for an enabled client with LocalRouter access.</p>
                </div>
                <Button variant="outline" size="sm" onClick={onClients}>Manage Clients</Button>
              </CardContent>
            </Card>
          </TabsContent>
          <TabsContent value="try-it-out">
            <McpTab localrouterOnly innerPath={null} onPathChange={() => {}} onManageClients={onClients} />
          </TabsContent>
        </Tabs>
      </div>
    </ScrollArea>
  )
}
