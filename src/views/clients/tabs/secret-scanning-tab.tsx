import { useState, useEffect, useCallback, useRef } from "react"
import { invoke } from "@tauri-apps/api/core"
import { toast } from "sonner"
import { BellOff, Trash2 } from "lucide-react"
import { FEATURES } from "@/constants/features"
import { Card, CardHeader, CardTitle, CardDescription, CardContent } from "@/components/ui/Card"
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
  AlertDialogTrigger,
} from "@/components/ui/alert-dialog"
import { cn } from "@/lib/utils"
import type {
  ClientSecretScanningConfig,
  SecretScanningConfig,
  SecretScanAction,
  DismissedSecret,
} from "@/types/tauri-commands"

interface Client {
  id: string
  name: string
  client_id: string
}

interface SecretScanningTabProps {
  client: Client
  onUpdate: () => void
  onViewChange?: (view: string, subTab?: string | null) => void
}

const ACTION_LABELS: Record<SecretScanAction, string> = {
  ask: "Ask",
  notify: "Notify",
  off: "Off",
}

type ButtonValue = "default" | SecretScanAction

const BUTTON_STYLES: Record<ButtonValue, string> = {
  default: "bg-zinc-500 text-white",
  ask: "bg-amber-500 text-white",
  notify: "bg-blue-500 text-white",
  off: "bg-red-500 text-white",
}

