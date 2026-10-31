import { useEffect, useRef, useState } from "react"
import { invoke } from "@tauri-apps/api/core"
import type { GetMcpServerPushStatusParams, McpServerPushStatus } from "@/types/tauri-commands"

/** How often the live status is re-read while the page is visible. */
export const PUSH_STATUS_POLL_MS = 5_000

/**
 * Live push status of each server in `serverIds`, read from the running relay
 * and re-read every few seconds while the page is visible.
 */
export function usePushStatuses(serverIds: string[]): Record<string, McpServerPushStatus> {
  const [statuses, setStatuses] = useState<Record<string, McpServerPushStatus>>({})
  const key = serverIds.join("\u001f")
  const idsRef = useRef(serverIds)
  idsRef.current = serverIds

  useEffect(() => {
    let cancelled = false
    const load = async () => {
      if (document.visibilityState !== "visible") return
      const ids = idsRef.current
      const results = await Promise.all(
        ids.map((serverId) =>
          invoke<McpServerPushStatus>("get_mcp_server_push_status", {
            serverId,
          } satisfies GetMcpServerPushStatusParams).catch((err) => {
            console.error("Failed to load push status:", err)
            return null
          }),
        ),
      )
      if (cancelled) return
      const next: Record<string, McpServerPushStatus> = {}
      results.forEach((status, i) => {
        if (status) next[ids[i]] = status
      })
      setStatuses(next)
    }
    load()
    const timer = window.setInterval(load, PUSH_STATUS_POLL_MS)
    document.addEventListener("visibilitychange", load)
    return () => {
      cancelled = true
      window.clearInterval(timer)
      document.removeEventListener("visibilitychange", load)
    }
  }, [key])

  return statuses
}
