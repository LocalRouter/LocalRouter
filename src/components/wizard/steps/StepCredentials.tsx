/**
 * Step 4: View Credentials
 *
 * Display the newly created credentials with connection instructions.
 */

import { HowToConnect } from "@/components/client/HowToConnect"
import type { ClientMode } from "@/types/tauri-commands"

interface StepCredentialsProps {
  clientId: string
  clientUuid: string
  secret: string | null
  templateId?: string | null
  clientMode?: ClientMode
}

export function StepCredentials({ clientId, clientUuid, secret, templateId, clientMode }: StepCredentialsProps) {
  return (
    <HowToConnect
      clientId={clientId}
      clientUuid={clientUuid}
      secret={secret}
      showRotateCredentials={false}
      templateId={templateId}
      clientMode={clientMode}
      className="border-0 shadow-none"
    />
  )
}
