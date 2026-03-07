import { Settings as SettingsIcon } from "lucide-react"
import { Tabs, TabsContent, TabsList, TabsTrigger } from "@/components/ui/tabs"
import { ServerTab } from "./server-tab"
import { LoggingTab } from "./logging-tab"
import { UpdatesTab } from "./updates-tab"
import { AppearanceTab } from "./appearance-tab"
import { LicensesTab } from "./licenses-tab"

interface SettingsViewProps {
  activeSubTab: string | null
  onTabChange: (view: string, subTab?: string | null) => void
}

export function SettingsView({ activeSubTab, onTabChange }: SettingsViewProps) {
  const section = activeSubTab || "server"

  const handleSectionChange = (newSection: string) => {
    onTabChange("settings", newSection)
  }

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
          <TabsTrigger value="appearance">Appearance</TabsTrigger>
          <TabsTrigger value="logs">Logs</TabsTrigger>
          <TabsTrigger value="updates">Updates</TabsTrigger>
          <TabsTrigger value="licenses">Licenses</TabsTrigger>
        </TabsList>

        <TabsContent value="server">
          <ServerTab />
        </TabsContent>

        <TabsContent value="appearance">
          <AppearanceTab />
        </TabsContent>

        <TabsContent value="logs">
          <LoggingTab />
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
