import { Tabs, TabsList, TabsTrigger, TabsContent } from "@/components/ui/tabs"
import { SecretScanEvent } from "./SecretScanEvent"
import { JwtContent } from "./JwtContent"
import type { DecodedSecretJwt, SecretFindingSummary } from "@/types/tauri-commands"

export function SecretFindingDetails({ finding, requestJson, revealed, loadJwt }: { finding: SecretFindingSummary; requestJson?: string | null; revealed?: string; loadJwt?: () => Promise<DecodedSecretJwt | null> }) {
  return <Tabs defaultValue="event" className="flex min-h-0 flex-1 flex-col">
    <TabsList className="h-8 w-fit shrink-0">
      <TabsTrigger className="text-xs" value="event">Event</TabsTrigger>
      {finding.rule_id === 'jwt-token' && <TabsTrigger className="text-xs" value="jwt">JWT</TabsTrigger>}
    </TabsList>
    <TabsContent value="event" className="min-h-0 flex-1">
      <SecretScanEvent finding={finding} requestJson={requestJson} revealed={revealed} />
    </TabsContent>
    <TabsContent value="jwt" className="min-h-0 flex-1 overflow-auto pr-1">
      <JwtContent load={loadJwt ?? (async () => null)} />
    </TabsContent>
  </Tabs>
}