export function ClientSecretScanningTab({ client, onUpdate, onViewChange }: SecretScanningTabProps) {
  const [config, setConfig] = useState<ClientSecretScanningConfig>({ action: null })
  const [globalConfig, setGlobalConfig] = useState<SecretScanningConfig | null>(null)
  const [dismissed, setDismissed] = useState<DismissedSecret[]>([])
  const [loading, setLoading] = useState(true)

  const loadReqIdRef = useRef(0)

  const loadConfig = useCallback(async () => {
    const reqId = ++loadReqIdRef.current
    try {
      const [clientConfig, global, exceptions] = await Promise.all([
        invoke<ClientSecretScanningConfig>("get_client_secret_scanning_config", {
          clientId: client.id,
        } as Record<string, unknown>),
        invoke<SecretScanningConfig>("get_secret_scanning_config"),
        invoke<DismissedSecret[]>("list_client_dismissed_secrets", {
          clientId: client.id,
        } as Record<string, unknown>),
      ])
      if (loadReqIdRef.current !== reqId) return
      setConfig(clientConfig)
      setGlobalConfig(global)
      setDismissed(exceptions)
    } catch (err) {
      if (loadReqIdRef.current !== reqId) return
      console.error("Failed to load secret scanning config:", err)
      toast.error("Failed to load secret scanning configuration")
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

  const saveConfig = async (newConfig: ClientSecretScanningConfig) => {
    setConfig(newConfig)
    try {
      await invoke("update_client_secret_scanning_config", {
        clientId: client.id,
        configJson: JSON.stringify(newConfig),
      } as Record<string, unknown>)
      onUpdate()
    } catch (err) {
      toast.error("Failed to save secret scanning configuration")
      loadConfig()
    }
  }

  /** Start flagging a previously-ignored value again (entryId omitted = all). */
  const removeException = async (entryId?: string) => {
    try {
      await invoke("remove_client_dismissed_secret", {
        clientId: client.id,
        entryId: entryId ?? null,
      } as Record<string, unknown>)
      setDismissed((prev) => (entryId ? prev.filter((d) => d.id !== entryId) : []))
      toast.success(
        entryId
          ? "This value will be flagged again"
          : "All ignored values will be flagged again"
      )
      onUpdate()
    } catch (err) {
      toast.error("Failed to remove the exception")
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

  const globalActionLabel = globalConfig ? ACTION_LABELS[globalConfig.action] : "Off"
  const current: ButtonValue = config.action ?? "default"

  const buttons: { key: ButtonValue; label: string }[] = [
    { key: "default", label: `Default (${globalActionLabel})` },
    { key: "ask", label: "Ask" },
    { key: "notify", label: "Notify" },
    { key: "off", label: "Off" },
  ]

  return (
    <div className="space-y-4">
      <Card>
        <CardHeader>
          <div className="flex items-center justify-between">
            <div className="flex items-center gap-2">
              <FEATURES.secretScanning.icon className={`h-5 w-5 ${FEATURES.secretScanning.color}`} />
              <CardTitle>Secret Scanning</CardTitle>
            </div>
            <div className="inline-flex rounded-md border border-border bg-muted/50">
              {buttons.map(({ key, label }, i) => {
                const isActive = current === key
                return (
                  <button
                    key={key}
                    type="button"
                    onClick={() => saveConfig({ action: key === "default" ? null : key })}
                    className={cn(
                      "px-2 py-0.5 text-xs transition-colors font-medium",
                      isActive
                        ? BUTTON_STYLES[key]
                        : "text-muted-foreground hover:text-foreground hover:bg-muted",
                      i === 0 && "rounded-l-md",
                      i === buttons.length - 1 && "rounded-r-md"
                    )}
                  >
                    {label}
                  </button>
                )
              })}
            </div>
          </div>
          <CardDescription>
            Detect potential secrets (API keys, tokens, passwords) in outbound requests
            before they reach the provider. Configure global scanning rules in{" "}
            {onViewChange ? (
              <button
                className="text-blue-500 hover:underline"
                onClick={() => onViewChange("secret-scanning")}
              >
                Secret Scanning settings
              </button>
            ) : (
              "Secret Scanning settings"
            )}.
          </CardDescription>
        </CardHeader>
      </Card>

      {dismissed.length > 0 && (
        <Card>
          <CardHeader>
            <div className="flex items-center justify-between">
              <div className="flex items-center gap-2">
                <BellOff className="h-5 w-5 text-muted-foreground" />
                <CardTitle>Ignored Values ({dismissed.length})</CardTitle>
              </div>
              <AlertDialog>
                <AlertDialogTrigger asChild>
                  <Button variant="outline" size="sm">
                    Reset All
                  </Button>
                </AlertDialogTrigger>
                <AlertDialogContent>
                  <AlertDialogHeader>
                    <AlertDialogTitle>Flag all these values again?</AlertDialogTitle>
                    <AlertDialogDescription>
                      {client.name} will be checked against every one of these{" "}
                      {dismissed.length} values again. Because only a hash was stored,
                      the exceptions cannot be restored afterwards — each value has to
                      be ignored again from its popup.
                    </AlertDialogDescription>
                  </AlertDialogHeader>
                  <AlertDialogFooter>
                    <AlertDialogCancel>Cancel</AlertDialogCancel>
                    <AlertDialogAction onClick={() => removeException()}>
                      Reset All
                    </AlertDialogAction>
                  </AlertDialogFooter>
                </AlertDialogContent>
              </AlertDialog>
            </div>
            <CardDescription>
              Values you chose never to flag again for {client.name}. Only a salted
              hash of each is stored — never the value itself, which is why they are
              shown masked here. Removing one starts flagging it again immediately.
            </CardDescription>
          </CardHeader>
          <CardContent className="space-y-2">
            {dismissed.map((entry) => (
              <div
                key={entry.id}
                className="flex items-center gap-3 rounded border border-border bg-muted/30 px-3 py-2"
              >
                <div className="min-w-0 flex-1">
                  <div className="flex items-center gap-2">
                    <span className="text-sm font-medium truncate">
                      {entry.rule_description}
                    </span>
                    <span className="font-mono text-[10px] text-muted-foreground truncate">
                      {entry.rule_id}
                    </span>
                  </div>
                  <div className="flex items-center gap-3 text-xs text-muted-foreground">
                    <code className="font-mono truncate">{entry.hint}</code>
                    <span className="ml-auto flex-shrink-0">
                      {new Date(entry.dismissed_at).toLocaleDateString()}
                    </span>
                  </div>
                </div>
                <Button
                  variant="ghost"
                  size="sm"
                  className="flex-shrink-0 text-muted-foreground hover:text-destructive"
                  onClick={() => removeException(entry.id)}
                  title="Flag this value again"
                  aria-label="Flag this value again"
                >
                  <Trash2 className="h-4 w-4" />
                </Button>
              </div>
            ))}
          </CardContent>
        </Card>
      )}
    </div>
  )
}
