/**
 * CredentialsDisplay Component
 *
 * Displays client credentials (API key and OAuth) with copy functionality.
 * Used in both client detail view and creation wizard.
 */

import { useState } from "react"
import { toast } from "sonner"
import { Copy, Check, Eye, EyeOff, Key, Shield, Loader2 } from "lucide-react"
import { Button } from "@/components/ui/Button"
import { Label } from "@/components/ui/label"
import { Tabs, TabsContent, TabsList, TabsTrigger } from "@/components/ui/tabs"
import { cn } from "@/lib/utils"

interface CredentialsDisplayProps {
  clientId: string
  secret: string | null
  loadingSecret?: boolean
  className?: string
}

export function CredentialsDisplay({
  clientId,
  secret,
  loadingSecret = false,
  className,
}: CredentialsDisplayProps) {
  const [showSecret, setShowSecret] = useState(false)
  const [copiedApiKey, setCopiedApiKey] = useState(false)
  const [copiedClientId, setCopiedClientId] = useState(false)
  const [copiedClientSecret, setCopiedClientSecret] = useState(false)

  const maskedSecret = "••••••••••••••••••••••••••••••••"
  const displaySecret = loadingSecret ? "Loading..." : (secret || "Error loading secret")

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
      await navigator.clipboard.writeText(clientId)
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

  return (
    <div className={cn("space-y-4", className)}>
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
                {clientId}
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
    </div>
  )
}
