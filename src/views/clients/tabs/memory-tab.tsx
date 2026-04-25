import { useState, useEffect, useCallback, useRef } from "react"
import { invoke } from "@tauri-apps/api/core"
import { toast } from "sonner"
import { FEATURES } from "@/constants/features"
import { Card, CardContent, CardHeader, CardTitle, CardDescription } from "@/components/ui/Card"
import { Switch } from "@/components/ui/switch"
import { Button } from "@/components/ui/Button"
import { FolderOpen } from "lucide-react"
import type { MemoryConfig } from "@/types/tauri-commands"
import {
  AlertDialog,
  AlertDialogAction,
  AlertDialogCancel,
  AlertDialogContent,
  AlertDialogDescription,
  AlertDialogFooter,
  AlertDialogHeader,
  AlertDialogTitle,
  AlertDialogTrigger,
} from "@/components/ui/alert-dialog"

interface Client {
  id: string
  name: string
  client_id: string
}

interface ClientMemoryTabProps {
  client: Client
  onUpdate: () => void
  onViewChange?: (view: string, subTab?: string | null) => void
}

export function ClientMemoryTab({ client, onUpdate }: ClientMemoryTabProps) {
  const [memoryEnabled, setMemoryEnabled] = useState<boolean | null>(null)
  const [memoryConfig, setMemoryConfig] = useState<MemoryConfig | null>(null)
  const [loading, setLoading] = useState(true)

  const loadReqIdRef = useRef(0)

  const loadConfig = useCallback(async () => {
    const reqId = ++loadReqIdRef.current
    try {
      const [result, globalMemoryConfig] = await Promise.all([
        invoke<{ memory_enabled: boolean | null }>("get_client_memory_config", {
          clientId: client.id,
        }),
        invoke<MemoryConfig>("get_memory_config"),
      ])
      if (loadReqIdRef.current !== reqId) return
      setMemoryEnabled(result.memory_enabled)
      setMemoryConfig(globalMemoryConfig)
    } catch (err) {
      if (loadReqIdRef.current !== reqId) return
      console.error("Failed to load memory config:", err)
    } finally {
      if (loadReqIdRef.current === reqId) setLoading(false)
    }
  }, [client.id])

  useEffect(() => {
    setLoading(true)
    loadConfig()
    return () => {
      loadReqIdRef.current++
    }
  }, [loadConfig])

  const toggleMemory = async (enabled: boolean) => {
    try {
      await invoke("update_client_memory_config", {
        clientId: client.id,
        enabled,
      })
      setMemoryEnabled(enabled)
      onUpdate()
      toast.success(enabled ? "Memory enabled for this client" : "Memory disabled for this client")
    } catch (err: any) {
      toast.error(`Failed to update: ${err.message || err}`)
      loadConfig()
    }
  }

  if (loading) {
    return (
      <div className="flex items-center justify-center py-8">
        <p className="text-muted-foreground">Loading...</p>
      </div>
    )
  }

  return (
    <div className="space-y-4">
      <Card>
        <CardHeader>
          <div className="flex items-center justify-between">
            <div className="flex items-center gap-2">
              <FEATURES.memory.icon className={`h-5 w-5 ${FEATURES.memory.color}`} />
              <CardTitle>{FEATURES.memory.name}</CardTitle>
            </div>
            <Switch
              checked={memoryEnabled === true}
              onCheckedChange={toggleMemory}
            />
          </div>
          <CardDescription>
            {memoryEnabled
              ? "Conversations with this client are recorded and stored locally for future recall."
              : "Enable to record conversations and make them searchable via the MemoryRecall tool."}
          </CardDescription>
        </CardHeader>
        {memoryConfig && (
          <CardContent className="pt-0 pb-3">
            <p className="text-xs text-muted-foreground mb-1.5">Exposed tools:</p>
            <div className="flex flex-wrap gap-1.5">
              <code className="text-[11px] px-1.5 py-0.5 rounded bg-muted">{memoryConfig.recall_tool_name}</code>
              <code className="text-[11px] px-1.5 py-0.5 rounded bg-muted">
                {memoryConfig.recall_tool_name.endsWith("Search")
                  ? memoryConfig.recall_tool_name.replace(/Search$/, "Read")
                  : memoryConfig.recall_tool_name.endsWith("Recall")
                    ? memoryConfig.recall_tool_name.replace(/Recall$/, "Read")
                    : `${memoryConfig.recall_tool_name}Read`}
              </code>
            </div>
          </CardContent>
        )}
        {memoryEnabled && (
          <CardContent className="pt-0">
            <div className="flex gap-2">
              <Button
                variant="outline"
                size="sm"
                onClick={() => invoke("open_client_memory_folder", { clientId: client.id }).catch((e: any) => toast.error(`${e}`))}
              >
                <FolderOpen className="h-3.5 w-3.5 mr-1.5" />
                Open Folder
              </Button>
              <AlertDialog>
                <AlertDialogTrigger asChild>
                  <Button variant="destructive" size="sm">Clear Memory</Button>
                </AlertDialogTrigger>
                <AlertDialogContent>
                  <AlertDialogHeader>
                    <AlertDialogTitle>Clear memory for &ldquo;{client.name}&rdquo;?</AlertDialogTitle>
                    <AlertDialogDescription>
                      This will permanently delete all stored conversations, session transcripts, and the search index for this client. This action cannot be undone.
                    </AlertDialogDescription>
                  </AlertDialogHeader>
                  <AlertDialogFooter>
                    <AlertDialogCancel>Cancel</AlertDialogCancel>
                    <AlertDialogAction
                      onClick={async () => {
                        try {
                          await invoke("clear_client_memory", { clientId: client.id })
                          toast.success("Memory cleared")
                          onUpdate()
                        } catch (err: any) {
                          toast.error(`Failed to clear: ${err.message || err}`)
                        }
                      }}
                      className="bg-red-600 text-white hover:bg-red-700 dark:bg-red-600 dark:hover:bg-red-700"
                    >
                      Clear Memory
                    </AlertDialogAction>
                  </AlertDialogFooter>
                </AlertDialogContent>
              </AlertDialog>
            </div>
          </CardContent>
        )}
    </Card>
    </div>
  )
}
