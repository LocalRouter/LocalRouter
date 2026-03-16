import { useState, useEffect, useCallback } from "react"
import { invoke } from "@tauri-apps/api/core"
import { toast } from "sonner"
import { FEATURES } from "@/constants/features"
import { Card, CardContent, CardHeader, CardTitle, CardDescription } from "@/components/ui/Card"
import { Switch } from "@/components/ui/switch"
import { Button } from "@/components/ui/Button"
import { AlertTriangle, FolderOpen } from "lucide-react"
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

export function ClientMemoryTab({ client, onUpdate, onViewChange }: ClientMemoryTabProps) {
  const [memoryEnabled, setMemoryEnabled] = useState<boolean | null>(null)
  const [loading, setLoading] = useState(true)

  const loadConfig = useCallback(async () => {
    try {
      const result = await invoke<{ memory_enabled: boolean | null }>("get_client_memory_config", {
        clientId: client.id,
      })
      setMemoryEnabled(result.memory_enabled)
    } catch (err) {
      console.error("Failed to load memory config:", err)
    } finally {
      setLoading(false)
    }
  }, [client.id])

  useEffect(() => {
    loadConfig()
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
              <CardTitle>Memory</CardTitle>
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
        {memoryEnabled && (
          <CardContent className="space-y-3">
            <div className="flex items-start gap-2 text-xs text-amber-600 dark:text-amber-400">
              <AlertTriangle className="h-3.5 w-3.5 mt-0.5 shrink-0" />
              <span>
                Full conversations are stored locally when memory is enabled.{" "}
                <button
                  className="underline hover:no-underline"
                  onClick={() => onViewChange?.("memory", "sessions")}
                >
                  View sessions
                </button>
              </span>
            </div>
            <div className="flex gap-2">
              <Button
                variant="outline"
                size="sm"
                onClick={() => invoke("open_client_memory_folder", { clientId: client.id }).catch((e: any) => toast.error(`${e}`))}
              >
                <FolderOpen className="h-3.5 w-3.5 mr-1.5" />
                Open Folder
              </Button>
            </div>
          </CardContent>
        )}
      </Card>

      {memoryEnabled && (
        <Card className="border-red-200 dark:border-red-900">
          <CardHeader>
            <CardTitle className="text-red-600 dark:text-red-400">Danger Zone</CardTitle>
          </CardHeader>
          <CardContent>
            <div className="flex items-center justify-between">
              <div>
                <p className="text-sm font-medium">Clear Memory</p>
                <p className="text-sm text-muted-foreground">
                  Permanently delete all stored conversations and search index for this client.
                </p>
              </div>
              <AlertDialog>
                <AlertDialogTrigger asChild>
                  <Button variant="destructive" size="sm">Clear</Button>
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
        </Card>
      )}
    </div>
  )
}
