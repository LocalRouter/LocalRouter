import { useState, useCallback } from "react"
import { Bug, ExternalLink, Check, Camera, FileCode } from "lucide-react"
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
import { Separator } from "@/components/ui/separator"

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
      toast.success("Screenshot copied — paste it into the issue")
      setTimeout(() => setCopiedScreenshot(false), 2000)
    } catch {
      toast.error("Failed to copy screenshot")
    }
  }, [screenshotDataUrl])

  const handleCopyConfig = useCallback(async () => {
    try {
      const rawConfig = await invoke<unknown>("get_config")
      const sanitized = sanitizeConfig(rawConfig)
      const text = "```json\n" + JSON.stringify(sanitized, null, 2) + "\n```"
      await invoke("copy_text_to_clipboard", { text })
      setCopiedConfig(true)
      toast.success("Configuration copied — paste it into the issue")
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
              Open an issue on GitHub, then copy any helpful info to paste in.
            </DialogDescription>
          </DialogHeader>

          {/* Primary action */}
          <Button className="w-full" size="lg" onClick={handleOpenIssue}>
            <ExternalLink className="mr-2 h-4 w-4" />
            Open GitHub Issue
          </Button>

          <Separator />

          {/* Copy-to-clipboard helpers */}
          <div className="space-y-1.5">
            <p className="text-xs font-medium text-muted-foreground">
              Copy to clipboard and paste into the issue
            </p>

            <div className="flex gap-2">
              {screenshotDataUrl && (
                <Button
                  variant="outline"
                  size="sm"
                  className="flex-1"
                  onClick={handleCopyScreenshot}
                >
                  {copiedScreenshot ? (
                    <Check className="mr-1.5 h-3.5 w-3.5 text-green-500" />
                  ) : (
                    <Camera className="mr-1.5 h-3.5 w-3.5" />
                  )}
                  {copiedScreenshot ? "Copied!" : "Screenshot"}
                </Button>
              )}

              <Button
                variant="outline"
                size="sm"
                className="flex-1"
                onClick={handleCopyConfig}
              >
                {copiedConfig ? (
                  <Check className="mr-1.5 h-3.5 w-3.5 text-green-500" />
                ) : (
                  <FileCode className="mr-1.5 h-3.5 w-3.5" />
                )}
                {copiedConfig ? "Copied!" : "Config"}
                <span className="ml-1 text-[10px] text-muted-foreground">
                  (secrets removed)
                </span>
              </Button>
            </div>
          </div>

          {/* Screenshot preview */}
          {screenshotDataUrl && (
            <img
              src={screenshotDataUrl}
              alt="Screenshot preview"
              className="max-h-32 w-full rounded-md border object-contain opacity-60"
            />
          )}
        </DialogContent>
      </Dialog>
    </>
  )
}
