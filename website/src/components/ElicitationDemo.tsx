/**
 * Website demo wrapper for the Elicitation form popup.
 * Static version of the real elicitation form (src/views/elicitation-form.tsx)
 * without Tauri dependencies, suitable for the landing page.
 */
import { MessageSquare } from "lucide-react"
import { Button } from "@app/components/ui/Button"

export function ElicitationDemo() {
  return (
    <div className="dark rounded-lg border-2 border-border shadow-2xl max-w-sm bg-background">
      <div className="flex flex-col p-4">
        {/* Header */}
        <div className="mb-3">
          <div className="flex items-center gap-2 mb-0.5">
            <MessageSquare className="h-5 w-5 text-purple-500" />
            <h1 className="text-sm font-bold">Input Required</h1>
          </div>
          <p className="text-xs text-muted-foreground">
            Server <span className="font-medium text-foreground">deploy-manager</span> is requesting user input
          </p>
        </div>

        {/* Message */}
        <p className="text-xs mb-3">Select the deployment target and confirm settings.</p>

        {/* Form Fields */}
        <div className="space-y-2">
          <div className="grid grid-cols-[auto_1fr] gap-x-3 items-center">
            <label className="text-xs text-muted-foreground">Region:</label>
            <select
              className="h-7 px-2 text-xs rounded border border-border bg-background text-foreground font-mono"
              defaultValue="us-east-1"
            >
              <option value="us-east-1">us-east-1</option>
              <option value="eu-west-1">eu-west-1</option>
              <option value="ap-southeast-1">ap-southeast-1</option>
            </select>
          </div>
          <div className="grid grid-cols-[auto_1fr] gap-x-3 items-center">
            <label className="text-xs text-muted-foreground">Branch:</label>
            <input
              className="h-7 px-2 text-xs rounded border border-border bg-background text-foreground font-mono"
              defaultValue="main"
              readOnly
            />
          </div>
          <div className="grid grid-cols-[auto_1fr] gap-x-3 items-center">
            <label className="text-xs text-muted-foreground">Dry run:</label>
            <div className="flex items-center">
              <div className="h-5 w-9 rounded-full bg-primary relative cursor-default">
                <div className="absolute right-0.5 top-0.5 h-4 w-4 rounded-full bg-white shadow" />
              </div>
            </div>
          </div>
        </div>

        {/* Action Buttons */}
        <div className="flex gap-2 pt-3 mt-3">
          <Button
            variant="destructive"
            className="flex-1 h-10 font-bold"
            disabled
          >
            Deny
          </Button>
          <Button
            className="flex-1 h-10 bg-emerald-600 hover:bg-emerald-700 text-white font-bold"
            disabled
          >
            Submit
          </Button>
        </div>
      </div>
    </div>
  )
}
