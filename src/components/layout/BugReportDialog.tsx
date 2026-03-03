import { useState, useCallback } from "react"
import { Bug, ExternalLink } from "lucide-react"
import { toPng } from "html-to-image"
import { invoke } from "@tauri-apps/api/core"
import { open } from "@tauri-apps/plugin-shell"
import { toast } from "sonner"
import { Button } from "@/components/ui/Button"
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog"
import { Checkbox } from "@/components/ui/checkbox"
import { Label } from "@/components/ui/label"
import { Input } from "@/components/ui/Input"
import { Textarea } from "@/components/ui/textarea"
import { cn } from "@/lib/utils"

const SENSITIVE_KEY_PATTERN =
  /api_key|secret|token|password|^key$|auth_header|credential|private_key/i

function sanitizeConfig(obj: unknown): unknown {
  if (obj === null || obj === undefined) return obj
  if (Array.isArray(obj)) return obj.map(sanitizeConfig)
  if (typeof obj === "object") {
    const result: Record<string, unknown> = {}
    for (const [key, value] of Object.entries(obj as Record<string, unknown>)) {
      if (SENSITIVE_KEY_PATTERN.test(key)) {
        result[key] = "[REDACTED]"
      } else {
        result[key] = sanitizeConfig(value)
      }
    }
    return result
  }
  return obj
}

const MAX_URL_LENGTH = 8000

export function BugReportDialog() {
  const [dialogOpen, setDialogOpen] = useState(false)
  const [screenshotDataUrl, setScreenshotDataUrl] = useState<string | null>(null)
  const [includeScreenshot, setIncludeScreenshot] = useState(false)
  const [includeConfig, setIncludeConfig] = useState(false)
  const [title, setTitle] = useState("")
  const [description, setDescription] = useState("")
  const [isCapturing, setIsCapturing] = useState(false)

  const captureAndOpen = useCallback(async () => {
    setIsCapturing(true)
    try {
      const dataUrl = await toPng(document.documentElement, {
        cacheBust: true,
        pixelRatio: 1,
      })
      setScreenshotDataUrl(dataUrl)
    } catch {
      setScreenshotDataUrl(null)
    }
    setIsCapturing(false)
    setIncludeScreenshot(false)
    setIncludeConfig(false)
    setTitle("")
    setDescription("")
    setDialogOpen(true)
  }, [])

  const handleSubmit = useCallback(async () => {
    let appVersion = "unknown"
    try {
      appVersion = await invoke<string>("get_app_version")
    } catch {
      // ignore
    }

    let configSection = ""
    if (includeConfig) {
      try {
        const rawConfig = await invoke<unknown>("get_config")
        const sanitized = sanitizeConfig(rawConfig)
        configSection = `\n\n<details>\n<summary>Configuration (sanitized)</summary>\n\n\`\`\`json\n${JSON.stringify(sanitized, null, 2)}\n\`\`\`\n\n</details>`
      } catch {
        configSection = "\n\n*Could not retrieve configuration.*"
      }
    }

    const screenshotNote = includeScreenshot
      ? "\n\n**Screenshot:** *(paste from clipboard below)*\n"
      : ""

    const body = [
      description || "*No description provided.*",
      "",
      "---",
      `**App version:** ${appVersion}`,
      `**OS:** ${navigator.userAgent}`,
      screenshotNote,
      configSection,
    ].join("\n")

    let issueUrl = `https://github.com/LocalRouter/LocalRouter/issues/new?title=${encodeURIComponent(title)}&body=${encodeURIComponent(body)}`

    // Truncate config if URL is too long
    if (issueUrl.length > MAX_URL_LENGTH && includeConfig) {
      const bodyWithoutConfig = [
        description || "*No description provided.*",
        "",
        "---",
        `**App version:** ${appVersion}`,
        `**OS:** ${navigator.userAgent}`,
        screenshotNote,
        "\n\n*Configuration omitted (too large for URL).*",
      ].join("\n")
      issueUrl = `https://github.com/LocalRouter/LocalRouter/issues/new?title=${encodeURIComponent(title)}&body=${encodeURIComponent(bodyWithoutConfig)}`
    }

    // Copy screenshot to clipboard if included
    if (includeScreenshot && screenshotDataUrl) {
      try {
        const res = await fetch(screenshotDataUrl)
        const blob = await res.blob()
        await navigator.clipboard.write([
          new ClipboardItem({ "image/png": blob }),
        ])
        toast.success("Screenshot copied to clipboard — paste it into the GitHub issue")
      } catch {
        toast.error("Failed to copy screenshot to clipboard")
      }
    }

    await open(issueUrl)
    setDialogOpen(false)
  }, [title, description, includeScreenshot, includeConfig, screenshotDataUrl])

  return (
    <>
      <Button
        variant="ghost"
        size="icon"
        onClick={captureAndOpen}
        disabled={isCapturing}
      >
        <Bug className="h-4 w-4" />
        <span className="sr-only">Report a bug</span>
      </Button>

      <Dialog open={dialogOpen} onOpenChange={setDialogOpen}>
        <DialogContent className="sm:max-w-lg">
          <DialogHeader>
            <DialogTitle>Report a Bug</DialogTitle>
            <DialogDescription>
              This will open a new issue on GitHub with the details below.
            </DialogDescription>
          </DialogHeader>

          <div className="space-y-4">
            {/* Screenshot preview */}
            {screenshotDataUrl && (
              <div className="space-y-2">
                <div className="flex items-center gap-2">
                  <Checkbox
                    id="include-screenshot"
                    checked={includeScreenshot}
                    onCheckedChange={(checked) =>
                      setIncludeScreenshot(checked === true)
                    }
                  />
                  <Label htmlFor="include-screenshot">Include screenshot</Label>
                </div>
                <img
                  src={screenshotDataUrl}
                  alt="Screenshot preview"
                  className={cn(
                    "max-h-40 w-full rounded-md border object-contain",
                    !includeScreenshot && "opacity-40"
                  )}
                />
              </div>
            )}

            {/* Include config */}
            <div className="flex items-center gap-2">
              <Checkbox
                id="include-config"
                checked={includeConfig}
                onCheckedChange={(checked) =>
                  setIncludeConfig(checked === true)
                }
              />
              <Label htmlFor="include-config">
                Include configuration{" "}
                <span className="text-xs text-muted-foreground">
                  (API keys and secrets are removed)
                </span>
              </Label>
            </div>

            {/* Title */}
            <div className="space-y-1.5">
              <Label htmlFor="bug-title">Title</Label>
              <Input
                id="bug-title"
                placeholder="Brief summary of the issue"
                value={title}
                onChange={(e) => setTitle(e.target.value)}
              />
            </div>

            {/* Description */}
            <div className="space-y-1.5">
              <Label htmlFor="bug-description">Description</Label>
              <Textarea
                id="bug-description"
                placeholder="What happened? What did you expect?"
                value={description}
                onChange={(e) => setDescription(e.target.value)}
                rows={4}
              />
            </div>
          </div>

          <DialogFooter>
            <Button variant="outline" onClick={() => setDialogOpen(false)}>
              Cancel
            </Button>
            <Button onClick={handleSubmit}>
              <ExternalLink className="mr-2 h-4 w-4" />
              Open GitHub Issue
            </Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>
    </>
  )
}
