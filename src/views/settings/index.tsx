import { Settings as SettingsIcon } from "lucide-react"
import { TAB_ICONS, TAB_ICON_CLASS } from "@/constants/tab-icons"
import { Tabs, TabsContent, TabsList, TabsTrigger } from "@/components/ui/tabs"
import { LoggingTab } from "./logging-tab"
import { UpdatesTab } from "./updates-tab"
import { AppearanceTab } from "./appearance-tab"
import { HealthChecksTab } from "./health-checks-tab"
import { LicensesTab } from "./licenses-tab"
import { MemoryTab } from "./memory-tab"

interface SettingsViewProps {
  activeSubTab: string | null
  onTabChange: (view: string, subTab?: string | null) => void
}

export function SettingsView({ activeSubTab, onTabChange }: SettingsViewProps) {
  const section = activeSubTab || "appearance"

  const handleSectionChange = (newSection: string) => {
    onTabChange("settings", newSection)
  }

  return (
    <div className="space-y-4 max-w-5xl">
      <div>
        <h1 className="text-2xl font-bold tracking-tight flex items-center gap-2"><SettingsIcon className="h-6 w-6" />Settings</h1>
        <p className="text-sm text-muted-foreground">
          Configure application preferences
        </p>
      </div>

      <Tabs
        value={section}
        onValueChange={handleSectionChange}
        className="space-y-4"
      >
        <TabsList>
          <TabsTrigger value="appearance"><TAB_ICONS.appearance className={TAB_ICON_CLASS} />Appearance</TabsTrigger>
          <TabsTrigger value="health-checks"><TAB_ICONS.healthChecks className={TAB_ICON_CLASS} />Health Checks</TabsTrigger>
          <TabsTrigger value="logs"><TAB_ICONS.logs className={TAB_ICON_CLASS} />Logs</TabsTrigger>
          <TabsTrigger value="memory"><TAB_ICONS.memory className={TAB_ICON_CLASS} />Memory</TabsTrigger>
          <TabsTrigger value="updates"><TAB_ICONS.updates className={TAB_ICON_CLASS} />Updates</TabsTrigger>
          <TabsTrigger value="licenses"><TAB_ICONS.licenses className={TAB_ICON_CLASS} />Licenses</TabsTrigger>
        </TabsList>

        <TabsContent value="appearance">
          <AppearanceTab />
        </TabsContent>

        <TabsContent value="health-checks">
          <HealthChecksTab />
        </TabsContent>

        <TabsContent value="logs">
          <LoggingTab />
        </TabsContent>

        <TabsContent value="memory">
          <MemoryTab />
        </TabsContent>

        <TabsContent value="updates">
          <UpdatesTab />
        </TabsContent>

        <TabsContent value="licenses">
          <LicensesTab />
        </TabsContent>
      </Tabs>
    </div>
  )
}
