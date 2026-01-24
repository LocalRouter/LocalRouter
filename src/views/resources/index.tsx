import { Tabs, TabsContent, TabsList, TabsTrigger } from "@/components/ui/tabs"
import { ProvidersPanel } from "./providers-panel"
import { StrategiesPanel } from "./strategies-panel"

interface LlmProvidersViewProps {
  activeSubTab: string | null
  onTabChange: (view: string, subTab?: string | null) => void
}

export function ResourcesView({ activeSubTab, onTabChange }: LlmProvidersViewProps) {

  // Parse subTab to determine which resource type and item is selected
  // Format: "providers", "strategies"
  // Or: "providers/instance-name", "strategies/strategy-id"
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
        <h1 className="text-2xl font-bold tracking-tight">LLM Providers</h1>
        <p className="text-sm text-muted-foreground">
          Manage providers and routing strategies
        </p>
      </div>

      <Tabs
        value={resourceType}
        onValueChange={handleResourceChange}
        className="flex flex-col flex-1 min-h-0"
      >
        <TabsList className="flex-shrink-0 w-fit">
          <TabsTrigger value="providers">Providers</TabsTrigger>
          <TabsTrigger value="strategies">Model Routing</TabsTrigger>
        </TabsList>

        <TabsContent value="providers" className="flex-1 min-h-0 mt-4">
          <ProvidersPanel
            selectedId={resourceType === "providers" ? itemId : null}
            onSelect={(id) => handleItemSelect("providers", id)}
          />
        </TabsContent>

        <TabsContent value="strategies" className="flex-1 min-h-0 mt-4">
          <StrategiesPanel
            selectedId={resourceType === "strategies" ? itemId : null}
            onSelect={(id) => handleItemSelect("strategies", id)}
            onNavigateToClient={(clientId) => onTabChange("clients", clientId)}
          />
        </TabsContent>
      </Tabs>
    </div>
  )
}
