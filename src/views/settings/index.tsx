import { Tabs, TabsContent, TabsList, TabsTrigger } from "@/components/ui/tabs"
import { ServerTab } from "./server-tab"
import { RoutingTab } from "./routing-tab"
import { RouteLLMTab } from "./routellm-tab"
import { LoggingTab } from "./logging-tab"
import { UpdatesTab } from "./updates-tab"
import { AboutTab } from "./about-tab"
import { DocsTab } from "./docs-tab"

interface SettingsViewProps {
  activeSubTab: string | null
  onTabChange: (view: string, subTab?: string | null) => void
}

export function SettingsView({ activeSubTab, onTabChange }: SettingsViewProps) {
  // Parse subTab to get settings section and optional detail id
  // Format: "server", "routing", "routing/strategy-id", "routellm", "updates", "docs"
  const parseSubTab = (subTab: string | null) => {
    if (!subTab) return { section: "server", detailId: null }
    const parts = subTab.split("/")
    const section = parts[0] || "server"
    const detailId = parts.slice(1).join("/") || null
    return { section, detailId }
  }

  const { section, detailId } = parseSubTab(activeSubTab)

  const handleSectionChange = (newSection: string) => {
    onTabChange("settings", newSection)
  }

  const handleDetailChange = (sectionName: string, id: string | null) => {
    onTabChange("settings", id ? `${sectionName}/${id}` : sectionName)
  }

  // Check if we're in development mode for showing docs tab
  const isDev = import.meta.env.DEV

  return (
    <div className="space-y-4">
      <div>
        <h1 className="text-2xl font-bold tracking-tight">Settings</h1>
        <p className="text-sm text-muted-foreground">
          Configure server, routing, and application preferences
        </p>
      </div>

      <Tabs
        value={section}
        onValueChange={handleSectionChange}
        className="space-y-4"
      >
        <TabsList>
          <TabsTrigger value="server">Server</TabsTrigger>
          <TabsTrigger value="routing">Routing</TabsTrigger>
          <TabsTrigger value="routellm">RouteLLM</TabsTrigger>
          <TabsTrigger value="logging">Logging</TabsTrigger>
          <TabsTrigger value="updates">Updates</TabsTrigger>
          <TabsTrigger value="about">About</TabsTrigger>
          {isDev && <TabsTrigger value="docs">Docs</TabsTrigger>}
        </TabsList>

        <TabsContent value="server">
          <ServerTab />
        </TabsContent>

        <TabsContent value="routing">
          <RoutingTab
            selectedStrategyId={section === "routing" ? detailId : null}
            onSelectStrategy={(id) => handleDetailChange("routing", id)}
          />
        </TabsContent>

        <TabsContent value="routellm">
          <RouteLLMTab />
        </TabsContent>

        <TabsContent value="logging">
          <LoggingTab />
        </TabsContent>

        <TabsContent value="updates">
          <UpdatesTab />
        </TabsContent>

        <TabsContent value="about">
          <AboutTab />
        </TabsContent>

        {isDev && (
          <TabsContent value="docs">
            <DocsTab />
          </TabsContent>
        )}
      </Tabs>
    </div>
  )
}
