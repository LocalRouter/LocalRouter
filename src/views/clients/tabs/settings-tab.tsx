
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
import { ClientModeSelector } from "@/components/client/ClientModeSelector"
import { PermissionStateButton } from "@/components/permissions/PermissionStateButton"
import ServiceIcon from "@/components/ServiceIcon"
import type { PermissionState } from "@/components/permissions"
import type { ClientMode, SetClientModeParams, SetClientTemplateParams, SetClientSamplingPermissionParams, SetClientElicitationPermissionParams } from "@/types/tauri-commands"

interface Client {
  id: string
  name: string
  client_id: string
  enabled: boolean
  strategy_id: string
  client_mode?: ClientMode
  template_id?: string | null
  mcp_sampling_permission?: PermissionState
  mcp_elicitation_permission?: PermissionState
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

  const handleSamplingPermissionChange = async (state: PermissionState) => {
    try {
      await invoke("set_client_sampling_permission", {
        clientId: client.client_id,
        state,
      } satisfies SetClientSamplingPermissionParams)
      toast.success("Sampling permission updated")
      onUpdate()
    } catch (error) {
      console.error("Failed to update sampling permission:", error)
      toast.error("Failed to update sampling permission")
    }
  }

  const handleElicitationPermissionChange = async (state: PermissionState) => {
    try {
      await invoke("set_client_elicitation_permission", {
        clientId: client.client_id,
        state,
      } satisfies SetClientElicitationPermissionParams)
      toast.success("Elicitation permission updated")
      onUpdate()
    } catch (error) {
      console.error("Failed to update elicitation permission:", error)
      toast.error("Failed to update elicitation permission")
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
        <CardContent>
          <ClientModeSelector mode={clientMode} onModeChange={handleModeChange} template={template} />
        </CardContent>
      </Card>

      {/* MCP Capabilities - visible when mode uses MCP */}
      {clientMode !== "llm_only" && (
        <Card>
          <CardHeader>
            <CardTitle>MCP Capabilities</CardTitle>
            <CardDescription>
              Controls how backend MCP servers can request LLM completions and user input
            </CardDescription>
          </CardHeader>
          <CardContent className="space-y-4">
            {/* Sampling */}
            <div className="flex items-center justify-between">
              <div className="flex items-center gap-2">
                <div>
                  <p className="text-sm font-medium">Sampling</p>
                  <p className="text-xs text-muted-foreground">
                    {clientMode === "mcp_via_llm"
                      ? ({
                          allow: "Automatically route to LLM",
                          ask: "Show approval popup first",
                          off: "Reject sampling requests",
                        } as Record<string, string>)[client.mcp_sampling_permission || "ask"]
                      : ({
                          allow: "Forward to client",
                          ask: "Show approval popup, then forward",
                          off: "Reject sampling requests",
                        } as Record<string, string>)[client.mcp_sampling_permission || "ask"]
                    }
                  </p>
                </div>
                {(client.mcp_sampling_permission || "ask") === "ask" && (
                  <Button
                    variant="ghost"
                    size="sm"
                    className="text-xs h-6 px-2 text-muted-foreground"
                    onClick={async () => {
                      try {
                        await invoke("debug_trigger_sampling_approval_popup")
                      } catch (e) {
                        console.error("Failed to trigger sampling popup:", e)
                      }
                    }}
                  >
                    Test
                  </Button>
                )}
              </div>
              <PermissionStateButton
                value={client.mcp_sampling_permission || "ask"}
                onChange={handleSamplingPermissionChange}
                size="sm"
              />
            </div>

            {/* Elicitation */}
            <div className="flex items-center justify-between pt-3 border-t">
              <div className="flex items-center gap-2">
                <div>
                  <p className="text-sm font-medium">Elicitation</p>
                  <p className="text-xs text-muted-foreground">
                    {clientMode === "mcp_via_llm"
                      ? ({
                          ask: "Show form popup for user input",
                          off: "Reject elicitation requests",
                        } as Record<string, string>)[client.mcp_elicitation_permission === "allow" ? "ask" : (client.mcp_elicitation_permission || "ask")]
                      : ({
                          allow: "Forward to client",
                          ask: "Show form popup locally",
                          off: "Reject elicitation requests",
                        } as Record<string, string>)[client.mcp_elicitation_permission || "ask"]
                    }
                  </p>
                </div>
                {(() => {
                  const perm = client.mcp_elicitation_permission || "ask"
                  const effective = clientMode === "mcp_via_llm" && perm === "allow" ? "ask" : perm
                  return effective === "ask"
                })() ? (
                  <Button
                    variant="ghost"
                    size="sm"
                    className="text-xs h-6 px-2 text-muted-foreground"
                    onClick={async () => {
                      try {
                        await invoke("debug_trigger_elicitation_form_popup")
                      } catch (e) {
                        console.error("Failed to trigger elicitation popup:", e)
                      }
                    }}
                  >
                    Test
                  </Button>
                ) : null}
              </div>
              {clientMode === "mcp_via_llm" ? (
                // MCP via LLM: only Ask/Off (no client to passthrough to)
                <div className="inline-flex rounded-md border border-border bg-muted/50">
                  {(["ask", "off"] as PermissionState[]).map((state) => (
                    <button
                      key={state}
                      type="button"
                      onClick={() => handleElicitationPermissionChange(state)}
                      className={`px-2 py-0.5 text-xs font-medium transition-colors ${
                        (client.mcp_elicitation_permission === "allow" ? "ask" : (client.mcp_elicitation_permission || "ask")) === state
                          ? state === "ask" ? "bg-amber-500 text-white" : "bg-zinc-500 text-white"
                          : "text-muted-foreground hover:text-foreground hover:bg-muted"
                      } ${state === "ask" ? "rounded-l-md" : "rounded-r-md"}`}
                    >
                      {state === "ask" ? "Ask" : "Off"}
                    </button>
                  ))}
                </div>
              ) : (
                <PermissionStateButton
                  value={client.mcp_elicitation_permission || "ask"}
                  onChange={handleElicitationPermissionChange}
                  size="sm"
                />
              )}
            </div>
          </CardContent>
        </Card>
      )}

      {/* Danger Zone */}
      <Card className="border-red-200 dark:border-red-900">
        <CardHeader>
          <CardTitle className="text-red-600 dark:text-red-400">Danger Zone</CardTitle>
          <CardDescription>
            Irreversible and destructive actions for this client
          </CardDescription>
        </CardHeader>
        <CardContent className="space-y-4">
          <div className="flex items-center justify-between">
            <div>
              <p className="text-sm font-medium">Enable client</p>
              <p className="text-sm text-muted-foreground">
                When disabled, this client's API key will be rejected
              </p>
            </div>
            <Switch
              checked={client.enabled}
              onCheckedChange={handleToggleEnabled}
            />
          </div>
          <div className="flex items-center justify-between pt-4 border-t">
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
