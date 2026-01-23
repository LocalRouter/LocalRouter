import { useState, useEffect, useCallback } from "react"
import { Search, Download, RefreshCw, ChevronRight, AlertCircle, FileText, Image, Code, Bell, BellOff } from "lucide-react"
import { Button } from "@/components/ui/Button"
import { Input } from "@/components/ui/Input"
import { ScrollArea } from "@/components/ui/scroll-area"
import { Badge } from "@/components/ui/Badge"
import { cn } from "@/lib/utils"
import type { McpClientWrapper, Resource, ResourceContent } from "@/lib/mcp-client"

interface ResourcesPanelProps {
  mcpClient: McpClientWrapper | null
  isConnected: boolean
}

export function ResourcesPanel({ mcpClient, isConnected }: ResourcesPanelProps) {
  const [resources, setResources] = useState<Resource[]>([])
  const [filteredResources, setFilteredResources] = useState<Resource[]>([])
  const [searchQuery, setSearchQuery] = useState("")
  const [selectedResource, setSelectedResource] = useState<Resource | null>(null)
  const [isLoading, setIsLoading] = useState(false)
  const [isReading, setIsReading] = useState(false)
  const [content, setContent] = useState<ResourceContent | null>(null)
  const [error, setError] = useState<string | null>(null)
  const [subscribedUris, setSubscribedUris] = useState<Set<string>>(new Set())

  // Fetch resources list using MCP client
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
    if (isConnected) {
      fetchResources()
    } else {
      setResources([])
      setFilteredResources([])
      setSelectedResource(null)
      setContent(null)
      setSubscribedUris(new Set())
    }
  }, [isConnected, fetchResources])

  // Filter resources based on search
  useEffect(() => {
    if (!searchQuery.trim()) {
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

  // Read resource content using MCP client
  const handleRead = async () => {
    if (!selectedResource || !mcpClient) return

    setIsReading(true)
    setContent(null)
    setError(null)

    try {
      const result = await mcpClient.readResource(selectedResource.uri)
      const contents = result.contents || []
      if (contents.length > 0) {
        const firstContent = contents[0]
        setContent({
          uri: firstContent.uri,
          mimeType: firstContent.mimeType,
          text: "text" in firstContent ? firstContent.text : undefined,
          blob: "blob" in firstContent ? firstContent.blob : undefined,
        })
      }
    } catch (err) {
      setError(err instanceof Error ? err.message : "Failed to read resource")
    } finally {
      setIsReading(false)
    }
  }

  // Subscribe/unsubscribe to resource updates
  const handleToggleSubscription = async () => {
    if (!selectedResource || !mcpClient) return

    const uri = selectedResource.uri
    const isSubscribed = subscribedUris.has(uri)

    try {
      if (isSubscribed) {
        await mcpClient.unsubscribeFromResource(uri)
        setSubscribedUris(prev => {
          const next = new Set(prev)
          next.delete(uri)
          return next
        })
      } else {
        await mcpClient.subscribeToResource(uri, (updatedUri, result) => {
          // Handle resource update notification
          if (selectedResource?.uri === updatedUri) {
            const contents = result.contents || []
            if (contents.length > 0) {
              const firstContent = contents[0]
              setContent({
                uri: firstContent.uri,
                mimeType: firstContent.mimeType,
                text: "text" in firstContent ? firstContent.text : undefined,
                blob: "blob" in firstContent ? firstContent.blob : undefined,
              })
            }
          }
        })
        setSubscribedUris(prev => new Set(prev).add(uri))
      }
    } catch (err) {
      setError(err instanceof Error ? err.message : "Failed to toggle subscription")
    }
  }

  const getResourceIcon = (mimeType?: string) => {
    if (!mimeType) return <FileText className="h-4 w-4" />
    if (mimeType.startsWith("image/")) return <Image className="h-4 w-4" />
    if (mimeType.includes("json") || mimeType.includes("javascript") || mimeType.includes("code"))
      return <Code className="h-4 w-4" />
    return <FileText className="h-4 w-4" />
  }

  const renderContent = () => {
    if (!content) return null

    if (content.blob) {
      const mimeType = content.mimeType || "application/octet-stream"
      if (mimeType.startsWith("image/")) {
        return (
          <img
            src={`data:${mimeType};base64,${content.blob}`}
            alt={selectedResource?.name}
            className="max-w-full h-auto rounded"
          />
        )
      }
      return (
        <div className="p-4 bg-muted rounded-md">
          <p className="text-sm text-muted-foreground">
            Binary content ({content.blob.length} bytes base64)
          </p>
          <Button
            variant="outline"
            size="sm"
            className="mt-2"
            onClick={() => {
              const link = document.createElement("a")
              link.href = `data:${mimeType};base64,${content.blob}`
              link.download = selectedResource?.name || "download"
              link.click()
            }}
          >
            <Download className="h-4 w-4 mr-2" />
            Download
          </Button>
        </div>
      )
    }

    if (content.text) {
      const isJson = content.mimeType?.includes("json") || content.text.trim().startsWith("{")

      // Safely format JSON with error handling
      let displayText = content.text
      if (isJson) {
        try {
          displayText = JSON.stringify(JSON.parse(content.text), null, 2)
        } catch {
          // If JSON parse fails, just show the raw text
          displayText = content.text
        }
      }

      return (
        <pre
          className={cn(
            "p-3 bg-muted rounded-md text-sm overflow-auto max-h-96",
            isJson && "whitespace-pre"
          )}
        >
          {displayText}
        </pre>
      )
    }

    return <p className="text-sm text-muted-foreground">No content available</p>
  }

  if (!isConnected) {
    return (
      <div className="flex items-center justify-center h-full text-muted-foreground">
        <p>Connect to an MCP server to view resources</p>
      </div>
    )
  }

  const isSubscribed = selectedResource ? subscribedUris.has(selectedResource.uri) : false

  return (
    <div className="flex h-full gap-4">
      {/* Left: Resources List */}
      <div className="w-80 flex flex-col border rounded-lg">
        <div className="p-3 border-b flex items-center gap-2">
          <Search className="h-4 w-4 text-muted-foreground" />
          <Input
            placeholder="Search resources..."
            value={searchQuery}
            onChange={(e) => setSearchQuery(e.target.value)}
            className="h-8 border-0 p-0 focus-visible:ring-0"
          />
          <Button
            variant="ghost"
            size="icon"
            className="h-8 w-8"
            onClick={fetchResources}
            disabled={isLoading}
          >
            <RefreshCw className={cn("h-4 w-4", isLoading && "animate-spin")} />
          </Button>
        </div>

        <ScrollArea className="flex-1">
          {error ? (
            <div className="p-4 text-sm text-destructive flex items-center gap-2">
              <AlertCircle className="h-4 w-4" />
              {error}
            </div>
          ) : filteredResources.length === 0 ? (
            <div className="p-4 text-sm text-muted-foreground text-center">
              {isLoading ? "Loading resources..." : "No resources available"}
            </div>
          ) : (
            <div className="p-2 space-y-1">
              {filteredResources.map((resource) => (
                <button
                  key={resource.uri}
                  onClick={() => {
                    setSelectedResource(resource)
                    setContent(null)
                  }}
                  className={cn(
                    "w-full text-left p-2 rounded-md transition-colors",
                    "hover:bg-accent",
                    selectedResource?.uri === resource.uri && "bg-accent"
                  )}
                >
                  <div className="flex items-center gap-2">
                    {getResourceIcon(resource.mimeType)}
                    <span className="text-sm truncate flex-1">{resource.name}</span>
                    {subscribedUris.has(resource.uri) && (
                      <Bell className="h-3 w-3 text-primary" />
                    )}
                    <ChevronRight className="h-3 w-3 text-muted-foreground" />
                  </div>
                  <p className="text-xs text-muted-foreground truncate mt-1 font-mono">
                    {resource.uri}
                  </p>
                </button>
              ))}
            </div>
          )}
        </ScrollArea>

        <div className="p-2 border-t text-xs text-muted-foreground text-center">
          {filteredResources.length} resource{filteredResources.length !== 1 ? "s" : ""}
          {subscribedUris.size > 0 && ` (${subscribedUris.size} subscribed)`}
        </div>
      </div>

      {/* Right: Resource Details & Content */}
      <div className="flex-1 flex flex-col border rounded-lg">
        {selectedResource ? (
          <>
            <div className="p-4 border-b">
              <div className="flex items-center justify-between">
                <div className="flex items-center gap-2">
                  {getResourceIcon(selectedResource.mimeType)}
                  <h3 className="font-semibold">{selectedResource.name}</h3>
                </div>
                <div className="flex items-center gap-2">
                  <Button
                    variant="outline"
                    size="sm"
                    onClick={handleToggleSubscription}
                    title={isSubscribed ? "Unsubscribe from updates" : "Subscribe to updates"}
                  >
                    {isSubscribed ? (
                      <>
                        <BellOff className="h-4 w-4 mr-2" />
                        Unsubscribe
                      </>
                    ) : (
                      <>
                        <Bell className="h-4 w-4 mr-2" />
                        Subscribe
                      </>
                    )}
                  </Button>
                  <Button onClick={handleRead} disabled={isReading}>
                    {isReading ? (
                      <RefreshCw className="h-4 w-4 mr-2 animate-spin" />
                    ) : (
                      <Download className="h-4 w-4 mr-2" />
                    )}
                    Read
                  </Button>
                </div>
              </div>
              <p className="text-xs text-muted-foreground font-mono mt-1">
                {selectedResource.uri}
              </p>
              {selectedResource.mimeType && (
                <Badge variant="secondary" className="mt-2">
                  {selectedResource.mimeType}
                </Badge>
              )}
              {selectedResource.description && (
                <p className="text-sm text-muted-foreground mt-2">
                  {selectedResource.description}
                </p>
              )}
            </div>

            <ScrollArea className="flex-1 p-4">
              {content ? (
                renderContent()
              ) : (
                <div className="flex items-center justify-center h-full text-muted-foreground">
                  <p>Click "Read" to fetch resource content</p>
                </div>
              )}
            </ScrollArea>
          </>
        ) : (
          <div className="flex items-center justify-center h-full text-muted-foreground">
            <p>Select a resource to view details</p>
          </div>
        )}
      </div>
    </div>
  )
}
