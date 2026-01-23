import { Tabs, TabsContent, TabsList, TabsTrigger } from "@/components/ui/tabs"
import { ProvidersPanel } from "./providers-panel"
import { ModelsPanel } from "./models-panel"
import { StrategiesPanel } from "./strategies-panel"
import { McpServersPanel } from "./mcp-servers-panel"
import { useMetricsSubscription } from "@/hooks/useMetricsSubscription"

interface ResourcesViewProps {
  activeSubTab: string | null
  onTabChange: (view: string, subTab?: string | null) => void
}

export function ResourcesView({ activeSubTab, onTabChange }: ResourcesViewProps) {
  const refreshKey = useMetricsSubscription()

  // Parse subTab to determine which resource type and item is selected
  // Format: "providers", "models", "mcp-servers"
  // Or: "providers/instance-name", "mcp-servers/server-id"
  const parseSubTab = (subTab: string | null) => {
    if (!subTab) return { resourceType: "providers", itemId: null }
    const parts = subTab.split("/")
    const resourceType = parts[0] || "providers"
    const itemId = parts.slice(1).join("/") || null
    return { resourceType, itemId }
  }

  const { resourceType, itemId } = parseSubTab(activeSubTab)

  const handleResourceChange = (type: string) => {
    onTabChange("resources", type)
  }

  const handleItemSelect = (type: string, id: string | null) => {
    onTabChange("resources", id ? `${type}/${id}` : type)
  }

  return (
    <div className="flex flex-col h-full min-h-0">
      <div className="flex-shrink-0 pb-4">
        <h1 className="text-2xl font-bold tracking-tight">Resources</h1>
        <p className="text-sm text-muted-foreground">
          Manage providers, models, and MCP servers
        </p>
      </div>

      <Tabs
        value={resourceType}
        onValueChange={handleResourceChange}
        className="flex flex-col flex-1 min-h-0"
      >
        <TabsList className="flex-shrink-0 w-fit">
          <TabsTrigger value="providers">Providers</TabsTrigger>
          <TabsTrigger value="models">Models</TabsTrigger>
          <TabsTrigger value="strategies">Model Routing</TabsTrigger>
          <TabsTrigger value="mcp-servers">MCP Servers</TabsTrigger>
        </TabsList>

        <TabsContent value="providers" className="flex-1 min-h-0 mt-4">
          <ProvidersPanel
            selectedId={resourceType === "providers" ? itemId : null}
            onSelect={(id) => handleItemSelect("providers", id)}
            refreshTrigger={refreshKey}
          />
        </TabsContent>

        <TabsContent value="models" className="flex-1 min-h-0 mt-4">
          <ModelsPanel
            selectedId={resourceType === "models" ? itemId : null}
            onSelect={(id) => handleItemSelect("models", id)}
            refreshTrigger={refreshKey}
          />
        </TabsContent>

        <TabsContent value="strategies" className="flex-1 min-h-0 mt-4">
          <StrategiesPanel
            selectedId={resourceType === "strategies" ? itemId : null}
            onSelect={(id) => handleItemSelect("strategies", id)}
            refreshTrigger={refreshKey}
          />
        </TabsContent>

        <TabsContent value="mcp-servers" className="flex-1 min-h-0 mt-4">
          <McpServersPanel
            selectedId={resourceType === "mcp-servers" ? itemId : null}
            onSelect={(id) => handleItemSelect("mcp-servers", id)}
            refreshTrigger={refreshKey}
          />
        </TabsContent>
      </Tabs>
    </div>
  )
}
