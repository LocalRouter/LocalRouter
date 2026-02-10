/**
 * StepTemplate - Template selection step for the client creation wizard.
 *
 * Two-tab layout matching the provider creation pattern:
 * - Templates: Grid of known app templates (Claude Code, Cursor, etc.)
 * - Custom: Manual setup for any OpenAI-compatible app
 */

import { useState } from "react"
import { Grid, Settings } from "lucide-react"
import { Tabs, TabsContent, TabsList, TabsTrigger } from "@/components/ui/tabs"
import { ClientTemplates, CUSTOM_CLIENT_TEMPLATE } from "@/components/client/ClientTemplates"
import type { ClientTemplate } from "@/components/client/ClientTemplates"

interface StepTemplateProps {
  onSelect: (template: ClientTemplate) => void
}

export function StepTemplate({ onSelect }: StepTemplateProps) {
  const [tab, setTab] = useState<"templates" | "custom">("templates")

  return (
    <Tabs value={tab} onValueChange={(v) => setTab(v as typeof tab)}>
      <TabsList className="grid w-full grid-cols-2">
        <TabsTrigger value="templates" className="gap-2">
          <Grid className="h-4 w-4" />
          Templates
        </TabsTrigger>
        <TabsTrigger value="custom" className="gap-2">
          <Settings className="h-4 w-4" />
          Custom
        </TabsTrigger>
      </TabsList>

      <TabsContent value="templates" className="mt-4">
        <ClientTemplates onSelectTemplate={onSelect} />
      </TabsContent>

      <TabsContent value="custom" className="mt-4">
        <div className="space-y-4">
          <div className="bg-blue-50 dark:bg-blue-900/20 border border-blue-200 dark:border-blue-800 rounded p-3">
            <p className="text-sm font-medium text-blue-900 dark:text-blue-100">
              OpenAI-Compatible Application
            </p>
            <p className="text-xs text-blue-700 dark:text-blue-300 mt-1">
              Manual setup for any application that supports the OpenAI API format.
              You'll configure the connection details after creating the client.
            </p>
          </div>
          <button
            onClick={() => onSelect(CUSTOM_CLIENT_TEMPLATE)}
            className="w-full flex items-center gap-3 p-4 rounded-lg border-2 border-muted hover:border-primary hover:bg-accent transition-colors focus:outline-none focus:ring-2 focus:ring-ring focus:ring-offset-2 text-left"
          >
            <span className="text-2xl">⚙️</span>
            <div>
              <p className="font-medium text-sm">Create Custom Client</p>
              <p className="text-xs text-muted-foreground">
                Full control over name, models, MCP servers, and skills access.
              </p>
            </div>
          </button>
        </div>
      </TabsContent>
    </Tabs>
  )
}

export { CUSTOM_CLIENT_TEMPLATE }
