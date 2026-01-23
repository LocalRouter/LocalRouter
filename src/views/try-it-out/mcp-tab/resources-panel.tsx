import { useState, useEffect, useCallback } from "react"
import { Search, RefreshCw, ChevronRight, FileText, Eye, Bell, BellOff } from "lucide-react"
import { Button } from "@/components/ui/Button"
import { Input } from "@/components/ui/Input"
import { ScrollArea } from "@/components/ui/scroll-area"
import { Badge } from "@/components/ui/Badge"
import { cn } from "@/lib/utils"
import type { McpClientWrapper, Resource, ReadResourceResult } from "@/lib/mcp-client"

interface ResourcesPanelProps {
  mcpClient: McpClientWrapper | null
  isConnected: boolean
}

export function ResourcesPanel({
  mcpClient,
  isConnected,
}: ResourcesPanelProps) {
  const [resources, setResources] = useState<Resource[]>([])
  const [filteredResources, setFilteredResources] = useState<Resource[]>([])
  const [searchQuery, setSearchQuery] = useState("")
  const [selectedResource, setSelectedResource] = useState<Resource | null>(null)
  const [isLoading, setIsLoading] = useState(false)
  const [isReading, setIsReading] = useState(false)
  const [content, setContent] = useState<ReadResourceResult | null>(null)
  const [error, setError] = useState<string | null>(null)
  const [subscribedUris, setSubscribedUris] = useState<Set<string>>(new Set())

  // Fetch resources list using MCP SDK
  const fetchResources = useCallback(async () => {
    if (!mcpClient || !isConnected) return

    setIsLoading(true)
    setError(null)

    try {
      const resourcesList = await mcpClient.listResources()
      setResources(resourcesList)
      setFilteredResources(resourcesList)
    } catch (err) {
      setError(err instanceof Error ? err.message : "Failed to fetch resources")
    } finally {
      setIsLoading(false)
    }
  }, [mcpClient, isConnected])

  useEffect(() => {
    if (isConnected && mcpClient) {
      fetchResources()
    } else {
      setResources([])
      setFilteredResources([])
      setSelectedResource(null)
      setContent(null)
      setSubscribedUris(new Set())
    }
  }, [isConnected, mcpClient, fetchResources])

  // Filter resources by search query
  useEffect(() => {
    if (!searchQuery) {
      setFilteredResources(resources)
    } else {
      const query = searchQuery.toLowerCase()
      setFilteredResources(
        resources.filter(
          (r) =>
            r.name.toLowerCase().includes(query) ||
            r.uri.toLowerCase().includes(query) ||
            r.description?.toLowerCase().includes(query)
        )
      )
    }
  }, [searchQuery, resources])

  // Read resource content
  const readResource = async (resource: Resource) => {
    if (!mcpClient) return

    setIsReading(true)
    setContent(null)
    setError(null)

    try {
      const result = await mcpClient.readResource(resource.uri)
      setContent(result)
    } catch (err) {
      setError(err instanceof Error ? err.message : "Failed to read resource")
    } finally {
      setIsReading(false)
    }
  }

  // Toggle subscription to resource updates
  const toggleSubscription = async (resource: Resource) => {
    if (!mcpClient) return

    const uri = resource.uri
    const isSubscribed = subscribedUris.has(uri)

    try {
      if (isSubscribed) {
        await mcpClient.unsubscribeFromResource(uri)
        setSubscribedUris((prev) => {
          const newSet = new Set(prev)
          newSet.delete(uri)
          return newSet
        })
      } else {
        await mcpClient.subscribeToResource(uri, (updatedUri, updatedContent) => {
          // Update content if this resource is currently selected
          if (selectedResource?.uri === updatedUri) {
            setContent(updatedContent)
          }
        })
        setSubscribedUris((prev) => new Set(prev).add(uri))
      }
    } catch (err) {
      setError(err instanceof Error ? err.message : "Failed to toggle subscription")
    }
  }

  const handleResourceSelect = (resource: Resource) => {
    setSelectedResource(resource)
    setContent(null)
    setError(null)
    readResource(resource)
  }

  const renderContent = () => {
    if (!content) return null

    return content.contents.map((c, idx) => (
      <div key={idx} className="space-y-2">
        <div className="flex items-center gap-2 text-xs text-muted-foreground">
          <span>{c.uri}</span>
          {c.mimeType && <Badge variant="outline">{c.mimeType}</Badge>}
        </div>
        {c.text && (
          <pre className="p-3 bg-muted rounded-md text-xs overflow-auto max-h-96 whitespace-pre-wrap">
            {c.text}
          </pre>
        )}
        {c.blob && (
          <div className="p-3 bg-muted rounded-md text-xs">
            <p className="text-muted-foreground">Binary content ({c.blob.length} bytes base64)</p>
          </div>
        )}
      </div>
    ))
  }

  if (!isConnected) {
    return (
      <div className="flex items-center justify-center h-full text-muted-foreground">
        <p>Connect to an MCP server to browse resources</p>
      </div>
    )
  }

  return (
    <div className="flex h-full gap-4">
      {/* Left: Resources list */}
      <div className="w-72 flex flex-col border rounded-lg">
        <div className="p-3 border-b">
          <div className="flex items-center gap-2 mb-2">
            <span className="font-medium text-sm">Resources</span>
            <Badge variant="secondary">{resources.length}</Badge>
            <Button
              variant="ghost"
              size="icon"
              className="h-6 w-6 ml-auto"
              onClick={fetchResources}
              disabled={isLoading}
            >
              <RefreshCw className={cn("h-3 w-3", isLoading && "animate-spin")} />
            </Button>
          </div>
          <div className="relative">
            <Search className="absolute left-2 top-2.5 h-4 w-4 text-muted-foreground" />
            <Input
              placeholder="Search resources..."
              className="pl-8 h-9"
              value={searchQuery}
              onChange={(e) => setSearchQuery(e.target.value)}
            />
          </div>
        </div>

        <ScrollArea className="flex-1">
          {error && !selectedResource && (
            <div className="p-4 text-sm text-destructive">{error}</div>
          )}
          <div className="p-2">
            {filteredResources.map((resource) => (
              <button
                key={resource.uri}
                onClick={() => handleResourceSelect(resource)}
                className={cn(
                  "w-full text-left px-3 py-2 rounded-md text-sm transition-colors",
                  "hover:bg-accent",
                  selectedResource?.uri === resource.uri && "bg-accent"
                )}
              >
                <div className="flex items-center gap-2">
                  <ChevronRight className="h-3 w-3 text-muted-foreground" />
                  <FileText className="h-3 w-3 text-muted-foreground" />
                  <span className="font-medium truncate">{resource.name}</span>
                  {subscribedUris.has(resource.uri) && (
                    <Bell className="h-3 w-3 text-primary ml-auto" />
                  )}
                </div>
                <p className="text-xs text-muted-foreground truncate ml-8 mt-0.5">
                  {resource.uri}
                </p>
                {resource.description && (
                  <p className="text-xs text-muted-foreground truncate ml-8">
                    {resource.description}
                  </p>
                )}
              </button>
            ))}
          </div>
        </ScrollArea>
      </div>

      {/* Right: Resource content */}
      <div className="flex-1 flex flex-col border rounded-lg">
        {selectedResource ? (
          <>
            <div className="p-4 border-b">
              <div className="flex items-center justify-between">
                <div>
                  <h3 className="font-semibold">{selectedResource.name}</h3>
                  <p className="text-xs text-muted-foreground">{selectedResource.uri}</p>
                </div>
                <div className="flex items-center gap-2">
                  <Button
                    variant="outline"
                    size="sm"
                    onClick={() => toggleSubscription(selectedResource)}
                    title={subscribedUris.has(selectedResource.uri) ? "Unsubscribe" : "Subscribe to updates"}
                  >
                    {subscribedUris.has(selectedResource.uri) ? (
                      <>
                        <BellOff className="h-4 w-4 mr-1" />
                        Unsubscribe
                      </>
                    ) : (
                      <>
                        <Bell className="h-4 w-4 mr-1" />
                        Subscribe
                      </>
                    )}
                  </Button>
                  <Button
                    variant="outline"
                    size="sm"
                    onClick={() => readResource(selectedResource)}
                    disabled={isReading}
                  >
                    {isReading ? (
                      <RefreshCw className="h-4 w-4 mr-1 animate-spin" />
                    ) : (
                      <Eye className="h-4 w-4 mr-1" />
                    )}
                    Read
                  </Button>
                </div>
              </div>
              {selectedResource.description && (
                <p className="text-sm text-muted-foreground mt-2">
                  {selectedResource.description}
                </p>
              )}
            </div>

            <ScrollArea className="flex-1 p-4">
              {error && (
                <div className="p-3 bg-destructive/10 text-destructive rounded-md text-sm mb-4">
                  {error}
                </div>
              )}
              {isReading && (
                <div className="flex items-center justify-center h-32 text-muted-foreground">
                  <RefreshCw className="h-4 w-4 mr-2 animate-spin" />
                  Reading resource...
                </div>
              )}
              {!isReading && content && (
                <div className="space-y-4">{renderContent()}</div>
              )}
              {!isReading && !content && !error && (
                <div className="flex items-center justify-center h-32 text-muted-foreground">
                  Click "Read" to load resource content
                </div>
              )}
            </ScrollArea>
          </>
        ) : (
          <div className="flex items-center justify-center h-full text-muted-foreground">
            <p>Select a resource to view its content</p>
          </div>
        )}
      </div>
    </div>
  )
}
