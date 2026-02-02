import { Info, Server, Monitor, CheckCircle2, XCircle, Zap, FileText, MessageSquare, Wrench, HelpCircle, Radio, FolderTree } from "lucide-react"
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/Card"
import { Badge } from "@/components/ui/Badge"
import { ScrollArea } from "@/components/ui/scroll-area"
import { Separator } from "@/components/ui/separator"
import type { McpConnectionState } from "@/lib/mcp-client"

interface ConnectionInfoPanelProps {
  connectionState: McpConnectionState
}

function CapabilityItem({
  label,
  enabled,
  details,
  icon: Icon,
}: {
  label: string
  enabled: boolean
  details?: string
  icon?: React.ComponentType<{ className?: string }>
}) {
  return (
    <div className="flex items-center justify-between py-2">
      <div className="flex items-center gap-2">
        {Icon && <Icon className="h-4 w-4 text-muted-foreground" />}
        <span className="text-sm">{label}</span>
        {details && (
          <span className="text-xs text-muted-foreground">({details})</span>
        )}
      </div>
      {enabled ? (
        <Badge variant="outline" className="bg-green-50 text-green-700 border-green-200 dark:bg-green-950 dark:text-green-300 dark:border-green-800">
          <CheckCircle2 className="h-3 w-3 mr-1" />
          Enabled
        </Badge>
      ) : (
        <Badge variant="outline" className="bg-muted text-muted-foreground">
          <XCircle className="h-3 w-3 mr-1" />
          Disabled
        </Badge>
      )}
    </div>
  )
}

export function ConnectionInfoPanel({ connectionState }: ConnectionInfoPanelProps) {
  const { isConnected, serverInfo, clientInfo, serverCapabilities, clientCapabilities } = connectionState

  if (!isConnected) {
    return (
      <div className="flex items-center justify-center h-full text-muted-foreground">
        <div className="text-center">
          <Info className="h-8 w-8 mx-auto mb-2 opacity-50" />
          <p>Connect to an MCP server to view connection details</p>
        </div>
      </div>
    )
  }

  return (
    <ScrollArea className="h-full">
      <div className="space-y-4 pr-4">
        {/* Server Information */}
        <Card>
          <CardHeader className="pb-3">
            <CardTitle className="text-sm flex items-center gap-2">
              <Server className="h-4 w-4" />
              Server Information
            </CardTitle>
          </CardHeader>
          <CardContent className="space-y-3">
            <div className="grid grid-cols-2 gap-4">
              <div>
                <p className="text-xs text-muted-foreground">Name</p>
                <p className="text-sm font-medium">{serverInfo?.name || "Unknown"}</p>
              </div>
              <div>
                <p className="text-xs text-muted-foreground">Version</p>
                <p className="text-sm font-medium">{serverInfo?.version || "Unknown"}</p>
              </div>
              <div>
                <p className="text-xs text-muted-foreground">Protocol Version</p>
                <p className="text-sm font-medium">{serverInfo?.protocolVersion || "Unknown"}</p>
              </div>
            </div>

          </CardContent>
        </Card>

        {/* Client Information */}
        <Card>
          <CardHeader className="pb-3">
            <CardTitle className="text-sm flex items-center gap-2">
              <Monitor className="h-4 w-4" />
              Client Information
            </CardTitle>
          </CardHeader>
          <CardContent>
            <div className="grid grid-cols-2 gap-4">
              <div>
                <p className="text-xs text-muted-foreground">Name</p>
                <p className="text-sm font-medium">{clientInfo?.name || "Unknown"}</p>
              </div>
              <div>
                <p className="text-xs text-muted-foreground">Version</p>
                <p className="text-sm font-medium">{clientInfo?.version || "Unknown"}</p>
              </div>
            </div>
          </CardContent>
        </Card>

        {/* Server Capabilities */}
        <Card>
          <CardHeader className="pb-3">
            <CardTitle className="text-sm flex items-center gap-2">
              <Zap className="h-4 w-4" />
              Server Capabilities
              <span className="text-xs text-muted-foreground font-normal">
                (what the server provides)
              </span>
            </CardTitle>
          </CardHeader>
          <CardContent className="divide-y">
            <CapabilityItem
              icon={Wrench}
              label="Tools"
              enabled={!!serverCapabilities?.tools}
              details={serverCapabilities?.tools?.listChanged ? "listChanged" : undefined}
            />
            <CapabilityItem
              icon={FileText}
              label="Resources"
              enabled={!!serverCapabilities?.resources}
              details={[
                serverCapabilities?.resources?.subscribe && "subscribe",
                serverCapabilities?.resources?.listChanged && "listChanged",
              ].filter(Boolean).join(", ") || undefined}
            />
            <CapabilityItem
              icon={MessageSquare}
              label="Prompts"
              enabled={!!serverCapabilities?.prompts}
              details={serverCapabilities?.prompts?.listChanged ? "listChanged" : undefined}
            />
            <CapabilityItem
              label="Logging"
              enabled={!!serverCapabilities?.logging}
            />
            <CapabilityItem
              label="Completions"
              enabled={!!serverCapabilities?.completions}
            />
            {serverCapabilities?.experimental && Object.keys(serverCapabilities.experimental).length > 0 && (
              <div className="py-2">
                <p className="text-sm mb-2">Experimental Features</p>
                <div className="flex flex-wrap gap-1">
                  {Object.keys(serverCapabilities.experimental).map((key) => (
                    <Badge key={key} variant="secondary" className="text-xs">
                      {key}
                    </Badge>
                  ))}
                </div>
              </div>
            )}
          </CardContent>
        </Card>

        {/* Client Capabilities */}
        <Card>
          <CardHeader className="pb-3">
            <CardTitle className="text-sm flex items-center gap-2">
              <Zap className="h-4 w-4" />
              Client Capabilities
              <span className="text-xs text-muted-foreground font-normal">
                (what the client declared)
              </span>
            </CardTitle>
          </CardHeader>
          <CardContent className="divide-y">
            <CapabilityItem
              icon={Radio}
              label="Sampling"
              enabled={!!clientCapabilities?.sampling}
              details="Can receive sampling/createMessage requests"
            />
            <CapabilityItem
              icon={HelpCircle}
              label="Elicitation (Form)"
              enabled={!!clientCapabilities?.elicitation?.form}
              details="Can receive form-based elicitation requests"
            />
            <CapabilityItem
              icon={FolderTree}
              label="Roots"
              enabled={!!clientCapabilities?.roots}
              details={clientCapabilities?.roots?.listChanged ? "listChanged" : undefined}
            />
          </CardContent>
        </Card>
      </div>
    </ScrollArea>
  )
}
