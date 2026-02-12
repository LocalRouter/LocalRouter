/**
 * StepTemplate - Template selection step for the client creation wizard.
 *
 * Layout:
 * - Big "Custom" button at the top for manual setup
 * - Separator
 * - Grid of known app templates (Claude Code, Cursor, etc.)
 */

import { Settings } from "lucide-react"
import { Separator } from "@/components/ui/separator"
import { ClientTemplates, CUSTOM_CLIENT_TEMPLATE } from "@/components/client/ClientTemplates"
import type { ClientTemplate } from "@/components/client/ClientTemplates"

interface StepTemplateProps {
  onSelect: (template: ClientTemplate) => void
}

export function StepTemplate({ onSelect }: StepTemplateProps) {
  return (
    <div className="space-y-4">
      <button
        onClick={() => onSelect(CUSTOM_CLIENT_TEMPLATE)}
        className="w-full flex items-center gap-4 p-5 rounded-lg border-2 border-dashed border-muted hover:border-primary hover:bg-accent transition-colors focus:outline-none focus:ring-2 focus:ring-ring focus:ring-offset-2 text-left"
      >
        <div className="flex items-center justify-center h-10 w-10 rounded-lg bg-muted">
          <Settings className="h-5 w-5 text-muted-foreground" />
        </div>
        <div>
          <p className="font-medium text-sm">Custom</p>
          <p className="text-xs text-muted-foreground">
            Manual setup for any OpenAI-compatible application.
          </p>
        </div>
      </button>

      <Separator />

      <ClientTemplates onSelectTemplate={onSelect} />
    </div>
  )
}

export { CUSTOM_CLIENT_TEMPLATE }
