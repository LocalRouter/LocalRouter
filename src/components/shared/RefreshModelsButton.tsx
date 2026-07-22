/**
 * RefreshModelsButton — force-refreshes provider model lists.
 *
 * Invokes `refresh_models_incremental { force: true }`, which invalidates the
 * per-provider model caches and refetches in parallel. Progress is broadcast
 * via the `models-refresh-started` / `models-changed` events that every
 * `useIncrementalModels` instance already listens to, so all model lists in
 * the app update automatically.
 */

import { useEffect, useState } from "react"
import { invoke } from "@tauri-apps/api/core"
import { RefreshCw } from "lucide-react"
import { toast } from "sonner"
import { listenSafe } from "@/hooks/useTauriListener"
import { Button } from "@/components/ui/Button"
import { cn } from "@/lib/utils"

interface RefreshModelsButtonProps {
  /** Show the "Refresh models" label next to the icon (default: icon only) */
  showLabel?: boolean
  size?: "default" | "sm" | "lg" | "icon"
  variant?: "default" | "outline" | "ghost"
  className?: string
}

export function RefreshModelsButton({
  showLabel = false,
  size = "sm",
  variant = "ghost",
  className,
}: RefreshModelsButtonProps) {
  const [refreshing, setRefreshing] = useState(false)

  // Track refresh lifecycle globally so the spinner also reflects refreshes
  // started elsewhere (another button, provider toggle, mount refresh).
  useEffect(() => {
    const started = listenSafe("models-refresh-started", () => setRefreshing(true))
    const done = listenSafe("models-changed", () => setRefreshing(false))
    return () => {
      started.cleanup()
      done.cleanup()
    }
  }, [])

  const handleClick = async () => {
    setRefreshing(true)
    try {
      await invoke("refresh_models_incremental", { force: true })
    } catch (error) {
      console.error("Failed to refresh models:", error)
      toast.error("Failed to refresh models")
      setRefreshing(false)
    }
  }

  return (
    <Button
      variant={variant}
      size={size}
      onClick={handleClick}
      disabled={refreshing}
      title="Refresh model lists from all providers"
      className={className}
    >
      <RefreshCw className={cn("h-3.5 w-3.5", refreshing && "animate-spin", showLabel && "mr-1.5")} />
      {showLabel && "Refresh models"}
    </Button>
  )
}
