import { useState } from "react"
import { Shield, ChevronDown, ChevronUp } from "lucide-react"
import { Button } from "@/components/ui/button"

interface DemoDetails {
  client_name: string
  tool_name: string
  server_name: string
  arguments_preview: string
}

const demoDetails: DemoDetails = {
  client_name: "Claude Code",
  tool_name: "read_file",
  server_name: "filesystem",
  arguments_preview: '{\n  "path": "/Users/matus/project/src/index.ts"\n}',
}

export function FirewallApprovalDemo() {
  const [showArgs, setShowArgs] = useState(true)

  return (
    <div className="flex flex-col bg-slate-900 rounded-lg border-2 border-slate-700 shadow-2xl max-w-sm">
      {/* Header */}
      <div className="flex items-center justify-between p-3 border-b border-slate-700">
        <div className="flex items-center gap-2">
          <Shield className="h-5 w-5 text-amber-500" />
          <h1 className="text-sm font-semibold text-white">Tool Approval Required</h1>
        </div>
      </div>

      {/* Details */}
      <div className="space-y-2 flex-1 overflow-auto p-4">
        <div className="grid grid-cols-[auto_1fr] gap-x-3 gap-y-1.5 text-xs">
          <span className="text-slate-400">Client:</span>
          <span className="font-medium text-white truncate">{demoDetails.client_name}</span>

          <span className="text-slate-400">Tool:</span>
          <code className="font-mono bg-slate-800 px-1 py-0.5 rounded truncate text-emerald-400">
            {demoDetails.tool_name}
          </code>

          <span className="text-slate-400">Server:</span>
          <span className="text-white truncate">{demoDetails.server_name}</span>
        </div>

        {/* Arguments Preview */}
        {demoDetails.arguments_preview && (
          <div>
            <button
              type="button"
              onClick={() => setShowArgs(!showArgs)}
              className="text-xs text-slate-400 hover:text-white transition-colors flex items-center gap-1"
            >
              {showArgs ? <ChevronUp className="h-3 w-3" /> : <ChevronDown className="h-3 w-3" />}
              {showArgs ? "Hide" : "Show"} arguments
            </button>
            {showArgs && (
              <pre className="mt-1 text-xs bg-slate-800 p-2 rounded overflow-auto max-h-24 font-mono text-slate-300">
                {demoDetails.arguments_preview}
              </pre>
            )}
          </div>
        )}
      </div>

      {/* Action Buttons */}
      <div className="flex gap-2 p-4 pt-0">
        <Button
          variant="destructive"
          size="sm"
          className="flex-1"
          disabled
        >
          Deny
        </Button>
        <Button
          variant="outline"
          size="sm"
          className="flex-1 border-slate-600 text-slate-300 hover:bg-slate-800"
          disabled
        >
          Allow Once
        </Button>
        <Button
          size="sm"
          className="flex-1 bg-emerald-600 hover:bg-emerald-700 text-white"
          disabled
        >
          Allow Session
        </Button>
      </div>
    </div>
  )
}
