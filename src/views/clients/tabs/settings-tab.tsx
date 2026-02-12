
import { useState, useEffect, useRef, useCallback } from "react"
import { invoke } from "@tauri-apps/api/core"
import { toast } from "sonner"
import { Loader2, Unlink } from "lucide-react"
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
import { CLIENT_TEMPLATES } from "@/components/client/ClientTemplates"
import ServiceIcon from "@/components/ServiceIcon"
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/Select"
import type { ClientMode, SetClientModeParams, SetClientTemplateParams } from "@/types/tauri-commands"

interface Client {
  id: string
  name: string
  client_id: string
  enabled: boolean
  strategy_id: string
  client_mode?: ClientMode
  template_id?: string | null
  guardrails_enabled?: boolean | null
}

interface SettingsTabProps {
  client: Client
  onUpdate: () => void
  onDelete: () => void
}

const MODE_OPTIONS: { value: ClientMode; label: string; description: string }[] = [
  { value: "both", label: "Both", description: "Full access to LLM routing and MCP servers" },
  { value: "llm_only", label: "LLM Only", description: "Only LLM routing (hides MCP/Skills tabs)" },
  { value: "mcp_only", label: "MCP Only", description: "Only MCP proxy (hides Models tab)" },
]

export function ClientSettingsTab({ client, onUpdate, onDelete }: SettingsTabProps) {
  const [name, setName] = useState(client.name)
  const [saving, setSaving] = useState(false)
  const [showDeleteDialog, setShowDeleteDialog] = useState(false)
  const [showRotateDialog, setShowRotateDialog] = useState(false)
  const [rotating, setRotating] = useState(false)

  // Debounce ref for name updates
  const nameTimeoutRef = useRef<ReturnType<typeof setTimeout> | null>(null)

  const clientMode = client.client_mode || "both"
  const template = client.template_id
    ? CLIENT_TEMPLATES.find(t => t.id === client.template_id) || null
    : null

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

  const handleModeChange = async (mode: ClientMode) => {
    try {
      await invoke("set_client_mode", {
        clientId: client.client_id,
        mode,
      } satisfies SetClientModeParams)
      toast.success("Client mode updated")
      onUpdate()
    } catch (error) {
      console.error("Failed to update client mode:", error)
      toast.error("Failed to update client mode")
    }
  }

  const handleDetachTemplate = async () => {
    try {
      await invoke("set_client_template", {
        clientId: client.client_id,
        templateId: null,
      } satisfies SetClientTemplateParams)
      toast.success("Client detached from template — all modes now available")
      onUpdate()
    } catch (error) {
      console.error("Failed to detach template:", error)
      toast.error("Failed to detach template")
    }
  }

  // Check if a mode is allowed by the template (or allow all if no template)
  const isModeAllowed = (mode: ClientMode): boolean => {
    if (!template) return true
    if (mode === "both") return template.supportsLlm && template.supportsMcp
    if (mode === "llm_only") return template.supportsLlm
    if (mode === "mcp_only") return template.supportsMcp
    return true
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

      {/* Template Info */}
      {template && (
        <Card>
          <CardHeader>
            <CardTitle>Client Template</CardTitle>
            <CardDescription>
              This client was created from a template
            </CardDescription>
          </CardHeader>
          <CardContent>
            <div className="flex items-center justify-between">
              <div className="flex items-center gap-3">
                <ServiceIcon service={template.id} size={24} />
                <div>
                  <p className="text-sm font-medium">{template.name}</p>
                  <p className="text-xs text-muted-foreground">{template.description}</p>
                </div>
              </div>
              <Button
                variant="outline"
                size="sm"
                onClick={handleDetachTemplate}
                title="Detach from template to unlock all mode options"
              >
                <Unlink className="h-4 w-4 mr-2" />
                Detach
              </Button>
            </div>
            {(!template.supportsMcp || !template.supportsLlm) && (
              <p className="text-xs text-muted-foreground mt-3">
                {template.name} supports {template.supportsLlm ? "LLM routing" : ""}{template.supportsLlm && template.supportsMcp ? " and " : ""}{template.supportsMcp ? "MCP proxy" : ""}.
                Detach to unlock all modes.
              </p>
            )}
          </CardContent>
        </Card>
      )}

      {/* Client Mode */}
      <Card>
        <CardHeader>
          <CardTitle>Client Mode</CardTitle>
          <CardDescription>
            Controls which features are available to this client
          </CardDescription>
        </CardHeader>
        <CardContent className="space-y-3">
          <div className="grid gap-2">
            {MODE_OPTIONS.map((option) => {
              const allowed = isModeAllowed(option.value)
              return (
                <label
                  key={option.value}
                  className={`flex items-start gap-3 p-3 rounded-lg border transition-colors
                    ${!allowed ? "opacity-50 cursor-not-allowed" : "cursor-pointer"}
                    ${clientMode === option.value ? "border-primary bg-accent" : allowed ? "border-muted hover:border-primary/50" : "border-muted"}`}
                >
                  <input
                    type="radio"
                    name="client-mode"
                    value={option.value}
                    checked={clientMode === option.value}
                    disabled={!allowed}
                    onChange={() => handleModeChange(option.value)}
                    className="mt-1"
                  />
                  <div>
                    <p className="text-sm font-medium">{option.label}</p>
                    <p className="text-xs text-muted-foreground">
                      {option.description}
                      {!allowed && template && " (not supported by " + template.name + ")"}
                    </p>
                  </div>
                </label>
              )
            })}
          </div>
        </CardContent>
      </Card>

      {/* GuardRails Override */}
      <Card>
        <CardHeader>
          <CardTitle>GuardRails</CardTitle>
          <CardDescription>
            Override the global guardrails setting for this client
          </CardDescription>
        </CardHeader>
        <CardContent>
          <Select
            value={client.guardrails_enabled === true ? "enabled" : client.guardrails_enabled === false ? "disabled" : "global"}
            onValueChange={async (value) => {
              try {
                const enabled = value === "global" ? null : value === "enabled"
                await invoke("set_client_guardrails_enabled", {
                  clientId: client.id,
                  enabled,
                })
                onUpdate()
                toast.success("GuardRails setting updated")
              } catch (err) {
                console.error("Failed to update guardrails setting:", err)
                toast.error("Failed to update GuardRails setting")
              }
            }}
          >
            <SelectTrigger className="w-48">
              <SelectValue />
            </SelectTrigger>
            <SelectContent>
              <SelectItem value="global">Use Global Setting</SelectItem>
              <SelectItem value="enabled">Enabled</SelectItem>
              <SelectItem value="disabled">Disabled</SelectItem>
            </SelectContent>
          </Select>
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
