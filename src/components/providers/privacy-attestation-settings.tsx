import { invoke } from '@tauri-apps/api/core'
import { useEffect, useState } from 'react'
import {
  Card,
  CardContent,
  CardDescription,
  CardHeader,
  CardTitle,
} from '@/components/ui/Card'
import { Button } from '@/components/ui/Button'
import { Input } from '@/components/ui/Input'
import { Checkbox } from '@/components/ui/checkbox'
import { Label } from '@/components/ui/label'
import type {
  ProviderPrivacyAttestation,
  AttestProviderPrivacyParams,
} from '@/types/tauri-commands'

export const PRIVACY_ATTESTATION_KEY = 'localrouter_privacy_attestation'
export function ProviderPrivacySettings({
  value,
  onChange,
  disabled,
  instanceName,
}: {
  value: string
  instanceName: string
  onChange: (value: string) => void
  disabled?: boolean
}) {
  const [local, setLocal] = useState(false)
  const [retention, setRetention] = useState(false)
  const [source, setSource] = useState('')
  const [expiry, setExpiry] = useState('')
  const [reviewed, setReviewed] = useState<number | null>(null)
  const [error, setError] = useState<string | null>(null)
  useEffect(() => {
    try {
      const data: ProviderPrivacyAttestation | null = value
        ? JSON.parse(value).attestation
        : null
      setLocal(data?.local_execution ?? false)
      setRetention(data?.no_retention ?? false)
      setSource(data?.source ?? '')
      setExpiry(
        data?.expires_at
          ? new Date(data.expires_at * 1000).toISOString().slice(0, 16)
          : '',
      )
      setReviewed(data?.reviewed_at ?? null)
      setError(null)
    } catch {
      setError(
        'Saved attestation is invalid; replace it with a reviewed statement.',
      )
    }
  }, [value])
  async function save() {
    if (!source.trim()) {
      setError('Add a policy or configuration reference.')
      return
    }
    const expirySeconds = expiry
      ? Math.floor(Date.parse(`${expiry}:00Z`) / 1000)
      : null
    const now = Math.floor(Date.now() / 1000)
    if (
      expirySeconds !== null &&
      (!Number.isFinite(expirySeconds) || expirySeconds <= now)
    ) {
      setError('Choose a future expiry in UTC.')
      return
    }
    setError(null)
    try {
      const sealed = await invoke<string>('attest_provider_privacy', {
        instanceName,
        localExecution: local,
        noRetention: retention,
        source: source.trim(),
        expiresAt: expirySeconds,
      } satisfies AttestProviderPrivacyParams)
      onChange(sealed)
    } catch (error) {
      setError(String(error))
    }
  }
  return (
    <Card>
      <CardHeader>
        <CardTitle>Privacy evidence</CardTitle>
        <CardDescription>
          Record a reviewed account or local-service policy. User attestations
          are stored as declarations with provenance. Changes to credentials,
          endpoint, settings or application version require a new review.
        </CardDescription>
      </CardHeader>
      <CardContent className="space-y-4">
        <label className="flex items-center gap-3 text-sm">
          <Checkbox
            checked={local}
            onCheckedChange={(value) => setLocal(value === true)}
            disabled={disabled}
          />
          I reviewed local execution for this service
        </label>
        <p className="text-xs text-muted-foreground">
          Local execution also requires a configured loopback endpoint. Managed
          embedded engines already provide local-process evidence.
        </p>
        <label className="flex items-center gap-3 text-sm">
          <Checkbox
            checked={retention}
            onCheckedChange={(value) => setRetention(value === true)}
            disabled={disabled}
          />
          I reviewed no-retention settings for this account and inference chain
        </label>
        <div className="space-y-2">
          <Label htmlFor="privacy-evidence-source">
            Policy or configuration reference
          </Label>
          <Input
            id="privacy-evidence-source"
            value={source}
            onChange={(event) => setSource(event.target.value)}
            disabled={disabled}
            maxLength={1024}
          />
          <p className="text-xs text-muted-foreground">
            Use a policy/configuration reference. Keep credentials and request
            content out of this field.
          </p>
        </div>
        <div className="space-y-2">
          <Label htmlFor="privacy-evidence-expiry">
            Evidence expiry (UTC, optional)
          </Label>
          <Input
            id="privacy-evidence-expiry"
            type="datetime-local"
            value={expiry}
            onChange={(event) => setExpiry(event.target.value)}
            disabled={disabled}
          />
        </div>
        {reviewed !== null && (
          <p className="text-xs text-muted-foreground">
            Last reviewed: {new Date(reviewed * 1000).toLocaleString()}
          </p>
        )}
        {error && (
          <p role="alert" className="text-sm text-destructive">
            {error}
          </p>
        )}
        <div className="flex gap-2">
          <Button disabled={disabled} onClick={() => void save()}>
            Save reviewed attestation
          </Button>
          <Button
            variant="outline"
            disabled={disabled || !value}
            onClick={() => onChange('')}
          >
            Remove attestation
          </Button>
        </div>
      </CardContent>
    </Card>
  )
}
