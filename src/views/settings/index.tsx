import { Settings as SettingsIcon } from "lucide-react"
import { Tabs, TabsContent, TabsList, TabsTrigger } from "@/components/ui/tabs"
import { ServerTab } from "./server-tab"
// DEPRECATED: Strategy UI hidden - 1:1 client-to-strategy relationship
// import { RoutingTab } from "./routing-tab"
import { RouteLLMTab } from "./routellm-tab"
import { LoggingTab } from "./logging-tab"
import { UpdatesTab } from "./updates-tab"

interface SettingsViewProps {
  activeSubTab: string | null
  onTabChange: (view: string, subTab?: string | null) => void
}

export function SettingsView({ activeSubTab, onTabChange }: SettingsViewProps) {
  // Parse subTab to get settings section and optional detail id
  // Format: "server", "routing", "routing/strategy-id", "routellm", "updates"
  const parseSubTab = (subTab: string | null) => {
    if (!subTab) return { section: "server", detailId: null }
    const parts = subTab.split("/")
    const section = parts[0] || "server"
    const detailId = parts.slice(1).join("/") || null
    return { section, detailId }
  }

  const { section, detailId: _detailId } = parseSubTab(activeSubTab) // DEPRECATED: detailId unused - Strategy UI hidden

  const handleSectionChange = (newSection: string) => {
    onTabChange("settings", newSection)
  }

  // DEPRECATED: Strategy UI hidden - 1:1 client-to-strategy relationship
  // const handleDetailChange = (sectionName: string, id: string | null) => {
  //   onTabChange("settings", id ? `${sectionName}/${id}` : sectionName)
  // }

  return (
    <div className="space-y-4">
      <div>
        <h1 className="text-2xl font-bold tracking-tight flex items-center gap-2"><SettingsIcon className="h-6 w-6" />Settings</h1>
        <p className="text-sm text-muted-foreground">
          Configure server and application preferences
        </p>
      </div>

      <Tabs
        value={section}
        onValueChange={handleSectionChange}
        className="space-y-4"
      >
        <TabsList>
          <TabsTrigger value="server">Server</TabsTrigger>
          {/* DEPRECATED: Strategy UI hidden - 1:1 client-to-strategy relationship */}
          {/* <TabsTrigger value="routing">Strategies</TabsTrigger> */}
          <TabsTrigger value="routellm">Strong/Weak</TabsTrigger>
          <TabsTrigger value="logs">Logs</TabsTrigger>
          <TabsTrigger value="updates">Updates</TabsTrigger>
        </TabsList>

        <TabsContent value="server">
          <ServerTab />
        </TabsContent>

        {/* DEPRECATED: Strategy UI hidden - 1:1 client-to-strategy relationship */}
        {/* <TabsContent value="routing">
          <RoutingTab
            selectedStrategyId={section === "routing" ? detailId : null}
            onSelectStrategy={(id) => handleDetailChange("routing", id)}
          />
        </TabsContent> */}

        <TabsContent value="routellm">
          <RouteLLMTab />
        </TabsContent>

        <TabsContent value="logs">
          <LoggingTab />
        </TabsContent>

        <TabsContent value="updates">
          <UpdatesTab />
        </TabsContent>
      </Tabs>
    </div>
  )
}
