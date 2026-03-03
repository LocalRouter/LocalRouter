import { useState, useCallback } from "react"
import { Bug, ExternalLink, Copy, Image, FileText } from "lucide-react"
import { toPng } from "html-to-image"
import { invoke } from "@tauri-apps/api/core"
import { open } from "@tauri-apps/plugin-shell"
import { toast } from "sonner"
import { Button } from "@/components/ui/Button"
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog"

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

export function BugReportDialog() {
  const [dialogOpen, setDialogOpen] = useState(false)
  const [screenshotDataUrl, setScreenshotDataUrl] = useState<string | null>(null)
  const [isCapturing, setIsCapturing] = useState(false)
  const [copiedScreenshot, setCopiedScreenshot] = useState(false)
  const [copiedConfig, setCopiedConfig] = useState(false)

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
    setCopiedScreenshot(false)
    setCopiedConfig(false)
    setDialogOpen(true)
  }, [])

  const handleOpenIssue = useCallback(async () => {
    await open("https://github.com/LocalRouter/LocalRouter/issues/new")
  }, [])

  const handleCopyScreenshot = useCallback(async () => {
    if (!screenshotDataUrl) return
    try {
      await invoke("copy_image_to_clipboard", { imageBase64: screenshotDataUrl })
      setCopiedScreenshot(true)
      toast.success("Screenshot copied to clipboard")
      setTimeout(() => setCopiedScreenshot(false), 2000)
    } catch {
      toast.error("Failed to copy screenshot to clipboard")
    }
  }, [screenshotDataUrl])

  const handleCopyConfig = useCallback(async () => {
    try {
      const rawConfig = await invoke<unknown>("get_config")
      const sanitized = sanitizeConfig(rawConfig)
      const text = "```json\n" + JSON.stringify(sanitized, null, 2) + "\n```"
      await navigator.clipboard.writeText(text)
      setCopiedConfig(true)
      toast.success("Configuration copied to clipboard")
      setTimeout(() => setCopiedConfig(false), 2000)
    } catch {
      toast.error("Failed to copy configuration")
    }
  }, [])

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
        <DialogContent className="sm:max-w-md">
          <DialogHeader>
            <DialogTitle>Report a Bug</DialogTitle>
            <DialogDescription>
              Open a GitHub issue and optionally attach a screenshot or configuration.
            </DialogDescription>
          </DialogHeader>

          <div className="space-y-4">
            {/* Open GitHub Issue - primary action */}
            <Button className="w-full" size="lg" onClick={handleOpenIssue}>
              <ExternalLink className="mr-2 h-4 w-4" />
              Open GitHub Issue
            </Button>

            {/* Screenshot preview + copy */}
            {screenshotDataUrl && (
              <div className="space-y-2">
                <img
                  src={screenshotDataUrl}
                  alt="Screenshot preview"
                  className="max-h-40 w-full rounded-md border object-contain"
                />
                <Button
                  variant="outline"
                  className="w-full"
                  onClick={handleCopyScreenshot}
                >
                  {copiedScreenshot ? (
                    <Copy className="mr-2 h-4 w-4 text-green-500" />
                  ) : (
                    <Image className="mr-2 h-4 w-4" />
                  )}
                  {copiedScreenshot ? "Copied!" : "Copy Screenshot"}
                </Button>
              </div>
            )}

            {/* Copy config */}
            <Button
              variant="outline"
              className="w-full"
              onClick={handleCopyConfig}
            >
              {copiedConfig ? (
                <Copy className="mr-2 h-4 w-4 text-green-500" />
              ) : (
                <FileText className="mr-2 h-4 w-4" />
              )}
              {copiedConfig ? "Copied!" : "Copy Configuration"}
              <span className="ml-1 text-xs text-muted-foreground">
                (secrets removed)
              </span>
            </Button>
          </div>
        </DialogContent>
      </Dialog>
    </>
  )
}
