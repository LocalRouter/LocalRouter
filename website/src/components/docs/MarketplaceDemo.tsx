/**
 * Demo wrapper showing marketplace search results using the real app's UI components.
 * Matches the card layout from src/views/marketplace/index.tsx.
 */
import { Search, Download, ExternalLink, Package, Globe } from "lucide-react"
import { Card, CardHeader, CardTitle, CardContent } from "@app/components/ui/Card"
import { Badge } from "@app/components/ui/Badge"
import { Button } from "@app/components/ui/Button"
import { Input } from "@app/components/ui/Input"

const DEMO_SERVERS = [
  {
    name: "Filesystem",
    vendor: "Anthropic",
    description: "Read, write, and manage local files with secure sandboxing",
    available_transports: ["stdio"],
    remotes: [] as { transport_type: string; url: string }[],
    homepage: "https://github.com/modelcontextprotocol/servers/tree/main/src/filesystem",
  },
  {
    name: "GitHub",
    vendor: "GitHub",
    description: "Official GitHub MCP server for repository management, issues, PRs, and Actions",
    available_transports: ["stdio", "sse"],
    remotes: [{ transport_type: "sse", url: "https://mcp.github.com/sse" }],
    homepage: "https://github.com/modelcontextprotocol/servers/tree/main/src/github",
  },
  {
    name: "PostgreSQL",
    vendor: "Community",
    description: "Connect to PostgreSQL databases for schema introspection and read-only queries",
    available_transports: ["stdio"],
    remotes: [] as { transport_type: string; url: string }[],
    homepage: null,
  },
]

export function MarketplaceDemo() {
  const noop = () => {}

  return (
    <div className="dark max-w-lg">
      <div className="flex flex-col rounded-lg border">
        {/* Search bar - matches src/views/marketplace/index.tsx lines 607-628 */}
        <div className="flex-shrink-0 p-4 border-b">
          <div className="flex gap-2">
            <div className="relative flex-1">
              <Search className="absolute left-3 top-1/2 -translate-y-1/2 h-4 w-4 text-muted-foreground" />
              <Input
                placeholder="Search MCP servers..."
                defaultValue="filesystem"
                className="pl-9"
                readOnly
              />
            </div>
            <Button onClick={noop}>Search</Button>
          </div>
        </div>

        {/* Results - matches src/views/marketplace/index.tsx lines 642-692 */}
        <div className="p-4 space-y-4">
          <div className="grid gap-4">
            {DEMO_SERVERS.map((server) => (
              <Card key={server.name}>
                <CardHeader className="pb-2">
                  <div className="flex items-start justify-between">
                    <div>
                      <CardTitle className="text-base">{server.name}</CardTitle>
                      {server.vendor && (
                        <p className="text-xs text-muted-foreground">by {server.vendor}</p>
                      )}
                    </div>
                    <div className="flex gap-1">
                      {server.available_transports.includes("stdio") && (
                        <Badge variant="secondary" className="text-xs">
                          <Package className="h-3 w-3 mr-1" />
                          stdio
                        </Badge>
                      )}
                      {server.remotes.length > 0 && (
                        <Badge variant="secondary" className="text-xs">
                          <Globe className="h-3 w-3 mr-1" />
                          remote
                        </Badge>
                      )}
                    </div>
                  </div>
                </CardHeader>
                <CardContent>
                  <p className="text-sm text-muted-foreground mb-3">{server.description}</p>
                  <div className="flex items-center justify-between">
                    <div className="flex gap-2">
                      {server.homepage && (
                        <Button variant="ghost" size="sm" onClick={noop}>
                          <ExternalLink className="h-4 w-4 mr-1" />
                          Homepage
                        </Button>
                      )}
                    </div>
                    <Button size="sm" onClick={noop}>
                      <Download className="h-4 w-4 mr-1" />
                      Install
                    </Button>
                  </div>
                </CardContent>
              </Card>
            ))}
          </div>
        </div>
      </div>
    </div>
  )
}
