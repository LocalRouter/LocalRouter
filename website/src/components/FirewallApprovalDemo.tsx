/**
 * Website demo wrapper for the shared FirewallApprovalCard.
 * Renders the real app component with static demo data in a dark-themed container.
 * The `dark` class scopes the CSS variables so theme-aware classes (bg-background, etc.) resolve correctly.
 *
 * Buttons are enabled for interactivity. "Modify" toggles a static edit mode view
 * that mirrors the real app's key-value editor.
 */
import { useState } from "react"
import { Button } from "@app/components/ui/Button"
import {
  FirewallApprovalCard,
  FirewallApprovalHeader,
} from "@app/components/shared/FirewallApprovalCard"

const DEMO_ARGS = { path: "/Users/matus/project/src/index.ts" }

export function FirewallApprovalDemo() {
  const [editMode, setEditMode] = useState(false)
  const [kvPairs, setKvPairs] = useState(
    Object.entries(DEMO_ARGS).map(([key, value]) => ({ key, value })),
  )
  const [editorMode, setEditorMode] = useState<"kv" | "raw">("kv")
  const [rawJson, setRawJson] = useState(JSON.stringify(DEMO_ARGS, null, 2))

  const noop = () => {}

  if (editMode) {
    return (
      <div className="dark rounded-lg border-2 border-border shadow-2xl max-w-sm bg-background">
        <div className="flex flex-col p-4">
          {/* Header */}
          <FirewallApprovalHeader requestType="tool" />

          {/* Context info */}
          <div className="grid grid-cols-[auto_1fr] gap-x-3 gap-y-1 text-xs mb-3">
            <span className="text-muted-foreground">Client:</span>
            <span className="font-medium truncate">Claude Code</span>
            <span className="text-muted-foreground">Tool:</span>
            <code className="font-mono bg-muted px-1 py-0.5 rounded truncate">read_file</code>
          </div>

          {/* Editor mode tabs */}
          <div className="flex gap-1 mb-2">
            <button
              className={`text-xs px-2 py-1 rounded ${editorMode === "kv" ? "bg-primary text-primary-foreground" : "bg-muted text-muted-foreground"}`}
              onClick={() => setEditorMode("kv")}
            >
              Fields
            </button>
            <button
              className={`text-xs px-2 py-1 rounded ${editorMode === "raw" ? "bg-primary text-primary-foreground" : "bg-muted text-muted-foreground"}`}
              onClick={() => setEditorMode("raw")}
            >
              JSON
            </button>
          </div>

          {/* Editor content */}
          {editorMode === "kv" ? (
            <div className="flex flex-col gap-1.5">
              {kvPairs.map(({ key, value }, idx) => (
                <div key={key} className="grid grid-cols-[100px_1fr] gap-2 items-start">
                  <label className="text-xs text-muted-foreground pt-1 truncate" title={key}>
                    {key}:
                  </label>
                  <textarea
                    className="min-h-[28px] px-2 py-1 text-xs rounded border border-border bg-background text-foreground font-mono resize-y focus:outline-none focus:ring-1 focus:ring-ring"
                    value={value}
                    rows={value.length > 60 ? 3 : 1}
                    onChange={(e) => {
                      const newPairs = [...kvPairs]
                      newPairs[idx] = { key, value: e.target.value }
                      setKvPairs(newPairs)
                    }}
                  />
                </div>
              ))}
            </div>
          ) : (
            <textarea
              className="h-24 px-2 py-1 text-xs rounded border border-border font-mono resize-none bg-background text-foreground focus:outline-none focus:ring-1 focus:ring-ring"
              value={rawJson}
              onChange={(e) => setRawJson(e.target.value)}
              spellCheck={false}
            />
          )}

          {/* Edit mode actions */}
          <div className="flex gap-2 pt-3 mt-auto">
            <Button
              variant="ghost"
              className="h-10"
              onClick={() => setEditMode(false)}
            >
              Back
            </Button>
            <div className="flex-1" />
            <Button
              className="h-10 bg-emerald-600 hover:bg-emerald-700 text-white font-bold"
              onClick={() => setEditMode(false)}
            >
              Allow with Edits
            </Button>
          </div>
        </div>
      </div>
    )
  }

  return (
    <div className="dark rounded-lg border-2 border-border shadow-2xl max-w-sm bg-background">
      <FirewallApprovalCard
        className="flex flex-col p-4"
        clientName="Claude Code"
        toolName="read_file"
        serverName="filesystem"
        argumentsPreview={JSON.stringify(DEMO_ARGS)}
        onAction={noop}
        onEdit={() => setEditMode(true)}
      />
    </div>
  )
}
