import { useState, useEffect } from "react"
import { invoke } from "@tauri-apps/api/core"
import { toast } from "sonner"
import { Copy, Check, Eye, EyeOff, RefreshCw, Key, Shield, Info, Loader2 } from "lucide-react"
import { Card, CardContent, CardHeader, CardTitle, CardDescription } from "@/components/ui/Card"
import { Button } from "@/components/ui/Button"
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
}

interface AuthTabProps {
  client: Client
  onUpdate: () => void
}

export function ClientAuthTab({ client, onUpdate }: AuthTabProps) {
  const [secret, setSecret] = useState<string | null>(null)
  const [loadingSecret, setLoadingSecret] = useState(true)
  const [showSecret, setShowSecret] = useState(false)
  const [copiedApiKey, setCopiedApiKey] = useState(false)
  const [copiedClientId, setCopiedClientId] = useState(false)
  const [copiedClientSecret, setCopiedClientSecret] = useState(false)
  const [rotating, setRotating] = useState(false)

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
      {/* Authentication Overview */}
      <Card>
        <CardHeader>
          <CardTitle className="flex items-center gap-2">
            <Info className="h-5 w-5" />
            Authentication
          </CardTitle>
          <CardDescription>
            Choose how your application authenticates with LocalRouter
          </CardDescription>
        </CardHeader>
        <CardContent className="space-y-4">
          <p className="text-sm text-muted-foreground">
            LocalRouter supports two authentication methods. Both use the same underlying credentials,
            so you can choose whichever fits your application best:
          </p>
          <div className="grid gap-4 md:grid-cols-2">
            <div className="rounded-lg border p-4">
              <div className="flex items-center gap-2 font-medium mb-2">
                <Key className="h-4 w-4" />
                API Key
              </div>
              <p className="text-sm text-muted-foreground">
                Simple bearer token authentication. Include the API key in your request headers.
                Best for scripts, CLI tools, and simple integrations.
              </p>
            </div>
            <div className="rounded-lg border p-4">
              <div className="flex items-center gap-2 font-medium mb-2">
                <Shield className="h-4 w-4" />
                OAuth 2.0
              </div>
              <p className="text-sm text-muted-foreground">
                Client credentials flow for token-based auth. Exchange credentials for short-lived
                access tokens. Best for production applications and services.
              </p>
            </div>
          </div>
        </CardContent>
      </Card>

      {/* Credentials Tabs */}
      <Card>
        <CardHeader>
          <CardTitle>Credentials</CardTitle>
          <CardDescription>
            Your authentication credentials for this client
          </CardDescription>
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

      {/* Key Management */}
      <Card>
        <CardHeader>
          <CardTitle className="flex items-center gap-2">
            <RefreshCw className="h-5 w-5" />
            Key Management
          </CardTitle>
          <CardDescription>
            Rotate credentials if they may have been compromised
          </CardDescription>
        </CardHeader>
        <CardContent>
          <div className="flex items-center justify-between">
            <div className="space-y-1">
              <p className="text-sm font-medium">Rotate Credentials</p>
              <p className="text-xs text-muted-foreground">
                Generates a new API key and client secret. The current credentials will be
                immediately invalidated.
              </p>
            </div>
            <AlertDialog>
              <AlertDialogTrigger asChild>
                <Button variant="destructive" disabled={rotating}>
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
        </CardContent>
      </Card>
    </div>
  )
}
