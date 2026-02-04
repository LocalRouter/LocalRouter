
import { useState, useEffect, useRef, useCallback } from "react"
import { invoke } from "@tauri-apps/api/core"
import { toast } from "sonner"
import { Loader2 } from "lucide-react"
import { Card, CardContent, CardHeader, CardTitle, CardDescription } from "@/components/ui/Card"
import { Input } from "@/components/ui/Input"
import { Switch } from "@/components/ui/Toggle"
import { Button } from "@/components/ui/Button"
import {
  AlertDialog,
  AlertDialogAction,
  AlertDialogCancel,
  AlertDialogContent,
  AlertDialogDescription,
  AlertDialogFooter,
  AlertDialogHeader,
  AlertDialogTitle,
} from "@/components/ui/alert-dialog"

interface Client {
  id: string
  name: string
  client_id: string
  enabled: boolean
  strategy_id: string
}

interface SettingsTabProps {
  client: Client
  onUpdate: () => void
  onDelete: () => void
}

export function ClientSettingsTab({ client, onUpdate, onDelete }: SettingsTabProps) {
  const [name, setName] = useState(client.name)
  const [saving, setSaving] = useState(false)
  const [showDeleteDialog, setShowDeleteDialog] = useState(false)
  const [showRotateDialog, setShowRotateDialog] = useState(false)
  const [rotating, setRotating] = useState(false)

  // Debounce ref for name updates
  const nameTimeoutRef = useRef<ReturnType<typeof setTimeout> | null>(null)

  // Sync name state when client prop updates
  useEffect(() => {
    setName(client.name)
  }, [client.name])

  // Cleanup timeout on unmount
  useEffect(() => {
    return () => {
      if (nameTimeoutRef.current) {
        clearTimeout(nameTimeoutRef.current)
      }
    }
  }, [])

  // Debounced name save
  const handleNameChange = useCallback((newName: string) => {
    setName(newName)

    // Clear existing timeout
    if (nameTimeoutRef.current) {
      clearTimeout(nameTimeoutRef.current)
    }

    // Debounce the save
    nameTimeoutRef.current = setTimeout(async () => {
      if (newName === client.name || !newName.trim()) return

      try {
        setSaving(true)
        await invoke("update_client_name", {
          clientId: client.client_id,
          name: newName,
        })
        toast.success("Client name updated")
        onUpdate()
      } catch (error) {
        console.error("Failed to update client:", error)
        toast.error("Failed to update client")
      } finally {
        setSaving(false)
      }
    }, 500)
  }, [client.name, client.client_id, onUpdate])

  const handleToggleEnabled = async () => {
    try {
      await invoke("toggle_client_enabled", {
        clientId: client.client_id,
        enabled: !client.enabled,
      })
      toast.success(`Client ${client.enabled ? "disabled" : "enabled"}`)
      onUpdate()
    } catch (error) {
      console.error("Failed to toggle client:", error)
      toast.error("Failed to update client")
    }
  }

  const handleRotateConfirm = async () => {
    try {
      setRotating(true)
      await invoke("rotate_client_secret", { clientId: client.id })
      toast.success("Credentials rotated successfully. View the new credentials in the Connect tab.")
      onUpdate()
    } catch (error) {
      console.error("Failed to rotate credentials:", error)
      toast.error("Failed to rotate credentials")
    } finally {
      setRotating(false)
      setShowRotateDialog(false)
    }
  }

  const handleDeleteConfirm = async () => {
    try {
      await invoke("delete_client", { clientId: client.client_id })
      toast.success("Client deleted")
      onDelete()
    } catch (error) {
      console.error("Failed to delete client:", error)
      toast.error("Failed to delete client")
    } finally {
      setShowDeleteDialog(false)
    }
  }

  return (
    <div className="space-y-6">
      {/* Client Name */}
      <Card>
        <CardHeader>
          <CardTitle>Client Name</CardTitle>
          <CardDescription>
            Display name for this client
          </CardDescription>
        </CardHeader>
        <CardContent>
          <div className="flex items-center gap-2">
            <Input
              id="name"
              value={name}
              onChange={(e) => handleNameChange(e.target.value)}
              placeholder="Enter client name"
              className="max-w-md"
            />
            {saving && (
              <Loader2 className="h-4 w-4 animate-spin text-muted-foreground" />
            )}
          </div>
        </CardContent>
      </Card>

      {/* Enable/Disable */}
      <Card>
        <CardHeader>
          <CardTitle>Enable Client</CardTitle>
          <CardDescription>
            When disabled, this client's API key will be rejected
          </CardDescription>
        </CardHeader>
        <CardContent>
          <div className="flex items-center gap-3">
            <Switch
              checked={client.enabled}
              onCheckedChange={handleToggleEnabled}
            />
            <span className="text-sm">
              {client.enabled ? "Enabled" : "Disabled"}
            </span>
          </div>
        </CardContent>
      </Card>

      {/* Danger Zone */}
      <Card className="border-red-200 dark:border-red-900">
        <CardHeader>
          <CardTitle className="text-red-600 dark:text-red-400">Danger Zone</CardTitle>
          <CardDescription>
            Irreversible actions for this client
          </CardDescription>
        </CardHeader>
        <CardContent className="space-y-4">
          <div className="flex items-center justify-between">
            <div>
              <p className="text-sm font-medium">Rotate credentials</p>
              <p className="text-sm text-muted-foreground">
                Generate a new API key. The old key will stop working immediately.
              </p>
            </div>
            <Button
              variant="outline"
              onClick={() => setShowRotateDialog(true)}
              disabled={rotating}
              className="border-red-200 text-red-600 hover:bg-red-50 dark:border-red-900 dark:text-red-400 dark:hover:bg-red-950"
            >
              {rotating ? <Loader2 className="h-4 w-4 animate-spin mr-2" /> : null}
              Rotate Credentials
            </Button>
          </div>
          <div className="flex items-center justify-between pt-4 border-t">
            <div>
              <p className="text-sm font-medium">Delete this client</p>
              <p className="text-sm text-muted-foreground">
                Permanently delete "{client.name}" and revoke its API key
              </p>
            </div>
            <Button
              variant="destructive"
              onClick={() => setShowDeleteDialog(true)}
            >
              Delete Client
            </Button>
          </div>
        </CardContent>
      </Card>

      <AlertDialog open={showRotateDialog} onOpenChange={setShowRotateDialog}>
        <AlertDialogContent>
          <AlertDialogHeader>
            <AlertDialogTitle>Rotate Credentials?</AlertDialogTitle>
            <AlertDialogDescription>
              This will generate a new API key for "{client.name}". The old key will
              stop working immediately. You'll need to update any applications using
              this client's credentials.
            </AlertDialogDescription>
          </AlertDialogHeader>
          <AlertDialogFooter>
            <AlertDialogCancel>Cancel</AlertDialogCancel>
            <AlertDialogAction
              onClick={handleRotateConfirm}
              className="bg-red-600 text-white hover:bg-red-700 dark:bg-red-600 dark:hover:bg-red-700"
            >
              Rotate
            </AlertDialogAction>
          </AlertDialogFooter>
        </AlertDialogContent>
      </AlertDialog>

      <AlertDialog open={showDeleteDialog} onOpenChange={setShowDeleteDialog}>
        <AlertDialogContent>
          <AlertDialogHeader>
            <AlertDialogTitle>Delete Client?</AlertDialogTitle>
            <AlertDialogDescription>
              This will permanently delete "{client.name}" and revoke its API key.
              This action cannot be undone.
            </AlertDialogDescription>
          </AlertDialogHeader>
          <AlertDialogFooter>
            <AlertDialogCancel>Cancel</AlertDialogCancel>
            <AlertDialogAction
              onClick={handleDeleteConfirm}
              className="bg-red-600 text-white hover:bg-red-700 dark:bg-red-600 dark:hover:bg-red-700"
            >
              Delete
            </AlertDialogAction>
          </AlertDialogFooter>
        </AlertDialogContent>
      </AlertDialog>
    </div>
  )
}
