import { useEffect, useState } from 'react'
import { invoke } from '@tauri-apps/api/core'
import {
  Card,
  CardContent,
  CardDescription,
  CardHeader,
  CardTitle,
} from '@/components/ui/Card'
import { Button } from '@/components/ui/Button'
import { Checkbox } from '@/components/ui/checkbox'
import type {
  ClientPrivacyStatus,
  GetClientPrivacyParams,
  SetClientPrivacyParams,
} from '@/types/tauri-commands'

export function ClientPrivacyPanel({
  clientId,
  onUpdate,
}: {
  clientId: string
  onUpdate: () => void
}) {
  const [status, setStatus] = useState<ClientPrivacyStatus | null>(null)
  const [local, setLocal] = useState(false)
  const [retention, setRetention] = useState(false)
  const [metadata, setMetadata] = useState(false)
  const [busy, setBusy] = useState(false)
  const [error, setError] = useState<string | null>(null)
  useEffect(() => {
    let live = true
    setStatus(null)
    void invoke<ClientPrivacyStatus>('get_client_privacy', {
      clientId,
    } satisfies GetClientPrivacyParams)
      .then((value) => {
        if (!live) return
        setStatus(value)
        setLocal(value.local_only)
        setRetention(value.no_retention)
        setMetadata(value.metadata_only)
      })
      .catch((error) => {
        if (live) setError(String(error))
      })
    return () => {
      live = false
    }
  }, [clientId])
  async function save() {
    setBusy(true)
    setError(null)
    try {
      const value = await invoke<ClientPrivacyStatus>('set_client_privacy', {
        clientId,
        localOnly: local,
        noRetention: retention,
        metadataOnly: metadata,
      } satisfies SetClientPrivacyParams)
      setStatus(value)
      onUpdate()
    } catch (error) {
      setError(String(error))
    } finally {
      setBusy(false)
    }
  }
  const names = {
    managed_local_engine: 'Managed local engine',
    user_attestation: 'User attestation',
    unknown: 'Unknown',
  }
  return (
    <Card>
      <CardHeader>
        <CardTitle>Privacy profile</CardTitle>
        <CardDescription>
          Route requirements use configured evidence. Current permissions and
          endpoint support also apply.
        </CardDescription>
      </CardHeader>
      <CardContent className="space-y-4">
        <label className="flex items-start gap-3 text-sm">
          <Checkbox
            checked={local}
            onCheckedChange={(value) => setLocal(value === true)}
            disabled={busy || !status}
          />
          <span>
            Require local execution
            <span className="mt-1 block text-xs text-muted-foreground">
              Uses managed local engines or an explicit current attestation on a
              configured loopback endpoint.
            </span>
          </span>
        </label>
        <label className="flex items-start gap-3 text-sm">
          <Checkbox
            checked={retention}
            onCheckedChange={(value) => setRetention(value === true)}
            disabled={busy || !status}
          />
          <span>
            Require no retention
            <span className="mt-1 block text-xs text-muted-foreground">
              Requires reviewed storage/account/chain evidence even for local
              engines, enforces supported gateway ZDR filters, and disables new
              automatic payload storage, response caching and session reuse. New
              persistent files and batch jobs are rejected.
            </span>
          </span>
        </label>
        <label className="flex items-start gap-3 text-sm">
          <Checkbox
            checked={metadata || retention}
            onCheckedChange={(value) => setMetadata(value === true)}
            disabled={busy || !status || retention}
          />
          <span>
            Metadata-only monitoring
            <span className="mt-1 block text-xs text-muted-foreground">
              New monitor events omit bodies, previews, findings and raw errors.
              Usage and lifecycle metadata remain.
            </span>
          </span>
        </label>
        <p className="text-xs text-muted-foreground">
          Restricted profiles use native LLM gateway traffic. Raw proxy, MCP and
          unverified modality mappings and active direct safety-model scans are
          unavailable under these profiles. Existing captures, in-flight events
          and manually saved artifacts retain their earlier policy; use the
          existing Monitor clear and artifact cleanup controls deliberately.
        </p>
        {error && (
          <p role="alert" className="text-sm text-destructive">
            {error}
          </p>
        )}
        <Button disabled={busy || !status} onClick={() => void save()}>
          Save privacy policy
        </Button>
        {status && (
          <div className="space-y-2">
            <p className="text-xs font-medium">
              Evidence under the saved policy
            </p>
            {status.providers.map((provider) => (
              <div
                key={provider.instance_name}
                className="rounded-md border p-2 text-xs"
              >
                <p className="font-medium">
                  {provider.instance_name} ·{' '}
                  {!provider.enabled
                    ? 'disabled'
                    : provider.assessment.allowed
                      ? 'meets saved requirements'
                      : 'unverified for saved requirements'}
                </p>
                <p className="text-muted-foreground">
                  Local execution: {names[provider.assessment.local_execution]}{' '}
                  · no retention: {names[provider.assessment.no_retention]}
                </p>
                {provider.assessment.source && (
                  <p className="mt-1 break-all text-muted-foreground">
                    {provider.assessment.source}
                    {provider.assessment.reviewed_at !== null
                      ? ` · reviewed ${new Date(provider.assessment.reviewed_at * 1000).toLocaleString()}`
                      : ''}
                  </p>
                )}
                {provider.assessment.reason && (
                  <p className="mt-1 text-muted-foreground">
                    {provider.assessment.reason}
                  </p>
                )}
              </div>
            ))}
          </div>
        )}
      </CardContent>
    </Card>
  )
}
