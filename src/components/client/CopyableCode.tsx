import { useState } from "react"
import { toast } from "sonner"
import { Copy, Check, Eye, Loader2 } from "lucide-react"
import { Button } from "@/components/ui/Button"

export function CopyableCode({
  value,
  masked = false,
  showValue = true,
  onToggleShow,
  loading = false,
}: {
  value: string
  masked?: boolean
  showValue?: boolean
  onToggleShow?: () => void
  loading?: boolean
}) {
  const [copied, setCopied] = useState(false)
  const maskedValue = "••••••••••••••••••••••••••••••••"

  const handleCopy = async () => {
    try {
      await navigator.clipboard.writeText(value)
      setCopied(true)
      setTimeout(() => setCopied(false), 2000)
      toast.success("Copied to clipboard")
    } catch {
      toast.error("Failed to copy")
    }
  }

  return (
    <div className="flex items-center gap-2 min-w-0">
      <code className="flex-1 min-w-0 p-3 text-sm bg-muted rounded-md font-mono break-all">
        {loading ? (
          <span className="flex items-center gap-2 text-muted-foreground">
            <Loader2 className="h-4 w-4 animate-spin" />
            Loading...
          </span>
        ) : masked ? (showValue ? value : maskedValue) : value}
      </code>
      {masked && onToggleShow && !showValue && (
        <Button
          variant="outline"
          size="icon"
          className="shrink-0"
          onClick={onToggleShow}
          title="Show"
          disabled={loading || !value}
        >
          <Eye className="h-4 w-4" />
        </Button>
      )}
      <Button
        variant="outline"
        size="icon"
        className="shrink-0"
        onClick={handleCopy}
        title="Copy to clipboard"
        disabled={loading || !value}
      >
        {copied ? <Check className="h-4 w-4 text-green-500" /> : <Copy className="h-4 w-4" />}
      </Button>
    </div>
  )
}

// Helper component for copyable multi-line code blocks
export function CopyableCodeBlock({ value, copyValue, className = "" }: { value: string; copyValue?: string; className?: string }) {
  const [copied, setCopied] = useState(false)

  const handleCopy = async () => {
    try {
      await navigator.clipboard.writeText(copyValue ?? value)
      setCopied(true)
      setTimeout(() => setCopied(false), 2000)
      toast.success("Copied to clipboard")
    } catch {
      toast.error("Failed to copy")
    }
  }

  return (
    <div className={`relative min-w-0 ${className}`}>
      <pre className="text-xs font-mono bg-muted p-3 pr-10 rounded-lg whitespace-pre-wrap break-words overflow-hidden">
        {value}
      </pre>
      <Button
        variant="outline"
        size="icon"
        className="absolute top-2 right-2 h-7 w-7 shrink-0"
        onClick={handleCopy}
        title="Copy to clipboard"
      >
        {copied ? <Check className="h-3 w-3 text-green-500" /> : <Copy className="h-3 w-3" />}
      </Button>
    </div>
  )
}
