/**
 * Step 4: View Credentials
 *
 * Display the newly created credentials with copy functionality.
 */

import { useState } from "react"
import { toast } from "sonner"
import { Copy, Check } from "lucide-react"
import { Button } from "@/components/ui/Button"
import { CredentialsDisplay } from "@/components/client/CredentialsDisplay"

interface StepCredentialsProps {
  clientId: string
  secret: string | null
}

export function StepCredentials({ clientId, secret }: StepCredentialsProps) {
  const [copied, setCopied] = useState(false)

  const handleCopyApiKey = async () => {
    if (!secret) return
    try {
      await navigator.clipboard.writeText(secret)
      setCopied(true)
      setTimeout(() => setCopied(false), 2000)
      toast.success("API key copied to clipboard")
    } catch {
      toast.error("Failed to copy API key")
    }
  }

  return (
    <div className="space-y-4">
      <CredentialsDisplay
        clientId={clientId}
        secret={secret}
      />

      <div className="flex justify-center pt-4">
        <Button
          size="lg"
          onClick={handleCopyApiKey}
          disabled={!secret}
          className="min-w-[200px]"
        >
          {copied ? (
            <>
              <Check className="mr-2 h-4 w-4" />
              Copied!
            </>
          ) : (
            <>
              <Copy className="mr-2 h-4 w-4" />
              Copy API Key
            </>
          )}
        </Button>
      </div>
    </div>
  )
}
