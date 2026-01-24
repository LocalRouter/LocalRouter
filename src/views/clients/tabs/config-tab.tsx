
import { useState, useEffect, useRef, useCallback } from "react"
import { invoke } from "@tauri-apps/api/core"
import { toast } from "sonner"
import { Copy, Check, Eye, EyeOff, RefreshCw, Key, Shield, Loader2 } from "lucide-react"
import { Card, CardContent, CardHeader, CardTitle, CardDescription } from "@/components/ui/Card"
import { Button } from "@/components/ui/Button"
import { Input } from "@/components/ui/Input"
import { Label } from "@/components/ui/label"
import { Tabs, TabsContent, TabsList, TabsTrigger } from "@/components/ui/tabs"
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
  enabled: boolean
  strategy_id: string
  allowed_llm_providers: string[]
  mcp_access_mode: "none" | "all" | "specific"
  mcp_servers: string[]
}

interface ConfigTabProps {
  client: Client
  onUpdate: () => void
}

export function ClientConfigTab({ client, onUpdate }: ConfigTabProps) {
  const [name, setName] = useState(client.name)
  const [saving, setSaving] = useState(false)

  // Credentials state
  const [secret, setSecret] = useState<string | null>(null)
  const [loadingSecret, setLoadingSecret] = useState(true)
  const [showSecret, setShowSecret] = useState(false)
  const [copiedApiKey, setCopiedApiKey] = useState(false)
  const [copiedClientId, setCopiedClientId] = useState(false)
  const [copiedClientSecret, setCopiedClientSecret] = useState(false)
  const [rotating, setRotating] = useState(false)

  // Debounce ref for name updates
  const nameTimeoutRef = useRef<ReturnType<typeof setTimeout> | null>(null)

  // Sync name state when client prop updates
  useEffect(() => {
    setName(client.name)
  }, [client.name])

  // Fetch the secret from keychain when component mounts or client changes
  useEffect(() => {
    const fetchSecret = async () => {
      setLoadingSecret(true)
      try {
        const value = await invoke<string>("get_client_value", { id: client.id })
        setSecret(value)
      } catch (error) {
        console.error("Failed to fetch client secret:", error)
        setSecret(null)
      } finally {
        setLoadingSecret(false)
      }
    }
    fetchSecret()
  }, [client.id])

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
    }, 500) // 500ms debounce
  }, [client.name, client.client_id, onUpdate])

  const handleCopyApiKey = async () => {
    if (!secret) return
    try {
      await navigator.clipboard.writeText(secret)
      setCopiedApiKey(true)
      setTimeout(() => setCopiedApiKey(false), 2000)
      toast.success("API key copied to clipboard")
    } catch {
      toast.error("Failed to copy API key")
    }
  }

  const handleCopyClientId = async () => {
    try {
      await navigator.clipboard.writeText(client.id)
      setCopiedClientId(true)
      setTimeout(() => setCopiedClientId(false), 2000)
      toast.success("Client ID copied to clipboard")
    } catch {
      toast.error("Failed to copy Client ID")
    }
  }

  const handleCopyClientSecret = async () => {
    if (!secret) return
    try {
      await navigator.clipboard.writeText(secret)
      setCopiedClientSecret(true)
      setTimeout(() => setCopiedClientSecret(false), 2000)
      toast.success("Client secret copied to clipboard")
    } catch {
      toast.error("Failed to copy client secret")
    }
  }

  const handleRotateKey = async () => {
    try {
      setRotating(true)
      await invoke("rotate_client_secret", { clientId: client.id })
      // Refetch the new secret after rotation
      const newSecret = await invoke<string>("get_client_value", { id: client.id })
      setSecret(newSecret)
      toast.success("Credentials rotated successfully")
      onUpdate()
    } catch (error) {
      console.error("Failed to rotate credentials:", error)
      toast.error("Failed to rotate credentials")
    } finally {
      setRotating(false)
    }
  }

  const maskedSecret = "••••••••••••••••••••••••••••••••"
  const displaySecret = loadingSecret ? "Loading..." : (secret || "Error loading secret")

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

      {/* Credentials */}
      <Card>
        <CardHeader>
          <div className="flex items-center justify-between">
            <div>
              <CardTitle>Credentials</CardTitle>
              <CardDescription>
                Authentication credentials for this client
              </CardDescription>
            </div>
            <AlertDialog>
              <AlertDialogTrigger asChild>
                <Button variant="destructive" size="sm" disabled={rotating}>
                  <RefreshCw className={`h-4 w-4 mr-2 ${rotating ? "animate-spin" : ""}`} />
                  Rotate
                </Button>
              </AlertDialogTrigger>
              <AlertDialogContent>
                <AlertDialogHeader>
                  <AlertDialogTitle>Rotate Credentials?</AlertDialogTitle>
                  <AlertDialogDescription>
                    This will generate new credentials and invalidate the current ones.
                    Any applications using the current API key or client secret will stop
                    working immediately.
                  </AlertDialogDescription>
                </AlertDialogHeader>
                <AlertDialogFooter>
                  <AlertDialogCancel>Cancel</AlertDialogCancel>
                  <AlertDialogAction
                    onClick={handleRotateKey}
                    className="bg-destructive text-destructive-foreground hover:bg-destructive/90"
                  >
                    Rotate Credentials
                  </AlertDialogAction>
                </AlertDialogFooter>
              </AlertDialogContent>
            </AlertDialog>
          </div>
        </CardHeader>
        <CardContent>
          <Tabs defaultValue="api-key">
            <TabsList className="mb-4">
              <TabsTrigger value="api-key" className="gap-2">
                <Key className="h-4 w-4" />
                API Key
              </TabsTrigger>
              <TabsTrigger value="oauth" className="gap-2">
                <Shield className="h-4 w-4" />
                OAuth 2.0
              </TabsTrigger>
            </TabsList>

            <TabsContent value="api-key" className="space-y-4">
              <div className="space-y-2">
                <Label>API Key</Label>
                <div className="flex items-center gap-2">
                  <code className="flex-1 p-3 text-sm bg-muted rounded-md font-mono break-all">
                    {loadingSecret ? (
                      <span className="flex items-center gap-2 text-muted-foreground">
                        <Loader2 className="h-4 w-4 animate-spin" />
                        Loading...
                      </span>
                    ) : showSecret ? displaySecret : maskedSecret}
                  </code>
                  <Button
                    variant="outline"
                    size="icon"
                    onClick={() => setShowSecret(!showSecret)}
                    title={showSecret ? "Hide" : "Show"}
                    disabled={loadingSecret || !secret}
                  >
                    {showSecret ? (
                      <EyeOff className="h-4 w-4" />
                    ) : (
                      <Eye className="h-4 w-4" />
                    )}
                  </Button>
                  <Button
                    variant="outline"
                    size="icon"
                    onClick={handleCopyApiKey}
                    title="Copy to clipboard"
                    disabled={loadingSecret || !secret}
                  >
                    {copiedApiKey ? (
                      <Check className="h-4 w-4 text-green-500" />
                    ) : (
                      <Copy className="h-4 w-4" />
                    )}
                  </Button>
                </div>
              </div>
              <div className="rounded-lg bg-muted/50 p-4 space-y-2">
                <p className="text-sm font-medium">Usage</p>
                <p className="text-xs text-muted-foreground">
                  Include the API key in your request headers:
                </p>
                <code className="block text-xs bg-muted p-2 rounded">
                  Authorization: Bearer {'<api_key>'}
                </code>
              </div>
            </TabsContent>

            <TabsContent value="oauth" className="space-y-4">
              <div className="space-y-2">
                <Label>Client ID</Label>
                <div className="flex items-center gap-2">
                  <code className="flex-1 p-3 text-sm bg-muted rounded-md font-mono break-all">
                    {client.id}
                  </code>
                  <Button
                    variant="outline"
                    size="icon"
                    onClick={handleCopyClientId}
                    title="Copy to clipboard"
                  >
                    {copiedClientId ? (
                      <Check className="h-4 w-4 text-green-500" />
                    ) : (
                      <Copy className="h-4 w-4" />
                    )}
                  </Button>
                </div>
              </div>

              <div className="space-y-2">
                <Label>Client Secret</Label>
                <div className="flex items-center gap-2">
                  <code className="flex-1 p-3 text-sm bg-muted rounded-md font-mono break-all">
                    {loadingSecret ? (
                      <span className="flex items-center gap-2 text-muted-foreground">
                        <Loader2 className="h-4 w-4 animate-spin" />
                        Loading...
                      </span>
                    ) : showSecret ? displaySecret : maskedSecret}
                  </code>
                  <Button
                    variant="outline"
                    size="icon"
                    onClick={() => setShowSecret(!showSecret)}
                    title={showSecret ? "Hide" : "Show"}
                    disabled={loadingSecret || !secret}
                  >
                    {showSecret ? (
                      <EyeOff className="h-4 w-4" />
                    ) : (
                      <Eye className="h-4 w-4" />
                    )}
                  </Button>
                  <Button
                    variant="outline"
                    size="icon"
                    onClick={handleCopyClientSecret}
                    title="Copy to clipboard"
                    disabled={loadingSecret || !secret}
                  >
                    {copiedClientSecret ? (
                      <Check className="h-4 w-4 text-green-500" />
                    ) : (
                      <Copy className="h-4 w-4" />
                    )}
                  </Button>
                </div>
              </div>

              <div className="rounded-lg bg-muted/50 p-4 space-y-2">
                <p className="text-sm font-medium">Usage</p>
                <p className="text-xs text-muted-foreground">
                  Exchange credentials for an access token:
                </p>
                <code className="block text-xs bg-muted p-2 rounded whitespace-pre">{`POST /oauth/token
Content-Type: application/x-www-form-urlencoded

grant_type=client_credentials
&client_id=<client_id>
&client_secret=<client_secret>`}</code>
              </div>
            </TabsContent>
          </Tabs>
        </CardContent>
      </Card>
    </div>
  )
}
