import { Tabs, TabsContent, TabsList, TabsTrigger } from "@/components/ui/tabs"
import { LlmTab } from "./llm-tab"
import { McpTab } from "./mcp-tab"

interface TryItOutViewProps {
  activeSubTab: string | null
  onTabChange: (view: string, subTab?: string | null) => void
}

export function TryItOutView({ activeSubTab, onTabChange }: TryItOutViewProps) {
  // Parse subTab to determine which main tab is selected
  // Format: "llm" or "mcp" or "llm/..." or "mcp/..."
  const parseSubTab = (subTab: string | null) => {
    if (!subTab) return { mainTab: "llm", innerPath: null }
    const parts = subTab.split("/")
    const mainTab = parts[0] || "llm"
    const innerPath = parts.slice(1).join("/") || null
    return { mainTab, innerPath }
  }

  const { mainTab, innerPath } = parseSubTab(activeSubTab)

  const handleTabChange = (tab: string) => {
    onTabChange("try-it-out", tab)
  }

  const handleInnerPathChange = (tab: string, path: string | null) => {
    onTabChange("try-it-out", path ? `${tab}/${path}` : tab)
  }

  return (
    <div className="flex flex-col h-full min-h-0">
      <div className="flex-shrink-0 pb-4">
        <h1 className="text-2xl font-bold tracking-tight">Try It Out</h1>
        <p className="text-sm text-muted-foreground">
          Test LLM completions and MCP server capabilities
        </p>
      </div>

      <Tabs
        value={mainTab}
        onValueChange={handleTabChange}
        className="flex flex-col flex-1 min-h-0"
      >
        <TabsList className="flex-shrink-0 w-fit">
          <TabsTrigger value="llm">LLM</TabsTrigger>
          <TabsTrigger value="mcp">MCP</TabsTrigger>
        </TabsList>

        <TabsContent value="llm" className="flex-1 min-h-0 mt-4">
          <LlmTab />
        </TabsContent>

        <TabsContent value="mcp" className="flex-1 min-h-0 mt-4">
          <McpTab
            innerPath={mainTab === "mcp" ? innerPath : null}
            onPathChange={(path) => handleInnerPathChange("mcp", path)}
          />
        </TabsContent>
      </Tabs>
    </div>
  )
}
