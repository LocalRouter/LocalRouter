/**
 * Step 4: View Credentials
 *
 * Display the newly created credentials with connection instructions.
 */

import { HowToConnect } from "@/components/client/HowToConnect"

interface StepCredentialsProps {
  clientId: string
  clientUuid: string
  secret: string | null
}

export function StepCredentials({ clientId, clientUuid, secret }: StepCredentialsProps) {
  return (
    <HowToConnect
      clientId={clientId}
      clientUuid={clientUuid}
      secret={secret}
      showRotateCredentials={false}
      className="border-0 shadow-none"
    />
  )
}
