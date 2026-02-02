import { lazy, Suspense } from "react"
import { Settings as SettingsIcon } from "lucide-react"
import { Tabs, TabsContent, TabsList, TabsTrigger } from "@/components/ui/tabs"
import { ServerTab } from "./server-tab"
import { RoutingTab } from "./routing-tab"
import { RouteLLMTab } from "./routellm-tab"
import { LoggingTab } from "./logging-tab"
import { UpdatesTab } from "./updates-tab"

// Lazy load DocsTab only in dev mode to exclude rapidoc (864KB) from production
const DocsTab = import.meta.env.DEV
  ? lazy(() => import("./docs-tab").then(m => ({ default: m.DocsTab })))
  : () => null

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
        <h1 className="text-2xl font-bold tracking-tight flex items-center gap-2"><SettingsIcon className="h-6 w-6" />Settings</h1>
        <p className="text-sm text-muted-foreground">
          Configure server, strategies, and application preferences
        </p>
      </div>

      <Tabs
        value={section}
        onValueChange={handleSectionChange}
        className="space-y-4"
      >
        <TabsList>
          <TabsTrigger value="server">Server</TabsTrigger>
          <TabsTrigger value="routing">Strategies</TabsTrigger>
          <TabsTrigger value="routellm">Strong/Weak</TabsTrigger>
          <TabsTrigger value="logs">Logs</TabsTrigger>
          <TabsTrigger value="updates">Updates</TabsTrigger>
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

        <TabsContent value="logs">
          <LoggingTab />
        </TabsContent>

        <TabsContent value="updates">
          <UpdatesTab />
        </TabsContent>

        {isDev && (
          <TabsContent value="docs">
            <Suspense fallback={
              <div className="flex items-center justify-center h-96">
                <div className="text-center space-y-4">
                  <div className="text-6xl animate-spin">⚙️</div>
                  <p className="text-muted-foreground">Loading API documentation...</p>
                </div>
              </div>
            }>
              <DocsTab />
            </Suspense>
          </TabsContent>
        )}
      </Tabs>
    </div>
  )
}
